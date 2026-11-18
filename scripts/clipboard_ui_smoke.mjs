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
const html = readFileSync(path.join(root,'scripts/fixtures/clipboard-ui.html'),'utf8');
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
async function visit(url, frame = false, activate = true) {
    framed=frame;await call('Browser.resetPermissions');await call('Page.navigate',{url});
    await wait('clipboard UI',()=>evaluate('window.ready'));
    await call('Page.bringToFront');
    if(activate)await click('#remote');else await evaluate(`document.getElementById('remote').focus()`);
    // postMessage reaches the parent asynchronously after the initial remote click.
    if(frame&&activate)await wait('initial embedded interaction',()=>evaluate('window.overlayInteractions===1',true));
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
const blockLegacy = () => evaluate(`document.execCommand=()=>false`);
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
    await evaluate(`document.getElementById('remote').value='UNCHANGED_LOCAL_SELECTION';document.getElementById('remote').setSelectionRange(2,8)`);
    await receive('HTTP_AUTO');
    await wait('HTTP automatic copy',()=>evaluate('controller.copySnapshot()===null'));
    assert.equal(await evaluate('document.activeElement.id'),'remote');
    assert.deepEqual(await evaluate(`({value:remote.value,start:remote.selectionStart,end:remote.selectionEnd})`),{value:'UNCHANGED_LOCAL_SELECTION',start:2,end:8});
    await verifyClipboard('HTTP_AUTO');
    pass('ordinary HTTP automatically copies plaintext during a real user activation without changing focus');

    await visit(httpUrl);await seedClipboard('STALE_BEFORE_REMOTE');await visit(httpUrl,false,false);
    assert.equal(await evaluate('navigator.userActivation.hasBeenActive'),false);await shortcut('C');
    await evaluate(`new Promise(resolve=>setTimeout(()=>{receive('HTTP_ASYNC_COPY');resolve()},250))`);
    await wait('asynchronous HTTP shortcut copy',()=>evaluate('controller.copySnapshot()===null'));
    assert.equal(await evaluate('document.activeElement.id'),'remote');await verifyClipboard('HTTP_ASYNC_COPY');
    pass('trusted Ctrl+C remains usable after asynchronous remote delivery and replaces an old clipboard');

    await visit(httpUrl);await seedClipboard('STALE_BEFORE_PASTE');await visit(httpUrl,false,false);
    await shortcut('C');await shortcut('V');
    assert.equal(await evaluate(`sent.some(m=>m.startsWith('paste,'))`),false);
    await receive('HTTP_COPY_THEN_PASTE');
    await wait('remote paste commit',()=>evaluate(`sent.some(m=>m.startsWith('paste,')&&JSON.parse(m.slice(6)).type==='commit')`));
    assert.equal(await evaluate(`sent.filter(m=>m.startsWith('paste,')).map(m=>JSON.parse(m.slice(6))).filter(m=>m.type==='chunk').map(m=>atob(m.data)).join('')`),'HTTP_COPY_THEN_PASTE');
    assert.equal(await evaluate(`sent.some(m=>m.startsWith('paste,')&&JSON.parse(m.slice(6)).type==='cancel')`),false);
    assert.deepEqual(await evaluate('errors'),[]);await verifyClipboard('HTTP_COPY_THEN_PASTE');
    pass('HTTP copy followed immediately by paste waits for current remote content and compatibility copy does not cancel the transaction');

    await visit(httpUrl,false,false);
    await evaluate(`window.copyAttempts=0;window.nativeExec=document.execCommand.bind(document);document.execCommand=c=>{copyAttempts++;return nativeExec(c)}`);
    await receive('HTTP_CLICK');await notice();
    assert.equal(await evaluate('navigator.userActivation.isActive'),false);
    assert.equal(await evaluate('copyAttempts'),0);
    assert.equal(await evaluate(`document.querySelector('.clipboard-copy').hidden`),false);
    await click('.clipboard-copy');
    await wait('HTTP click success',()=>evaluate(`document.querySelector('.clipboard-assist-message').textContent.includes('Copied to')`));
    assert.equal(await evaluate('copyAttempts'),1);assert.equal(await evaluate('document.activeElement.id'),'remote');
    await verifyClipboard('HTTP_CLICK');pass('HTTP with no user activation offers a single trusted click that copies directly');

    await visit(httpUrl);await wait('user activation expires',()=>evaluate('!navigator.userActivation.isActive'));
    await receive('HTTP_LATE');await notice();
    assert.equal(await evaluate(`document.querySelector('.clipboard-assist-message').textContent.includes('Click')`),true);
    await click('.clipboard-copy');
    await wait('late HTTP retry succeeds',()=>evaluate(`document.querySelector('.clipboard-assist-message').textContent.includes('Copied to')`));
    await verifyClipboard('HTTP_LATE');pass('late remote delivery never assumes a fixed activation timeout and can be copied with one click');

    await visit(httpUrl);
    await evaluate('receiveImage()');
    await notice();assert.equal(await evaluate(`document.querySelector('.clipboard-copy').hidden&&document.querySelector('.clipboard-manual').hidden`),true);
    assert.match(await evaluate(`document.querySelector('.clipboard-assist-message').textContent`),/cannot write images.*HTTPS/);
    await wait('HTTP image preview',()=>evaluate(`document.querySelector('.clipboard-image').naturalWidth===3`));
    assert.equal(await evaluate(`document.querySelector('.clipboard-download').download`),'ivnc-clipboard.png');
    assert.equal(await evaluate(`document.querySelector('.clipboard-download').hidden`),false);
    await call('Browser.setDownloadBehavior',{behavior:'allow',downloadPath:path.join(work,'downloads')});
    await click('.clipboard-download');
    const downloaded=await wait('PNG download',()=>readFileSync(path.join(work,'downloads/ivnc-clipboard.png')));
    assert.equal(downloaded.readUInt32BE(16),3);assert.equal(downloaded.readUInt32BE(20),2);
    await evaluate(`window.previewMenuPrevented=null;document.addEventListener('contextmenu',e=>queueMicrotask(()=>window.previewMenuPrevented=e.defaultPrevented),{capture:true})`);
    const imagePoint=await evaluate(`(()=>{const r=document.querySelector('.clipboard-image').getBoundingClientRect();return{x:r.left+r.width/2,y:r.top+r.height/2}})()`);
    for(const type of ['mousePressed','mouseReleased'])await call('Input.dispatchMouseEvent',{type,...imagePoint,button:'right',buttons:type==='mousePressed'?2:0,clickCount:1});
    await wait('native image context menu',()=>evaluate('window.previewMenuPrevented===false'));
    await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    pass('HTTP images expose a full-resolution PNG preview, download and an unblocked native browser image menu');

    await visit(httpUrl);await evaluate('receiveImage(true)');await notice();
    await wait('partial image copy',()=>evaluate(`document.querySelector('.clipboard-assist-message').textContent.includes('Only text was copied')`));
    assert.equal(await evaluate(`document.querySelector('.clipboard-copy').textContent`),'Copy text');
    await wait('mixed content preview',()=>evaluate(`document.querySelector('.clipboard-image').naturalWidth===3`));
    pass('HTTP mixed content reports text-only success and retains the actual image');

    await visit(secureUrl);
    await evaluate(`navigator.clipboard.write=()=>Promise.reject(new DOMException('Gesture required','NotAllowedError'))`);
    await evaluate('receiveImage()');await notice();
    await wait('HTTPS image preview',()=>evaluate(`document.querySelector('.clipboard-image').naturalWidth===3`));
    await evaluate(`delete navigator.clipboard.write`);await click('.clipboard-copy');
    await wait('image write success',()=>evaluate(`document.querySelector('.clipboard-assist-message').textContent.includes('Copied to')`));
    await evaluate(`document.getElementById('local').focus()`);await shortcut('V');
    await wait('native image paste outside the remote view',()=>evaluate('window.pastedImage'));
    assert.deepEqual(await evaluate('window.pastedImage'),{width:3,height:2,pixel:[51,85,170,255]});
    pass('HTTPS trusted image retry pastes the original dimensions and pixels into an independent native input');

    await visit(httpUrl);
    await evaluate(`window.revoked=[];const revoke=URL.revokeObjectURL.bind(URL);URL.revokeObjectURL=url=>{revoked.push(url);revoke(url)}`);
    await evaluate('receiveImage()');await notice();
    await wait('first preview',()=>evaluate(`document.querySelector('.clipboard-image').naturalWidth===3`));
    const firstPreview=await evaluate(`document.querySelector('.clipboard-image').src`);
    await evaluate('receiveImage()');await notice();
    await wait('old blob released',()=>evaluate(`revoked.length===1`));
    assert.equal(await evaluate(`revoked[0]`),firstPreview);
    await click('.clipboard-close');assert.equal(await evaluate('revoked.length'),2);
    assert.equal(await evaluate(`document.querySelector('.clipboard-image').hasAttribute('src')`),false);
    pass('replacing and dismissing images revoke blob URLs and release the preview');

    await visit(httpUrl);await seedClipboard('OLD_BEFORE_AUTO_EMPTY');await receive('');
    await wait('HTTP empty copy',()=>evaluate('controller.copySnapshot()===null'));await verifyClipboard('');
    pass('HTTP compatibility copy clears an old clipboard for an empty remote string');

    await visit(httpUrl);const autoUnicode='中文😀\n'.repeat(12000);await receive(autoUnicode);
    await wait('HTTP Unicode copy',()=>evaluate('controller.copySnapshot()===null'));await verifyClipboard(autoUnicode);
    pass('HTTP compatibility copy preserves large Unicode text');

    await visit(httpUrl);await blockLegacy();
    await receive('HTTP_MANUAL');await manualCopy('HTTP_MANUAL');pass('ordinary HTTP copies via visible native selection without Clipboard API');

    await visit(httpUrl);await blockLegacy();await seedClipboard('OLD_BEFORE_EMPTY');await receive('');await manualCopy('');
    pass('native manual copy clears an old clipboard for an empty remote string');

    await visit(httpUrl);await blockLegacy();const unicode='中文😀\n'.repeat(12000);await receive(unicode);await manualCopy(unicode);
    pass('large Unicode text survives manual selection and native clipboard');

    await visit(httpUrl);await blockLegacy();await receive('A');await receive('LATEST_B');await notice();await click('.clipboard-manual');
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

    await visit(httpUrl+'?policy=deny');await receive('HTTP_POLICY');await notice();
    assert.match(await evaluate(`document.querySelector('.clipboard-assist-message').textContent`),/policy/);
    assert.equal(await evaluate(`document.querySelector('.clipboard-copy').hidden`),true);
    await manualCopy('HTTP_POLICY');pass('HTTP compatibility copy respects an explicit clipboard-write policy and retains native manual selection');

    await visit(secureUrl+'?frame=1',true);await receive('IFRAME_DEFAULT');
    await wait('iframe copy',()=>evaluate('controller.copySnapshot()===null'));await verifyClipboard('IFRAME_DEFAULT');
    pass('active same-origin embedded view copies with its delegated clipboard policy');

    await visit(secureUrl+'?frame=1&policy=deny',true);await receive('IFRAME_POLICY');
    const interactions=await evaluate('window.overlayInteractions',true);await manualCopy('IFRAME_POLICY');
    assert.equal(await evaluate('window.overlayInteractions',true),interactions);
    pass('parent policy restrictions retain the manual copy route inside embedded views');

    await visit(httpUrl);await blockLegacy();
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

    await visit(httpUrl);await blockLegacy();await receive('DISCONNECT');await notice();await evaluate(`controller.disconnected()`);
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
