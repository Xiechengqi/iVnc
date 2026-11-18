// Real image copy: Wayland Chrome, worker policy, remote relay and independent native paste.
// Local only; all instances, profiles, sockets and generated images are isolated.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, rmSync, writeFileSync, readFileSync } from 'node:fs';
import { tmpdir, networkInterfaces } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import http from 'node:http';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-image-'));
const binary = process.env.IVNC_BIN || path.join(root, 'target/debug/ivnc');
async function freePort() {
    const server = net.createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve)); return port;
}
const port = await freePort(), browserPort = await freePort(), remotePort = await freePort();
const base = `http://127.0.0.1:${port}`;
const httpAddress = Object.values(networkInterfaces()).flat().find(address => address.family === 'IPv4' && !address.internal)?.address;
assert(httpAddress, 'An actual non-loopback IPv4 address is required for HTTP clipboard coverage');
const httpBase = `http://${httpAddress}:${port}`;
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

let passed = 0, local, remote, relayBase, imageSequence=0, expectedPixel;
const pass = name => { ++passed; console.log(`PASS ${name}`); };
const fixture = http.createServer((req,res)=>{
    res.setHeader('Content-Type','text/html');
    res.end(`<!doctype html><textarea autofocus style="width:90vw;height:50vh"></textarea><script>
    window.copyImage=async(seed,large)=>{const c=document.createElement('canvas');c.width=large?1024:7;c.height=large?768:5;
    const x=c.getContext('2d');if(large){const pixels=x.createImageData(c.width,c.height);let rng=seed;
    for(let i=0;i<pixels.data.length;i++){rng=(Math.imul(rng,1664525)+1013904223)>>>0;pixels.data[i]=i%4===3?255:rng>>>24}x.putImageData(pixels,0,0)}
    x.fillStyle='rgb('+(51+seed%100)+',85,170)';if(!large)x.fillRect(0,0,c.width,c.height);x.fillRect(0,0,1,1);x.fillRect(c.width-1,c.height-1,1,1);
    const png=await new Promise(r=>c.toBlob(r));window.pngBytes=png.size;await navigator.clipboard.write([new ClipboardItem({'image/png':png})]);return [51+seed%100,85,170,255];};
    document.querySelector('textarea').addEventListener('paste',async e=>{const f=[...e.clipboardData.items].find(i=>i.type==='image/png')?.getAsFile();if(!f)return;e.preventDefault();
    const b=await createImageBitmap(f),c=document.createElement('canvas');c.width=b.width;c.height=b.height;const x=c.getContext('2d');x.drawImage(b,0,0);
    window.pastedImage={width:b.width,height:b.height,pixel:[...x.getImageData(0,0,1,1).data],lastPixel:[...x.getImageData(b.width-1,b.height-1,1,1).data]};b.close()});
    window.addEventListener('keydown',e=>{if(e.key==='F8'){e.preventDefault();copyImage(window.copySeed,window.copyLarge).then(pixel=>window.copyDone={seed:window.copySeed,pixel},error=>window.copyError=error.name)}});
    window.ready=true;</script>`);
});
const apiAt = async (origin,url,method='GET',body) => {
    const res=await fetch(origin+url,{method,headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});
    const data=await res.json();assert(res.ok,JSON.stringify(data));return data;
};
const input = async letter => {
    for(const e of [{type:'keyDown',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:2},
        {type:'keyDown',key:letter,code:'Key'+letter.toUpperCase(),windowsVirtualKeyCode:letter.toUpperCase().charCodeAt(0),modifiers:2},
        {type:'keyUp',key:letter,code:'Key'+letter.toUpperCase(),windowsVirtualKeyCode:letter.toUpperCase().charCodeAt(0),modifiers:2},
        {type:'keyUp',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:0}])await local.call('Input.dispatchKeyEvent',e);
};
async function click(selector) {
    const point=await local.evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return{x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    for(const type of ['mouseMoved','mousePressed','mouseReleased'])await local.call('Input.dispatchMouseEvent',{type,...point,button:type==='mouseMoved'?'none':'left',buttons:type==='mousePressed'?1:0,clickCount:1});
}
async function visit(url) {
    await local.call('Page.navigate',{url});
    await waitFor('data channel',()=>local.evaluate('window.__ivncWebRTC?._send_channel?.readyState==="open"'),60000);
    await waitFor('decoded video',()=>local.evaluate('document.querySelector("video")?.videoWidth>0'),60000);
    await local.evaluate(`(()=>{window.pngUpdates=0;window.caps=null;const rtc=window.__ivncWebRTC,formats=rtc.onclipboardformats,message=rtc.onclipboardmessage;
        rtc.onclipboardformats=(r,m)=>{pngUpdates++;formats(r,m)};
        rtc.onclipboardmessage=(t,d)=>{if(t==='clipboard_caps')caps=d;if(t==='clipboard_status')(window.imageEvents||=[]).push({type:t,data:d,focus:document.hasFocus(),active:document.activeElement?.id});message(t,d)};
        window.imageEvents=[];for(const type of ['focusin','blur','visibilitychange','focus'])addEventListener(type,e=>imageEvents.push({type,target:e.target?.id,focus:document.hasFocus(),active:document.activeElement?.id,notice:document.querySelector('.clipboard-assist-message')?.textContent}),true);
        rtc.sendDataChannelMessage('paste,{"type":"hello"}');})();`);
    await waitFor('capabilities',()=>local.evaluate('window.caps'));
    await local.call('Page.bringToFront');await click('#overlayInput');
}
async function sourcePage(fixtureOrigin) {
    remote = await cdp(remotePort);
    await remote.call('Page.navigate',{url:fixtureOrigin});
    await waitFor('source fixture',()=>remote.evaluate('window.ready'));
    await remote.call('Browser.grantPermissions',{origin:fixtureOrigin,permissions:['clipboardReadWrite','clipboardSanitizedWrite']});
    await remote.call('Page.bringToFront');
}
const copyFromApp = async (large = false) => {
    const seed=++imageSequence;
    await remote.evaluate(`window.copySeed=${seed};window.copyLarge=${large};window.copyDone=null;window.copyError=null`);
    await local.evaluate(`document.getElementById('overlayInput').focus()`);
    for(const type of ['keyDown','keyUp'])await local.call('Input.dispatchKeyEvent',{type,key:'F8',code:'F8',windowsVirtualKeyCode:119});
    await waitFor('native source copy action',()=>remote.evaluate(`window.copyDone?.seed===${seed}||window.copyError`));
    assert.equal(await remote.evaluate('window.copyError'),null);
    expectedPixel=await remote.evaluate('window.copyDone.pixel');
};
async function copied(large = false) {
    const before=await local.evaluate('pngUpdates');
    await copyFromApp(large);
    await waitFor('PNG over real WebRTC',()=>local.evaluate(`pngUpdates>${before}`));
}
async function pasteImage(large = false) {
    await local.evaluate(`(()=>{document.getElementById('external-paste')?.remove();window.pastedImage=null;
        const t=document.createElement('textarea');t.id='external-paste';t.className='allow-native-input';document.body.appendChild(t);
        t.addEventListener('paste',async e=>{const f=[...e.clipboardData.items].find(i=>i.type==='image/png')?.getAsFile();if(!f)return;
        const b=await createImageBitmap(f),c=document.createElement('canvas');c.width=b.width;c.height=b.height;const x=c.getContext('2d');x.drawImage(b,0,0);
        pastedImage={width:b.width,height:b.height,pixel:[...x.getImageData(0,0,1,1).data],lastPixel:[...x.getImageData(b.width-1,b.height-1,1,1).data]};b.close();});t.focus();})()`);
    await input('v');await waitFor('native image paste',()=>local.evaluate('window.pastedImage'));
    assert.deepEqual(await local.evaluate('pastedImage'),{width:large?1024:7,height:large?768:5,pixel:expectedPixel,lastPixel:expectedPixel});
    await local.evaluate(`document.getElementById('external-paste').remove();document.getElementById('overlayInput').focus()`);
}
async function workerView() {
    const view=await api('/api/apps/builtin-chrome/view','POST');
    await visit(base+view.url);
    assert([404,405].includes((await fetch(base+view.url+'api/console/clipboard-policy',{method:'PUT',headers:{'Content-Type':'application/json'},body:'{"enabled":true}'})).status));
}
try {
    await new Promise(r=>fixture.listen(0,'127.0.0.1',r));
    const fixtureOrigin=`http://127.0.0.1:${fixture.address().port}/`;
    launch(binary,['--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','800','--height','600'],'server',{env,cwd:root});
    await waitFor('server',()=>api('/api/apps'));await api('/api/apps/builtin-chrome/start','POST');
    await sourcePage(fixtureOrigin);
    launch(process.env.CHROME_BIN||'google-chrome',['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
        '--no-first-run','--no-default-browser-check','--disable-background-networking','--password-store=basic','--autoplay-policy=no-user-gesture-required',
        '--no-proxy-server',`--user-data-dir=${path.join(work,'viewer')}`,`--remote-debugging-port=${browserPort}`,'--window-size=1280,800',base],'viewer');
    local=await cdp(browserPort);await visit(base);
    assert((await local.evaluate('caps.mime_types')).includes('image/png'));
    await copied();await waitFor('automatic image write finishes',()=>local.evaluate('!document.querySelector(".clipboard-assist")||document.querySelector(".clipboard-assist").hidden'));
    await pasteImage();pass('default settings copy a real Wayland Chrome PNG to a separate local native paste target');

    await copied(true);assert((await remote.evaluate('pngBytes'))>1024*1024);
    await waitFor('large automatic image write',()=>local.evaluate('__ivncClipboard.imageSnapshot()===null'));await pasteImage(true);
    await remote.evaluate('window.pastedImage=null');await input('v');
    await waitFor('large native image paste back into Wayland app',()=>remote.evaluate('window.pastedImage'));
    assert.deepEqual(await remote.evaluate('pastedImage'),{width:1024,height:768,pixel:expectedPixel,lastPixel:expectedPixel});
    pass('multi-megabyte PNG copies and pastes in both directions without truncated chunks or DTLS queue overflow');

    await visit(httpBase);assert.equal(await local.evaluate('isSecureContext'),false);
    await copied();await waitFor('HTTP original image preview',()=>local.evaluate('document.querySelector(".clipboard-image")?.naturalWidth===7'));
    assert.equal(await local.evaluate('document.querySelector(".clipboard-image").naturalHeight'),5);
    pass('ordinary HTTP exposes the real application image at its original dimensions');

    await visit(base);
    await local.evaluate(`(()=>{window.imageWrites=0;const write=navigator.clipboard.write.bind(navigator.clipboard);navigator.clipboard.write=(...args)=>{imageWrites++;return write(...args)};
        const rtc=window.__ivncWebRTC,receive=rtc.onclipboardformats;rtc.onclipboardformats=(records,metadata)=>{window.delayedImage=()=>receive(records,metadata)};})()`);
    await copyFromApp();await waitFor('delayed image payload',()=>local.evaluate('!!window.delayedImage'));
    const writes=await local.evaluate('imageWrites');
    const other=await local.call('Target.createTarget',{url:'about:blank'});
    await waitFor('real hidden page',()=>local.evaluate('document.visibilityState==="hidden"'));
    await local.evaluate('delayedImage()');await local.call('Page.bringToFront');
    await waitFor('late preview',()=>local.evaluate('document.querySelector(".clipboard-image")?.naturalWidth===7'));
    assert.equal(await local.evaluate('imageWrites'),writes);
    await click('.clipboard-copy');await waitFor('explicit late image write',()=>local.evaluate(`document.querySelector('.clipboard-assist-message').textContent.includes('Copied to')`));await pasteImage();
    await local.call('Target.closeTarget',{targetId:other.targetId});
    pass('a real tab switch retains the delayed image without background writes; a trusted click recovers native paste');

    await api('/api/apps/builtin-chrome/stop','POST');
    await api('/api/console/settings','PUT',{target_fps:25,binary_clipboard_enabled:false});
    await api('/api/apps/builtin-chrome/sharing','PUT',{enabled:true,password:'generated-image-smoke-password'});
    await api('/api/apps/builtin-chrome/start','POST');await sourcePage(fixtureOrigin);await workerView();
    assert(!(await local.evaluate('caps.mime_types')).includes('image/png'));
    pass('a newly launched worker inherits the saved disabled policy instead of the startup default');
    const applied=await api('/api/console/settings','PUT',{binary_clipboard_enabled:true});assert.equal(applied.pending_workers,0);
    await waitFor('worker PNG capability without reconnect',()=>local.evaluate('caps.mime_types.includes("image/png")'));
    assert.equal((await api('/api/console/settings')).saved.target_fps,25);
    await local.evaluate(`__ivncWebRTC.sendDataChannelMessage('SETTINGS,{"enable_binary_clipboard":false}');__ivncWebRTC.sendDataChannelMessage('paste,{"type":"hello"}')`);
    await copied(true);await waitFor('worker copy completes',()=>local.evaluate('__ivncClipboard.imageSnapshot()===null'));await pasteImage(true);
    pass('live worker policy changes, stale viewer settings and partial settings saves preserve working image copy');
    await api('/api/console/settings','PUT',{binary_clipboard_enabled:false});
    await waitFor('worker disable capability',()=>local.evaluate('!caps.mime_types.includes("image/png")'));
    await copyFromApp();await waitFor('disabled image explanation',()=>local.evaluate('document.querySelector(".clipboard-assist-message")?.textContent.includes("图片同步已关闭")||document.querySelector(".clipboard-assist-message")?.textContent.includes("Image sync is off")'));
    pass('disabling image sync reaches an existing worker and explains a subsequent image copy');
    await api('/api/apps/builtin-chrome/stop','POST');await api('/api/apps/builtin-chrome/start','POST');await sourcePage(fixtureOrigin);await workerView();
    assert(!(await local.evaluate('caps.mime_types')).includes('image/png'));
    pass('worker restart preserves owner policy and discards its old private settings');

    await api('/api/console/settings','PUT',{binary_clipboard_enabled:true});
    const relayPort=await freePort();relayBase=`http://127.0.0.1:${relayPort}`;
    const relayEnv={...env,IVNC_HOME:path.join(work,'relay'),XDG_RUNTIME_DIR:path.join(work,'relay-run')};mkdirSync(relayEnv.XDG_RUNTIME_DIR,{mode:0o700});
    launch(binary,['--http-port',String(relayPort),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','800','--height','600'],'relay',{env:relayEnv,cwd:root});
    await waitFor('relay server',()=>apiAt(relayBase,'/api/apps'));
    await apiAt(relayBase,'/api/console/settings','PUT',{binary_clipboard_enabled:false});
    const added=await apiAt(relayBase,'/api/apps','POST',{app_type:'remote',name:'Relayed image fixture',url:base+'/remote/v1/apps/builtin-chrome',password:'generated-image-smoke-password'});
    const id=added.app.id;await apiAt(relayBase,`/api/apps/${id}/start`,'POST');
    const relayView=await apiAt(relayBase,`/api/apps/${id}/view`,'POST');await visit(relayBase+relayView.url);
    assert((await local.evaluate('caps.mime_types')).includes('image/png'));
    await copied(true);await waitFor('relayed image write',()=>local.evaluate('__ivncClipboard.imageSnapshot()===null'));await pasteImage(true);
    assert.equal((await apiAt(relayBase,'/api/console/settings')).current.binary_clipboard_enabled,false);
    pass('remote application PNG copy traverses the relay using source policy without changing relay settings');
    console.log(`Image clipboard smoke: ${passed} passed`);
} catch(error) {
    console.error(error);console.error(`Artifacts: ${work}`);process.exitCode=1;
    for(const [name,peer] of [['local',local],['remote',remote]]){
        try{if(!peer)continue;const shot=await peer.call('Page.captureScreenshot');writeFileSync(path.join(work,name+'.png'),Buffer.from(shot.data,'base64'));
            console.error(name,await peer.evaluate(`({focus:document.hasFocus(),active:document.activeElement?.id,caps:window.caps,notice:document.querySelector('.clipboard-assist-message')?.textContent,events:window.imageEvents})`));}catch{}
    }
} finally {
    fixture.close();for(const connection of connections)connection.close();
    try{await api('/api/apps/builtin-chrome/stop','POST');}catch{}
    for(const child of children.reverse()){
        if(child.exitCode!==null||child.signalCode)continue;child.kill('SIGTERM');
        for(let i=0;i<50&&child.exitCode===null&&!child.signalCode;i++)await delay(100);
        if(child.exitCode===null&&!child.signalCode)child.kill('SIGKILL');
    }
    if(!process.env.KEEP_TMP&&!process.exitCode)rmSync(work,{recursive:true,force:true});
}
