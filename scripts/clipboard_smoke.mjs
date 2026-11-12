// Real browser clipboard -> WebRTC -> Wayland Chrome. Both browsers and the
// server are isolated. IVNC_BIN=target/debug/ivnc node scripts/clipboard_smoke.mjs
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
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-clipboard-'));
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
let passed = 0, local, remote;
const pass = name => { ++passed; console.log(`PASS ${name}`); };
try {
    launch(binary, ['--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','800','--height','600'], 'server', {env,cwd:root});
    await waitFor('server', () => api('/api/apps'));
    await api('/api/apps/builtin-chrome/start', 'POST');
    remote = await cdp(remotePort);
    const html = '<html><body style="margin:0"><textarea id="text" style="width:98vw;height:85vh;font-size:24px"></textarea></body></html>';
    await remote.call('Page.navigate', {url:'data:text/html,' + encodeURIComponent(html)});
    await waitFor('remote textarea', () => remote.evaluate('!!document.getElementById("text")'));
    launch(process.env.CHROME_BIN || 'google-chrome', ['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
        '--no-first-run','--no-default-browser-check','--disable-background-networking','--password-store=basic','--autoplay-policy=no-user-gesture-required',
        `--user-data-dir=${path.join(work,'viewer')}`,`--remote-debugging-port=${browserPort}`,'--window-size=1280,800',base], 'viewer');
    local = await cdp(browserPort);
    // Transport cases inspect the OS clipboard directly. Default/denied write
    // permissions are exercised separately below and in clipboard_ui_smoke.mjs.
    await local.call('Browser.grantPermissions', {origin:base,permissions:['clipboardReadWrite','clipboardSanitizedWrite']});
    await waitFor('WebRTC', () => local.evaluate('window.__ivncWebRTC?._send_channel?.readyState === "open"'),60000);
    await waitFor('decoded video', () => local.evaluate('document.querySelector("video")?.videoWidth > 0'),60000);
    await local.evaluate(`(() => { window.__clipSent=[]; window.__clipReplies=[]; window.__clipUpdates=[];
        const rtc=window.__ivncWebRTC,send=rtc.sendDataChannelMessage.bind(rtc),reply=rtc.onclipboardmessage;
        const content=rtc.onclipboardcontent;rtc.onclipboardcontent=(text,meta)=>{window.__clipUpdates.push(meta);content(text,meta)};
        rtc.sendDataChannelMessage=m=>{if(m.startsWith('paste,'))window.__clipSent.push(m);return send(m)};
        rtc.onclipboardmessage=(type,data)=>{window.__clipReplies.push({type,data});reply(type,data)};
        rtc.sendDataChannelMessage('paste,{"type":"hello"}');
    })()`);
    await waitFor('paste capability', () => local.evaluate('window.__clipReplies.some(m=>m.type==="clipboard_caps")'));
    const point = await local.evaluate(`(() => {const r=document.getElementById('overlayInput').getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    for (const type of ['mouseMoved','mousePressed','mouseReleased']) await local.call('Input.dispatchMouseEvent',{type,...point,button:type==='mouseMoved'?'none':'left',buttons:type==='mousePressed'?1:0,clickCount:1});
    await remote.call('Page.bringToFront');
    await remote.evaluate('document.getElementById("text").focus()');
    const reset = () => remote.evaluate(`(() => {const t=document.getElementById('text');t.value='';t.focus();})()`);
    const value = () => remote.evaluate('document.getElementById("text").value');
    const writeLocal = text => local.evaluate(`navigator.clipboard.writeText(${JSON.stringify(text)})`);
    const paste = async (mode = 'ctrl-v') => {
        const shift = mode !== 'ctrl-v', ctrl = mode !== 'shift-insert', modifiers = (shift?8:0) | (ctrl?2:0);
        if (ctrl) await local.call('Input.dispatchKeyEvent',{type:'keyDown',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:2});
        if (shift) await local.call('Input.dispatchKeyEvent',{type:'keyDown',key:'Shift',code:'ShiftLeft',windowsVirtualKeyCode:16,modifiers});
        const fields = mode === 'shift-insert' ? {key:'Insert',code:'Insert',windowsVirtualKeyCode:45} : {key:shift?'V':'v',code:'KeyV',windowsVirtualKeyCode:86};
        await local.call('Input.dispatchKeyEvent',{type:'keyDown',...fields,modifiers});
        await local.call('Input.dispatchKeyEvent',{type:'keyUp',...fields,modifiers});
        if (shift) await local.call('Input.dispatchKeyEvent',{type:'keyUp',key:'Shift',code:'ShiftLeft',windowsVirtualKeyCode:16,modifiers:ctrl?2:0});
        if (ctrl) await local.call('Input.dispatchKeyEvent',{type:'keyUp',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:0});
    };
    const copy = async (key = 'c') => {
        for (const event of [
            {type:'keyDown',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:2},
            {type:'keyDown',key,code:'Key'+key.toUpperCase(),windowsVirtualKeyCode:key.toUpperCase().charCodeAt(0),modifiers:2},
            {type:'keyUp',key,code:'Key'+key.toUpperCase(),windowsVirtualKeyCode:key.toUpperCase().charCodeAt(0),modifiers:2},
            {type:'keyUp',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:0},
        ]) await local.call('Input.dispatchKeyEvent',event);
    };
    // Prime the actual remote selection with A; B is copied locally immediately
    // before the first native key gesture, with no synchronization sleep.
    await local.evaluate(`window.__ivncWebRTC.sendDataChannelMessage('cw,'+btoa('OLD_A'))`);
    await waitFor('legacy clipboard applied', () => local.evaluate(`window.__clipUpdates.some(m=>m?.origin==='browser')`));
    await reset(); await writeLocal('NEW_B'); await paste();
    await waitFor('first paste B', async () => await value() === 'NEW_B');
    pass('first native paste uses newly copied text');
    await reset(); await writeLocal('same'); await paste(); await paste();
    await waitFor('rapid repeated text', async () => await value() === 'samesame');
    pass('rapid identical pastes both execute exactly once');
    await reset(); await writeLocal('A'); await paste(); await writeLocal('B'); await paste();
    await waitFor('A then B', async () => await value() === 'AB');
    pass('rapid different clipboard snapshots retain order');
    for (const mode of ['ctrl-shift-v','shift-insert']) {
        await reset(); await writeLocal(mode); await paste(mode);
        await waitFor(mode, async () => await value() === mode);
        pass(mode + ' captures current clipboard');
    }
    await remote.evaluate(`(() => { const t=document.getElementById('text');t.value='REMOTE_COPY';t.focus();t.select();
        window.__pasted=[];t.addEventListener('paste',e=>window.__pasted.push(e.clipboardData.getData('text/plain'))); })()`);
    await local.evaluate(`(() => {const rtc=window.__ivncWebRTC;window.__originalClipboard=rtc.onclipboardcontent;
        rtc.onclipboardcontent=(text,meta)=>{if(meta?.origin==='remote')setTimeout(()=>window.__originalClipboard(text,meta),150);else window.__originalClipboard(text,meta)};})()`);
    await writeLocal('STALE_LOCAL'); await copy(); await paste();
    await waitFor('immediate remote copy/paste', () => remote.evaluate(`window.__pasted.length===1&&window.__pasted[0]==='REMOTE_COPY'`));
    assert.equal(await value(),'REMOTE_COPY');
    await waitFor('remote copy reaches local', () => local.evaluate(`navigator.clipboard.readText().then(t=>t==='REMOTE_COPY')`));
    await local.evaluate('window.__ivncWebRTC.onclipboardcontent=window.__originalClipboard');
    pass('remote copy immediately followed by paste survives delayed clipboard reply');
    const large = '中文😀\n'.repeat(12000);
    await reset(); await writeLocal(large); await paste();
    await waitFor('large UTF-8 text', async () => await value() === large);
    pass('large Unicode clipboard crosses chunks and pipe capacity');
    const remoteCopies = await local.evaluate(`window.__clipUpdates.filter(m=>m?.origin==='remote').length`);
    await remote.evaluate(`document.getElementById('text').select()`); await copy();
    await waitFor('new remote clipboard notification', () => local.evaluate(`window.__clipUpdates.filter(m=>m?.origin==='remote').length>${remoteCopies}`));
    await waitFor('large remote text copied locally', () => local.evaluate(`navigator.clipboard.readText().then(t=>t===${JSON.stringify(large)})`));
    pass('large remote copy round trip decodes all base64 chunks');

    for (const type of ['keyDown','keyUp']) await local.call('Input.dispatchKeyEvent',{
        type,key:'F',code:'KeyF',windowsVirtualKeyCode:70,modifiers:10});
    await waitFor('fullscreen pointer lock', () => local.evaluate(`!!document.fullscreenElement&&!!document.pointerLockElement`));
    await local.call('Browser.grantPermissions', {origin:base,permissions:[]});
    await remote.evaluate(`(() => {const t=document.getElementById('text');t.value='REMOTE_DENIED_WRITE';t.focus();t.select()})()`);
    await copy('x');
    await waitFor('remote cut', async () => await value() === '');
    await waitFor('manual copy offer', () => local.evaluate(`!document.querySelector('.clipboard-assist')?.hidden`));
    await waitFor('copy controls release pointer lock', () => local.evaluate('!document.pointerLockElement'));
    const manualPoint = await local.evaluate(`(() => {const r=document.querySelector('.clipboard-manual').getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    for (const type of ['mousePressed','mouseReleased']) await local.call('Input.dispatchMouseEvent',{type,...manualPoint,button:'left',buttons:type==='mousePressed'?1:0,clickCount:1});
    assert.equal(await local.evaluate(`document.activeElement?.value`), 'REMOTE_DENIED_WRITE');
    await copy();
    await local.call('Browser.grantPermissions', {origin:base,permissions:['clipboardReadWrite','clipboardSanitizedWrite']});
    assert.equal(await local.evaluate('navigator.clipboard.readText()'),'REMOTE_DENIED_WRITE');
    const closePoint = await local.evaluate(`(() => {const r=document.querySelector('.clipboard-close').getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    for (const type of ['mousePressed','mouseReleased']) await local.call('Input.dispatchMouseEvent',{type,...closePoint,button:'left',buttons:type==='mousePressed'?1:0,clickCount:1});
    assert.equal(await local.evaluate(`document.activeElement?.id`),'overlayInput');
    assert.equal(await value(),'');
    await local.evaluate('document.exitFullscreen()');
    await waitFor('fullscreen exit', () => local.evaluate('!document.fullscreenElement'));
    pass('real remote cut survives denied local writes in fullscreen; native manual copy restores remote input focus');

    await reset(); await writeLocal('once'); await paste();
    await waitFor('once', async () => await value() === 'once');
    await local.evaluate(`(() => { const tx=window.__clipSent.map(m=>JSON.parse(m.slice(6))); const id=tx.filter(m=>m.type==='commit').at(-1).id;
        for(const m of tx.filter(m=>m.id===id))window.__ivncWebRTC.sendDataChannelMessage('paste,'+JSON.stringify(m)); })()`);
    await waitFor('duplicate rejection', () => local.evaluate(`window.__clipReplies.some(m=>m.data?.status==='duplicate')`));
    assert.equal(await value(),'once');
    pass('replayed transaction cannot paste twice');
    await local.evaluate(`(() => {const n=document.querySelector('.toolbar-list [data-app-id="builtin-chrome"]');n.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true}));})()`);
    await waitFor('rename input', () => local.evaluate(`document.activeElement?.classList.contains('toolbar-rename-input')`));
    const before = await local.evaluate('window.__clipSent.length');
    await writeLocal('local rename text'); await paste();
    assert.equal(await local.evaluate(`document.querySelector('.toolbar-rename-input').value`), 'local rename text');
    assert.equal(await local.evaluate('window.__clipSent.length'),before);
    assert.equal(await value(),'once');
    pass('native paste into toolbar rename stays local');
    // Reserve the old surface, then open another real Wayland window before commit.
    const interrupted = await local.evaluate(`(() => {const id=Math.max(...window.__clipSent.map(m=>JSON.parse(m.slice(6)).id||0))+1;
        window.__ivncWebRTC.sendDataChannelMessage('paste,'+JSON.stringify({type:'start',id,size:4,shortcut:'ctrl-v',target:0}));return id;})()`);
    await remote.call('Target.createTarget',{url:'about:blank',newWindow:true});
    await waitFor('new focused window', () => remote.evaluate('!document.hasFocus()'));
    await local.evaluate(`(() => {const rtc=window.__ivncWebRTC;rtc.sendDataChannelMessage('paste,'+JSON.stringify({type:'chunk',id:${interrupted},data:btoa('NOPE')}));rtc.sendDataChannelMessage('paste,'+JSON.stringify({type:'commit',id:${interrupted}}));})()`);
    await waitFor('focus cancellation', () => local.evaluate(`window.__clipReplies.some(m=>m.data?.id===${interrupted}&&m.data.status==='focus_changed')`));
    assert.equal(await value(),'once');
    pass('focus change during transfer rejects paste instead of targeting another window');
    console.log(`Clipboard smoke: ${passed} passed`);
} catch (error) {
    console.error(error); console.error(`Artifacts: ${work}`); process.exitCode = 1;
    for (const [name, peer] of [['local',local],['remote',remote]]) {
        if (!peer) continue;
        try {
            console.error(name, await peer.evaluate(`({focus:document.hasFocus(),active:document.activeElement?.outerHTML.slice(0,200),value:document.getElementById('text')?.value.slice(0,80),replies:window.__clipReplies,updates:window.__clipUpdates,requests:window.__clipSent?.map(m=>JSON.parse(m.slice(6))).filter(m=>m.type!=='chunk'),notice:document.querySelector('.upload-toast')?.textContent})`));
            const screenshot = await peer.call('Page.captureScreenshot');
            writeFileSync(path.join(work,name+'.png'),Buffer.from(screenshot.data,'base64'));
        } catch (_) {}
    }
} finally {
    for (const socket of connections) socket.close();
    try { await api('/api/apps/builtin-chrome/stop','POST'); } catch (_) {}
    for (const child of children.reverse()) {
        if (child.exitCode !== null || child.signalCode) continue;
        child.kill('SIGTERM');
        for (let i=0;i<50 && child.exitCode===null && !child.signalCode;i++) await delay(100);
        if (child.exitCode===null && !child.signalCode) child.kill('SIGKILL');
    }
    if (!process.env.KEEP_TMP && !process.exitCode) rmSync(work,{recursive:true,force:true});
}
