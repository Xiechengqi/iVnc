// Real production frontend -> WebRTC -> Electron. Two isolated instances,
// covering main/shared views and A -> B remote input without touching deployment.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const root=path.resolve(new URL('..',import.meta.url).pathname);
const engines=createRequire(new URL('../web/ivnc/package.json',import.meta.url))('playwright');
const electron=process.env.ELECTRON_BIN;
assert(electron,'Set ELECTRON_BIN to an installed Electron 38+ runtime');
const work=mkdtempSync(path.join(tmpdir(),'ivnc-ime-e2e-')),servers=[],apps=[];
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
async function wait(name,fn,ms=30000){const end=Date.now()+ms;let last;while(Date.now()<end){try{const v=await fn();if(v)return v}catch(e){last=e}await delay(75)}throw Error(`${name}: ${last||'timed out'}`)}
const quote=s=>`'${s.replaceAll("'","'\\''")}'`;
async function api(base,url,method='GET',body){const r=await fetch(base+url,{method,headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});const d=await r.json();assert(r.ok,JSON.stringify(d));return d}
async function instance(name){
    const base=`http://127.0.0.1:${await port()}`;
    const env={...process.env,IVNC_HOME:path.join(work,name,'ivnc'),XDG_RUNTIME_DIR:path.join(work,name,'run'),XDG_CONFIG_HOME:path.join(work,name,'config'),XDG_CACHE_HOME:path.join(work,name,'cache'),XDG_DATA_HOME:path.join(work,name,'data')};
    for(const key of ['IVNC_HOME','XDG_RUNTIME_DIR','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_DATA_HOME'])mkdirSync(env[key],{recursive:true,mode:0o700});
    const log=openSync(path.join(work,name,'server.log'),'a',0o600);
    const server=spawn(process.env.IVNC_BIN||path.join(root,'target/debug/ivnc'),['--http-port',new URL(base).port,'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','960','--height','720'],{env,cwd:root,detached:true,stdio:['ignore',log,log]});closeSync(log);servers.push(server);
    await wait(`${name} server`,()=>api(base,'/api/apps'));return base;
}
async function editor(base,name,shared=false,legacy=false){
    const controlPort=await port();
    const result=await api(base,'/api/apps','POST',{name,app_type:'desktop',autostart:false,exec_command:[electron,path.join(root,'scripts/fixtures/electron-wayland/main.cjs'),...(legacy?['--disable-wayland-ime']:[])].map(quote).join(' '),env_vars:{ELECTRON_CONTROL_PORT:String(controlPort)}});
    const app={base,id:result.app.id,name,legacy,control:async js=>{const r=await fetch(`http://127.0.0.1:${controlPort}/eval?js=${encodeURIComponent(js)}`);const d=await r.json();assert(r.ok,JSON.stringify(d));return d}};apps.push(app);
    if(shared)await api(base,`/api/apps/${app.id}/sharing`,'PUT',{enabled:true,password:'ime-test-only'});
    await api(base,`/api/apps/${app.id}/start`,'POST');await wait(`${name} editor`,()=>app.control('!!document.getElementById("text")'));return app;
}
let passed=0;
const pass=message=>{passed++;console.log('PASS '+message)};
async function video(view,label){
    return wait(label+' video',async()=>{if(await view.locator('#playButton').isVisible())await view.locator('#playButton').click();return view.evaluate(()=>document.querySelector('video')?.videoWidth>0)},label.startsWith('chromium')?30000:5000).catch(async error=>{
        if(label.startsWith('chromium')||process.env.REQUIRE_VIDEO==='1')throw error;
        console.log('NOTE '+label+': Linux headless WebKit video presentation was not verified');
        return false;
    });
}
async function select(page,id){
    await page.locator(`.toolbar-list [data-app-id="${id}"]`).click();
    await page.waitForFunction(id=>document.querySelector(`.toolbar-list [data-app-id="${id}"]`)?.classList.contains('focused'),id);
    await page.waitForFunction(()=>!document.querySelector('.app-stream.active')||document.querySelector('.app-stream.active iframe')?.contentWindow?.__ivncInput?.inputAttached);
    const url=await page.evaluate(()=>document.querySelector('.app-stream.active iframe')?.src);
    return url?page.frames().find(f=>f.url()===url):page;
}
async function exercise(page,view,app,label){
    const presented=await video(view,label);
    await app.control('document.getElementById("text").value="";document.getElementById("text").focus();true');
    await view.waitForFunction(legacy=>{const s=window.__ivncInput?.textInput.remote;return s?.commit===1&&!!s.focus&&(legacy?!s.enabled:s.enabled)},app.legacy);
    await view.waitForFunction(()=>window.__ivncInput.textInput.focused());
    await view.evaluate(()=>{window.__imeResults=[];const rtc=window.__ivncWebRTC,handler=rtc.ontextinputresult;rtc.ontextinputresult=data=>{window.__imeResults.push(data);handler(data)}});
    const text=label+'中文,🙂';await page.keyboard.insertText(text);
    await wait(label+' real text',async()=>await app.control('document.getElementById("text").value')===text);
    await view.waitForFunction(legacy=>window.__imeResults.some(r=>r.status===(legacy?'applied':'dispatched')),app.legacy);
    assert.equal(await view.evaluate(()=>window.__ivncInput.textCommits.pending.size),0);
    pass(label+': Unicode reaches the application once, with a connection-local receipt');
    if(!app.legacy&&presented){
        const before=await view.evaluate(()=>[window.__ivncInput._imeTextarea.style.left,window.__ivncInput._imeTextarea.style.top]);
        await app.control('document.getElementById("text").value="x";document.getElementById("text").setSelectionRange(1,1);true');
        await view.waitForFunction(before=>{const el=window.__ivncInput._imeTextarea;return el.style.left!==before[0]||el.style.top!==before[1]},before);
        const geometry=await view.evaluate(()=>{const i=window.__ivncInput,r=i.textInput.remote.cursor,v=document.querySelector('video').getBoundingClientRect(),s=i.displayState.desktop,contain=document.querySelector('video').style.objectFit==='contain',video=document.querySelector('video'),scale=Math.min(v.width/video.videoWidth,v.height/video.videoHeight),w=contain?video.videoWidth*scale:v.width,h=contain?video.videoHeight*scale:v.height;return {actual:[parseFloat(i._imeTextarea.style.left),parseFloat(i._imeTextarea.style.top)],expected:[v.left+(v.width-w)/2+r[0]*w/s.width,v.top+(v.height-h)/2+(r[1]+r[3])*h/s.height]}});
        for(let n=0;n<2;n++)assert(Math.abs(geometry.actual[n]-geometry.expected[n])<1);
        pass(label+': reported editor caret continuously repositions the browser proxy');
    }
}
async function queuedPaste(page,view,app,label){
    await app.control('document.getElementById("text").value="";document.getElementById("text").focus();true');
    await view.evaluate(()=>{
        const rtc=window.__ivncWebRTC,original=rtc.onclipboardmessage;window.__heldPaste=[];
        rtc.onclipboardmessage=(type,data)=>{if(type==='paste_ack')window.__heldPaste.push([type,data]);else original(type,data)};
        window.__releasePaste=()=>{rtc.onclipboardmessage=original;for(const [type,data]of window.__heldPaste)original(type,data)};
        const el=window.__ivncInput._imeTextarea;
        window.__ivncClipboard.beginPaste({target:el,preventDefault(){},stopPropagation(){}},'ctrl-v');
        const data=new DataTransfer();data.setData('text/plain','先粘贴-');
        el.dispatchEvent(new ClipboardEvent('paste',{bubbles:true,cancelable:true,clipboardData:data}));
    });
    await view.waitForFunction(()=>window.__heldPaste.length>0);
    await page.keyboard.insertText('队列中文');
    // Wait past the receipt timeout while deliberately withholding the previous
    // paste's result. The following edit has not left the ordered queue yet.
    await delay(3200);
    assert.equal(await app.control('document.getElementById("text").value'),'先粘贴-');
    assert(await view.evaluate(()=>window.__ivncInput.textCommits.pending.size===1&&[...window.__ivncInput.textCommits.pending.values()].every(p=>p.timer===null)));
    await view.evaluate(()=>window.__releasePaste());
    await wait(label+' queued text',async()=>await app.control('document.getElementById("text").value')==='先粘贴-队列中文');
    await view.waitForFunction(()=>window.__ivncInput.textCommits.pending.size===0);
    pass(label+': text waits for the preceding paste and starts its receipt timer only after send');
}
try{
    const baseA=await instance('a'),baseB=await instance('b');
    const local=await editor(baseA,'IME Local'),shared=await editor(baseA,'IME Shared Legacy',true,true),remoteEditor=await editor(baseB,'IME Remote',true);
    const remoteResult=await api(baseA,'/api/apps','POST',{app_type:'remote',url:`${baseB}/remote/v1/apps/${remoteEditor.id}`,password:'ime-test-only'});
    const remote={base:baseA,id:remoteResult.app.id};apps.push(remote);await api(baseA,`/api/apps/${remote.id}/start`,'POST');
    await wait('A -> B connection',async()=>(await api(baseA,`/api/apps/${remote.id}`)).remote_state==='connected');
    for(const name of (process.env.BROWSERS||'chromium,webkit').split(',')){
        const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
        try{
            const page=await browser.newPage({viewport:{width:1152,height:820},deviceScaleFactor:2}),errors=[];page.on('pageerror',e=>errors.push(e.message));
            await page.goto(baseA);await page.waitForFunction(()=>window.__ivncInput?.inputAttached,{timeout:60000});
            await select(page,local.id);
            assert.deepEqual(await page.locator('.toolbar-tools .taskbar-pin').evaluateAll(nodes=>nodes.slice(0,3).map(n=>n.id||n.classList.contains('toolbar-display-mode')&&'display'||'pin')),['pin','display','keyboard-toggle']);
            assert.equal(await page.locator('.toolbar-display-mode').getAttribute('aria-pressed'),'false');
            await page.locator('#keyboard-toggle').click();await exercise(page,page,local,name+'/main');
            pass(name+': toolbar order and fit default match the production UI');
            await page.reload();await page.waitForFunction(()=>window.__ivncInput?.inputAttached&&window.__ivncInput.textInput.focused());
            assert.equal(await page.locator('#keyboard-toggle').getAttribute('aria-pressed'),'true');pass(name+': refresh preserves desktop IME preference and focus');
            const sharedView=await select(page,shared.id);await exercise(page,sharedView,shared,name+'/shared-legacy');
            const remoteView=await select(page,remote.id);await exercise(page,remoteView,remoteEditor,name+'/A-to-B');
            assert.equal(await sharedView.evaluate(()=>window.__ivncInput.textInput.focused()),false);
            assert.equal(await page.locator('#keyboard-toggle').getAttribute('aria-pressed'),'true');
            pass(name+': shared/remote switching preserves mode and keeps hidden views unfocused');
            if(name==='chromium'){
                await remoteEditor.control('document.getElementById("text").value="";document.getElementById("text").focus();true');
                const cdp=await page.context().newCDPSession(page);
                try{await cdp.send('Input.imeSetComposition',{text:'远程候选',selectionStart:4,selectionEnd:4});await cdp.send('Input.insertText',{text:'远程候选🙂'});}finally{await cdp.detach()}
                await wait('native remote composition',async()=>await remoteEditor.control('document.getElementById("text").value')==='远程候选🙂');
                pass(name+': native browser composition reaches B once without preedit leakage');
            }
            await remoteView.evaluate(()=>window.__ivncWebRTC.reset());await remoteView.waitForFunction(()=>window.__ivncInput?.inputAttached&&window.__ivncInput.textInput.focused());
            await exercise(page,remoteView,remoteEditor,name+'/A-to-B-reconnected');
            pass(name+': remote reconnection restores desired IME state idempotently');
            await queuedPaste(page,remoteView,remoteEditor,name+'/A-to-B');
            await page.evaluate(()=>{const modal=document.createElement('div');modal.className='web-terminal-modal';modal.id='ime-local-popup';modal.style.cssText='position:fixed;top:80px;left:80px;z-index:99999';modal.innerHTML='<input id="ime-local-edit" class="allow-native-input">';document.body.appendChild(modal);modal.firstChild.focus()});
            await remoteView.evaluate(()=>parent.postMessage({type:'ivnc-view-ready',view:location.pathname},location.origin));
            await delay(200);assert.equal(await page.evaluate(()=>document.activeElement.id),'ime-local-edit');
            assert.equal(await remoteView.evaluate(()=>window.__ivncInput.textInput.focused()),false);
            await page.evaluate(()=>document.getElementById('ime-local-popup').remove());await select(page,remote.id);await remoteView.waitForFunction(()=>window.__ivncInput.textInput.focused());
            pass(name+': repeated view-ready synchronization preserves focus in a local popup');
            const saved=await remoteView.evaluate(()=>({...window.__ivncInput.textInput.remote}));
            const oldText=await remoteEditor.control('document.getElementById("text").value');
            await remoteView.evaluate(()=>{const i=window.__ivncInput;i.setTextInputState({...i.textInput.remote,focus:'00000000-0000-4000-8000-000000000001'});i._sendCommittedText('不得进入错误窗口')});
            await page.locator('.upload-toast.error').filter({hasText:/输入目标已变化|input target changed/}).waitFor();
            assert.equal(await remoteEditor.control('document.getElementById("text").value'),oldText);
            await remoteView.evaluate(state=>window.__ivncInput.setTextInputState(state),saved);
            pass(name+': stale focus is rejected by B and the active parent shows an actionable error');
            await remoteView.evaluate(()=>{const el=window.__ivncInput._imeTextarea;el.focus();el.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}))});
            await select(page,local.id);await remoteView.evaluate(()=>window.__ivncInput._imeTextarea.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'切换后晚到文字'})));
            assert.equal(await remoteEditor.control('document.getElementById("text").value'),oldText);
            await page.waitForFunction(()=>window.__ivncInput.textInput.focused());
            pass(name+': returning to the desktop restores focus and cancels the old composition');
            await select(page,remote.id);await remoteView.waitForFunction(()=>window.__ivncInput.textInput.focused());await page.keyboard.press('Control+Shift+Space');
            await page.waitForFunction(()=>document.getElementById('keyboard-toggle').getAttribute('aria-pressed')==='false');
            await page.reload();await page.waitForFunction(()=>window.__ivncInput?.inputAttached);assert.equal(await page.evaluate(()=>window.__ivncInput.textInput.enabled),false);
            assert.deepEqual(errors,[]);pass(name+': embedded shortcut persists to the owning desktop with no browser exceptions');
        }finally{await browser.close()}
    }
    console.log(`PASS ${passed} IME end-to-end checks; artifacts: ${work}`);
}finally{
    for(const app of apps.reverse()){try{await api(app.base,`/api/apps/${app.id}/stop`,'POST')}catch{}}
    for(const server of servers){try{process.kill(-server.pid,'SIGTERM')}catch{}}
    await delay(800);
    for(const server of servers){try{process.kill(-server.pid,'SIGKILL')}catch{}}
    console.log('Artifacts: '+work);
}
