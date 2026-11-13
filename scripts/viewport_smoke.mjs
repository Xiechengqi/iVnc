// Real browser work-area, resolution and input regression. Both browsers and
// the server are isolated. IVNC_BIN=target/debug/ivnc node scripts/viewport_smoke.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, rmSync, writeFileSync, readFileSync, renameSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-viewport-'));
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
    XDG_RUNTIME_DIR: path.join(work, 'run'), IVNC_CHROME_DEBUG_PORT: String(remotePort), RUST_LOG:'info,ivnc::compositor::clipboard=debug' };
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
async function cdp(port, targetId) {
    const page = await waitFor('CDP page', async () => (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t => t.type === 'page' && (!targetId || t.id === targetId)));
    const socket = new WebSocket(page.webSocketDebuggerUrl); connections.push(socket);
    await new Promise((resolve, reject) => { socket.on('open', resolve); socket.on('error', reject); });
    const pending = new Map(); let sequence = 0;
    socket.on('message', raw => {
        const msg = JSON.parse(raw); if(msg.method==='Runtime.exceptionThrown') console.error('Browser exception',JSON.stringify(msg.params.exceptionDetails)); const p = pending.get(msg.id); if (!p) return;
        pending.delete(msg.id); clearTimeout(p.timer);
        if (msg.error) p.reject(Error(msg.error.message)); else p.resolve(msg.result);
    });
    const call = (method, params = {}) => new Promise((resolve, reject) => {
        const id = ++sequence, timer = setTimeout(() => {pending.delete(id); reject(Error(`CDP timeout: ${method}`));}, 20000);
        pending.set(id, {resolve,reject,timer}); socket.send(JSON.stringify({id,method,params}));
    });
    const evaluate = async expression => {
        const r = await call('Runtime.evaluate', {expression,awaitPromise:true,returnByValue:true,userGesture:true});
        if (r.exceptionDetails) throw Error(JSON.stringify(r.exceptionDetails)); return r.result.value;
    };
    await call('Runtime.enable');
    await call('Page.enable');
    return {call,evaluate};
}
let passed = 0, local, remote;
const pass = name => { ++passed; console.log(`PASS ${name}`); };
const bounds = `(() => {
    const rect=n=>{const r=n.getBoundingClientRect();return {left:r.left,top:r.top,right:r.right,bottom:r.bottom,width:r.width,height:r.height}};
    const video=document.getElementById('stream'),bar=document.querySelector('.taskbar');
    return {video:rect(video),input:rect(document.getElementById('overlayInput')),area:rect(document.querySelector('.video-container')),
        bar:bar?rect(bar):null,pinned:!!bar?.classList.contains('pinned'),fullscreen:!!document.fullscreenElement,
        width:innerWidth,height:innerHeight,frame:[video.videoWidth,video.videoHeight],dpr:devicePixelRatio,
        manual:window.isManualResolutionMode,scroll:document.documentElement.scrollHeight};
})()`;
const matches = (a,b) => ['left','top','width','height'].every(key=>Math.abs(a[key]-b[key])<1);
const pin = () => local.evaluate(`document.querySelector('.toolbar-tools .taskbar-pin').click()`);
const message = data => local.evaluate(`window.postMessage(${JSON.stringify(data)},location.origin)`);
async function reload() {
    await local.call('Page.bringToFront');
    await local.evaluate('window.__viewportReloading=true');
    await local.call('Page.reload');
    await waitFor('new document and WebRTC',()=>local.evaluate(`!window.__viewportReloading && !!document.getElementById('stream') && window.__ivncWebRTC?._send_channel?.readyState==='open'`),60000);
}
async function automatic(peer = local) {
    await peer.call('Page.bringToFront');
    return waitFor('independent fitted work area and decoded resolution', async () => {
        const b = await peer.evaluate(bounds), state = await peer.evaluate('window.__ivncDisplayState');
        const height = b.height - (b.pinned && !b.fullscreen ? b.bar.height : 0);
        if (!state || !b.frame[0]) return false;
        const scale = Math.min(b.area.width/b.frame[0], b.area.height/b.frame[1]);
        if (Math.abs(b.area.height-height)>1 || !matches(b.video,b.input)) return false;
        if (Math.abs(b.video.width - b.frame[0]*scale)>1 || Math.abs(b.video.height - b.frame[1]*scale)>1) return false;
        if (Math.abs(b.video.left - (b.area.left+(b.area.width-b.video.width)/2))>1 || Math.abs(b.video.top-(b.area.top+(b.area.height-b.video.height)/2))>1) return false;
        if (b.frame[0]!==state.encoded.width || b.frame[1]!==state.encoded.height) return false;
        assert(b.scroll<=b.height+1, JSON.stringify(b));
        return b;
    });
}
async function clickBottom(peer = local, fitted = true) {
    await peer.call('Page.bringToFront');
    if (fitted) await automatic(peer);
    const before = await remote.evaluate('window.clicks');
    const b = await peer.evaluate(bounds);
    const state = await api('/api/console/display');
    // Chrome enforces a 500px minimum window width even on narrow outputs.
    // Use its real horizontal position and distance from the content bottom;
    // Chrome's outerHeight/screenY can include off-screen CSD decorations.
    const p=await remote.evaluate(`(() => {const r=document.getElementById('bottom').getBoundingClientRect();
        return {x:r.left+r.width/2,bottom:innerHeight-r.top-r.height/2}})()`);
    const x=b.video.left+p.x*b.video.width/state.desktop.width, y=b.video.bottom-p.bottom*b.video.height/state.desktop.height;
    for (const type of ['mouseMoved','mousePressed','mouseReleased']) await peer.call('Input.dispatchMouseEvent', {
        type,x,y,button:type==='mouseMoved'?'none':'left',buttons:type==='mousePressed'?1:0,clickCount:1,
    });
    await waitFor('remote bottom button receives the click',()=>remote.evaluate(`window.clicks===${before+1}`));
}
let viewer2, lowViewer;
let server;
const display = () => api('/api/console/display');
const geometry = () => remote.evaluate('({outer:[outerWidth,outerHeight],inner:[innerWidth,innerHeight],screen:[screen.width,screen.height]})');
async function assertStable(before, native) {
    assert.deepEqual((await display()).desktop, before);
    assert.deepEqual(await geometry(), native);
}
async function newViewer(width, height, dpr = 1, low = false) {
    const target = await local.call('Target.createTarget',{url:'about:blank'});
    const peer = await cdp(browserPort,target.targetId);
    peer.targetId = target.targetId;
    await peer.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:dpr,mobile:false});
    if (low) await peer.call('Page.addScriptToEvaluateOnNewDocument',{source:`Object.defineProperty(navigator,'mediaCapabilities',{value:undefined});window.__viewportForcedLow=true;`});
    await peer.call('Page.navigate',{url:base});
    await waitFor('viewer WebRTC',()=>peer.evaluate('window.__ivncWebRTC?._send_channel?.readyState==="open" && !!window.__ivncDisplayState'),60000);
    if (low) assert(await peer.evaluate('window.__viewportForcedLow'), 'low capability hook runs before the offer');
    await automatic(peer); return peer;
}
async function closeViewer(peer) { await local.call('Target.closeTarget',{targetId:peer.targetId}); }
try {
    const args = ['--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','1920','--height','1080'];
    server=launch(binary,args,'server',{env,cwd:root});
    await waitFor('server',()=>api('/api/apps'));
    await api('/api/apps/builtin-chrome/start','POST');
    remote = await cdp(remotePort);
    const html='<html><body style="margin:0;background:#eef"><div>Independent viewport regression</div><button id="bottom" style="position:fixed;bottom:4px;left:calc(50% - 60px);width:120px;height:24px" onclick="window.clicks++">Bottom button</button><script>window.clicks=0;window.moves=[];addEventListener("mousemove",e=>window.moves.push([e.clientX,e.clientY]));window.ups=0;addEventListener("mouseup",()=>window.ups++);window.wheels=0;addEventListener("wheel",()=>window.wheels++)</script></body></html>';
    await remote.call('Page.navigate',{url:'data:text/html,'+encodeURIComponent(html)});
    await waitFor('remote page',()=>remote.evaluate('!!document.getElementById("bottom")'));
    launch(process.env.CHROME_BIN || 'google-chrome', ['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
        '--no-first-run','--no-default-browser-check','--disable-background-networking','--autoplay-policy=no-user-gesture-required',
        `--user-data-dir=${path.join(work,'viewer')}`,`--remote-debugging-port=${browserPort}`,'--window-size=1280,800',base], 'viewer');
    local=await cdp(browserPort);
    await local.call('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});
    await waitFor('WebRTC',()=>local.evaluate('window.__ivncWebRTC?._send_channel?.readyState==="open" && !!window.__ivncDisplayState'),60000);
    const initial=(await display()).desktop;
    assert.deepEqual(initial,{width:1920,height:1080});
    let b=await automatic(); assert.equal(b.area.height,764);
    await clickBottom();
    const native=await geometry();
    pass('configured desktop and native application geometry are stable; fitted bottom controls are clickable');
    viewer2=await newViewer(720,1100,2); await clickBottom(viewer2);
    await automatic(local); await assertStable(initial,native);
    pass('landscape and portrait clients independently fit the same desktop at different DPRs');
    for(const peer of [viewer2,local]) {
        await peer.call('Page.bringToFront');
        await peer.evaluate('window.__viewportReloading=true');
        await peer.call('Page.reload');
        await waitFor('reloaded WebRTC',()=>peer.evaluate('!window.__viewportReloading && window.__ivncWebRTC?._send_channel?.readyState==="open" && !!window.__ivncDisplayState'),60000);
        await automatic(peer); await clickBottom(peer); await assertStable(initial,native);
    }
    pass('refresh order cannot take over desktop or native window size');
    await local.call('Page.bringToFront');
    await pin(); await automatic(); await clickBottom();
    await pin(); await automatic();
    await local.evaluate(`document.querySelector('.taskbar').style.height='52px'`);
    b=await automatic(); assert.equal(b.area.height,748); await clickBottom();
    await local.evaluate(`document.querySelector('.taskbar').style.height=''`); await automatic();
    await assertStable(initial,native);
    pass('toolbar pin and measured height only change the local work area');
    for (const [width,height,dpr] of [[375,640,2],[1440,900,2],[1024,601,1]]) {
        await local.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:dpr,mobile:false});
        await automatic(); await clickBottom(); await assertStable(initial,native);
    }
    pass('resize, odd viewport dimensions, orientation and DPR preserve remote layout and input alignment');
    const before=await remote.evaluate('({moves:window.moves.length,wheels:window.wheels,clicks:window.clicks})');
    b=await automatic();
    const blank={x:2,y:b.area.height-4};
    assert(blank.x < b.video.left || blank.y > b.video.bottom || blank.y < b.video.top);
    for(const type of ['mouseMoved','mousePressed','mouseReleased']) await local.call('Input.dispatchMouseEvent',{type,...blank,button:type==='mouseMoved'?'none':'left',buttons:type==='mousePressed'?1:0,clickCount:1});
    await local.call('Input.dispatchMouseEvent',{type:'mouseWheel',...blank,deltaX:0,deltaY:100});
    await delay(300);
    assert.deepEqual(await remote.evaluate('({moves:window.moves.length,wheels:window.wheels,clicks:window.clicks})'),before);
    pass('letterbox mouse and wheel input never reaches the remote application');
    b=await automatic();
    const center={x:b.video.left+b.video.width/2,y:b.video.top+b.video.height/2};
    const ups=await remote.evaluate('window.ups');
    await local.call('Input.dispatchMouseEvent',{type:'mousePressed',...center,button:'left',buttons:1,clickCount:1});
    await local.call('Input.dispatchMouseEvent',{type:'mouseMoved',...blank,button:'left',buttons:1});
    await local.call('Input.dispatchMouseEvent',{type:'mouseReleased',...blank,button:'left',buttons:0,clickCount:1});
    await waitFor('release outside content',()=>remote.evaluate(`window.ups>${ups}`));
    pass('drag release outside the image does not leave the remote button held');
    await message({type:'setDisplayMode',value:'native'});
    await waitFor('native scrolling',()=>local.evaluate(`document.querySelector('.video-container').classList.contains('manual-exact') && document.getElementById('stream').getBoundingClientRect().width===document.getElementById('stream').videoWidth`));
    await local.evaluate(`(() => {const c=document.querySelector('.video-container');c.scrollTop=c.scrollHeight;c.scrollLeft=(document.getElementById('stream').videoWidth-c.clientWidth)/2;})()`);
    // clickBottom's fitted-layout wait is intentionally skipped in native mode.
    b=await local.evaluate(bounds);
    assert(matches(b.video,b.input)); assert(b.video.top<0); await clickBottom(local,false); await assertStable(initial,native);
    await reload();
    await waitFor('native preference restored',()=>local.evaluate(`document.querySelector('.video-container').classList.contains('manual-exact')`));
    await message({type:'setDisplayMode',value:'fit'}); await automatic();
    assert.equal(await local.evaluate('document.querySelector(".video-container").scrollTop'),0);
    await clickBottom();
    pass('original-size scrolling and reload preserve only local preference; returning to fit clears scrolling');
    await local.evaluate(`(() => {const prefix=Object.keys(localStorage).find(k=>k.endsWith('_display_mode')).slice(0,-'display_mode'.length);localStorage.removeItem(prefix+'display_mode');localStorage.setItem(prefix+'manual_width','3840');localStorage.setItem(prefix+'manual_height','2160');localStorage.setItem(prefix+'is_manual_resolution_mode','true');localStorage.setItem(prefix+'scaleLocallyManual','true');})()`);
    await reload(); await automatic(); await assertStable(initial,native);
    assert.equal(await local.evaluate(`Object.keys(localStorage).some(k=>k.endsWith('_manual_width')||k.endsWith('_manual_height'))`),false);
    await local.evaluate(`window.__ivncWebRTC.sendDataChannelMessage('r,3840x2160')`); await delay(400); await assertStable(initial,native);
    pass('old stored dimensions and legacy r messages cannot change shared geometry');
    await local.evaluate(`document.querySelector('.video-container').requestFullscreen()`); b=await automatic(); assert(b.fullscreen);
    await local.evaluate('document.exitFullscreen()'); await automatic(); await assertStable(initial,native);
    pass('fullscreen affects this client only');
    lowViewer=await newViewer(800,600,1,true);
    await waitFor('common stream reduced for level 3.1',async()=>{const s=await display();return s.encoded.width<1920 && s.encoded.height<1080});
    await automatic(); await automatic(viewer2); await clickBottom(lowViewer); await clickBottom(); await assertStable(initial,native);
    pass('low H264 receive level scales encoded frames without changing desktop/native window; both clients click accurately');
    await closeViewer(lowViewer); lowViewer=null;
    await waitFor('full stream restored',async()=>{const s=await display();return s.encoded.width===1920 && s.encoded.height===1080});
    await automatic(); await assertStable(initial,native);
    pass('leaving low-capability client restores video quality without a desktop resize');
    const old=await display();
    const changed=await api('/api/console/display','PUT',{width:1600,height:1000,revision:old.revision});
    assert.equal(changed.revision,old.revision+1); assert.deepEqual(changed.desktop,{width:1600,height:1000});
    await waitFor('application actually reconfigured',()=>remote.evaluate('screen.width===1600 && screen.height===1000 && outerWidth===1600'));
    await automatic(); await automatic(viewer2); await clickBottom(); await clickBottom(viewer2);
    const conflict=await fetch(base+'/api/console/display',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({width:1280,height:720,revision:old.revision})});assert.equal(conflict.status,409);
    const invalid=await fetch(base+'/api/console/display',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({width:7680,height:4320,revision:changed.revision})});assert.equal(invalid.status,400);
    pass('explicit global resize updates both viewers and native window; stale revisions and excessive resource requests fail');
    const configPath=path.join(env.IVNC_HOME,'console.json'), savedConfig=readFileSync(configPath,'utf8');
    renameSync(configPath,configPath+'.saved'); mkdirSync(configPath);
    const failed=await fetch(base+'/api/console/display',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({width:1280,height:720,revision:changed.revision})});assert.equal(failed.status,500);
    rmSync(configPath,{recursive:true});renameSync(configPath+'.saved',configPath);
    assert.deepEqual((await display()).desktop,changed.desktop);assert.equal((await display()).revision,changed.revision);
    assert((await display()).last_error.includes('Cannot save desktop size'));
    await automatic(); await clickBottom(); assert.equal(readFileSync(configPath,'utf8'),savedConfig);
    pass('persistence failure rolls back actual geometry and revision; video/input recover');
    const app=(await api('/api/apps','POST',{name:'Viewport shared GTK',app_type:'desktop',exec_command:'gtk3-demo'})).app;
    await api(`/api/apps/${app.id}/sharing`,'PUT',{enabled:true,password:'viewport-smoke-password'});await api(`/api/apps/${app.id}/start`,'POST');
    await waitFor('shared app tab',()=>local.evaluate(`!!document.querySelector('[data-key="app:${app.id}"]')`));
    await local.evaluate(`document.querySelector('[data-key="app:${app.id}"]').click()`);
    await waitFor('embedded app connection',()=>local.evaluate(`document.querySelector('.app-stream.active iframe')?.contentWindow?.__ivncWebRTC?._send_channel?.readyState==='open'`),60000);
    const embeddedFits=()=>local.evaluate(`(() => {const f=document.querySelector('.app-stream.active iframe'),d=f.contentDocument;const a=document.querySelector('.video-container').getBoundingClientRect(),r=f.getBoundingClientRect(),c=d.querySelector('.video-container').getBoundingClientRect();return Math.abs(a.height-r.height)<1 && Math.abs(r.height-c.height)<1 && !d.querySelector('.taskbar');})()`);
    await waitFor('embedded layout',embeddedFits);
    await local.evaluate(`window.__savedFrame=document.querySelector('.app-stream.active iframe');window.__savedChannel=window.__savedFrame.contentWindow.__ivncWebRTC._send_channel`);
    await pin(); await waitFor('embedded unpin',embeddedFits);await pin();await waitFor('embedded pin',embeddedFits);
    const workerUrl=`/api/console/display?app_id=${app.id}`, workerState=await api(workerUrl);
    await api(workerUrl,'PUT',{width:1280,height:720,revision:workerState.revision});
    await waitFor('worker resized',()=>local.evaluate(`document.querySelector('.app-stream.active iframe').contentWindow.__ivncDisplayState?.desktop.width===1280`));
    assert.deepEqual((await display()).desktop,changed.desktop);
    assert(await local.evaluate(`window.__savedFrame===document.querySelector('.app-stream.active iframe') && window.__savedChannel===window.__savedFrame.contentWindow.__ivncWebRTC._send_channel`));
    const view=await api(`/api/apps/${app.id}/view`,'POST');
    const denied=await fetch(base+view.url+'api/console/display');assert.equal(denied.status,404);
    await api(`${view.url}close`,'POST');
    pass('shared worker has independent managed geometry; iframe/channel survive local resize; viewers cannot access management');
    const consoleTarget=await local.call('Target.createTarget',{url:base+'/console'}), consolePage=await cdp(browserPort,consoleTarget.targetId);
    await waitFor('console initialized',()=>consolePage.evaluate(`!!document.getElementById('display-scope') && document.querySelector('#display-scope + .console-select')`));
    // Opening during another administrator's resize must recover when it ends.
    await consolePage.evaluate(`(() => {const original=window.fetch.bind(window);window.__displayReads=0;window.fetch=async (url,options)=>{const response=await original(url,options);if(String(url)==='/api/console/display' && ++window.__displayReads===1){const state=await response.json();return new Response(JSON.stringify({...state,pending:state.desktop}),{status:200,headers:{'Content-Type':'application/json'}});}return response;};})()`);
    await consolePage.evaluate(`document.getElementById('nav-settings').click()`);
    await waitFor('console display state',()=>consolePage.evaluate(`document.getElementById('display-width').value==='1600' && !document.getElementById('display-apply').disabled`));
    assert(await consolePage.evaluate(`window.__displayReads>=2`));
    await consolePage.evaluate(`document.querySelector('#display-scope + .console-select').click()`);
    assert(await consolePage.evaluate(`!document.getElementById('display-scope-listbox').hidden`));
    await consolePage.evaluate(`document.querySelector('#display-scope-listbox [role="option"]:nth-child(2)').click()`);
    await waitFor('console scoped state',()=>consolePage.evaluate(`document.getElementById('display-width').value==='1280' && document.getElementById('display-height').value==='720'`));
    await consolePage.evaluate(`document.getElementById('display-width').value='1366';document.getElementById('display-height').value='768';document.getElementById('display-apply').click()`);
    await waitFor('console saved',()=>consolePage.evaluate(`!document.getElementById('display-apply').disabled && document.getElementById('display-status').textContent.includes('1366')`));
    assert.deepEqual((await api(workerUrl)).desktop,{width:1366,height:768});
    assert.deepEqual((await display()).desktop,changed.desktop);
    await local.call('Target.closeTarget',{targetId:consoleTarget.targetId});
    pass('console styled desktop selector loads main/worker dimensions and applies an explicit scoped change');
    // Stop/restart confirms per-worker persistent size and process-level main restoration.
    await api(`/api/apps/${app.id}/stop`,'POST');await api(`/api/apps/${app.id}/start`,'POST');
    assert.deepEqual((await api(workerUrl)).desktop,{width:1366,height:768});
    await api(`/api/apps/${app.id}/stop`,'POST');await api('/api/apps/builtin-chrome/stop','POST');
    server.kill('SIGTERM');await waitFor('server stops',()=>server.exitCode!==null || !!server.signalCode);
    server=launch(binary,args,'server-restarted',{env,cwd:root});await waitFor('restarted server',()=>display());
    assert.deepEqual((await display()).desktop,changed.desktop);
    pass('main and per-worker administrator selections survive service/worker restart');
    server.kill('SIGTERM');await waitFor('server stops before locked startup',()=>server.exitCode!==null || !!server.signalCode);
    const lockedConfig=path.join(work,'locked.toml');writeFileSync(lockedConfig,'[display]\nlocked = true\n');
    server=launch(binary,['-c',lockedConfig,...args],'server-locked',{env,cwd:root});await waitFor('locked server',()=>display());
    const locked=await display();assert(locked.locked);assert.deepEqual(locked.desktop,initial);
    const lockedWrite=await fetch(base+'/api/console/display',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({width:1280,height:720,revision:locked.revision})});assert.equal(lockedWrite.status,423);
    pass('locked server configuration overrides saved dimensions and rejects runtime changes');
    console.log(`Viewport smoke: ${passed} passed`);
} catch(error) {
    console.error(error);console.error(`Artifacts: ${work}`);process.exitCode=1;
    for (const [name,peer] of [['local',local],['remote',remote]]) {
        if(!peer)continue;
        try {console.error(name,await peer.evaluate(name==='local'?bounds:'({width:innerWidth,height:innerHeight,outer:[outerWidth,outerHeight],screen:[screen.width,screen.height],clicks:window.clicks})'));const screenshot=await peer.call('Page.captureScreenshot');writeFileSync(path.join(work,name+'.png'),Buffer.from(screenshot.data,'base64'));}catch(_){}
    }
} finally {
    for(const socket of connections)socket.close();
    try{await api('/api/apps/builtin-chrome/stop','POST');}catch(_){}
    for(const child of children.reverse()) {
        if(child.exitCode!==null || child.signalCode)continue;
        child.kill('SIGTERM');for(let i=0;i<50 && child.exitCode===null && !child.signalCode;i++)await delay(100);
        if(child.exitCode===null && !child.signalCode)child.kill('SIGKILL');
    }
    if(!process.env.KEEP_TMP && !process.exitCode)rmSync(work,{recursive:true,force:true});
}
