// Real browser permissions, trusted copy/paste and production clipboard UI.
// Does not grant blanket clipboard permissions or touch a running iVnc instance.
import assert from 'node:assert/strict';
import http from 'node:http';
import https from 'node:https';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-copy-ui-'));
const html = `<!doctype html><meta charset="utf-8"><style>
body{margin:0;background:#252a34;color:white}#remote{position:absolute;top:120px;left:20px;width:400px;height:240px}
#local{position:absolute;top:410px;left:20px;width:400px;height:120px}</style>
<div class="video-container"><textarea id="remote" readonly></textarea></div><textarea id="local" class="allow-native-input"></textarea>
<script type="module">
import {createClipboard,pasteShortcut} from '/clipboard.js';
import {createClipboardUI} from '/clipboard-ui.js';
import {installOverlayGuard} from '/overlay-manager.js';
import {setLang} from '/lib/i18n.js?v=1';
setLang('en');window.__IVNC_EMBED__=window.parent!==window;installOverlayGuard(()=>{});
const remote=document.getElementById('remote');
window.sent=[];window.errors=[];window.remoteClicks=0;window.active=true;let revision=0;
window.ui=createClipboardUI({getController:()=>window.controller,restoreFocus:()=>remote.focus()});
window.controller=createClipboard({webrtc:{_send_channel:{readyState:'open',bufferedAmount:0},sendDataChannelMessage:m=>{
sent.push(m);if(m.startsWith('paste,')){const p=JSON.parse(m.slice(6));if(p.type==='commit')queueMicrotask(()=>controller.message('paste_ack',{id:p.id,status:'applied'}));}}},
overlay:remote,imeTarget:()=>null,isActive:()=>window.active,getTarget:()=>1,report:e=>errors.push(e),onCopyState:s=>ui.update(s)});
controller.message('clipboard_caps',{version:1,enabled:true,max_bytes:16777216,chunk_bytes:12288});
remote.addEventListener('keydown',e=>{const shortcut=pasteShortcut(e);if(shortcut)controller.beginPaste(e,shortcut)});
remote.addEventListener('pointerdown',()=>window.remoteClicks++);
window.addEventListener('blur',()=>controller.suspend());
window.receive=text=>controller.receive(text,{revision:++revision,origin:'remote'});
window.setLang=setLang;window.ready=true;
</script>`;
function serve(request, response) {
    const url = new URL(request.url, 'http://localhost');
    if (url.searchParams.has('policy')) response.setHeader('Permissions-Policy','clipboard-write=()');
    if (url.searchParams.has('frame')) {
        response.setHeader('Content-Type','text/html');
        response.end('<script>window.overlayInteractions=0;window.addEventListener("message",e=>{if(e.data?.type==="ivnc-overlay-app-interaction")window.overlayInteractions++})</script>'+
            '<iframe style="position:fixed;inset:0;width:100%;height:100%;border:0" allow="clipboard-read; clipboard-write" src="/child"></iframe>');
    } else if (url.pathname.endsWith('.js')) {
        const allowed = new Set(['/clipboard.js','/clipboard-ui.js','/overlay-manager.js','/lib/i18n.js']);
        if (!allowed.has(url.pathname)) { response.writeHead(404); response.end(); return; }
        response.setHeader('Content-Type','text/javascript');
        response.end(readFileSync(path.join(root,'web/ivnc',url.pathname)));
    } else { response.setHeader('Content-Type','text/html'); response.end(html); }
}
const servers = [], pending = new Map();
let child, socket, sequence = 0, passed = 0, framed = false, browserLog = '';
const pass = name => { passed++; console.log('PASS '+name); };
const wait = async (name, fn) => {
    const deadline = Date.now()+20000; let error;
    while (Date.now()<deadline) { try {const r=await fn();if(r)return r;} catch(e){error=e;} await delay(50); }
    throw Error(`Timeout: ${name}${error?': '+error.message:''}`);
};
const call = (method, params = {}) => new Promise((resolve,reject) => {
    const id=++sequence, timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP timeout: '+method));},20000);
    pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));
});
async function evaluate(expression, parent = false) {
    if (framed && !parent) expression = `document.querySelector('iframe').contentWindow.eval(${JSON.stringify(expression)})`;
    const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:false});
    if(result.exceptionDetails)throw Error(JSON.stringify(result.exceptionDetails));return result.result.value;
}
async function visit(url, frame = false) {
    framed=frame;await call('Browser.resetPermissions');await call('Page.navigate',{url});
    await wait('clipboard UI',()=>evaluate('window.ready'));
    await call('Page.bringToFront');await click('#remote');
    // postMessage reaches the parent asynchronously after the initial remote click.
    if(frame)await wait('initial embedded interaction',()=>evaluate('window.overlayInteractions===1',true));
}
async function click(selector) {
    const p=await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return{x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    for(const type of ['mouseMoved','mousePressed','mouseReleased'])await call('Input.dispatchMouseEvent',{
        type,...p,button:type==='mouseMoved'?'none':'left',buttons:type==='mousePressed'?1:0,clickCount:1});
}
async function shortcut(letter) {
    const fields={key:letter.toLowerCase(),code:'Key'+letter,windowsVirtualKeyCode:letter.charCodeAt(0)};
    for(const e of [{type:'keyDown',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:2},
        {type:'keyDown',...fields,modifiers:2},{type:'keyUp',...fields,modifiers:2},
        {type:'keyUp',key:'Control',code:'ControlLeft',windowsVirtualKeyCode:17,modifiers:0}])await call('Input.dispatchKeyEvent',e);
}
const receive = text => evaluate(`receive(${JSON.stringify(text)})`);
const notice = () => wait('copy notice',()=>evaluate(`!document.querySelector('.clipboard-assist').hidden`));
async function verifyClipboard(text) {
    await evaluate(`(()=>{const t=document.getElementById('local');t.value='';t.focus()})()`);await shortcut('V');
    assert.equal(await evaluate(`document.getElementById('local').value`),text);
}
async function seedClipboard(text) {
    await evaluate(`(()=>{const t=document.getElementById('local');t.value=${JSON.stringify(text)};t.focus();t.select()})()`);
    await shortcut('C');await evaluate(`document.getElementById('remote').focus()`);
}
async function manualCopy(text) {
    await notice();await click('.clipboard-manual');
    assert.equal(await evaluate(`document.activeElement===document.querySelector('.clipboard-assist textarea')`),true);
    assert.equal(await evaluate(`document.activeElement.value`),text);
    await shortcut('C');await verifyClipboard(text);
}
try {
    execFileSync('openssl',['req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=ivnc-copy.test',
        '-keyout',path.join(work,'key.pem'),'-out',path.join(work,'cert.pem')],{stdio:'ignore'});
    const plain=http.createServer(serve),secure=https.createServer({key:readFileSync(path.join(work,'key.pem')),cert:readFileSync(path.join(work,'cert.pem'))},serve);
    for(const server of [plain,secure]){servers.push(server);await new Promise(r=>server.listen(0,'127.0.0.1',r));}
    const secureUrl=`https://127.0.0.1:${secure.address().port}/`,httpUrl=`http://ivnc-copy.test:${plain.address().port}/`;
    // Certificate bypass is confined to this temporary browser and local test cert.
    child=spawn(process.env.CHROME_BIN||'google-chrome',['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage',
        '--no-first-run','--no-default-browser-check','--disable-background-networking','--no-proxy-server','--password-store=basic',
        '--ignore-certificate-errors','--host-resolver-rules=MAP ivnc-copy.test 127.0.0.1',
        `--user-data-dir=${work}/profile`,'--remote-debugging-port=0','--window-size=1280,800','about:blank'],{stdio:['ignore','ignore','pipe']});
    child.stderr.on('data',data=>browserLog=(browserLog+data).slice(-6000));
    const debugPort=await wait('browser',()=>Number(readFileSync(path.join(work,'profile/DevToolsActivePort'),'utf8').split('\n')[0]));
    const target=(await(await fetch(`http://127.0.0.1:${debugPort}/json`)).json()).find(t=>t.type==='page');
    socket=new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve,reject)=>{socket.once('open',resolve);socket.once('error',reject)});
    socket.on('message',raw=>{const m=JSON.parse(raw),p=pending.get(m.id);if(!p)return;pending.delete(m.id);clearTimeout(p.timer);
        if(m.error)p.reject(Error(m.error.message));else p.resolve(m.result)});
    await visit(secureUrl);assert.equal(await evaluate('isSecureContext'),true);
    await receive('DEFAULT_PERMISSIONS');
    await wait('automatic copy completes',()=>evaluate('controller.copySnapshot()===null'));
    await verifyClipboard('DEFAULT_PERMISSIONS');pass('HTTPS automatically copies with default permissions and no synthetic user gesture');

    await visit(secureUrl);await call('Browser.grantPermissions',{origin:secureUrl,permissions:[]});
    await receive('DENIED_PERMISSION');await manualCopy('DENIED_PERMISSION');
    assert.equal(await evaluate(`sent.some(m=>m.startsWith('paste,'))`),false);
    pass('denied write permissions fall back to trusted native copy without remote input');

    await visit(secureUrl);await evaluate(`window.originalWrite=navigator.clipboard.writeText.bind(navigator.clipboard);
        navigator.clipboard.writeText=()=>Promise.reject(new DOMException('Gesture required','NotAllowedError'))`);
    await receive('CLICK_RETRY');await notice();
    await evaluate('navigator.clipboard.writeText=window.originalWrite');await click('.clipboard-copy');
    await wait('manual write success',()=>evaluate(`document.querySelector('.clipboard-assist-message').textContent.includes('Copied to')`));
    assert.equal(await evaluate('document.activeElement.id'),'remote');
    await verifyClipboard('CLICK_RETRY');pass('explicit retry invokes the real API in a trusted click and preserves the snapshot through focus changes');

    await visit(httpUrl);assert.equal(await evaluate('isSecureContext'),false);assert.equal(await evaluate('!!navigator.clipboard'),false);
    await receive('HTTP_MANUAL');await manualCopy('HTTP_MANUAL');pass('ordinary HTTP copies via visible native selection without Clipboard API');

    await visit(httpUrl);await seedClipboard('OLD_BEFORE_EMPTY');await receive('');await manualCopy('');
    pass('native manual copy clears an old clipboard for an empty remote string');

    await visit(httpUrl);const unicode='中文😀\n'.repeat(12000);await receive(unicode);await manualCopy(unicode);
    pass('large Unicode text survives manual selection and native clipboard');

    await visit(httpUrl);await receive('A');await receive('LATEST_B');await notice();await click('.clipboard-manual');
    assert.equal(await evaluate(`document.querySelector('.clipboard-assist textarea').value`),'LATEST_B');
    await receive('BACKGROUND_C');assert.equal(await evaluate(`document.querySelector('.clipboard-assist textarea').value`),'LATEST_B');
    await evaluate(`setLang('zh')`);assert.equal(await evaluate(`document.querySelector('.clipboard-assist textarea').value`),'LATEST_B');
    await evaluate(`window.minimized=0;window.__ivncOverlayControllers=[{isOpen:()=>true,isPinned:()=>false,contains:()=>false,minimize:()=>window.minimized++}]`);
    const clicks=await evaluate('remoteClicks');await click('#remote');
    assert.equal(await evaluate(`document.querySelector('.clipboard-assist').hidden`),true);
    assert.equal(await evaluate('remoteClicks'),clicks);assert.equal(await evaluate('minimized'),0);
    pass('latest offer is stable during manual copying, language changes and outside dismissal do not affect remote input or other overlays');

    await visit(secureUrl+'?policy=deny');await receive('POLICY');await notice();
    assert.match(await evaluate(`document.querySelector('.clipboard-assist-message').textContent`),/policy/);
    await manualCopy('POLICY');pass('Permissions-Policy restriction is diagnosed and native copy still works');

    await visit(secureUrl+'?frame=1',true);await receive('IFRAME_DEFAULT');
    await wait('iframe copy',()=>evaluate('controller.copySnapshot()===null'));await verifyClipboard('IFRAME_DEFAULT');
    pass('active same-origin embedded view copies with its delegated clipboard policy');

    await visit(secureUrl+'?frame=1&policy=deny',true);await receive('IFRAME_POLICY');
    const interactions=await evaluate('window.overlayInteractions',true);await manualCopy('IFRAME_POLICY');
    assert.equal(await evaluate('window.overlayInteractions',true),interactions);
    pass('parent policy restrictions retain the manual copy route inside embedded views');

    await visit(httpUrl);
    await evaluate(`document.getElementById('remote').addEventListener('click',async()=>{
        await document.querySelector('.video-container').requestFullscreen();document.getElementById('remote').focus();
    },{once:true})`);
    await click('#remote');await wait('embedded fullscreen',()=>evaluate('!!document.fullscreenElement'));
    await receive('FULLSCREEN_MANUAL');await notice();
    assert.equal(await evaluate(`document.fullscreenElement.contains(document.querySelector('.clipboard-assist'))`),true);
    await click('.clipboard-manual');assert.equal(await evaluate('document.activeElement.value'),'FULLSCREEN_MANUAL');
    await shortcut('C');await evaluate('document.exitFullscreen()');
    await wait('fullscreen exit',()=>evaluate(`!document.fullscreenElement&&document.querySelector('.clipboard-assist').parentElement===document.body`));
    assert.equal(await evaluate('document.activeElement.value'),'FULLSCREEN_MANUAL');
    await verifyClipboard('FULLSCREEN_MANUAL');pass('manual copy remains visible in a fullscreen video container and preserves focus on exit');

    await visit(httpUrl);await receive('DISCONNECT');await notice();await evaluate(`controller.disconnected()`);
    assert.equal(await evaluate(`document.querySelector('.clipboard-assist').hidden`),true);
    assert.equal(await evaluate('controller.copySnapshot()'),null);pass('disconnect clears pending UI and text');
    console.log(`Clipboard UI smoke: ${passed} passed`);
} catch(error) {
    process.exitCode=1;console.error(error);console.error(browserLog);console.error('Artifacts: '+work);
    if(socket?.readyState===WebSocket.OPEN)try{const shot=await call('Page.captureScreenshot');writeFileSync(path.join(work,'failure.png'),Buffer.from(shot.data,'base64'));}catch{}
} finally {
    for(const p of pending.values())clearTimeout(p.timer);socket?.close();
    if(child){child.kill('SIGTERM');for(let i=0;i<30&&child.exitCode===null&&!child.signalCode;i++)await delay(100);
        if(child.exitCode===null&&!child.signalCode){child.kill('SIGKILL');await delay(300);}}
    for(const server of servers){server.closeAllConnections();await new Promise(r=>server.close(r));}
    if(!process.exitCode&&!process.env.KEEP_TMP)rmSync(work,{recursive:true,force:true});
}
