// Production bundle + real Chromium + isolated PTY. Build web/ivnc and ivnc first.
// IVNC_BIN=target/debug/ivnc node scripts/terminal_smoke.mjs
// KEEP_TMP=1 preserves logs on success; failures also retain a diagnostic screenshot.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-terminal-'));
const binary = path.resolve(process.env.IVNC_BIN || path.join(root, 'target/debug/ivnc'));
async function freePort() {
    const server = net.createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve)); return port;
}
const port = await freePort(), debugPort = await freePort(), base = `http://127.0.0.1:${port}`;
const env = { ...process.env, IVNC_HOME: path.join(work, 'ivnc'),
    XDG_CONFIG_HOME: path.join(work, 'config'), XDG_CACHE_HOME: path.join(work, 'cache'),
    XDG_DATA_HOME: path.join(work, 'data'), XDG_RUNTIME_DIR: path.join(work, 'run') };
for (const key of ['IVNC_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_DATA_HOME', 'XDG_RUNTIME_DIR']) mkdirSync(env[key], {mode:0o700});
// Early output catches the old 300ms automatic clear. Avoid user shell startup/history files.
const shell = path.join(work, 'test-shell');
writeFileSync(shell, '#!/bin/sh\nprintf "IVNC_START_A\\nIVNC_START_B\\n"\nexport HISTFILE=/dev/null INPUTRC=/dev/null LC_ALL=C.UTF-8\nPS1="ivnc-test# " exec /bin/bash --noprofile --norc -i\n', {mode:0o700});
const config = path.join(work, 'ivnc.toml');
writeFileSync(config, `[terminal]\nshell = ${JSON.stringify(shell)}\ncwd = ${JSON.stringify(work)}\n[audio]\nenabled = false\nsample_rate = 48000\nchannels = 2\nbitrate = 128000\n`);
const children = [];
let socket, call, evaluate, passed = 0;
function launch(command, args, name, extra = {}) {
    const fd = openSync(path.join(work, name + '.log'), 'a', 0o600);
    const child = spawn(command, args, {stdio:['ignore', fd, fd], ...extra});
    closeSync(fd); children.push(child); return child;
}
async function waitFor(name, fn, timeout = 30000) {
    const end = Date.now() + timeout; let error;
    while (Date.now() < end) {
        try { const value = await fn(); if (value) return value; } catch (e) { error = e; }
        await delay(50);
    }
    throw Error(`Timeout: ${name}${error ? ': ' + error.message : ''}`);
}
const pass = name => { ++passed; console.log(`PASS ${name}`); };
const rowsExpression = `[...document.querySelectorAll('.xterm-rows > div')].map(n=>n.textContent.trimEnd())`;
const rows = () => evaluate(rowsExpression);
const expectLine = text => waitFor(`rendered line ${text}`, async () => (await rows()).includes(text));
const diagnostics = `(() => {
    const rect=n=>{const r=n.getBoundingClientRect();return {left:r.left,top:r.top,right:r.right,bottom:r.bottom,width:r.width,height:r.height}};
    const screen=document.querySelector('.xterm-screen'),body=document.getElementById('web-terminal-body');
    return {rows:${rowsExpression},styles:document.querySelectorAll('#ivnc-terminal-styles').length,
        screen:screen&&rect(screen),body:body&&rect(body),
        measures:[...document.querySelectorAll('.xterm-char-measure-element')].map(n=>{
            const s=getComputedStyle(n);return {text:n.textContent,visibility:s.visibility,position:s.position,display:s.display,width:n.offsetWidth};
        }),resizes:window.__terminalSent?.filter(m=>m.type==='resize'),remoteInput:window.__remoteInput,
        sockets:window.__terminalSockets?.map(s=>s.readyState),sameTerminal:window.__savedTerminal===document.querySelector('.xterm'),
        input:window.__terminalSent?.filter(m=>m.type==='input').slice(-12),active:document.activeElement?.outerHTML,
        composition:document.querySelector('.composition-view')?.outerHTML};
})()`;
async function assertLayout() {
    return waitFor('terminal fits with hidden but measurable helpers', async () => {
        const d = await evaluate(diagnostics), last = d.resizes?.at(-1);
        assert.equal(d.styles, 1);
        assert(d.measures.length >= 4);
        for (const m of d.measures) {
            assert.equal(m.visibility, 'hidden'); assert.equal(m.position, 'absolute');
            assert.notEqual(m.display, 'none'); if (m.text) assert(m.width > 0);
        }
        assert(last?.cols >= 20 && last?.rows >= 5);
        assert.equal(d.rows.length, last.rows);
        assert(d.screen.width > 0 && d.screen.height > 0);
        assert(d.screen.left >= d.body.left - 1 && d.screen.right <= d.body.right + 1, JSON.stringify(d));
        assert(d.screen.top >= d.body.top - 1 && d.screen.bottom <= d.body.bottom + 1, JSON.stringify(d));
        return d;
    });
}
async function key(key, code, keyCode, modifiers = 0, text) {
    await call('Input.dispatchKeyEvent', {type:'keyDown', key, code, windowsVirtualKeyCode:keyCode, modifiers, ...(text ? {text} : {})});
    await call('Input.dispatchKeyEvent', {type:'keyUp', key, code, windowsVirtualKeyCode:keyCode, modifiers});
}
const enter = () => key('Enter', 'Enter', 13, 0, '\r');
const focus = () => evaluate(`document.querySelector('.xterm-helper-textarea').focus()`);
async function command(text) {
    await focus(); await call('Input.insertText', {text}); await enter();
}
async function shortcut(letter, shift = false) {
    const modifiers = 2 | (shift ? 8 : 0);
    await key(shift ? letter.toUpperCase() : letter, 'Key' + letter.toUpperCase(), letter.toUpperCase().charCodeAt(0), modifiers);
}
async function wheel(deltaY) {
    const p = await evaluate(`(() => {const r=document.querySelector('.xterm-screen').getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    await call('Input.dispatchMouseEvent', {type:'mouseWheel', ...p, deltaX:0, deltaY});
}
async function scrollTo(text, deltaY) {
    // xterm normalizes a wheel event to a few lines, even for a large delta.
    for (let i=0;i<80;i++) {
        if ((await rows()).includes(text)) return;
        await wheel(deltaY); await delay(30);
    }
    await expectLine(text);
}
try {
    launch(binary, ['--config',config,'--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false',
        '--proxy-autostart','false','--width','800','--height','600'], 'server', {env,cwd:root});
    await waitFor('server', async () => (await fetch(base + '/ui-config')).ok);
    launch(process.env.CHROME_BIN || 'google-chrome', ['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
        '--no-first-run','--no-default-browser-check','--disable-background-networking','--autoplay-policy=no-user-gesture-required',
        `--user-data-dir=${path.join(work,'browser')}`,`--remote-debugging-port=${debugPort}`,'--window-size=1280,800',base], 'browser');
    const page = await waitFor('CDP target', async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(t => t.type === 'page'));
    socket = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve,reject) => {socket.once('open',resolve);socket.once('error',reject);});
    const pending = new Map(); let sequence = 0;
    socket.on('message', raw => {
        const msg=JSON.parse(raw), p=pending.get(msg.id); if (!p) return;
        pending.delete(msg.id); clearTimeout(p.timer);
        if (msg.error) p.reject(Error(msg.error.message)); else p.resolve(msg.result);
    });
    socket.on('close', () => {for (const p of pending.values()) {clearTimeout(p.timer);p.reject(Error('CDP closed'));} pending.clear();});
    call = (method,params={}) => new Promise((resolve,reject) => {
        const id=++sequence, timer=setTimeout(()=>{pending.delete(id);reject(Error(`CDP timeout: ${method}`));},20000);
        pending.set(id,{resolve,reject,timer}); socket.send(JSON.stringify({id,method,params}));
    });
    evaluate = async expression => {
        const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true});
        if (r.exceptionDetails) throw Error(JSON.stringify(r.exceptionDetails)); return r.result.value;
    };
    await call('Page.enable');
    await call('Browser.grantPermissions',{origin:base,permissions:['clipboardReadWrite','clipboardSanitizedWrite']});
    const captureTerminal = `(() => {
        if (window.__terminalSockets) return;
        window.__terminalSockets=[];window.__terminalSent=[];window.__terminalOutput=[];
        const OriginalWebSocket=window.WebSocket;
        window.WebSocket=class extends OriginalWebSocket {
            constructor(...args) {
                super(...args);
                if (String(args[0]).endsWith('/terminal/ws')) {
                    window.__terminalSockets.push(this);
                    const send=this.send.bind(this);this.send=data=>{window.__terminalSent.push(JSON.parse(data));return send(data)};
                    this.addEventListener('message',event=>{const m=JSON.parse(event.data);if(m.type==='output')window.__terminalOutput.push(atob(m.data));});
                }
            }
        };
    })()`;
    await call('Page.addScriptToEvaluateOnNewDocument',{source:captureTerminal});
    await waitFor('desktop',()=>evaluate(`!!document.getElementById('terminal-btn') && window.__ivncWebRTC?._send_channel?.readyState==='open'`),60000);
    await evaluate(captureTerminal);
    await evaluate(`(() => {window.__remoteInput=[];const rtc=window.__ivncWebRTC,send=rtc.sendDataChannelMessage.bind(rtc);
        rtc.sendDataChannelMessage=m=>{if(/^(kd|ku|u),/.test(m))window.__remoteInput.push(m);return send(m)};document.getElementById('terminal-btn').click();})()`);
    await expectLine('IVNC_START_B');
    await assertLayout();
    assert(!(await rows()).some(line => /^(.)\1{15,}$/.test(line)));
    pass('first open loads complete styles and keeps measurement characters out of the terminal');
    await delay(650);
    await expectLine('IVNC_START_A'); await expectLine('IVNC_START_B');
    pass('real PTY output arriving at startup survives beyond the former clear timer');

    const hashes = '#'.repeat(32);
    await command(`printf '%s\\n' '${hashes}'`); await expectLine(hashes);
    assert((await evaluate('window.__terminalOutput.join("")')).includes(hashes));
    pass('a command deliberately printing repeated characters is displayed intact');
    await command("printf '\\033[1mBOLD\\033[0m \\033[3mITALIC\\033[0m 中文\\n'");
    await expectLine('BOLD ITALIC 中文');
    await assertLayout();
    pass('bold, italic and Unicode output do not expose font measurement nodes');

    await evaluate(`window.__savedTerminal=document.querySelector('.xterm');window.__savedSocket=window.__terminalSockets[0]`);
    for (let i=0;i<5;i++) {
        await evaluate(`document.getElementById('web-terminal-minimize').click()`);
        await evaluate(`document.getElementById('terminal-btn').click()`);
    }
    await assertLayout(); await expectLine('IVNC_START_A');
    assert(await evaluate(`window.__savedTerminal===document.querySelector('.xterm') && window.__terminalSockets.length===1 && window.__savedSocket.readyState===1`));
    pass('reopening preserves the terminal, PTY connection, history and a single stylesheet');

    for (const [width,height,dpr] of [[640,480,1],[1001,701,2],[1280,800,1]]) {
        await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:dpr,mobile:false});
        await delay(250); await assertLayout();
    }
    await evaluate(`window.postMessage({type:'setScaleLocally',value:true},location.origin);window.postMessage({type:'setManualResolution',width:1920,height:1080},location.origin)`);
    await waitFor('scaled modal',()=>evaluate(`Number(getComputedStyle(document.documentElement).getPropertyValue('--ivnc-modal-scale'))<0.9`));
    await assertLayout();
    await evaluate(`window.postMessage({type:'resetResolutionToWindow'},location.origin)`);
    await waitFor('normal modal scale',()=>evaluate(`Number(getComputedStyle(document.documentElement).getPropertyValue('--ivnc-modal-scale'))===1`));
    await assertLayout();
    pass('narrow, HiDPI and manually scaled desktops keep the terminal inside its body');

    await command('read -r answer; printf "IME:%s\\n" "$answer"');
    await call('Input.imeSetComposition',{text:'中文输入',selectionStart:4,selectionEnd:4});
    await waitFor('visible composition',()=>evaluate(`getComputedStyle(document.querySelector('.composition-view')).display==='block' && document.querySelector('.composition-view').textContent==='中文输入'`));
    await call('Input.insertText',{text:'中文输入'});
    await waitFor('IME commit delivered',()=>evaluate(`window.__terminalSent.some(m=>m.type==='input' && m.data==='中文输入')`));
    await enter();
    await expectLine('IME:中文输入');
    assert.equal(await evaluate(`getComputedStyle(document.querySelector('.composition-view')).display`),'none');
    pass('IME preedit is visible while composing and Chinese input commits once');

    for (const shift of [false,true]) {
        const value=shift?'SHIFT_PASTE_中文':'NATIVE_PASTE_中文';
        await command('read -r answer; printf "PASTE:%s\\n" "$answer"');
        await evaluate(`navigator.clipboard.writeText(${JSON.stringify(value)})`);
        // Ctrl+V is the terminal's literal-next key on Linux. Shift+Insert
        // invokes the browser's native paste path without changing that binding.
        if (shift) await shortcut('v',true);
        else await key('Insert','Insert',45,8);
        await waitFor('paste delivered',()=>evaluate(`window.__terminalSent.some(m=>m.type==='input' && m.data.includes(${JSON.stringify(value)}))`));
        await enter(); await expectLine('PASTE:'+value);
    }
    pass('native Shift+Insert paste and Ctrl+Shift+V each reach the shell once');

    const copyText='COPY_FROM_TERMINAL';
    await command(`printf '${copyText}\\n'`); await expectLine(copyText);
    const selection = await evaluate(`(() => {
        const row=[...document.querySelectorAll('.xterm-rows > div')].find(n=>n.textContent.trimEnd()===${JSON.stringify(copyText)});
        const r=row.getBoundingClientRect(),cols=window.__terminalSent.filter(m=>m.type==='resize').at(-1).cols;
        return {x:r.left+1,y:r.top+r.height/2,end:r.left+r.width/cols*${copyText.length}+1};
    })()`);
    await call('Input.dispatchMouseEvent',{type:'mousePressed',x:selection.x,y:selection.y,button:'left',buttons:1,clickCount:1});
    await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:selection.end,y:selection.y,button:'left',buttons:1});
    await call('Input.dispatchMouseEvent',{type:'mouseReleased',x:selection.end,y:selection.y,button:'left',buttons:0,clickCount:1});
    await evaluate(`navigator.clipboard.writeText('before-copy')`);
    await shortcut('c',true);
    await waitFor('copied selection',async()=>await evaluate('navigator.clipboard.readText()')===copyText);
    assert.deepEqual(await evaluate('window.__remoteInput'),[]);
    pass('mouse selection and Ctrl+Shift+C copy accurately without sending desktop input');

    await command('for i in {1..100}; do printf "HISTORY_%03d\\n" "$i"; done');
    await expectLine('HISTORY_100'); await scrollTo('IVNC_START_A',-120);
    const scrolled=await rows();
    await evaluate(`document.getElementById('web-terminal-minimize').click();document.getElementById('terminal-btn').click()`);
    await delay(200); assert.deepEqual(await rows(),scrolled);
    await scrollTo('HISTORY_100',120); await assertLayout();
    pass('scrollback and its position survive reopening, and the last line remains reachable');

    await evaluate('window.__savedSocket.close()');
    await waitFor('replacement connection',()=>evaluate('window.__terminalSockets.length===2 && window.__terminalSockets[1].readyState===1'));
    await expectLine('IVNC_START_B'); await delay(650); await expectLine('HISTORY_100'); await assertLayout();
    assert(await evaluate(`window.__savedTerminal===document.querySelector('.xterm')`));
    pass('reconnection reuses the renderer and stylesheet without clearing earlier output');

    // Force xterm's supported DOM-measurement fallback, which adds a W x 32 probe.
    await call('Page.addScriptToEvaluateOnNewDocument',{source:'window.OffscreenCanvas=undefined;'});
    await evaluate('window.__terminalReloading=true'); await call('Page.reload');
    await waitFor('new page',()=>evaluate(`!window.__terminalReloading && !!document.getElementById('terminal-btn')`));
    await evaluate(`document.getElementById('terminal-btn').click()`); await expectLine('IVNC_START_B');
    const fallback=await assertLayout();
    assert(fallback.measures.some(m=>m.text==='W'.repeat(32)));
    assert(!(await rows()).includes('W'.repeat(32)));
    pass('reload and DOM measurement fallback keep the W probe hidden and measurable');
    console.log(`Terminal smoke: ${passed} passed`);
} catch (error) {
    console.error(error); console.error(`Artifacts: ${work}`); process.exitCode=1;
    if (evaluate) {
        try {
            writeFileSync(path.join(work,'diagnostics.json'),JSON.stringify(await evaluate(diagnostics),null,2));
            const screenshot=await call('Page.captureScreenshot');
            writeFileSync(path.join(work,'failure.png'),Buffer.from(screenshot.data,'base64'));
        } catch (_) {}
    }
} finally {
    socket?.close();
    for (const child of children.reverse()) {
        if (child.exitCode!==null || child.signalCode) continue;
        child.kill('SIGTERM');
        for (let i=0;i<50 && child.exitCode===null && !child.signalCode;i++) await delay(100);
        if (child.exitCode===null && !child.signalCode) child.kill('SIGKILL');
    }
    if (!process.env.KEEP_TMP && !process.exitCode) rmSync(work,{recursive:true,force:true});
}
