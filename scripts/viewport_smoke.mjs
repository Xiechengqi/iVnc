// Real browser work-area, resolution and input regression. Both browsers and
// the server are isolated. IVNC_BIN=target/debug/ivnc node scripts/viewport_smoke.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, rmSync, writeFileSync } from 'node:fs';
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
        const msg = JSON.parse(raw), p = pending.get(msg.id); if (!p) return;
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
    await local.evaluate('window.__viewportReloading=true');
    await local.call('Page.reload');
    await waitFor('new document and WebRTC',()=>local.evaluate(`!window.__viewportReloading && !!document.getElementById('stream') && window.__ivncWebRTC?._send_channel?.readyState==='open'`),60000);
}
async function automatic(peer = local, physical = false) {
    return waitFor('automatic work area and decoded resolution', async () => {
        const b = await peer.evaluate(bounds);
        const height = b.height - (b.pinned && !b.fullscreen ? b.bar.height : 0);
        const dpr = physical ? b.dpr : 1;
        const w = Math.floor(b.width*dpr/2)*2;
        const h = Math.floor(height*dpr/2)*2;
        if (Math.abs(b.area.height-height)>1 || !matches(b.video,b.area) || !matches(b.video,b.input)) return false;
        if (b.frame[0]!==w || b.frame[1]!==h) return false;
        assert(b.scroll<=b.height+1, JSON.stringify(b));
        return b;
    });
}
async function clickBottom() {
    const before = await remote.evaluate('window.clicks');
    const b = await local.evaluate(bounds);
    // Chrome enforces a 500px minimum window width even on narrow outputs.
    // Use its real horizontal position and distance from the content bottom;
    // Chrome's outerHeight/screenY can include off-screen CSD decorations.
    const p=await remote.evaluate(`(() => {const r=document.getElementById('bottom').getBoundingClientRect();
        return {x:r.left+r.width/2,bottom:innerHeight-r.top-r.height/2}})()`);
    const x=b.video.left+p.x*b.video.width/b.frame[0], y=b.video.bottom-p.bottom*b.video.height/b.frame[1];
    for (const type of ['mouseMoved','mousePressed','mouseReleased']) await local.call('Input.dispatchMouseEvent', {
        type,x,y,button:type==='mouseMoved'?'none':'left',buttons:type==='mousePressed'?1:0,clickCount:1,
    });
    await waitFor('remote bottom button receives the click',()=>remote.evaluate(`window.clicks===${before+1}`));
}
try {
    launch(binary, ['--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','800','--height','600'], 'server', {env,cwd:root});
    await waitFor('server',()=>api('/api/apps'));
    await api('/api/apps/builtin-chrome/start','POST');
    remote = await cdp(remotePort);
    const html='<html><body style="margin:0;background:#eef"><div>Viewport regression</div><button id="bottom" style="position:fixed;bottom:4px;left:calc(50% - 60px);width:120px;height:24px" onclick="window.clicks++">Bottom button</button><script>window.clicks=0</script></body></html>';
    await remote.call('Page.navigate',{url:'data:text/html,'+encodeURIComponent(html)});
    await waitFor('remote page',()=>remote.evaluate('!!document.getElementById("bottom")'));
    launch(process.env.CHROME_BIN || 'google-chrome', ['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
        '--no-first-run','--no-default-browser-check','--disable-background-networking','--autoplay-policy=no-user-gesture-required',
        `--user-data-dir=${path.join(work,'viewer')}`,`--remote-debugging-port=${browserPort}`,'--window-size=1280,800',base], 'viewer');
    local=await cdp(browserPort);
    await local.call('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});
    await waitFor('WebRTC',()=>local.evaluate('window.__ivncWebRTC?._send_channel?.readyState==="open"'),60000);
    let b=await automatic();
    assert.equal(b.area.height,764);
    await clickBottom();
    pass('initial pinned toolbar reserves 36px; the real Chrome bottom button is visible and clickable');
    await local.evaluate(`(() => {window.__resolutions=[];const rtc=window.__ivncWebRTC,send=rtc.sendDataChannelMessage.bind(rtc);
        rtc.sendDataChannelMessage=m=>{if(m.startsWith('r,'))window.__resolutions.push(m);return send(m)};})()`);
    await pin(); await automatic(); await clickBottom();
    await local.call('Input.dispatchMouseEvent',{type:'mouseMoved',x:100,y:100});
    await waitFor('auto-hidden toolbar',()=>local.evaluate(`!document.querySelector('.taskbar').classList.contains('visible')`));
    const requests=await local.evaluate('window.__resolutions.length');
    await local.call('Input.dispatchMouseEvent',{type:'mouseMoved',x:100,y:799});
    await waitFor('hover toolbar',()=>local.evaluate(`document.querySelector('.taskbar').classList.contains('visible')`));
    await automatic();
    assert.equal(await local.evaluate('window.__resolutions.length'),requests);
    pass('unpin restores full height; temporary hover does not resize the desktop');
    await pin(); await automatic();
    await local.evaluate(`document.querySelector('.taskbar').style.height='52px'`);
    b=await automatic(); assert.equal(b.area.height,748); await clickBottom();
    pass('measured toolbar height changes update video, input and remote resolution');
    await local.evaluate(`document.querySelector('.taskbar').style.height=''`); await automatic();
    for (const [width,height,dpr] of [[375,640,2],[1440,900,2],[1024,601,1]]) {
        await local.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:dpr,mobile:false});
        await automatic(); await clickBottom();
    }
    pass('narrow screens, odd heights and HiDPI retain aligned input and accessible bottom controls');
    await local.evaluate(`localStorage.setItem(Object.keys(localStorage).find(k=>k.endsWith('_useCssScaling')),'false')`);
    await reload();
    await local.call('Emulation.setDeviceMetricsOverride',{width:800,height:600,deviceScaleFactor:2,mobile:false});
    await automatic(local,true); await clickBottom();
    await local.call('Emulation.setDeviceMetricsOverride',{width:800,height:600,deviceScaleFactor:1,mobile:false});
    await automatic(local,true); await clickBottom();
    await local.call('Emulation.setDeviceMetricsOverride',{width:801,height:601,deviceScaleFactor:2,mobile:false});
    await automatic(local,true); await clickBottom();
    pass('physical-pixel streaming applies DPR after subtracting toolbar height');
    await local.evaluate(`localStorage.setItem(Object.keys(localStorage).find(k=>k.endsWith('_useCssScaling')),'true')`);
    await reload();
    await automatic();
    await message({type:'setScaleLocally',value:true});
    await message({type:'setManualResolution',width:1920,height:1080});
    await waitFor('manual native resolution',()=>local.evaluate('document.getElementById("stream").videoWidth===1920 && document.getElementById("stream").videoHeight===1080'));
    b=await local.evaluate(bounds);
    assert(matches(b.video,b.input)); assert(b.video.bottom<=b.bar.top+1);
    await clickBottom();
    await local.call('Emulation.setDeviceMetricsOverride',{width:768,height:600,deviceScaleFactor:1,mobile:false});
    await waitFor('manual fit reflows',async()=>{const n=await local.evaluate(bounds);return n.area.width===768 && n.video.right<=768 && n.video.bottom<=n.bar.top+1 && matches(n.video,n.input)});
    assert.deepEqual((await local.evaluate(bounds)).frame,[1920,1080]); await clickBottom();
    pass('manual fitting reflows into the work area without changing the remote resolution');
    await reload();
    await waitFor('manual mode and frame restored',()=>local.evaluate('window.isManualResolutionMode && document.getElementById("stream").videoWidth===1920 && document.getElementById("stream").videoHeight===1080'));
    b=await local.evaluate(bounds); assert(matches(b.video,b.input)); assert(b.video.bottom<=b.bar.top+1);
    pass('manual dimensions and mode survive reload with the reserved work area');
    await message({type:'setScaleLocally',value:false});
    await waitFor('native-size scrolling',()=>local.evaluate(`(() => {const c=document.querySelector('.video-container');return c.classList.contains('manual-exact') && c.scrollHeight>c.clientHeight && c.scrollWidth>c.clientWidth})()`));
    await local.evaluate(`(() => {const c=document.querySelector('.video-container');c.scrollTop=c.scrollHeight;c.scrollLeft=(1920-c.clientWidth)/2;})()`);
    b=await local.evaluate(bounds); assert(matches(b.video,b.input)); assert.equal(b.video.width,1920);
    await clickBottom();
    pass('native-size mode scrolls to the bottom and keeps mouse coordinates correct');
    await message({type:'resetResolutionToWindow'}); await automatic();
    assert.equal(await local.evaluate('document.querySelector(".video-container").scrollTop'),0);
    pass('returning to automatic mode clears manual scroll and restores the correct resolution');
    await local.evaluate(`document.querySelector('.video-container').requestFullscreen()`);
    b=await automatic(); assert(b.fullscreen); assert.equal(b.area.height,b.height);
    await local.evaluate('document.exitFullscreen()'); await automatic();
    pass('video fullscreen uses the whole screen; exiting restores toolbar reservation');
    await local.evaluate(`(() => {window.__resolutions=[];const rtc=window.__ivncWebRTC,send=rtc.sendDataChannelMessage.bind(rtc);
        rtc.sendDataChannelMessage=m=>{if(m.startsWith('r,'))window.__resolutions.push(m);return send(m)};
        for(let i=0;i<4;i++)document.querySelector('.toolbar-tools .taskbar-pin').click();})()`);
    await delay(900); await automatic();
    assert.deepEqual(await local.evaluate('window.__resolutions'),[]);
    pass('rapid pin toggles coalesce and do not rebuild an unchanged resolution');
    const app=(await api('/api/apps','POST',{name:'Viewport shared GTK',app_type:'desktop',exec_command:'gtk3-demo'})).app;
    await api(`/api/apps/${app.id}/sharing`,'PUT',{enabled:true,password:'viewport-smoke-password'});
    await api(`/api/apps/${app.id}/start`,'POST');
    await waitFor('shared app tab',()=>local.evaluate(`!!document.querySelector('[data-key="app:${app.id}"]')`));
    await local.evaluate(`document.querySelector('[data-key="app:${app.id}"]').click()`);
    await waitFor('embedded app connection',()=>local.evaluate(`document.querySelector('.app-stream.active iframe')?.contentWindow?.__ivncWebRTC?._send_channel?.readyState==='open'`),60000);
    const embeddedFits=()=>local.evaluate(`(() => {const f=document.querySelector('.app-stream.active iframe'),d=f.contentDocument,w=f.contentWindow;
        const a=document.getElementById('app').getBoundingClientRect(),r=f.getBoundingClientRect(),c=d.querySelector('.video-container').getBoundingClientRect();
        return Math.abs(a.height-r.height)<1 && Math.abs(r.height-c.height)<1 && !d.querySelector('.taskbar') && w.__IVNC_EMBED__;})()`);
    await waitFor('embedded layout',embeddedFits);
    await local.evaluate(`window.__savedFrame=document.querySelector('.app-stream.active iframe');window.__savedChannel=window.__savedFrame.contentWindow.__ivncWebRTC._send_channel;window.__savedSrc=window.__savedFrame.src`);
    await pin(); await waitFor('embedded unpin',embeddedFits); await pin(); await waitFor('embedded pin',embeddedFits);
    assert(await local.evaluate(`window.__savedFrame===document.querySelector('.app-stream.active iframe') && window.__savedChannel===window.__savedFrame.contentWindow.__ivncWebRTC._send_channel && window.__savedChannel.readyState==='open' && window.__savedFrame.src===window.__savedSrc`));
    pass('shared iframe reserves toolbar space once and preserves its view, connection and selection');
    await message({type:'setScaleLocally',value:true});
    await message({type:'setManualResolution',width:1920,height:1080});
    await waitFor('main desktop letterboxing',()=>local.evaluate(`window.isManualResolutionMode && document.getElementById('stream').getBoundingClientRect().height<document.querySelector('.video-container').clientHeight-1`));
    await waitFor('shared view uses the whole work area',embeddedFits);
    assert(await local.evaluate(`window.__savedFrame===document.querySelector('.app-stream.active iframe') && window.__savedChannel===window.__savedFrame.contentWindow.__ivncWebRTC._send_channel`));
    await message({type:'resetResolutionToWindow'});
    await waitFor('main desktop returns to auto',()=>local.evaluate('!window.isManualResolutionMode'));
    pass('main desktop letterboxing does not shrink or recreate the shared application view');
    const view=await api(`/api/apps/${app.id}/view`,'POST');
    const target=await local.call('Target.createTarget',{url:base+view.url});
    const standalone=await cdp(browserPort,target.targetId);
    await waitFor('standalone connection',()=>standalone.evaluate('window.__ivncWebRTC?._send_channel?.readyState==="open"'),60000);
    b=await automatic(standalone); assert(b.pinned); assert.equal(b.area.height,b.height-b.bar.height);
    await local.call('Target.closeTarget',{targetId:target.targetId});
    await api(`${view.url}close`,'POST');
    pass('standalone app view reserves its own visible toolbar');
    console.log(`Viewport smoke: ${passed} passed`);
} catch(error) {
    console.error(error); console.error(`Artifacts: ${work}`); process.exitCode=1;
    for (const [name,peer] of [['local',local],['remote',remote]]) {
        if (!peer) continue;
        try {
            console.error(name,await peer.evaluate(name==='local'?bounds:'({width:innerWidth,height:innerHeight,clicks:window.clicks})'));
            const screenshot=await peer.call('Page.captureScreenshot');
            writeFileSync(path.join(work,name+'.png'),Buffer.from(screenshot.data,'base64'));
        } catch(_) {}
    }
} finally {
    for(const socket of connections) socket.close();
    try {await api('/api/apps/builtin-chrome/stop','POST');} catch(_) {}
    for(const child of children.reverse()) {
        if(child.exitCode!==null || child.signalCode) continue;
        child.kill('SIGTERM');
        for(let i=0;i<50 && child.exitCode===null && !child.signalCode;i++) await delay(100);
        if(child.exitCode===null && !child.signalCode) child.kill('SIGKILL');
    }
    if(!process.env.KEEP_TMP && !process.exitCode) rmSync(work,{recursive:true,force:true});
}
