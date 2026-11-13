// Isolated end-to-end H.264 negotiation, first-frame and recovery regression.
// No autoplay override and no synthetic user activation in Runtime.evaluate.
// IVNC_BIN=target/debug/ivnc node scripts/video_playback_smoke.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-video-'));
const children = [], sockets = [], viewers = [];
async function freePort() {
    const server = net.createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve));
    return port;
}
const port = await freePort(), remotePort = await freePort(), base = `http://127.0.0.1:${port}`;
const env = { ...process.env, IVNC_HOME: path.join(work, 'ivnc'), XDG_RUNTIME_DIR: path.join(work, 'run'),
    XDG_CONFIG_HOME: path.join(work, 'config'), XDG_CACHE_HOME: path.join(work, 'cache'), XDG_DATA_HOME: path.join(work, 'data'),
    IVNC_CHROME_DEBUG_PORT: String(remotePort), RUST_LOG: 'info' };
for (const dir of [env.IVNC_HOME, env.XDG_RUNTIME_DIR, env.XDG_CONFIG_HOME, env.XDG_CACHE_HOME, env.XDG_DATA_HOME]) mkdirSync(dir, { mode: 0o700 });
function launch(command, args, name, extra = {}) {
    const fd = openSync(path.join(work, name + '.log'), 'a', 0o600);
    const child = spawn(command, args, { stdio: ['ignore', fd, fd], ...extra });
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
    const res = await fetch(base + url, { method, headers: { 'Content-Type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    const result = await res.json(); assert(res.ok, JSON.stringify(result)); return result;
}
async function cdp(browserPort) {
    const page = await waitFor('CDP page', async () => (await (await fetch(`http://127.0.0.1:${browserPort}/json`)).json()).find(t => t.type === 'page'));
    const socket = new WebSocket(page.webSocketDebuggerUrl); sockets.push(socket);
    await new Promise((resolve, reject) => { socket.on('open', resolve); socket.on('error', reject); });
    let sequence = 0; const pending = new Map(), exceptions = [];
    socket.on('message', raw => {
        const msg = JSON.parse(raw);
        if (msg.method === 'Runtime.exceptionThrown') exceptions.push(msg.params.exceptionDetails);
        const p = pending.get(msg.id); if (!p) return;
        pending.delete(msg.id); clearTimeout(p.timer);
        if (msg.error) p.reject(Error(msg.error.message)); else p.resolve(msg.result);
    });
    const call = (method, params = {}) => new Promise((resolve, reject) => {
        const id = ++sequence, timer = setTimeout(() => { pending.delete(id); reject(Error(`CDP timeout: ${method}`)); }, 60000);
        pending.set(id, { resolve, reject, timer }); socket.send(JSON.stringify({ id, method, params }));
    });
    const evaluate = async expression => {
        const r = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
        if (r.exceptionDetails) throw Error(JSON.stringify(r.exceptionDetails)); return r.result.value;
    };
    await call('Page.enable'); await call('Runtime.enable');
    return { call, evaluate, exceptions };
}
const inspect = `(async () => {
    const rtc=window.__ivncWebRTC,v=document.getElementById('stream'),s=await rtc?.getConnectionStats();
    return {state:rtc?.playback.state(),first:rtc?.playback.firstFrame(),paused:v?.paused,width:v?.videoWidth,height:v?.videoHeight,
        frames:s?.video.framesDecoded,bytes:s?.video.bytesReceived,codec:s?.video.codecName,params:s?.video.codecParameters,pt:s?.video.payloadType,
        button:!!document.getElementById('playButton')&&!document.getElementById('playButton').classList.contains('hidden'),
        message:document.getElementById('status-display')?.textContent,
        statusHidden:document.getElementById('status-display')?.classList.contains('hidden')};
})()`;
async function viewer(name, mode = '') {
    const browserPort = await freePort();
    const child = launch(process.env.CHROME_BIN || 'google-chrome', ['--headless=new', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
        '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
        `--user-data-dir=${path.join(work, name)}`, `--remote-debugging-port=${browserPort}`, '--window-size=1280,800', 'about:blank'], name);
    const peer = await cdp(browserPort); peer.child = child; peer.name = name; viewers.push(peer);
    await peer.call('Page.addScriptToEvaluateOnNewDocument', { source: `(() => {
        if(location.origin!==${JSON.stringify(base)})return;
        const prefix=location.href.split('#')[0].replace(/[^a-zA-Z0-9.-_]/g,'_');
        localStorage.setItem(prefix+'_is_manual_resolution_mode','true');
        localStorage.setItem(prefix+'_manual_width','1920');localStorage.setItem(prefix+'_manual_height','1080');
        ${mode === 'low' ? `Object.defineProperty(navigator,'mediaCapabilities',{value:undefined});` : ''}
        ${mode === 'unsupported' ? `const add=RTCPeerConnection.prototype.addTransceiver;
            RTCPeerConnection.prototype.addTransceiver=function(kind,options){const t=add.call(this,kind,options);
                if(kind==='video')t.setCodecPreferences(RTCRtpReceiver.getCapabilities('video').codecs.filter(c=>c.mimeType==='video/VP8'));
                return t;};` : ''}
        ${mode === 'blocked' ? `window.__blockVideo=true;
            const descriptor=Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype,'srcObject');
            Object.defineProperty(HTMLMediaElement.prototype,'srcObject',{...descriptor,set(value){
                if(this.tagName==='VIDEO'&&window.__blockVideo)this.autoplay=false;descriptor.set.call(this,value);}});
            const play=HTMLMediaElement.prototype.play;
            HTMLMediaElement.prototype.play=function(){if(this.tagName==='VIDEO'&&window.__blockVideo)
                return Promise.reject(new DOMException('Blocked for regression','NotAllowedError'));return play.call(this);};` : ''}
    })()` });
    await peer.call('Page.navigate', { url: base });
    return peer;
}
async function playing(peer, width, height) {
    return waitFor(`${peer.name} displayed frame${width ? ` ${width}x${height}` : ''}`, async () => {
        const s = await peer.evaluate(inspect);
        return s.state === 'playing' && s.first && s.frames > 0 && !s.paused && (!width || (s.width === width && s.height === height)) ? s : false;
    }, 60000);
}
async function click(peer, selector) {
    const p = await peer.evaluate(`(() => {const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) await peer.call('Input.dispatchMouseEvent', {
        type, ...p, button: type === 'mouseMoved' ? 'none' : 'left', buttons: type === 'mousePressed' ? 1 : 0, clickCount: 1,
    });
}
let passed = 0;
const pass = name => { passed++; console.log(`PASS ${name}`); };
try {
    launch(process.env.IVNC_BIN || path.join(root, 'target/debug/ivnc'), ['--http-port', String(port), '--basic-auth-enabled', 'false',
        '--proxy-enabled', 'false', '--proxy-autostart', 'false', '--width', '1920', '--height', '1080'], 'server', { env, cwd: root });
    await waitFor('server', () => api('/api/apps'));
    await api('/api/apps/builtin-chrome/start', 'POST');
    const remote = await cdp(remotePort);
    const html = '<body style="margin:0;background:#def"><h1>H.264 desktop regression</h1><canvas id="c" width="600" height="240"></canvas><script>let n=0;setInterval(()=>{const x=c.getContext("2d");x.fillStyle=["#f33","#3a3","#33f"][n++%3];x.fillRect(0,0,600,240);x.fillStyle="white";x.font="48px sans-serif";x.fillText("Frame "+n,40,120)},100)</script>';
    await remote.call('Page.navigate', { url: 'data:text/html,' + encodeURIComponent(html) });

    const high = await viewer('high');
    const s = await playing(high, 1920, 1080);
    assert.equal(s.codec, 'H264'); assert.match(s.params, /profile-level-id=42[ce]034/i);
    assert(s.statusHidden && !s.button);
    const litPixels = await high.evaluate(`(() => {const c=document.createElement('canvas');c.width=160;c.height=90;
        const x=c.getContext('2d');x.drawImage(document.getElementById('stream'),0,0,160,90);
        const p=x.getImageData(0,0,160,90).data;let lit=0;
        for(let i=0;i<p.length;i+=4)if(p[i]+p[i+1]+p[i+2]>150)lit++;return lit;})()`);
    assert(litPixels > 1000, 'decoded video must contain the application image, rather than all-black pixels');
    const answer = await high.evaluate('window.__ivncWebRTC.peerConnection.remoteDescription.sdp');
    assert(answer.includes(`a=rtpmap:${s.pt} H264/90000`));
    pass('default autoplay displays real H.264 frames; payload and constrained-baseline receive level match SDP');

    await high.evaluate(`(() => {const rtc=window.__ivncWebRTC;
        const track=rtc.peerConnection.getReceivers().find(r=>r.track.kind==='video').track;
        rtc._ontrack({target:rtc.peerConnection,track,streams:[]});})()`);
    await playing(high, 1920, 1080);
    pass('a valid stream-less track event attaches and displays the received video track');

    const blocked = await viewer('blocked', 'blocked');
    await waitFor('actionable autoplay rejection', async () => (await blocked.evaluate(inspect)).state === 'blocked');
    const b = await blocked.evaluate(inspect);
    assert(b.button && !b.statusHidden && b.paused && !b.first);
    await blocked.evaluate('window.__blockVideo=false');
    await click(blocked, '#playButton'); await playing(blocked, 1920, 1080);
    pass('autoplay rejection stays visible after connection; a real mouse click starts the first frame');

    const low = await viewer('low', 'low');
    const l = await playing(low);
    assert.match(l.params, /profile-level-id=42[ce]01f/i);
    assert(l.width < 1920 && l.height < 1080);
    await playing(high, l.width, l.height);
    await playing(blocked, l.width, l.height);
    pass('a native level-3.1 receiver lowers the common output and existing receivers keep decoding');
    await low.evaluate('window.__ivncWebRTC.destroy()');
    low.child.kill('SIGTERM');
    await playing(high, 1920, 1080); await playing(blocked, 1920, 1080);
    pass('leaving the limited receiver restores requested resolution without reconnecting the other viewers');

    const before = (await high.evaluate(inspect)).frames;
    for (const [width, height] of [[1600, 900], [1280, 720], [1920, 1080]]) {
        const state = await api('/api/console/display');
        await api('/api/console/display', 'PUT', {width, height, revision:state.revision});
        await playing(high, width, height); await playing(blocked, width, height);
    }
    assert((await high.evaluate(inspect)).frames > before);
    pass('repeated resolution and encoder restarts retain a continuous decodable video timeline');

    const unsupported = await viewer('unsupported', 'unsupported');
    await waitFor('explicit codec rejection', async () => (await unsupported.evaluate('window.__ivncWebRTC?.playback.state()')) === 'unsupported');
    assert.match(await unsupported.evaluate('document.getElementById("status-display").textContent'), /视频格式|video format/);
    pass('an incompatible offer shows a format error instead of a successful black connection');

    const app = (await api('/api/apps', 'POST', { name: 'Video shared GTK', app_type: 'desktop', exec_command: 'gtk3-demo' })).app;
    await api(`/api/apps/${app.id}/sharing`, 'PUT', { enabled: true, password: 'isolated-video-smoke' });
    await api(`/api/apps/${app.id}/start`, 'POST');
    await waitFor('shared application tab', () => high.evaluate(`!!document.querySelector('[data-key="app:${app.id}"]')`));
    await click(high, `[data-key="app:${app.id}"]`);
    await waitFor('shared first frame', () => high.evaluate(`document.querySelector('.app-stream.active iframe')?.contentWindow?.__ivncWebRTC?.playback.state()==='playing'`), 60000);
    await high.evaluate(`(() => {const f=document.querySelector('.app-stream.active iframe');window.__savedVideoFrame=f;
        f.contentWindow.postMessage({type:'ivnc-view-active',active:false},location.origin);f.contentDocument.querySelector('video').pause();})()`);
    await waitFor('shared view inactive', () => high.evaluate(`window.__savedVideoFrame.contentWindow.__ivncViewActive===false`));
    await high.evaluate(`window.__savedVideoFrame.contentWindow.postMessage({type:'ivnc-view-active',active:true},location.origin)`);
    await waitFor('shared frame resumes', () => high.evaluate(`(() => {const f=window.__savedVideoFrame,v=f.contentDocument.querySelector('video');return f===document.querySelector('.app-stream.active iframe')&&!v.paused&&f.contentWindow.__ivncWebRTC.playback.state()==='playing'})()`));
    assert(await high.evaluate(`document.querySelector('.app-stream.active iframe').contentDocument.getElementById('status-display').classList.contains('hidden')`));
    pass('embedded views own playback error UI and resume a browser-paused stream on activation');

    // Verify actual encoder caps as well as browser-side negotiation/decoding.
    const log = readFileSync(path.join(work, 'server.log'), 'utf8');
    const caps = log.split('\n').filter(line => line.includes('Encoded video caps'));
    assert(caps.some(line => /profile-level-id:\s+\([^)]*\)\s+"42c0[0-9a-f]{2}"/.test(line)
        && line.includes('"constrained-baseline"')), 'actual RTP caps must identify constrained baseline');
    for (const peer of viewers) assert.deepEqual(peer.exceptions, [], peer.name + ' unhandled browser exceptions');
    pass('actual encoder output is constrained baseline; viewers have no unhandled JavaScript errors');
    console.log(`Video playback smoke: ${passed} passed`);
} catch (error) {
    console.error(error); console.error(`Artifacts: ${work}`); process.exitCode = 1;
    for (const peer of viewers) {
        if (peer.child.exitCode !== null || peer.child.signalCode) continue;
        try {
            console.error(peer.name, await peer.evaluate(inspect));
            writeFileSync(path.join(work, peer.name + '.png'), Buffer.from((await peer.call('Page.captureScreenshot')).data, 'base64'));
        } catch (_) {}
    }
} finally {
    for (const socket of sockets) socket.close();
    try { await api('/api/apps/builtin-chrome/stop', 'POST'); } catch (_) {}
    for (const child of children.reverse()) {
        if (child.exitCode !== null || child.signalCode) continue;
        child.kill('SIGTERM');
        await Promise.race([new Promise(resolve => child.once('exit', resolve)), delay(4000)]);
        if (child.exitCode === null && !child.signalCode) child.kill('SIGKILL');
    }
    if (!process.exitCode && !process.env.KEEP_SMOKE_ARTIFACTS) rmSync(work, { recursive: true, force: true });
}
