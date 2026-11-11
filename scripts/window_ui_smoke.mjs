// Real pointer/keyboard, production UI, isolated server/PTY and delayed requests.
// Build web/ivnc and ivnc first. IVNC_BIN=target/debug/ivnc node scripts/window_ui_smoke.mjs
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
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-window-ui-'));
const binary = path.resolve(process.env.IVNC_BIN || path.join(root, 'target/debug/ivnc'));
async function freePort() {
    const server = net.createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve)); return port;
}
const port = await freePort(), debugPort = await freePort(), base = `http://127.0.0.1:${port}`;
const env = { ...process.env, IVNC_HOME: path.join(work, 'ivnc'), XDG_CONFIG_HOME: path.join(work, 'config'),
    XDG_CACHE_HOME: path.join(work, 'cache'), XDG_DATA_HOME: path.join(work, 'data'), XDG_RUNTIME_DIR: path.join(work, 'run') };
for (const name of ['IVNC_HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_DATA_HOME','XDG_RUNTIME_DIR']) mkdirSync(env[name], {mode:0o700});
const shell = path.join(work, 'test-shell'), config = path.join(work, 'ivnc.toml');
writeFileSync(shell, '#!/bin/sh\nexport HISTFILE=/dev/null INPUTRC=/dev/null LC_ALL=C.UTF-8\nprintf "WINDOW_UI_READY\\n"\nPS1="window-test# " exec /bin/bash --noprofile --norc -i\n', {mode:0o700});
writeFileSync(config, `[terminal]\nshell = ${JSON.stringify(shell)}\ncwd = ${JSON.stringify(work)}\n[audio]\nenabled = false\nsample_rate = 48000\nchannels = 2\nbitrate = 128000\n`);
const children = [], pending = new Map();
let socket, seq = 0, passed = 0, mouseDown = false;
const pass = name => { passed++; console.log(`PASS ${name}`); };
function launch(command, args, name, extra = {}) {
    const fd = openSync(path.join(work, name + '.log'), 'a', 0o600);
    const child = spawn(command, args, {stdio:['ignore',fd,fd], ...extra});
    closeSync(fd); children.push(child); return child;
}
async function waitFor(name, fn, timeout = 20000) {
    const end = Date.now() + timeout; let error;
    while (Date.now() < end) {
        try { const value = await fn(); if (value) return value; } catch (e) { error = e; }
        await delay(50);
    }
    throw Error(`Timeout: ${name}${error ? ': ' + error.message : ''}`);
}
async function api(url, method = 'GET', body) {
    const response = await fetch(base + url, {method, headers:{'Content-Type':'application/json'}, body:body ? JSON.stringify(body) : undefined});
    const data = await response.json(); assert(response.ok, `${url}: ${JSON.stringify(data)}`); return data;
}
const call = (method, params = {}) => new Promise((resolve,reject) => {
    const id = ++seq, timer = setTimeout(() => {pending.delete(id);reject(Error(`CDP timeout: ${method}`));},20000);
    pending.set(id, {resolve,reject,timer}); socket.send(JSON.stringify({id,method,params}));
});
async function evaluate(expression) {
    const r = await call('Runtime.evaluate', {expression,awaitPromise:true,returnByValue:true,userGesture:true});
    if (r.exceptionDetails) throw Error(JSON.stringify(r.exceptionDetails)); return r.result.value;
}
const consoleWindow = `document.querySelector('iframe[title="iVnc Console"], iframe[title="iVnc 控制台"]')?.contentWindow`;
const con = expression => evaluate(`(() => {const w=${consoleWindow};return w.eval(${JSON.stringify(expression)});})()`);
const rectCode = node => `(() => {const r=${node}.getBoundingClientRect();return {left:r.left,top:r.top,right:r.right,bottom:r.bottom,width:r.width,height:r.height};})()`;
const terminalRect = () => evaluate(rectCode(`document.querySelector('[data-terminal-background]')`));
const pinAppearance = () => evaluate(`(() => {
    const button=document.querySelector('#web-terminal-pin'),style=getComputedStyle(button);
    const transform=new DOMMatrixReadOnly(getComputedStyle(button.querySelector('svg')).transform);
    return {pressed:button.getAttribute('aria-pressed'),background:style.backgroundColor,
        angle:Math.atan2(transform.b,transform.a)*180/Math.PI};
})()`);
const isTerminalOpen = () => evaluate(`!!document.querySelector('[data-terminal-background]:not(.minimized)')`);
const consoleOpen = () => evaluate(`!![...document.querySelectorAll('.web-console-modal')].find(n=>n.querySelector('#web-console-minimize')&&!n.classList.contains('minimized'))`);
const childOpen = () => con(`!!document.querySelector('.modal-bg.show')`);
async function mouse(type, x, y) {
    if (type === 'mousePressed') mouseDown = true;
    if (type === 'mouseReleased') mouseDown = false;
    await call('Input.dispatchMouseEvent', {type,x,y,button:mouseDown || type==='mouseReleased'?'left':'none',buttons:mouseDown?1:0,clickCount:type==='mouseMoved'?0:1});
}
async function clickPoint(x,y) { await mouse('mousePressed',x,y); await mouse('mouseReleased',x,y); }
async function point(selector, inside = false) {
    return evaluate(`(() => {
        const f=${inside ? `document.querySelector('iframe[title="iVnc Console"], iframe[title="iVnc 控制台"]')` : 'null'};
        const d=f?f.contentDocument:document,n=d.querySelector(${JSON.stringify(selector)});
        if(!n) throw Error('Missing target '+${JSON.stringify(selector)});
        n.scrollIntoView({block:'nearest',inline:'nearest'});
        const r=n.getBoundingClientRect(),fr=f?.getBoundingClientRect(),sx=f?fr.width/f.clientWidth:1,sy=f?fr.height/f.clientHeight:1;
        return {x:(fr?.left||0)+(r.left+r.width/2)*sx,y:(fr?.top||0)+(r.top+r.height/2)*sy};
    })()`);
}
async function click(selector, inside = false) { const p = await point(selector,inside); await clickPoint(p.x,p.y); }
async function key(name, code = name, keyCode, modifiers = 0, text) {
    const params={key:name,code,modifiers,...(keyCode?{windowsVirtualKeyCode:keyCode}:{}),...(text?{text}:{})};
    await call('Input.dispatchKeyEvent',{type:'keyDown',...params}); await call('Input.dispatchKeyEvent',{type:'keyUp',...params});
}
async function drag(selector, dx, dy, cancel = false) {
    const p=await point(selector); await mouse('mousePressed',p.x,p.y);
    for (let i=1;i<=6;i++) { await mouse('mouseMoved',p.x+dx*i/6,p.y+dy*i/6); await delay(20); }
    if (cancel) await key('Escape','Escape',27);
    await mouse('mouseReleased',p.x+dx,p.y+dy); await delay(150);
}
async function screenshot(name) {
    const r=await call('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(work,name),Buffer.from(r.data,'base64'));
}
async function openAdd() { await click('#add-app-btn',true); await waitFor('add dialog',childOpen); }
async function edit(id) {
    await click(`#app-list tr[data-app-id="${id}"] .btn-edit`,true);
    await waitFor('editor data',()=>con(`document.querySelector('#modal').classList.contains('show')&&!document.querySelector('#modal').classList.contains('loading')`));
}
async function closeChild() { await click('.web-console-modal .web-terminal-header'); await waitFor('child closed',async()=>!await childOpen()); }
async function assertBounds() {
    const r=await terminalRect(),area=await evaluate(rectCode(`document.querySelector('.video-container')`));
    assert(r.left>=area.left+7 && r.top>=area.top+7 && r.right<=area.right-7 && r.bottom<=area.bottom-7,JSON.stringify({r,area}));
    const dimensions=await evaluate(`window.__terminalSent.filter(m=>m.type==='resize').at(-1)`);
    assert(dimensions.cols>=20&&dimensions.cols<=400&&dimensions.rows>=5&&dimensions.rows<=160);
}
async function assertDefaultGeometry() {
    await assertBounds();
    const r=await terminalRect(),area=await evaluate(rectCode(`document.querySelector('.video-container')`));
    const scale=await evaluate(`Number(getComputedStyle(document.documentElement).getPropertyValue('--ivnc-modal-scale'))`);
    assert(Math.abs(r.width-Math.min(920*scale,area.width-16))<1,JSON.stringify({r,area,scale}));
    assert(Math.abs(r.height-Math.min(560*scale,area.height-16))<1,JSON.stringify({r,area,scale}));
    assert(Math.abs(r.left+r.width/2-(area.left+area.right)/2)<1);
    assert(Math.abs(r.top+r.height/2-(area.top+area.bottom)/2)<1);
}
try {
    launch(binary,['--config',config,'--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','800','--height','600'],'server',{env,cwd:root});
    await waitFor('server',()=>api('/api/apps'),30000);
    const a=(await api('/api/apps','POST',{name:'Window A',app_type:'desktop',exec_command:'sleep 600'})).app;
    const b=(await api('/api/apps','POST',{name:'Window B',app_type:'desktop',exec_command:'sleep 600'})).app;
    launch(process.env.CHROME_BIN||'google-chrome',['--headless=new','--no-sandbox','--disable-gpu','--disable-dev-shm-usage','--no-first-run','--no-default-browser-check','--disable-background-networking',
        `--user-data-dir=${path.join(work,'browser')}`,`--remote-debugging-port=${debugPort}`,'--window-size=1280,800',base],'browser');
    const target=await waitFor('browser',async()=>(await(await fetch(`http://127.0.0.1:${debugPort}/json`)).json()).find(t=>t.type==='page'));
    socket=new WebSocket(target.webSocketDebuggerUrl);await new Promise((resolve,reject)=>{socket.once('open',resolve);socket.once('error',reject);});
    socket.on('message',raw=>{const m=JSON.parse(raw),p=pending.get(m.id);if(!p)return;pending.delete(m.id);clearTimeout(p.timer);m.error?p.reject(Error(m.error.message)):p.resolve(m.result);});
    socket.on('close',()=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(Error('CDP closed'));}pending.clear();});
    await call('Page.enable');
    await call('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});
    const capture=`(() => {
        if(window.__terminalSent)return;window.__terminalSent=[];window.__terminalSockets=[];
        const Original=WebSocket;window.WebSocket=class extends Original{constructor(...args){super(...args);if(String(args[0]).endsWith('/terminal/ws')){window.__terminalSockets.push(this);const send=this.send.bind(this);this.send=d=>{window.__terminalSent.push(JSON.parse(d));return send(d)}}}};
    })()`;
    await call('Page.addScriptToEvaluateOnNewDocument',{source:capture});await evaluate(capture);
    await waitFor('desktop',()=>evaluate(`!!document.querySelector('#console-btn')&&window.__ivncWebRTC?._send_channel?.readyState==='open'`),60000);
    await evaluate(`window.__desktopInput=[];const rtc=window.__ivncWebRTC,send=rtc.sendDataChannelMessage.bind(rtc);rtc.sendDataChannelMessage=m=>{if(/^(m2?|kd|ku|u),/.test(m))__desktopInput.push(m);return send(m)};`);
    await click('#console-btn');
    await waitFor('console frame',()=>evaluate(`!!${consoleWindow}?.document.querySelector('.console-select')`));
    assert(await con(`(() => {const n=document.querySelector('.nav-list').getBoundingClientRect(),l=document.querySelector('#lang-toggle').getBoundingClientRect(),tabs=[...document.querySelectorAll('.nav-item')].map(n=>n.getBoundingClientRect());return Math.abs(tabs[0].top-tabs[2].top)<1&&l.left>n.right&&document.querySelector('.console-main').offsetLeft<40})()`));
    await click('#nav-apps',true); await waitFor('apps',()=>con(`document.querySelectorAll('#app-list tr[data-app-id]').length>=2`));
    pass('embedded console has horizontal navigation and an independent language button');

    await openAdd(); await click('.console-select',true);
    assert(await con(`document.querySelector('.console-select').getAttribute('aria-expanded')==='true'`));
    await key('End','End',35);await key('Enter','Enter',13);
    assert.equal(await con(`document.querySelector('#f-app-type').value`),'remote');
    assert(await con(`!document.querySelector('#remote-config').hidden`));
    await click('.console-select',true);await key('Home','Home',36);await key('ArrowDown','ArrowDown',40);await key('Enter','Enter',13);
    assert.equal(await con(`document.querySelector('#f-app-type').value`),'desktop');
    await click('.console-select',true);await key('Home','Home',36);await key('ArrowDown','ArrowDown',40);await key('ArrowDown','ArrowDown',40);await key('Enter','Enter',13);
    assert.equal(await con(`document.querySelector('#f-app-type').value`),'cli');
    assert(await con(`getComputedStyle(document.querySelector('#cli-config')).display!=='none'`));
    await click('.console-select',true);await key('Escape','Escape',27);
    assert(await childOpen()); assert(await con(`document.querySelector('.console-select-popup').hidden`));
    await key('Escape','Escape',27);assert(!await childOpen());assert(await consoleOpen());
    pass('styled type selector updates native values and dependent fields; Escape closes one level at a time');

    await openAdd();await screenshot('console-add.png');
    const beforeInput=await evaluate('__desktopInput.length');
    await closeChild();assert(await consoleOpen());assert.equal(await evaluate('__desktopInput.length'),beforeInput);
    await openAdd();await clickPoint(8,8);assert(!await childOpen());assert(await consoleOpen());
    assert(await evaluate(`document.querySelector('.console-dialog-shield').hidden`));
    await openAdd();
    await evaluate(`window.__iframeClicks=0;window.__testLayer=document.createElement('div');__testLayer.className='app-streams';__testLayer.style.cssText='position:fixed;inset:0;z-index:990;visibility:visible;pointer-events:auto';
        const f=document.createElement('iframe');f.style.cssText='width:100%;height:100%;border:0';__testLayer.appendChild(f);document.body.appendChild(__testLayer);
        f.contentDocument.body.style.cssText='margin:0;min-height:100vh';f.contentDocument.addEventListener('mousedown',()=>window.__iframeClicks++);
        f.contentWindow.eval("document.addEventListener('pointerdown',()=>parent.postMessage({type:'ivnc-overlay-app-interaction'},parent.location.origin))");`);
    await clickPoint(8,8);assert.equal(await evaluate('__iframeClicks'),0);assert(!await childOpen());assert(await consoleOpen());
    await clickPoint(8,8);await waitFor('iframe receives next click',()=>evaluate('__iframeClicks===1'));await waitFor('console auto minimized',async()=>!await consoleOpen());
    await evaluate('__testLayer.remove()');
    pass('outside dismissal consumes the complete click before it enters an application iframe and keeps the console open');

    await click('#console-btn');await waitFor('console restored',consoleOpen);
    await con(`window.__realFetch=window.fetch.bind(window);window.__held=[];window.__holdReads=new Set();
        window.fetch=(url,options={})=>{const path=String(url),method=options.method||'GET';
            if(method==='GET'&&__holdReads.has(path))return new Promise(resolve=>__held.push(()=>__realFetch(path).then(resolve)));
            return __realFetch(url,options);};__holdReads.add('/api/apps/${a.id}');`);
    await click(`#app-list tr[data-app-id="${a.id}"] .btn-edit`,true);
    await waitFor('held editor',()=>con('__held.length===1'));await closeChild();await edit(b.id);
    await con('__held.shift()();__holdReads.clear()');await delay(200);
    assert.equal(await con(`document.querySelector('#f-name').value`),b.name);
    assert(await con(`document.querySelector('.console-select').disabled`));
    await closeChild();
    await con(`__holdReads.add('/api/apps/${a.id}/logs')`);
    await click(`#app-list tr[data-app-id="${a.id}"] .btn-log`,true);await waitFor('held logs',()=>con('__held.length===1'));
    await closeChild();await click(`#app-list tr[data-app-id="${b.id}"] .btn-log`,true);
    await waitFor('new logs loaded',()=>con(`!document.querySelector('#log-content').textContent.includes('加载')&&!document.querySelector('#log-content').textContent.includes('Loading')`));
    const newLogs=await con(`document.querySelector('#log-content').textContent`);
    await con('__held.shift()();__holdReads.clear()');await delay(200);
    assert.equal(await con(`document.querySelector('#log-title').dataset.appName`),b.name);
    assert.equal(await con(`document.querySelector('#log-content').textContent`),newLogs);await closeChild();
    pass('late editor and log replies cannot modify a newer dialog, even when cancellation is ignored');

    await edit(a.id);await waitFor('sharing loaded',()=>con(`!document.querySelector('#f-share-enabled').disabled`));
    await con(`window.__writes=[];window.__holdWrite=true;window.fetch=(url,options={})=>{
        const path=String(url);if(options.method==='PUT')__writes.push(path);
        if(path==='/api/apps/${a.id}'&&options.method==='PUT'&&__holdWrite){__holdWrite=false;return new Promise(resolve=>__held.push(()=>__realFetch(url,options).then(resolve)))}
        return __realFetch(url,options);};
        document.querySelector('#f-exec').value='sleep 601';document.querySelector('#f-share-enabled').click();document.querySelector('#f-share-password').value='window-ui-test-password';`);
    await click('#modal-save',true);await waitFor('held save',()=>con('__held.length===1'));await closeChild();await edit(b.id);
    await con('__held.shift()()');
    await waitFor('snapshot sharing saved',async()=>(await api(`/api/apps/${a.id}/sharing`)).enabled);
    assert.equal((await api(`/api/apps/${a.id}`)).exec_command,'sleep 601');
    assert.equal((await api(`/api/apps/${b.id}`)).exec_command,'sleep 600');
    assert.equal(await con(`document.querySelector('#f-name').value`),b.name);assert(await childOpen());
    assert.deepEqual(await con('__writes'),[`/api/apps/${a.id}`,`/api/apps/${a.id}/sharing`]);
    await closeChild();
    await edit(a.id);await con(`__holdWrite=true;document.querySelector('#f-exec').value='sleep 602'`);
    await click('#modal-save',true);await waitFor('second held save',()=>con('__held.length===1'));await closeChild();
    await click(`#app-list tr[data-app-id="${a.id}"] .btn-edit`,true);
    assert(await con(`document.querySelector('#modal').classList.contains('loading')`));
    await con('__held.shift()()');await waitFor('serialized reopen',()=>con(`!document.querySelector('#modal').classList.contains('loading')&&document.querySelector('#f-exec').value==='sleep 602'`));
    await closeChild();await con('window.fetch=__realFetch');
    pass('submitted saves retain their app and sharing snapshots; reopening the same app waits for completion');

    await openAdd();await con(`document.querySelector('#f-app-type').value='remote';document.querySelector('#f-app-type').dispatchEvent(new Event('change'));
        document.querySelector('#f-remote-url').value='http://127.0.0.1:12345/share/test';document.querySelector('#f-remote-password').value='window-ui-test';
        window.fetch=(url,options={})=>String(url)==='/api/remote/test'?new Promise(resolve=>__held.push(()=>resolve(new Response(JSON.stringify({name:'Stale probe',status:'running',ready:true}),{status:200})))):__realFetch(url,options);`);
    await click('#remote-test',true);await waitFor('held probe',()=>con('__held.length===1'));await closeChild();await openAdd();
    await con(`document.querySelector('#f-name').value='New draft';__held.shift()();window.fetch=__realFetch`);await delay(150);
    assert.equal(await con(`document.querySelector('#f-name').value`),'New draft');await closeChild();
    await click('#lang-toggle',true);await waitFor('English labels',()=>con(`document.querySelector('#nav-overview').textContent==='Overview'`));
    await openAdd();assert.equal(await con(`document.querySelector('.console-select').textContent`),'Background App');await closeChild();
    pass('remote probes are scoped to their dialog; translated selectors retain their value and disabled state');

    await openAdd();const oldDialogId=await con('__ivncConsoleDialogState.id');
    await con('window.__consoleReloading=true;location.reload()');
    await waitFor('console reloaded',()=>con(`!window.__consoleReloading&&!!document.querySelector('.console-select')`));
    assert(await evaluate(`document.querySelector('.console-dialog-shield').hidden&&!document.documentElement.hasAttribute('data-ivnc-modal-blocked')`));
    await click('#nav-apps',true);await waitFor('reloaded app rows',()=>con(`document.querySelectorAll('#app-list tr[data-app-id]').length>=2`));
    await openAdd();await con(`__ivncConsoleDialogState.dismiss(${JSON.stringify(oldDialogId)})`);
    assert(await childOpen());await closeChild();
    pass('console reload releases its shield and rejects dismissal tokens from the previous document');

    await click('#web-console-minimize');await click('#terminal-btn');
    await waitFor('terminal dimensions',()=>evaluate(`window.__terminalSent.some(m=>m.type==='resize')`));
    await assertBounds();const initial=await terminalRect();
    await drag('.web-terminal-floating .web-terminal-title',80,-35);
    const moved=await terminalRect();assert(Math.abs(moved.left-initial.left-80)<2&&Math.abs(moved.top-initial.top+35)<2);
    await drag('.resize-se',-180,-100);const resized=await terminalRect();
    assert(Math.abs(resized.width-moved.width+180)<2&&Math.abs(resized.height-moved.height+100)<2);
    await evaluate(`document.querySelector('.xterm-helper-textarea').focus()`);
    await call('Input.insertText',{text:'stty size'});await key('Enter','Enter',13,0,'\r');
    await waitFor('PTY dimensions match',()=>evaluate(`(() => {const d=__terminalSent.filter(m=>m.type==='resize').at(-1);return [...document.querySelectorAll('.xterm-rows>div')].some(n=>n.textContent.trim()===d.rows+' '+d.cols)})()`));
    const stable=await terminalRect();await drag('.web-terminal-floating .web-terminal-title',45,40,true);const cancelled=await terminalRect();
    assert(Math.abs(cancelled.left-stable.left)<1&&Math.abs(cancelled.top-stable.top)<1);
    pass('drag and corner resize follow the pointer, update the real PTY, and Escape restores the starting rectangle');

    for(const edge of ['n','ne','e','se','s','sw','w','nw']) {
        const before=await terminalRect(),dx=edge.includes('w')?12:edge.includes('e')?-12:0,dy=edge.includes('n')?10:edge.includes('s')?-10:0;
        await drag(`.resize-${edge}`,dx,dy);const after=await terminalRect();
        if(dx)assert(after.width<before.width-8);if(dy)assert(after.height<before.height-6);
        await assertBounds();
    }
    const count=await evaluate('__terminalSent.length');await drag('.web-terminal-floating .web-terminal-title',-3000,3000);await assertBounds();
    assert(!await evaluate(`__terminalSent.slice(${count}).some(m=>m.type==='input')`));
    pass('all eight resize handles and work-area limits behave consistently without sending terminal input');

    await mouse('mouseMoved',8,8);
    await waitFor('unpinned diagonal icon',async()=>{const p=await pinAppearance();return p.pressed==='false'&&Math.abs(p.angle+35)<.1&&p.background==='rgba(255, 255, 255, 0.08)'});
    const actionsInput=await evaluate(`__terminalSent.filter(m=>m.type==='input').length`);
    await click('#web-terminal-pin');await mouse('mouseMoved',8,8);
    await waitFor('pinned vertical icon',async()=>{const p=await pinAppearance();return p.pressed==='true'&&Math.abs(p.angle)<.1&&p.background==='rgb(43, 76, 118)'});
    const pinPoint=await point('#web-terminal-pin');await mouse('mouseMoved',pinPoint.x,pinPoint.y);
    await waitFor('pinned hover stays blue',async()=>(await pinAppearance()).background==='rgb(53, 93, 142)');
    await screenshot('terminal-pinned-hover.png');
    await mouse('mousePressed',pinPoint.x,pinPoint.y);
    await waitFor('pinned active stays blue',async()=>(await pinAppearance()).background==='rgb(36, 66, 104)');
    await mouse('mouseReleased',pinPoint.x,pinPoint.y);
    await waitFor('unpinned hover has no selected highlight',async()=>{const p=await pinAppearance();return p.pressed==='false'&&Math.abs(p.angle+35)<.1&&p.background==='rgba(255, 255, 255, 0.16)'});
    await click('#web-terminal-pin');await mouse('mouseMoved',8,8);
    await evaluate(`document.querySelector('#web-terminal-restore').focus()`);
    await key('Tab','Tab',9,8);
    await waitFor('pinned keyboard focus stays blue',async()=>(await pinAppearance()).background==='rgb(53, 93, 142)');
    await key(' ','Space',32,0,' ');await waitFor('keyboard unpin',async()=>(await pinAppearance()).pressed==='false');
    await key('Enter','Enter',13,0,'\r');await waitFor('keyboard pin',async()=>(await pinAppearance()).pressed==='true');
    assert.equal(await evaluate('document.activeElement.id'),'web-terminal-pin');
    assert.equal(await evaluate(`__terminalSent.filter(m=>m.type==='input').length`),actionsInput);
    await call('Emulation.setEmulatedMedia',{features:[{name:'prefers-reduced-motion',value:'reduce'}]});
    assert(await evaluate(`getComputedStyle(document.querySelector('#web-terminal-pin')).transitionDuration==='0s'&&getComputedStyle(document.querySelector('#web-terminal-pin svg')).transitionDuration==='0s'`));
    await call('Emulation.setEmulatedMedia',{features:[]});
    pass('pin direction and selected colors survive real hover, active and keyboard focus; reduced motion disables transitions');

    await clickPoint(8,8);assert(await isTerminalOpen());
    await click('#console-btn');assert(await isTerminalOpen());assert(await consoleOpen());
    await openAdd();assert(await evaluate(`getComputedStyle(document.querySelector('.has-child-dialog')).zIndex>getComputedStyle(document.querySelector('[data-terminal-background]')).zIndex`));
    await screenshot('console-child-pinned.png');await closeChild();assert(await isTerminalOpen());
    await click('#web-console-minimize');const savedRect=await terminalRect();await click('#web-terminal-minimize');assert(!await isTerminalOpen());
    await click('#terminal-btn');assert(await isTerminalOpen());assert.equal(await evaluate(`document.querySelector('#web-terminal-pin').getAttribute('aria-pressed')`),'true');
    assert(Math.abs((await terminalRect()).width-savedRect.width)<1);
    pass('pinning survives tool switches and manual minimization while console child dialogs receive priority');

    await evaluate(`window.__restoreTerminal=document.querySelector('.xterm');window.__restoreSocket=__terminalSockets[0];document.querySelector('.xterm-helper-textarea').focus()`);
    await call('Input.insertText',{text:"printf 'RESTORE_ALIVE\\n'"});await key('Enter','Enter',13,0,'\r');
    await waitFor('restore session marker',()=>evaluate(`[...document.querySelectorAll('.xterm-rows>div')].some(n=>n.textContent.trim()==='RESTORE_ALIVE')`));
    await click('#web-terminal-background');
    assert.equal(await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalBackground`),'opaque');
    const restoreInput=await evaluate(`__terminalSent.filter(m=>m.type==='input').length`);
    await click('#web-terminal-restore');await assertDefaultGeometry();
    assert(await evaluate(`__restoreTerminal===document.querySelector('.xterm')&&__terminalSockets.length===1&&__restoreSocket===__terminalSockets[0]&&__restoreSocket.readyState===1`));
    assert(await evaluate(`[...document.querySelectorAll('.xterm-rows>div')].some(n=>n.textContent.trim()==='RESTORE_ALIVE')`));
    assert(await evaluate(`document.activeElement===document.querySelector('.xterm-helper-textarea')`));
    const stateKey=await evaluate(`'ivnc.terminal.window.v1:'+location.pathname`);
    const defaultPinned={width:920,height:560,x:.5,y:.5,pinned:true};
    assert.deepEqual(await evaluate(`JSON.parse(sessionStorage.getItem(${JSON.stringify(stateKey)}))`),defaultPinned);
    assert.equal(await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalBackground`),'opaque');
    await delay(150);const resetResizes=await evaluate(`__terminalSent.filter(m=>m.type==='resize').length`);
    for(let i=0;i<3;i++)await click('#web-terminal-restore');
    await delay(150);assert.equal(await evaluate(`__terminalSent.filter(m=>m.type==='resize').length`),resetResizes);
    assert.equal(await evaluate(`__terminalSent.filter(m=>m.type==='input').length`),restoreInput);
    await click('#web-terminal-minimize');await click('#terminal-btn');await assertDefaultGeometry();
    await screenshot('terminal-restored.png');
    pass('restore centers the default size, saves geometry, retains pin/background/session/history and avoids redundant PTY resizes');

    for(const [name,code,keyCode,text] of [['Enter','Enter',13,'\r'],[' ','Space',32,' ']]) {
        await drag('.resize-se',-90,-60);
        await evaluate(`document.querySelector('#web-terminal-restore').focus()`);
        await key(name,code,keyCode,0,text);await assertDefaultGeometry();
        assert.equal(await evaluate('document.activeElement.id'),'web-terminal-restore');
    }
    const dragPoint=await point('.web-terminal-floating .web-terminal-title');
    await mouse('mousePressed',dragPoint.x,dragPoint.y);await mouse('mouseMoved',dragPoint.x+30,dragPoint.y+20);
    await evaluate(`document.querySelector('#web-terminal-restore').click()`);
    await mouse('mouseMoved',dragPoint.x+60,dragPoint.y+40);await mouse('mouseReleased',dragPoint.x+60,dragPoint.y+40);
    await evaluate(`window.dispatchEvent(new Event('ivnc-overlay-layout'));document.querySelector('#web-terminal-restore').click()`);
    await delay(150);await assertDefaultGeometry();
    assert(!await evaluate(`document.querySelector('[data-terminal-background]').classList.contains('window-interacting')`));
    assert.equal(await evaluate(`__terminalSent.filter(m=>m.type==='input').length`),restoreInput);
    pass('Enter and Space restore without shell input; restoring during a captured drag cancels stale gestures and pending layouts');

    await evaluate(`window.__originalConsoleDocument=${consoleWindow}.document;window.__originalTerminal=document.querySelector('.xterm');document.querySelector('#overlayInput').focus()`);
    await key('F','KeyF',70,10);
    await waitFor('whole-page fullscreen',()=>evaluate('document.fullscreenElement===document.documentElement'));
    await click('#web-terminal-restore');await assertDefaultGeometry();
    assert(await isTerminalOpen());assert(await evaluate(`__originalConsoleDocument===${consoleWindow}.document&&__originalTerminal===document.querySelector('.xterm')&&!document.pointerLockElement`));
    await evaluate('document.exitFullscreen()');await delay(150);
    assert(await evaluate(`__originalConsoleDocument===${consoleWindow}.document&&__terminalSockets.length===1`));
    await screenshot('terminal-window.png');
    pass('fullscreen uses the common root, preserves live iframe/terminal sessions and leaves overlay pointer input unlocked');

    for(const [width,height,dpr] of [[375,600,2],[640,480,1],[1280,800,2],[4096,2160,1]]) {
        await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:dpr,mobile:false});await delay(250);await assertBounds();
        await click('#web-terminal-restore');await assertDefaultGeometry();
    }
    await drag('.resize-ne',3500,-1600);await assertBounds();
    assert((await evaluate(`__terminalSent.filter(m=>m.type==='resize').at(-1)`)).cols<=400);
    await call('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});await delay(200);
    await evaluate(`postMessage({type:'setScaleLocally',value:true},location.origin);postMessage({type:'setManualResolution',width:1920,height:1080},location.origin)`);
    await waitFor('desktop scaled',()=>evaluate(`Number(getComputedStyle(document.documentElement).getPropertyValue('--ivnc-modal-scale'))<.9`));
    // The desktop scale is published before floating-window paints its next
    // frame; wait for that frame before taking a real pointer target.
    await waitFor('scaled floating window painted',()=>evaluate(`Math.abs(new DOMMatrixReadOnly(getComputedStyle(document.querySelector('[data-terminal-background]')).transform).a-Number(getComputedStyle(document.documentElement).getPropertyValue('--ivnc-modal-scale')))<.001`));
    await click('#web-terminal-restore');await assertDefaultGeometry();
    await assertBounds();await drag('.web-terminal-floating .web-terminal-title',-40,-25);await assertBounds();
    const pinBefore=await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalPinned`);
    await click('#web-terminal-pin');assert.notEqual(await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalPinned`),pinBefore);
    await click('#web-terminal-pin');await click('#web-terminal-minimize');assert(!await isTerminalOpen());await click('#terminal-btn');assert(await isTerminalOpen());
    await evaluate(`postMessage({type:'resetResolutionToWindow'},location.origin)`);await delay(250);await assertBounds();
    pass('DPR, narrow and large viewports, manual scaling and fixed toolbar changes preserve reachable geometry and protocol limits');

    const toolbarPinned=await evaluate(`document.querySelector('.taskbar').classList.contains('pinned')`);
    for(const pinned of [!toolbarPinned,toolbarPinned]) {
        await evaluate(`document.querySelector('.taskbar-pin').click()`);
        await waitFor('toolbar pin change',()=>evaluate(`document.querySelector('.taskbar').classList.contains('pinned')===${pinned}`));
        await delay(250);await click('#web-terminal-restore');await assertDefaultGeometry();
    }
    pass('restore follows fullscreen, DPR, narrow/large screens, desktop scaling and fixed/unfixed toolbar work areas');

    const stored=await evaluate(`JSON.parse(sessionStorage.getItem(${JSON.stringify(stateKey)}))`);
    await evaluate('window.__windowUiReloading=true');await call('Page.reload');
    await waitFor('reloaded desktop',()=>evaluate(`!window.__windowUiReloading&&!!document.querySelector('#terminal-btn')`));await click('#terminal-btn');
    await waitFor('restored terminal',()=>evaluate(`__terminalSent.some(m=>m.type==='resize')`));await assertBounds();
    assert.equal(await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalPinned`),'true');
    await assertDefaultGeometry();
    assert.equal(await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalBackground`),'opaque');
    assert.deepEqual(await evaluate(`JSON.parse(sessionStorage.getItem(${JSON.stringify(stateKey)}))`),stored);
    await evaluate(`sessionStorage.setItem(${JSON.stringify(stateKey)},'invalid-json');window.__windowUiReloading=true`);await call('Page.reload');
    await waitFor('default desktop',()=>evaluate(`!window.__windowUiReloading&&!!document.querySelector('#terminal-btn')`));await click('#terminal-btn');
    await waitFor('default terminal',()=>evaluate(`__terminalSent.some(m=>m.type==='resize')`));await assertBounds();
    assert.equal(await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalPinned`),'false');
    pass('tab-local window preferences survive refresh and invalid storage falls back to usable defaults');

    await call('Page.addScriptToEvaluateOnNewDocument',{source:`(() => {
        for(const name of ['getItem','setItem']) {const original=Storage.prototype[name];
            Storage.prototype[name]=function(key,...args) {
                if(String(key).startsWith('ivnc.terminal.window.v1:'))throw new DOMException('Storage blocked','SecurityError');
                return original.call(this,key,...args);
            };
        }
    })()`});
    await evaluate('window.__windowUiReloading=true');await call('Page.reload');
    await waitFor('storage-blocked desktop',()=>evaluate(`!window.__windowUiReloading&&!!document.querySelector('#terminal-btn')`));await click('#terminal-btn');
    await waitFor('storage-blocked terminal',()=>evaluate(`__terminalSent.some(m=>m.type==='resize')`));
    await drag('.resize-se',-90,-60);await drag('.web-terminal-floating .web-terminal-title',25,20);
    await click('#web-terminal-pin');await click('#web-terminal-restore');await assertDefaultGeometry();
    await click('#web-terminal-minimize');await click('#terminal-btn');await assertDefaultGeometry();
    assert.equal(await evaluate(`document.querySelector('[data-terminal-background]').dataset.terminalPinned`),'true');
    assert.equal(await evaluate('__terminalSockets.length'),1);
    pass('blocked window storage still permits restore, pinning and reopening using in-memory state');

    await call('Page.navigate',{url:base+'/console'});await waitFor('standalone console',()=>evaluate(`!!document.querySelector('.console-select')`));
    await click('#nav-apps');await click('#add-app-btn');await click('.console-select');
    const popup=await evaluate(rectCode(`document.querySelector('.console-select-popup')`));assert(popup.top>=0&&popup.bottom<=800&&popup.width>100);
    await key('Escape','Escape',27);assert(await evaluate(`document.querySelector('#modal').classList.contains('show')`));
    await clickPoint(3,3);assert(!await evaluate(`document.querySelector('#modal').classList.contains('show')`));
    await call('Emulation.setDeviceMetricsOverride',{width:375,height:600,deviceScaleFactor:2,mobile:false});await delay(100);
    assert(await evaluate(`document.documentElement.scrollWidth<=innerWidth&&document.querySelector('#lang-toggle').getBoundingClientRect().right<=innerWidth`));
    await screenshot('console-narrow.png');
    pass('standalone console shares the styled selector, backdrop behavior and narrow horizontal navigation');
    console.log(`Window UI smoke: ${passed} passed`);
} catch(error) {
    process.exitCode=1;console.error(error.stack);console.error(`Artifacts: ${work}`);
    if(socket?.readyState===1)try{await screenshot('failure.png');writeFileSync(path.join(work,'diagnostics.json'),JSON.stringify(await evaluate(`({html:document.body.innerHTML,active:document.activeElement?.outerHTML,terminal:window.__terminalSent})`),null,2));}catch(_){}
} finally {
    socket?.close();
    for(const child of children.reverse()) {
        if(child.exitCode!==null||child.signalCode)continue;child.kill('SIGTERM');
        for(let i=0;i<50&&child.exitCode===null&&!child.signalCode;i++)await delay(100);
        if(child.exitCode===null&&!child.signalCode)child.kill('SIGKILL');
    }
    if(!process.env.KEEP_TMP&&!process.exitCode)rmSync(work,{recursive:true,force:true});
    else if(!process.exitCode)console.log(`Artifacts: ${work}`);
}
