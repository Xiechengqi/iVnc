// Real mobile browser -> WebRTC -> Electron Wayland editor, main and worker.
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
const work=mkdtempSync(path.join(tmpdir(),'ivnc-mobile-e2e-'));
const electron=process.env.ELECTRON_BIN;
assert(electron,'Set ELECTRON_BIN to an installed Electron 38+ runtime');
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p;}
const httpPort=await port(), controlPort=await port(), base=`http://127.0.0.1:${httpPort}`;
const env={...process.env,IVNC_HOME:path.join(work,'ivnc'),XDG_RUNTIME_DIR:path.join(work,'run'),XDG_CONFIG_HOME:path.join(work,'config'),XDG_CACHE_HOME:path.join(work,'cache'),XDG_DATA_HOME:path.join(work,'data')};
for(const key of ['IVNC_HOME','XDG_RUNTIME_DIR','XDG_CONFIG_HOME','XDG_CACHE_HOME','XDG_DATA_HOME'])mkdirSync(env[key],{mode:0o700});
const log=openSync(path.join(work,'server.log'),'a',0o600);
const server=spawn(process.env.IVNC_BIN||path.join(root,'target/debug/ivnc'),['--http-port',String(httpPort),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','960','--height','720'],{env,cwd:root,detached:true,stdio:['ignore',log,log]});closeSync(log);
async function wait(name,fn,ms=30000){const end=Date.now()+ms;let last;while(Date.now()<end){try{const v=await fn();if(v)return v;}catch(e){last=e;}await delay(75);}throw Error(`${name}: ${last||'timed out'}`);}
async function api(url,method='GET',body){const r=await fetch(base+url,{method,headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});const d=await r.json();assert(r.ok,JSON.stringify(d));return d;}
async function control(js){const r=await fetch(`http://127.0.0.1:${controlPort}/eval?`+new URLSearchParams({js}));const d=await r.json();assert(r.ok,JSON.stringify(d));return d;}
const quote=s=>"'"+s.replaceAll("'","'\\''")+"'";
let id,passed=0;
const pass=s=>{passed++;console.log(`PASS ${s}`);};
try{
    await wait('server',()=>api('/api/apps'));
    const result=await api('/api/apps','POST',{name:'Mobile Keyboard Fixture',app_type:'desktop',autostart:false,exec_command:[electron,path.join(root,'scripts/fixtures/electron-wayland/main.cjs')].map(quote).join(' '),env_vars:{ELECTRON_CONTROL_PORT:String(controlPort)}});id=result.app.id;
    for(const shared of [false,true]){
        if(shared)await api(`/api/apps/${id}/sharing`,'PUT',{enabled:true,password:'mobile-test-only'});
        await api(`/api/apps/${id}/start`,'POST');
        await wait('Electron editor',()=>control('!!document.getElementById("text")'));
        for(const name of (process.env.BROWSERS||'chromium,webkit').split(',')){
            const label=`${name}/${shared?'shared':'desktop'}`;
            const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
            try{
                const page=await browser.newPage({...engines.devices['iPhone 13'],viewport:{width:390,height:844}}),errors=[];
                page.on('pageerror',e=>errors.push(e.message));
                await page.goto(base);
                await page.waitForFunction(()=>window.__ivncInput?.inputAttached,{timeout:60000});
                let view=page;
                if(shared){
                    await page.locator('.toolbar-list .taskbar-item').filter({hasText:'Mobile Keyboard Fixture'}).first().tap();
                    await page.waitForFunction(()=>document.querySelector('.app-stream.active iframe')?.contentWindow?.__ivncInput?.inputAttached,{timeout:60000});
                    view=page.frames().find(f=>f.url().includes('/app-view/'));
                }
                await wait('video frame',async()=>{
                    if(await view.locator('#playButton').isVisible()) await view.locator('#playButton').tap();
                    return await view.evaluate(()=>document.querySelector('video')?.videoWidth>0);
                },name==='chromium'?30000:5000).catch(async error=>{
                    const diagnostic=await view.evaluate(async()=>({state:window.__ivncWebRTC.playback.state(),video:(await window.__ivncWebRTC.getConnectionStats())?.video,ready:document.querySelector('video')?.readyState}));
                    if(name==='chromium'||process.env.REQUIRE_VIDEO==='1')throw error;
                    // Linux headless WebKit may decode RTP without presenting
                    // video. Still exercise its real keyboard/DataChannel path,
                    // and report the separate media limitation explicitly.
                    console.log(`NOTE ${label}: video presentation not verified on Linux headless WebKit`,diagnostic);
                });
                await control('document.getElementById("text").value="";document.getElementById("text").focus();true');
                await view.waitForFunction(()=>window.__ivncInput?.textInput.remote?.enabled,{timeout:30000});
                const metadata=await view.evaluate(()=>window.__ivncInput.textInput.remote);
                assert.equal(metadata.protocol,1);assert(metadata.focus);assert(!('surrounding_text' in metadata));
                pass(`${label}: editor state reaches the mobile viewer without surrounding text`);
                assert.equal(await page.locator('#keyboard-toggle').getAttribute('aria-pressed'),'false');
                await page.locator('#keyboard-toggle').tap();
                assert(await view.evaluate(()=>window.__ivncInput.textInput.focused()));
                await page.keyboard.insertText('Mobile-中文🙂');
                await wait('committed mobile text',async()=>await control('document.getElementById("text").value')==='Mobile-中文🙂');
                pass(`${label}: toolbar keyboard and Unicode input reach the real application once`);
                await page.keyboard.press('Backspace');
                await wait('remote Backspace',async()=>await control('document.getElementById("text").value')==='Mobile-中文');
                await page.keyboard.press('Enter');await page.keyboard.insertText('next');
                await wait('remote newline',async()=>await control('document.getElementById("text").value')==='Mobile-中文\nnext');
                pass(`${label}: deletion and Enter retain native editor behavior`);
                await view.evaluate(()=>{
                    const el=window.__ivncInput._imeTextarea;
                    el.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:''}));
                    el.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'候选'}));
                    el.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:'候选'}));
                });
                await wait('composition committed once',async()=>await control('document.getElementById("text").value')==='Mobile-中文\nnext候选');
                pass(`${label}: composition and final input are deduplicated end to end`);
                if(name==='chromium'){
                    const cdp=await page.context().newCDPSession(page);
                    try{
                        await cdp.send('Input.imeSetComposition',{text:'原生',selectionStart:2,selectionEnd:2});
                        await cdp.send('Input.imeSetComposition',{text:'原生🙂',selectionStart:4,selectionEnd:4});
                        await cdp.send('Input.insertText',{text:'原生🙂'});
                        await wait('browser-native composition',async()=>await control('document.getElementById("text").value')==='Mobile-中文\nnext候选原生🙂');
                        pass(`${label}: browser-native IME composition commits once without preedit leakage`);
                    }finally{await cdp.detach();}
                }
                await page.screenshot({path:path.join(work,`${name}-${shared?'shared':'desktop'}.png`)});
                await page.locator('#keyboard-toggle').tap();
                assert.equal(await view.evaluate(()=>window.__ivncInput.textInput.focused()),false);
                await page.locator('#keyboard-toggle').tap();
                await view.evaluate(()=>window.dispatchEvent(new Event('ivnc-auth-ended')));
                assert.equal(await view.evaluate(()=>window.__ivncInput.textInput.enabled),false);
                assert.equal(await view.evaluate(()=>window.__ivncInput.textInput.focused()),false);
                assert.deepEqual(errors,[]);pass(`${label}: keyboard dismisses cleanly with no browser exceptions`);
            }finally{await browser.close();}
        }
        await api(`/api/apps/${id}/stop`,'POST');
        await wait('application stop',async()=>{try{await control('true');return false;}catch{return true;}});
    }
    console.log(`PASS ${passed} mobile input end-to-end checks; artifacts: ${work}`);
}finally{
    try{if(id)await api(`/api/apps/${id}/stop`,'POST');}catch{}
    try{process.kill(-server.pid,'SIGTERM');}catch{}
    await Promise.race([new Promise(r=>server.once('exit',r)),delay(3000)]);
    try{process.kill(-server.pid,'SIGKILL');}catch{}
    console.log(`Artifacts: ${work}`);
}
