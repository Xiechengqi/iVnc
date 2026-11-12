// Native Electron -> iVnc -> WebRTC. Isolated app, viewer, HOME and sockets.
// ELECTRON_BIN=/path/to/electron node scripts/electron_smoke.mjs
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-electron-'));
const binary = process.env.IVNC_BIN || path.join(root, 'target/debug/ivnc');
async function freePort() {
    const server = net.createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve)); return port;
}
const port = await freePort(), browserPort = await freePort(), remotePort = await freePort();
const base = `http://127.0.0.1:${port}`;
const env = { ...process.env, IVNC_HOME: path.join(work, 'ivnc'),
    XDG_CONFIG_HOME: path.join(work, 'config'), XDG_CACHE_HOME: path.join(work, 'cache'), XDG_DATA_HOME: path.join(work, 'data'),
    XDG_RUNTIME_DIR: path.join(work, 'run'),  RUST_LOG:'info,ivnc::compositor::clipboard=debug' };
for (const dir of [env.IVNC_HOME, env.XDG_CONFIG_HOME, env.XDG_CACHE_HOME, env.XDG_DATA_HOME, env.XDG_RUNTIME_DIR]) mkdirSync(dir, {mode:0o700});
const children = [], connections = [];
function launch(command, args, name, extra = {}) {
    const fd = openSync(path.join(work, name + '.log'), 'a', 0o600);
    const child = spawn(command, args, { stdio:['ignore',fd,fd], ...extra });
    closeSync(fd); children.push(child); return child;
}
async function waitFor(name, fn, timeout = 30000) {
    const until = Date.now() + timeout; let error;
    while (Date.now() < until) {
        try { const result = await fn(); if (result) return result; } catch (e) { error = e; }
        await delay(50);
    }
    throw Error(`Timeout: ${name}${error ? ': ' + error.message : ''}`);
}
async function api(url, method = 'GET', body) {
    const res = await fetch(base + url, {method, headers:{'Content-Type':'application/json'}, body:body ? JSON.stringify(body) : undefined});
    const data = await res.json(); assert(res.ok, JSON.stringify(data)); return data;
}
async function cdp(port) {
    const page = await waitFor('CDP page', async () => (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t => t.type === 'page'));
    const socket = new WebSocket(page.webSocketDebuggerUrl); connections.push(socket);
    await new Promise((resolve, reject) => { socket.on('open', resolve); socket.on('error', reject); });
    const pending = new Map(); let sequence = 0;
    socket.on('message', raw => {
        const msg = JSON.parse(raw), p = pending.get(msg.id); if (!p) return;
        pending.delete(msg.id); clearTimeout(p.timer);
        if (msg.error) p.reject(Error(msg.error.message)); else p.resolve(msg.result);
    });
    const call = (method, params = {}) => new Promise((resolve, reject) => {
        const id = ++sequence, timer = setTimeout(() => {pending.delete(id); reject(Error(`CDP timeout: ${method}`));}, 20000);
        pending.set(id, {resolve,reject,timer}); socket.send(JSON.stringify({id,method,params}));
    });
    const evaluate = async expression => {
        const r = await call('Runtime.evaluate', {expression,awaitPromise:true,returnByValue:true,userGesture:false});
        if (r.exceptionDetails) throw Error(JSON.stringify(r.exceptionDetails)); return r.result.value;
    };
    return {call,evaluate};
}
let passed=0, local, appId;
const fixtureApps=[];
const electron=process.env.ELECTRON_BIN;
assert(electron,'Set ELECTRON_BIN to an unpacked Electron runtime (38 or newer)');
const pass=name=>{++passed;console.log(`PASS ${name}`)};
const control=async (action, params={})=>{
    const r=await fetch(`http://127.0.0.1:${remotePort}/${action}?`+new URLSearchParams(params));
    const value=await r.json();assert(r.ok,JSON.stringify(value));return value;
};
const evaluate=js=>control('eval',{js});
const windowState=async name=>(await control('state')).windows.find(w=>w.name===name);
const send=message=>local.evaluate(`window.__ivncWebRTC.sendDataChannelMessage(${JSON.stringify(message)})`);
const tabs=()=>local.evaluate('window.__electronTabs || []');
const protocolTrace=()=>readFileSync(path.join(env.IVNC_HOME,'apps',appId,'app.log'),'utf8');
function protocolWindow(name) {
    const trace=protocolTrace();
    const title=[...trace.matchAll(/xdg_toplevel#(\d+)\.set_title\("([^"\n]*)"\)/g)].find(m=>m[2]===name);
    assert(title,`No protocol title for ${name}`);
    const top=title[1];
    const xdg=trace.match(new RegExp(`xdg_surface#(\\d+)\\.get_toplevel\\(new id xdg_toplevel#${top}\\)`))[1];
    const surface=trace.match(new RegExp(`get_xdg_surface\\(new id xdg_surface#${xdg}, wl_surface#(\\d+)\\)`))[1];
    return {top,xdg,surface};
}
async function configured(name, offset, request) {
    const {top,xdg,surface}=protocolWindow(name);
    await waitFor(`${name} ${request} configure committed`,()=>{
        const trace=protocolTrace().slice(offset), start=trace.indexOf(`xdg_toplevel#${top}.${request}(`);
        if(start<0)return false;
        // Wayland clients may acknowledge only the newest configure, skipping
        // intermediate focus/size states delivered in the same dispatch.
        const reply=[...trace.slice(start).matchAll(new RegExp(`xdg_surface#${xdg}\\.configure\\((\\d+)\\)`,'g'))].at(-1);
        if(!reply)return false;
        const ack=trace.indexOf(`xdg_surface#${xdg}.ack_configure(${reply[1]})`,start);
        return ack>=0&&trace.indexOf(`wl_surface#${surface}.commit()`,ack)>=0;
    });
}
try {
    launch(binary,['--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','960','--height','720','--upload-dir',path.join(work,'uploads')], 'server',{env,cwd:root});
    await waitFor('server',()=>api('/api/apps'));
    const quote=s=>"'"+s.replaceAll("'", "'\\''")+"'";
    const created=await api('/api/apps','POST',{name:'Electron Wayland smoke',app_type:'desktop',autostart:false,
        exec_command:[electron,path.join(root,'scripts/fixtures/electron-wayland/main.cjs')].map(quote).join(' '),
        env_vars:{ELECTRON_CONTROL_PORT:String(remotePort),WAYLAND_DEBUG:'1'}});
    appId=created.app?.id||created.id;
    assert(appId,JSON.stringify(created));
    await api(`/api/apps/${appId}/start`,'POST');
    const initial=await waitFor('Electron native window',()=>control('state'));
    assert.equal(initial.session,'wayland');assert.equal(initial.display,null);
    pass(`Electron ${initial.version} starts with a native Wayland session and no DISPLAY`);
    launch(process.env.CHROME_BIN||'google-chrome',['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
        '--no-first-run','--no-default-browser-check','--disable-background-networking','--password-store=basic','--autoplay-policy=no-user-gesture-required',
        `--user-data-dir=${path.join(work,'viewer')}`,`--remote-debugging-port=${browserPort}`,'--window-size=1280,900',base],'viewer');
    local=await cdp(browserPort);
    await local.call('Browser.grantPermissions',{origin:base,permissions:['clipboardReadWrite','clipboardSanitizedWrite']});
    await waitFor('WebRTC',()=>local.evaluate('window.__ivncWebRTC?._send_channel?.readyState === "open"'),60000);
    await waitFor('decoded Electron video',()=>local.evaluate('document.querySelector("video")?.videoWidth > 0'),60000);
    await local.evaluate(`(()=>{const rtc=window.__ivncWebRTC,taskbar=rtc.ontaskbarupdate;rtc.ontaskbarupdate=d=>{window.__electronTabs=d.windows||d;taskbar?.(d)};window.__cursorMessages=[];const cursor=rtc.oncursorchange;rtc.oncursorchange=d=>{window.__cursorMessages.push(d);cursor?.(d)}})()`);
    await local.evaluate(`(()=>{const rtc=window.__ivncWebRTC,clipboard=rtc.onclipboardmessage;rtc.onclipboardmessage=(t,d)=>{if(t==='clipboard_caps')window.__clipboardCaps=d;clipboard?.(t,d)}})()`);
    await control('second');
    await waitFor('separate main windows',async()=>{const t=await tabs();return Array.isArray(t)&&t.some(w=>w.title==='Electron Main')&&t.some(w=>w.title==='Electron Second')});
    pass('independent BrowserWindows from one process both appear in the toolbar');
    const second=(await tabs()).find(w=>w.title==='Electron Second');
    await control('minimize',{name:'Electron Second'});
    await waitFor('minimized stays in toolbar',async()=>(await tabs()).some(w=>w.title==='Electron Second'&&w.minimized));
    await send('focus,'+second.id);
    await waitFor('toolbar restores minimized window',async()=>{const s=await windowState('Electron Second');return s.focused&&!s.minimized});
    pass('minimized window stays addressable and toolbar restores it');
    await control('dialog',{name:'Electron Second'});
    await waitFor('modal window',async()=>!!(await windowState('Electron Dialog'))?.focused);
    // Electron's BrowserWindow.parent is currently implemented only on X11.
    // With no xdg parent on the wire it must remain an accessible toplevel.
    await waitFor('unparented Electron modal remains accessible',async()=>(await tabs()).some(w=>w.title==='Electron Dialog'));
    assert(!(await windowState('Electron Dialog')).maximized);
    await control('close',{name:'Electron Dialog'});
    pass('Electron modal without protocol parent stays accessible at its fixed size');
    await control('fixed');
    const fixed=await waitFor('fixed-size window',()=>windowState('Electron Fixed'));
    assert(!fixed.maximized);assert(fixed.bounds.width<600&&fixed.bounds.height<400,JSON.stringify(fixed));
    await control('close',{name:'Electron Fixed'});
    pass('fixed-size independent window keeps its requested size');
    await send('focus,'+second.id);
    await waitFor('second focused after closing fixed window',async()=>(await windowState('Electron Second')).focused);
    let offset=protocolTrace().length;
    await control('fullscreen',{name:'Electron Second',on:'1'});
    await configured('Electron Second',offset,'set_fullscreen');
    await waitFor('real fullscreen',async()=>(await windowState('Electron Second')).fullscreen);
    offset=protocolTrace().length;
    await control('fullscreen',{name:'Electron Second',on:'0'});
    await configured('Electron Second',offset,'unset_fullscreen');
    await waitFor('restore maximized after fullscreen',async()=>{const s=await windowState('Electron Second');return !s.fullscreen&&s.maximized});
    pass('fullscreen exit restores the previous maximized state');
    const maximizedBounds=(await windowState('Electron Second')).bounds;
    offset=protocolTrace().length;
    await control('unmaximize',{name:'Electron Second'});
    await configured('Electron Second',offset,'unset_maximized');
    await waitFor('ordinary window state',async()=>!(await windowState('Electron Second')).maximized);
    offset=protocolTrace().length;
    await control('bounds',{name:'Electron Second'});
    const normal=await waitFor('ordinary window geometry',async()=>{const s=await windowState('Electron Second');return !s.fullscreen&&!s.maximized&&s.bounds.width===680&&s.bounds.height===420&&s});
    const ordinary=protocolWindow('Electron Second');
    await waitFor('ordinary geometry submitted through Wayland',()=>{
        const trace=protocolTrace().slice(offset), geometry=trace.indexOf(`xdg_surface#${ordinary.xdg}.set_window_geometry(0, 0, 680, 420)`);
        return geometry>=0&&trace.indexOf(`wl_surface#${ordinary.surface}.commit()`,geometry)>=0;
    });
    offset=protocolTrace().length;
    await control('fullscreen',{name:'Electron Second',on:'1'});
    await configured('Electron Second',offset,'set_fullscreen');
    await waitFor('ordinary window enters fullscreen',async()=>{const s=await windowState('Electron Second');return s.fullscreen&&s.bounds.width===maximizedBounds.width&&s.bounds.height===maximizedBounds.height});
    offset=protocolTrace().length;
    await control('fullscreen',{name:'Electron Second',on:'0'});
    await configured('Electron Second',offset,'unset_fullscreen');
    const restored=await waitFor('ordinary geometry restored',async()=>{const s=await windowState('Electron Second');return !s.fullscreen&&!s.maximized&&s.bounds.width===normal.bounds.width&&s.bounds.height===normal.bounds.height&&s});
    assert.equal(restored.bounds.width,normal.bounds.width);assert.equal(restored.bounds.height,normal.bounds.height);
    await control('maximize',{name:'Electron Second'});
    await waitFor('maximized again',async()=>(await windowState('Electron Second')).maximized);
    pass('fullscreen also restores a previously ordinary window without forcing maximization');
    const clickRemote=async(x,y)=>{
        const name=await waitFor('focused Electron window',async()=>(await control('state')).windows.find(w=>w.focused)?.name);
        const clicks=await control('eval',{name,js:'window.clicks'});
        const p=await local.evaluate(`(()=>{const r=document.getElementById('overlayInput').getBoundingClientRect(),v=document.querySelector('video');return {x:r.left+${x}*r.width/v.videoWidth,y:r.top+${y}*r.height/v.videoHeight}})()`);
        for(const type of ['mouseMoved','mousePressed','mouseReleased']) await local.call('Input.dispatchMouseEvent',{type,...p,button:type==='mouseMoved'?'none':'left',buttons:type==='mousePressed'?1:0,clickCount:1});
        await waitFor('remote click delivery',()=>control('eval',{name,js:`window.clicks > ${clicks}`}));
    };
    await clickRemote(150,110);
    const ime='中文输入😀';
    await control('eval',{name:'Electron Second',js:'document.getElementById("text").focus()'});
    await send('t,'+ime);await send('t,连续输入');
    await waitFor('native IME commits',()=>control('eval',{name:'Electron Second',js:`document.getElementById('text').value === ${JSON.stringify(ime+'连续输入')}`}));
    const trace=readFileSync(path.join(env.IVNC_HOME,'apps',appId,'app.log'),'utf8');
    assert(trace.includes('commit_string(\"'+ime+'\")'));assert(/zwp_text_input_v3.*done\([1-9][0-9]*\)/.test(trace));
    assert(trace.includes('preferred_scale(120)'));
    pass('consecutive Unicode IME commits reach enabled text-input-v3 with valid serials and scale=1');
    await send('t,'+'長'.repeat(20000));await send('t,尾');
    await waitFor('large IME fallback',()=>control('eval',{name:'Electron Second',js:`document.getElementById('text').value.endsWith('長'.repeat(20000)+'尾')`}),10000);
    pass('large IME fallback is delivered before following commits');
    await send('focus,'+(await tabs()).find(w=>w.title==='Electron Main').id);
    await waitFor('main window focused',async()=>(await windowState('Electron Main')).focused);
    await clickRemote(150,110);
    await control('minimize',{name:'Electron Second'});
    await waitFor('background window minimized',async()=>(await tabs()).some(w=>w.title==='Electron Second'&&w.minimized));
    assert((await windowState('Electron Main')).focused);
    await control('restore',{name:'Electron Second'});
    await control('focus',{name:'Electron Second'});
    await waitFor('native activation restores Electron window',async()=>{const s=await windowState('Electron Second');return s.focused&&!s.minimized});
    pass('minimizing a background window preserves focus; native activation restores it after user input');
    await send('focus,'+(await tabs()).find(w=>w.title==='Electron Main').id);
    await waitFor('main focused for popup',async()=>(await windowState('Electron Main')).focused);
    const sampleMenu=()=>local.evaluate(`(()=>{const v=document.querySelector('video'),c=document.createElement('canvas');c.width=v.videoWidth;c.height=v.videoHeight;const x=c.getContext('2d');x.drawImage(v,0,0);return [...x.getImageData(330,330,1,1).data].slice(0,3)})()`);
    const beforeMenu=await sampleMenu(), colorDistance=(a,b)=>a.reduce((n,v,i)=>n+Math.abs(v-b[i]),0);
    offset=protocolTrace().length;
    await control('menu');
    await waitFor('native popup configured and drawn',()=>{
        const trace=protocolTrace().slice(offset), popup=trace.match(/get_xdg_surface\(new id xdg_surface#(\d+), wl_surface#(\d+)\)/);
        if(!popup||!trace.includes(`xdg_surface#${popup[1]}.get_popup(`)||!trace.includes(`xdg_surface#${popup[1]}.ack_configure(`))return false;
        const frame=trace.match(new RegExp(`wl_surface#${popup[2]}\\.frame\\(new id wl_callback#(\\d+)\\)`));
        return frame&&trace.includes(`wl_callback#${frame[1]}.done(`);
    });
    assert((await control('state')).menu.open);
    await waitFor('popup visible in WebRTC video',async()=>colorDistance(beforeMenu,await sampleMenu())>40);
    await send('k,65307,1');await send('k,65307,0');
    await waitFor('native popup dismissed',async()=>{const m=(await control('state')).menu;return !m.open&&m.closes===1});
    await waitFor('popup removed from video',async()=>colorDistance(beforeMenu,await sampleMenu())<20);
    pass('native popup is configured, rendered in video and dismissed by Escape without disconnect');
    await api('/api/console/settings','PUT',{binary_clipboard_enabled:true});
    await waitFor('rich clipboard enabled without reconnect',()=>local.evaluate('window.__clipboardCaps?.mime_types.includes("image/png")'));
    await clickRemote(150,110);
    await evaluate("document.getElementById('edit').focus()");
    await local.evaluate(`navigator.clipboard.write([new ClipboardItem({'text/plain':new Blob(['LOCAL_RICH'],{type:'text/plain'}),'text/html':new Blob(['<b>LOCAL_RICH</b>'],{type:'text/html'})})])`);
    const paste=async()=>{
        for(const e of [{type:'keyDown',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:2},
            {type:'keyDown',key:'v',code:'KeyV',windowsVirtualKeyCode:86,modifiers:2},
            {type:'keyUp',key:'v',code:'KeyV',windowsVirtualKeyCode:86,modifiers:2},
            {type:'keyUp',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:0}]) await local.call('Input.dispatchKeyEvent',e);
    };
    await paste();
    await waitFor('HTML paste',()=>evaluate("document.getElementById('edit').textContent==='LOCAL_RICH' && [...document.getElementById('edit').querySelectorAll('*')].some(e=>Number(getComputedStyle(e).fontWeight)>=600)"));
    const rich=await control('readClipboard');assert.equal(rich.text,'LOCAL_RICH');assert(rich.html.includes('LOCAL_RICH'));
    pass('native browser paste offers HTML and plain text in one acknowledged snapshot');
    await local.evaluate(`(async()=>{const c=document.createElement('canvas');c.width=c.height=2;const x=c.getContext('2d');x.fillStyle='#35a';x.fillRect(0,0,2,2);const blob=await new Promise(r=>c.toBlob(r));await navigator.clipboard.write([new ClipboardItem({'image/png':blob})])})()`);
    await paste();
    await waitFor('PNG native paste event',()=>evaluate('window.pastes.some(p=>p.types.includes("Files"))'));
    await waitFor('PNG reaches Electron',async()=>(await control('readClipboard')).image);
    pass('native browser PNG paste reaches Electron without wl-copy');
    await control('clipboard');
    await waitFor('Electron rich copy reaches browser',()=>local.evaluate(`navigator.clipboard.read().then(items=>items.some(i=>i.types.includes('image/png')&&i.types.includes('text/html')&&i.types.includes('text/plain')))`));
    assert.equal(await local.evaluate('navigator.clipboard.readText()'),'Electron text');
    pass('Electron text, HTML and PNG copy back to the local browser together');
    const cursorPoint=await evaluate("(()=>{const r=document.getElementById('custom').getBoundingClientRect();return{x:r.left+8,y:r.top+8}})()");
    await send(`m,${Math.round(cursorPoint.x)},${Math.round(cursorPoint.y)},0,0`);
    await waitFor('custom cursor PNG',()=>local.evaluate('window.__cursorMessages.some(c=>!!c.curdata)'));
    pass('custom bitmap cursor is sent as PNG to the viewer');
    await local.evaluate('window.__oldCursorChannel=window.__ivncWebRTC._send_channel;window.__cursorMessages=[];window.__ivncWebRTC.reset()');
    await waitFor('reconnected data channel',()=>local.evaluate('window.__ivncWebRTC._send_channel!==window.__oldCursorChannel&&window.__ivncWebRTC._send_channel?.readyState==="open"'),60000);
    await waitFor('cursor replay on reconnect',()=>local.evaluate('window.__cursorMessages.some(c=>!!c.curdata)'),1200);
    pass('a reconnected viewer immediately receives the cached bitmap cursor');
    const lock=await evaluate("(()=>{const r=document.getElementById('lock').getBoundingClientRect();return{x:r.left+8,y:r.top+8}})()");
    await send(`m,${Math.round(lock.x)},${Math.round(lock.y)},1,0`);await send(`m,${Math.round(lock.x)},${Math.round(lock.y)},0,0`);
    await waitFor('Electron pointer lock',()=>evaluate('!!document.pointerLockElement'));
    await evaluate('window.moves=[]');
    await send('m2,1,1,0,0');
    await waitFor('relative pointer binding ready',()=>evaluate('window.moves.some(m=>m[2]===1&&m[3]===1)'));
    await evaluate('window.moves=[]');
    await send('m2,2000,1500,0,0');await send('m2,25,10,0,0');
    await waitFor('relative motion beyond boundary',()=>evaluate('window.moves.reduce((n,m)=>n+m[2],0)===2025&&window.moves.reduce((n,m)=>n+m[3],0)===1510'));
    await local.evaluate(`document.querySelector('.toolbar-gesture-action').click()`);
    await waitFor('enabling gestures releases pointer lock',()=>local.evaluate('!document.pointerLockElement'));
    assert.equal(await local.evaluate(`document.querySelector('.toolbar-gesture-action').getAttribute('aria-pressed')`),'true');
    await local.evaluate(`document.querySelector('.toolbar-gesture-action').click()`);
    pass('locked pointer receives relative deltas and enabling gestures releases it');
    const uploaded='Electron 文件上传😀';
    const uploadBytes=[...new TextEncoder().encode(uploaded)];
    await send(`FILE_UPLOAD_START:electron-test.txt:${uploadBytes.length}`);
    await local.evaluate(`window.__ivncWebRTC._send_channel.send(new Uint8Array([1,${uploadBytes.join(',')}]))`);
    await send('FILE_UPLOAD_END:electron-test.txt');
    await waitFor('uploaded file accessible to Electron',async()=>(await control('file',{path:path.join(work,'uploads/electron-test.txt')}))===uploaded);
    pass('browser upload reaches the server filesystem and is readable by Electron');
    const picker=control('nativeFile');picker.catch(()=>{});
    await waitFor('GTK native file chooser',()=>readFileSync(path.join(env.IVNC_HOME,'apps',appId,'app.log'),'utf8').includes('set_title("Electron File Picker")'));
    await send('k,65307,1');await send('k,65307,0');
    const pickerResult=await picker;
    assert(pickerResult.canceled);
    pass('native GTK file chooser opens and cancels through Wayland input');
    const protocolBuild=path.join(work,'protocol');mkdirSync(protocolBuild);
    const xml=process.env.XDG_SHELL_XML||'/usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml';
    execFileSync('wayland-scanner',['client-header',xml,path.join(protocolBuild,'xdg-shell-client-protocol.h')]);
    execFileSync('wayland-scanner',['private-code',xml,path.join(protocolBuild,'xdg-shell-protocol.c')]);
    const flags=execFileSync('pkg-config',['--cflags','--libs','wayland-client'],{encoding:'utf8'}).trim().split(/\s+/);
    const protocolBin=path.join(protocolBuild,'window-client');
    execFileSync('cc',['-O2','-Wall','-I'+protocolBuild,path.join(root,'scripts/fixtures/wayland-window.c'),path.join(protocolBuild,'xdg-shell-protocol.c'),...flags,'-o',protocolBin]);
    const fixturePort=await freePort(), collisionPort=await freePort();
    const fixture=async(action='state',p=fixturePort)=>{
        const r=await fetch(`http://127.0.0.1:${p}/${action}`);return r.json();
    };
    const createFixture=async(name, vars)=>{
        const data=await api('/api/apps','POST',{name,app_type:'desktop',autostart:false,exec_command:quote(protocolBin),env_vars:{WAYLAND_DEBUG:'1',...vars}});
        const id=data.app?.id||data.id;fixtureApps.push(id);await api(`/api/apps/${id}/start`,'POST');return id;
    };
    await createFixture('Protocol parent',{FIXTURE_PORT:String(fixturePort)});
    const protocol=await waitFor('protocol parent and child',()=>fixture());
    await waitFor('parent protocol focus',async()=>(await fixture()).childFocused===1);
    await waitFor('parent alone in toolbar',async()=>(await tabs()).some(w=>w.title==='Protocol Parent'));
    assert(!(await tabs()).some(w=>w.title==='Protocol Child'));
    const parentTab=(await tabs()).find(w=>w.title==='Protocol Parent');
    await send('focus,'+(await tabs()).find(w=>w.title==='Electron Main').id);
    await waitFor('protocol family unfocused',async()=>(await fixture()).childFocused===0);
    await send('focus,'+parentTab.id);
    await waitFor('parent tab activates child',async()=>(await fixture()).childFocused===1);
    pass('explicit xdg child is centered, omitted from toolbar and activated with its parent');
    const activationCount=()=>readFileSync(path.join(env.IVNC_HOME,'apps',appId,'app.log'),'utf8').split(/xdg_activation_v1.*\.activate\(/).length;
    const priorActivation=activationCount();
    await control('focus');
    await waitFor('background Electron sends activation',()=>activationCount()>priorActivation);
    for(let i=0;i<5;i++){
        await delay(50);
        assert.equal((await fixture()).childFocused,1);
        assert(!(await windowState('Electron Main')).focused);
    }
    pass('a background Electron client cannot take focus from another Wayland client');
    await fixture('minimize');
    await waitFor('whole family minimized',async()=>(await tabs()).some(w=>w.title==='Protocol Parent'&&w.minimized));
    await send('focus,'+parentTab.id);
    await waitFor('child restored with parent',async()=>(await fixture()).childFocused===1);
    pass('toolbar restores an entire minimized parent/child family');
    const geometry=await local.evaluate('({width:document.querySelector("video").videoWidth,height:document.querySelector("video").videoHeight})');
    const childX=Math.round((geometry.width-320)/2), childY=Math.round((geometry.height-180)/2);
    await send(`m,${childX+15},${childY+15},0,0`);await send(`m,${childX+15},${childY+15},1,0`);
    await delay(100);await send(`m,${childX+135},${childY+85},1,0`);await send(`m,${childX+135},${childY+85},0,0`);
    await delay(100);await fixture('repaint');await delay(100);
    const presses=(await fixture()).childPressed;
    await send(`m,${childX+120+300},${childY+70+120},1,0`);await send(`m,${childX+120+300},${childY+70+120},0,0`);
    await waitFor('dialog does not jump back on commit',async()=>(await fixture()).childPressed===presses+1);
    pass('dragged child keeps its position across a repaint');
    await fixture('minimize');
    await waitFor('family hidden before resize',async()=>(await tabs()).some(w=>w.title==='Protocol Parent'&&w.minimized));
    await send('r,640x480');
    await waitFor('smaller desktop',()=>local.evaluate('document.querySelector("video").videoWidth===640&&document.querySelector("video").videoHeight===480'));
    await send('focus,'+parentTab.id);
    await waitFor('resized family restored',async()=>(await fixture()).childFocused===1);
    const restoredPresses=(await fixture()).childPressed;
    await send('m,620,460,1,0');await send('m,620,460,0,0');
    await waitFor('restored dialog clamped into output',async()=>(await fixture()).childPressed===restoredPresses+1);
    await send(`r,${geometry.width}x${geometry.height}`);
    await waitFor('desktop size restored',()=>local.evaluate(`document.querySelector("video").videoWidth===${geometry.width}&&document.querySelector("video").videoHeight===${geometry.height}`));
    pass('minimized dialog stays reachable after the desktop shrinks');
    await createFixture('Colliding client',{FIXTURE_PORT:String(collisionPort),FIXTURE_SINGLE:'1',FIXTURE_SURFACE_ID:String(protocol.childId),FIXTURE_NAME:'Colliding Main'});
    const collision=await waitFor('client with reused wire id',()=>fixture('state',collisionPort));
    assert.equal(collision.rootId,protocol.childId);
    await waitFor('colliding main stays in toolbar',async()=>(await tabs()).some(w=>w.title==='Colliding Main'));
    pass('a second Wayland client reusing a dialog wire ID remains an independent window');
    await fixture('unparent');
    await waitFor('unparented child is addressable',async()=>(await tabs()).some(w=>w.title==='Protocol Child'));
    await fixture('reparent');
    await waitFor('reparent removes child tab',async()=>!(await tabs()).some(w=>w.title==='Protocol Child'));
    pass('late parent changes update toolbar membership in both directions');
    await fixture('destroyParent');
    await waitFor('surviving child gets independent tab',async()=>{const t=await tabs();return t.some(w=>w.title==='Protocol Child')&&!t.some(w=>w.title==='Protocol Parent')});
    pass('destroying a parent leaves a surviving child accessible in the toolbar');
    const mainBefore=(await tabs()).find(w=>w.title==='Electron Main');
    const secondBefore=(await tabs()).find(w=>w.title==='Electron Second');
    const other=(await tabs()).find(w=>w.title==='Colliding Main');
    for(const tab of [mainBefore,secondBefore,other])assert.match(tab.window_key,/^[0-9a-f-]{36}$/);
    assert(await local.evaluate(`window.__stableWindowNode=document.querySelector('.toolbar-list [data-key="window:${secondBefore.window_key}"]');!!window.__stableWindowNode`));
    await send('focus-key,'+secondBefore.window_key);
    await waitFor('stable key focuses second window',async()=>(await windowState('Electron Second')).focused);
    assert.equal((await tabs()).find(w=>w.title==='Electron Second').window_key,secondBefore.window_key);
    await control('close',{name:'Electron Main'});
    const secondAfter=await waitFor('second index shifts after first window closes',async()=>{
        const t=await tabs(),second=t.find(w=>w.title==='Electron Second');
        return !t.some(w=>w.title==='Electron Main')&&second?.id!==secondBefore.id&&second;
    });
    assert.equal(secondAfter.window_key,secondBefore.window_key);
    assert(await local.evaluate(`window.__stableWindowNode===document.querySelector('.toolbar-list [data-key="window:${secondBefore.window_key}"]')`));
    await send('focus-key,'+other.window_key);
    await waitFor('other window focused',async()=>(await tabs()).some(w=>w.title==='Colliding Main'&&w.focused));
    await send('focus-key,'+mainBefore.window_key);
    await delay(150);
    assert((await tabs()).some(w=>w.title==='Colliding Main'&&w.focused),'a closed key must not focus another window');
    await send('focus-key,'+secondBefore.window_key);
    await waitFor('stable key survives index shift',async()=>(await windowState('Electron Second')).focused);
    pass('window keys survive index changes and stale keys cannot focus another window');
    const log=(await api(`/api/apps/${appId}/logs`)).logs;
    assert(!log.includes('Wayland protocol error'));
    console.log(`Electron Wayland smoke: ${passed} passed`);
} catch(error) {
    console.error(error);
    try{console.error('Native state',await control('state'));console.error('Input state',await evaluate('({moves:window.moves.slice(-10),locked:!!document.pointerLockElement,active:document.activeElement.id})'))}catch(_){}
    console.error(`Logs: ${work}`);process.exitCode=1;
} finally {
    for(const socket of connections)socket.close();
    for(const id of [...fixtureApps,appId].filter(Boolean))try{await api(`/api/apps/${id}/stop`,'POST')}catch(_){}
    for(const child of children.reverse()){child.kill('SIGTERM');await delay(100);if(child.exitCode===null)child.kill('SIGKILL')}
    if(!process.exitCode&&!process.env.KEEP_TMP)rmSync(work,{recursive:true,force:true});
    else console.log(`Kept ${work}`);
}
