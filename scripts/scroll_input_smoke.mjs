// Production frontend -> WebRTC -> real Chrome/GTK. Local isolated instances.
// Optional: LEGACY_IVNC_BIN=/path/to/old/ivnc tests a genuinely older peer.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, writeFileSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const root=path.resolve(new URL('..',import.meta.url).pathname);
const {chromium}=createRequire(new URL('../web/ivnc/package.json',import.meta.url))('playwright');
const work=mkdtempSync(path.join(tmpdir(),'ivnc-scroll-live-')),servers=[],apps=[],connections=[];
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
async function wait(name,fn,ms=30000){const end=Date.now()+ms;let last;while(Date.now()<end){try{const v=await fn();if(v)return v}catch(e){last=e}await delay(60)}throw Error(`${name}: ${last||'timed out'}`)}
async function api(base,url,method='GET',body){const r=await fetch(base+url,{method,headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});const d=await r.json();assert(r.ok,JSON.stringify(d));return d}
const quote=s=>`'${s.replaceAll("'","'\\''")}'`;
async function instance(name,binary=process.env.IVNC_BIN||path.join(root,'target/debug/ivnc')){
    const base=`http://127.0.0.1:${await port()}`,env={...process.env};
    for(const [key,dir] of Object.entries({IVNC_HOME:'ivnc',XDG_RUNTIME_DIR:'run',XDG_CONFIG_HOME:'config',XDG_CACHE_HOME:'cache',XDG_DATA_HOME:'data'})){
        env[key]=path.join(work,name,dir);mkdirSync(env[key],{recursive:true,mode:0o700});
    }
    const config=path.join(work,name,'config.toml');writeFileSync(config,'[audio]\nenabled=false\nsample_rate=48000\nchannels=2\nbitrate=128000\n[shell_profile]\napps=false\nterminal=false\n');
    const log=openSync(path.join(work,name,'server.log'),'a',0o600);
    const server=spawn(binary,['--config',config,'--http-port',new URL(base).port,'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','960','--height','720'],{env,cwd:root,detached:true,stdio:['ignore',log,log]});closeSync(log);servers.push(server);
    await wait(name+' server',()=>api(base,'/api/apps'));return base;
}
async function chrome(base,name,shared=false){
    const debug=await port();
    const result=await api(base,'/api/apps','POST',{name,app_type:'desktop',desktop_preset:'chrome',chrome_config:{debug_port:debug},autostart:false});
    const app={base,id:result.app.id};apps.push(app);
    if(shared)await api(base,`/api/apps/${app.id}/sharing`,'PUT',{enabled:true,password:'scroll-test-only'});
    await api(base,`/api/apps/${app.id}/start`,'POST');
    const connection=await wait(name+' CDP',()=>chromium.connectOverCDP(`http://127.0.0.1:${debug}`,{timeout:1000}));connections.push(connection);
    app.page=connection.contexts()[0].pages()[0];
    await app.page.goto('data:text/html,'+encodeURIComponent('<style>html{scroll-behavior:auto}body{margin:0;width:10000px;height:100000px}</style><h1>Scroll input fixture</h1>'));
    await app.page.evaluate(()=>{window.wheelTotal={x:0,y:0,count:0};addEventListener('wheel',event=>{wheelTotal.x+=event.deltaX;wheelTotal.y+=event.deltaY;wheelTotal.count++},{passive:true})});
    return app;
}
async function remote(base,target,name){
    const result=await api(base,'/api/apps','POST',{name,app_type:'remote',url:`${target.base}/remote/v1/apps/${target.id}`,password:'scroll-test-only'});
    const app={base,id:result.app.id,page:target.page};apps.push(app);
    await api(base,`/api/apps/${app.id}/start`,'POST');
    await wait(name+' connection',async()=>(await api(base,`/api/apps/${app.id}`)).remote_state==='connected');return app;
}
async function select(page,app){
    await page.locator(`.toolbar-list [data-app-id="${app.id}"]`).click();
    await page.waitForFunction(id=>document.querySelector(`.toolbar-list [data-app-id="${id}"]`)?.classList.contains('focused'),app.id);
    await page.waitForFunction(()=>!document.querySelector('.app-stream.active')||document.querySelector('.app-stream.active iframe')?.contentWindow?.__ivncInput?.inputAttached);
    const url=await page.evaluate(()=>document.querySelector('.app-stream.active iframe')?.src);
    const view=url?page.frames().find(f=>f.url()===url):page;
    await view.waitForFunction(()=>document.querySelector('video')?.videoWidth>0,{timeout:30000});
    await view.evaluate(()=>{
        if(window.__scrollWire)return;
        window.__scrollWire=[];
        const rtc=window.__ivncWebRTC,send=rtc.sendDataChannelMessage.bind(rtc);
        rtc.sendDataChannelMessage=message=>{if(/^(i,\d+,)?w2?,/.test(message))__scrollWire.push(message);return send(message)};
    });
    // Establish the compositor's pointer focus before scrolling.
    const box=await page.locator(url?'.app-stream.active iframe':'#overlayInput').boundingBox();
    await page.mouse.move(box.x+box.width*.5,box.y+box.height*.5);await delay(150);return view;
}
async function setSpeed(page,speed){
    await page.locator('#console-btn').click();
    const settings=page.frameLocator('#settings-frame');
    await settings.locator('#nav-advanced').click();
    await settings.locator('#scroll-speed').evaluate((element,speed)=>{element.value=String(speed);element.dispatchEvent(new Event('input',{bubbles:true}))},speed);
    await page.locator('#web-console-minimize').click();
}
async function gtk(base,page){
    let flags;
    try{flags=execFileSync('pkg-config',['--cflags','--libs','gtk+-3.0'],{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim().split(/\s+/)}catch{console.log('NOTE GTK smoke requires the GTK3 development package');return}
    const binary=path.join(work,'scroll-gtk'),state=path.join(work,'gtk-state.json');
    execFileSync('cc',[path.join(root,'scripts/fixtures/scroll-gtk.c'),...flags,'-o',binary]);
    const result=await api(base,'/api/apps','POST',{name:'GTK scroll',app_type:'desktop',exec_command:quote(binary),autostart:false,env_vars:{SCROLL_GTK_STATE:state}});
    const app={base,id:result.app.id};apps.push(app);await api(base,`/api/apps/${app.id}/start`,'POST');
    const snapshot=()=>JSON.parse(readFileSync(state,'utf8'));
    await wait('GTK fixture state',snapshot);
    const distances=[];
    for(const speed of [1,2]){
        await setSpeed(page,speed);const view=await select(page,app),before=snapshot();
        await view.evaluate(()=>document.getElementById('overlayInput').dispatchEvent(new WheelEvent('wheel',{deltaY:120,deltaX:0,clientX:innerWidth*.5,clientY:innerHeight*.5,bubbles:true,cancelable:true})));
        await wait('GTK smooth scroll',()=>snapshot().count>before.count);await delay(250);
        const after=snapshot(),delta=after.y-before.y,top=after.top-before.top;
        assert(Math.abs(delta-speed)<.001,JSON.stringify({speed,before,after}));assert(top>0);
        distances.push(top);console.log(`PASS GTK: 120px at ${speed}× -> smooth delta=${delta}, moved=${top}`);
    }
    assert(Math.abs(distances[1]/distances[0]-2)<.1,JSON.stringify(distances));
}
async function exercise(page,app,label){
    let view;
    for(const [speed,amount] of [[1,100],[.5,100],[2,100]]){
        await setSpeed(page,speed);
        view=await select(page,app);
        await app.page.evaluate(()=>{scrollTo(0,0);wheelTotal={x:0,y:0,count:0}});await delay(100);
        await view.evaluate(amount=>document.getElementById('overlayInput').dispatchEvent(new WheelEvent('wheel',{deltaY:amount,deltaX:0,clientX:innerWidth*.5,clientY:innerHeight*.5,bubbles:true,cancelable:true})),amount);
        await wait(label+' wheel',()=>app.page.evaluate(()=>wheelTotal.count>0));await delay(400);
        const got=await app.page.evaluate(()=>({...wheelTotal,top:scrollY}));
        assert(Math.abs(got.y-amount*speed)<=12 && Math.abs(got.top-amount*speed)<=13,`${label} ${speed}×: ${JSON.stringify(got)}`);
        console.log(`PASS ${label}: ${amount}px at ${speed}× -> wheel=${got.y}, scroll=${got.top}`);
    }
    await app.page.evaluate(()=>{scrollTo(0,0);wheelTotal={x:0,y:0,count:0}});await delay(100);
    await view.evaluate(async()=>{
        const el=document.getElementById('overlayInput');
        for(let n=0;n<60;n++){el.dispatchEvent(new WheelEvent('wheel',{deltaX:0,deltaY:10,clientX:innerWidth*.5,clientY:innerHeight*.5,bubbles:true,cancelable:true}));await new Promise(r=>setTimeout(r,16))}
    });await delay(400);
    const total=await app.page.evaluate(()=>({...wheelTotal,top:scrollY}));
    assert(Math.abs(total.y-1200)<=14,`${label} continuous trace: ${JSON.stringify(total)}`);
    console.log(`PASS ${label}: 60 samples retained wheel=${total.y}, scroll=${total.top}, events=${total.count}`);
    const kinds=await view.evaluate(()=>__scrollWire.map(message=>message.replace(/^i,\d+,/,'').split(',')[0]));
    assert(kinds.length && kinds.every(kind=>kind===(label.includes('old B')?'w':'w2')),JSON.stringify(kinds));
    if(!label.includes('old B')){
        await app.page.evaluate(()=>{scrollTo(0,0);wheelTotal={x:0,y:0,count:0}});await delay(100);
        await view.evaluate(async()=>{
            const el=document.getElementById('overlayInput');
            for(let n=0;n<128;n++){
                el.dispatchEvent(new WheelEvent('wheel',{deltaX:0,deltaY:.0078125,clientX:innerWidth*.5,clientY:innerHeight*.5,bubbles:true,cancelable:true}));
                await new Promise(r=>setTimeout(r,8));
            }
        });await delay(300);
        const tiny=await app.page.evaluate(()=>({...wheelTotal,top:scrollY}));
        assert(Math.abs(tiny.y-2)<.047 && Math.abs(tiny.top-2)<=1,JSON.stringify(tiny));
        console.log(`PASS ${label}: sub-Wayland-quantum samples retained wheel=${tiny.y}, scroll=${tiny.top}`);
    }
}
let viewer;
try{
    const a=await instance('a'),b=await instance('b');
    const local=await chrome(a,'Local Chrome'),shared=await chrome(a,'Shared Chrome',true),other=await chrome(b,'Remote Chrome',true),linked=await remote(a,other,'A to B');
    viewer=await chromium.launch({headless:true,executablePath:process.env.CHROME_BIN||'/usr/bin/google-chrome'});
    const page=await viewer.newPage({viewport:{width:1152,height:820},deviceScaleFactor:2});
    await page.goto(a);await page.waitForFunction(()=>window.__ivncInput?.inputAttached,{timeout:60000});
    for(const [app,label] of [[local,'main'],[shared,'shared'],[linked,'A -> B']])await exercise(page,app,label);
    await gtk(a,page);
    if(process.env.LEGACY_IVNC_BIN){
        const legacy=await instance('legacy',process.env.LEGACY_IVNC_BIN),old=await chrome(legacy,'Legacy Chrome',true),linkedOld=await remote(a,old,'A to old B');
        await exercise(page,linkedOld,'A -> old B');
    }
    console.log('Scroll smoke passed. Logs: '+work);
}finally{
    await viewer?.close();
    for(const connection of connections)await connection.close().catch(()=>{});
    for(const app of apps.reverse())await api(app.base,`/api/apps/${app.id}/stop`,'POST').catch(()=>{});
    for(const server of servers.reverse()){try{process.kill(-server.pid,'SIGTERM')}catch{}}
    for(const server of servers)await Promise.race([new Promise(r=>server.exitCode!==null?r():server.once('exit',r)),delay(6000)]);
    console.log('Artifacts: '+work);
}
