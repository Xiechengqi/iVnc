// Production display-mode entry points, preferences, streams and viewport CSS.
// Media is replaced by fixed dimensions; viewport_smoke covers real WebRTC.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const { createServer } = require('vite'), engines = require('playwright');
const root = fileURLToPath(new URL('../web/ivnc/', import.meta.url));
const source = readFileSync(path.join(root, 'ivnc-wr-core.js'), 'utf8');
const slice = (from,to) => {
    const start=source.indexOf(from),end=source.indexOf(to,start);
    assert(start>=0 && end>start,`production source: ${from}`);
    return source.slice(start,end);
};
const styles=source.match(/function InitUI\(\) \{[\s\S]*?style\.textContent = `([\s\S]*?)`;/)[1];
const fixture=`<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>${styles}#toolbar{position:fixed;bottom:0;height:36px;width:100%;background:#333;z-index:1000}</style>
<div id="app"><div class="video-container"><video id="stream"></video><div id="overlayInput"></div></div></div>
<div id="toolbar"><button id="mode"><span class="toolbar-tool-label"></span></button></div>
<script type="module">
import {createViewport,normalizeDisplayMode} from '/viewport.js';
import {createAppStreams} from '/app-streams.js';
import {getLang,onLangChange,setLang} from '/lib/i18n.js?v=1';
window.__IVNC_APP_VIEW__=location.pathname.startsWith('/app-view/');
window.__IVNC_EMBED__=window.__IVNC_APP_VIEW__ && parent!==window && new URLSearchParams(location.search).get('embed')==='1';
${slice('const urlForKey =','function formatNetworkMetric(')}
let displayMode='fit',displayModeButton=document.getElementById('mode'),scaleLocal=true,appStreams=null;
const container=document.querySelector('.video-container'),video=document.querySelector('video'),overlay=document.getElementById('overlayInput');
Object.defineProperties(video,{videoWidth:{get:()=>1920},videoHeight:{get:()=>1080}});
const viewportController=createViewport({root:document.getElementById('app'),container,video,overlay,toolbar:document.getElementById('toolbar'),isPinned:()=>true,embedded:window.__IVNC_EMBED__,onChange:()=>appStreams?.layout()});
${slice('function setDisplayMode(','function reportViewport()')}
${slice('function handleMessage(event)','function handleSettingsMessage(')}
${slice('// Migrate only the local fit/original preference;','encoder = getStringParam(')}
window.addEventListener('message',handleMessage);
${slice("displayModeButton.addEventListener('click',",'syncDisplayModeButton();')}
syncDisplayModeButton();onLangChange(syncDisplayModeButton);
if(window.__IVNC_EMBED__) document.getElementById('toolbar').remove();
if(!window.__IVNC_APP_VIEW__) {
    appStreams=createAppStreams({viewport:container,displayMode:()=>displayMode,onChange:()=>{},onEdgeHover:()=>{}});
    appStreams.update([
        {id:'shared',name:'Shared',kind:'shared',status:'running'},
        {id:'remote',name:'Remote',kind:'remote',status:'connected'},
        {id:'unopened',name:'Unopened',kind:'shared',status:'running'}
    ]);
}
const snapshot=()=>{
    const rect=e=>{const r=e.getBoundingClientRect();return {top:r.top,bottom:r.bottom,left:r.left,width:r.width,height:r.height}};
    return {mode:displayMode,area:rect(container),video:rect(video),input:rect(overlay),scroll:[container.scrollLeft,container.scrollTop],stored:localStorage.getItem(storageAppName+'_display_mode')};
};
window.fixture={legacyMode(){localStorage.setItem(storageAppName+'_display_mode','native')},streams:appStreams,viewport:viewportController,setDisplayMode,snapshot,setLang,received:[],
    ready(){parent.postMessage({type:'ivnc-view-ready',view:location.pathname},location.origin)},
    scroll(){container.scrollLeft=200;container.scrollTop=100}
};
addEventListener('message',e=>fixture.received.push(e.data.type));
if(window.__IVNC_EMBED__) {
    fixture.ready();parent.postMessage({type:'ivnc-view-state',state:'connected',view:location.pathname},location.origin);
}
</script>`;

const settled=page=>page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
const mode=(page,value)=>page.evaluate(value=>window.postMessage({type:'setDisplayMode',value},location.origin),value);
const activate=async(page,id)=>{
    await page.evaluate(id=>fixture.streams.activate(id),id);
    if(id)await page.waitForFunction(id=>[...document.querySelectorAll('.app-stream iframe')].some(f=>f.title===(id==='shared'?'Shared':'Remote') && f.contentWindow.fixture),id,{timeout:10000});
};
const modes=page=>page.evaluate(()=>fixture.streams.frames().map(f=>f.contentWindow.fixture.snapshot().mode));
const allModes=async(page,value)=>page.waitForFunction(value=>fixture.streams.frames().length>0 && fixture.streams.frames().every(f=>f.contentWindow.fixture?.snapshot().mode===value),value,{timeout:10000});
const cases=[
    ['hidden shared and remote views follow mode changes made on the main desktop',async page=>{
        await mode(page,'fill');await activate(page,'shared');await activate(page,'remote');await activate(page,null);
        assert.deepEqual(await modes(page),['fill','fill']);
        await mode(page,'fit');await allModes(page,'fit');
        await activate(page,'shared');await settled(page);
        const b=await page.evaluate(()=>{const f=fixture.streams.frames()[0];return {parent:fixture.snapshot(),child:f.contentWindow.fixture.snapshot(),frame:f.getBoundingClientRect().height}});
        assert.equal(b.parent.area.height,956);assert.equal(b.frame,956);assert.equal(b.child.area.height,956);
        assert(b.child.video.bottom<=956);assert.deepEqual(b.child.video,b.child.input);
        assert.equal(await page.locator('.app-stream iframe').count(),2);
    }],
    ['toolbar and compatibility controls synchronize every view without replacing frames or stealing focus',async page=>{
        let mints=0;page.on('request',request=>{if(/\/api\/apps\/[^/]+\/view$/.test(request.url()))mints++});
        await activate(page,'shared');await activate(page,'remote');
        await page.evaluate(()=>{window.savedFrames=fixture.streams.frames();fixture.received=[];document.getElementById('mode').focus()});
        await page.locator('#mode').click();await allModes(page,'fill');
        assert(await page.evaluate(()=>document.activeElement.id==='mode'));
        const extra=await page.evaluate(()=>fixture.streams.frames().flatMap(f=>f.contentWindow.fixture.received).filter(t=>['ivnc-view-active','ivnc-unmute'].includes(t)).length);
        await page.evaluate(()=>window.postMessage({type:'setScaleLocally',value:true},location.origin));await allModes(page,'fit');
        await page.evaluate(()=>window.postMessage({type:'setScaleLocally',value:false},location.origin));await allModes(page,'fill');
        await page.evaluate(()=>window.postMessage({type:'setManualResolution',width:3840,height:2160},location.origin));await allModes(page,'fill');
        await page.evaluate(()=>window.postMessage({type:'resetResolutionToWindow'},location.origin));await allModes(page,'fit');
        assert(await page.evaluate(()=>savedFrames.every((f,i)=>f===fixture.streams.frames()[i])));
        assert.equal(await page.evaluate(()=>fixture.streams.frames().flatMap(f=>f.contentWindow.fixture.received).filter(t=>['ivnc-view-active','ivnc-unmute'].includes(t)).length),extra);
        assert.equal(mints,2,'mode sync never opens an unopened application or mints another token');
    }],
    ['reactivation corrects stale child state and repeated fill synchronization keeps all edges visible',async page=>{
        await page.setViewportSize({width:1280,height:800});await settled(page);
        await mode(page,'fill');await activate(page,'shared');await allModes(page,'fill');
        await page.evaluate(()=>fixture.streams.frames()[0].contentWindow.fixture.scroll());await settled(page);
        await activate(page,null);await activate(page,'shared');await settled(page);
        assert.deepEqual(await page.evaluate(()=>fixture.streams.frames()[0].contentWindow.fixture.snapshot().scroll),[0,0]);
        const child=page.frames().find(f=>f.url().includes('/app-view/shared/'));
        const received=await child.evaluate(()=>fixture.received.length);
        await child.evaluate(()=>parent.postMessage({type:'ivnc-view-ready',view:'/app-view/expired/'},location.origin));await settled(page);
        assert.equal(await child.evaluate(()=>fixture.received.length),received,'stale view identity is ignored');
        await child.evaluate(()=>fixture.ready());await settled(page);
        assert.deepEqual(await child.evaluate(()=>fixture.snapshot().scroll),[0,0]);
        const picture=await child.evaluate(()=>fixture.snapshot());assert.deepEqual(picture.video,picture.area);assert.deepEqual(picture.video,picture.input);
        await page.evaluate(()=>fixture.streams.frames()[0].contentWindow.fixture.setDisplayMode('fit',false));
        await activate(page,null);await activate(page,'shared');await allModes(page,'fill');
    }],
    ['a delayed iframe and a reloaded iframe apply the latest parent mode',async page=>{
        let release;const gate=new Promise(r=>release=r);
        await page.route('**/app-view/remote/?embed=1',async route=>{await gate;await route.continue()});
        await page.evaluate(()=>fixture.streams.activate('remote'));
        await mode(page,'fill');await mode(page,'fit');await mode(page,'fill');release();
        await allModes(page,'fill');await mode(page,'fit');await allModes(page,'fit');
        await page.evaluate(()=>{const f=fixture.streams.frames()[0];f.contentWindow.__displayReloading=true;f.contentWindow.location.reload()});
        await page.waitForFunction(()=>{const w=fixture.streams.frames()[0]?.contentWindow;return w?.fixture && !w.__displayReloading});
        await allModes(page,'fit');await settled(page);
        assert((await page.evaluate(()=>fixture.streams.frames()[0].contentWindow.fixture.snapshot())).video.bottom<=956);
    }],
    ['embedded views neither inherit nor overwrite the standalone view preference',async(page,url)=>{
        const standalone=await page.context().newPage();await standalone.goto(url+'/app-view/standalone/');
        await standalone.waitForFunction(()=>!!window.fixture);
        await mode(standalone,'fill');await settled(standalone);
        assert.equal((await standalone.evaluate(()=>fixture.snapshot())).stored,'fill');
        await activate(page,'shared');await allModes(page,'fit');
        assert.equal(await page.evaluate(()=>fixture.streams.frames()[0].contentWindow.fixture.snapshot().stored),'fill');
        await standalone.reload();await standalone.waitForFunction(()=>!!window.fixture);
        assert.equal((await standalone.evaluate(()=>fixture.snapshot())).mode,'fill');await standalone.close();
    }],
    ['unrelated frames and invalid values cannot change display modes',async page=>{
        await activate(page,'shared');await mode(page,'fill');await allModes(page,'fill');
        await page.frames().find(f=>f.url().includes('/app-view/shared/')).evaluate(()=>parent.postMessage({type:'setDisplayMode',value:'fit'},location.origin));
        await page.evaluate(()=>{
            const f=fixture.streams.frames()[0];
            window.postMessage({type:'setDisplayMode',value:'invalid'},location.origin);
            f.contentWindow.postMessage({type:'setDisplayMode',value:'invalid'},location.origin);
        });await settled(page);
        assert.equal((await page.evaluate(()=>fixture.snapshot())).mode,'fill');assert.deepEqual(await modes(page),['fill']);
        await page.evaluate(()=>{
            const frame=document.createElement('iframe');frame.className='web-console-frame';document.body.appendChild(frame);
        });
        const consoleFrame=page.frames().find(f=>f!==page.mainFrame() && !f.url().includes('/app-view/shared/'));
        await consoleFrame.evaluate(()=>parent.document.querySelector('.app-stream iframe').contentWindow.postMessage({type:'setDisplayMode',value:'fit'},parent.location.origin));await settled(page);
        assert.deepEqual(await modes(page),['fill'],'an embedded view only accepts its parent');
        await consoleFrame.evaluate(()=>parent.postMessage({type:'resetResolutionToWindow'},parent.location.origin));
        await allModes(page,'fit');
    }],
    ['a same-origin parent dashboard retains control of an embedded desktop page',async(page,url)=>{
        await page.evaluate(url=>{const f=document.createElement('iframe');f.id='dashboard-view';f.src=url+'/__display-test';document.body.appendChild(f)},url);
        const desktop=await page.locator('#dashboard-view').contentFrame();await desktop.locator('#mode').waitFor();
        await page.evaluate(()=>document.getElementById('dashboard-view').contentWindow.postMessage({type:'setDisplayMode',value:'fill'},location.origin));
        await page.waitForFunction(()=>document.getElementById('dashboard-view').contentWindow.fixture?.snapshot().mode==='fill');
    }],
    ['legacy original-size preferences, legacy flags and control messages migrate to fill',async(page,url)=>{
        await page.evaluate(()=>fixture.legacyMode());
        await page.reload();await page.waitForFunction(()=>!!window.fixture);
        let state=await page.evaluate(()=>fixture.snapshot());assert.equal(state.mode,'fill');assert.equal(state.stored,'fill');
        assert.deepEqual(state.video,state.area);assert.deepEqual(state.scroll,[0,0]);
        await mode(page,'fit');await settled(page);await mode(page,'native');await settled(page);
        assert.equal((await page.evaluate(()=>fixture.snapshot())).mode,'fill');
        await page.goto(url+'/__display-test?is_manual_resolution_mode=true&scaleLocallyManual=false');await page.waitForFunction(()=>!!window.fixture);
        assert.equal((await page.evaluate(()=>fixture.snapshot())).mode,'fill');
    }],
    ['mode hints distinguish current state and the next action in both languages',async page=>{
        for(const [language,current,next] of [['zh','适应窗口','铺满屏幕'],['en','Fit window','Fill screen']]) {
            await page.evaluate(language=>fixture.setLang(language),language);await mode(page,'fit');await settled(page);
            const title=await page.locator('#mode').getAttribute('title');assert(title.includes(current) && title.includes(next));
            assert.equal(await page.locator('#mode').getAttribute('aria-label'),title);
            assert.equal(await page.locator('#mode').getAttribute('aria-pressed'),'false');
            await page.locator('#mode').click();assert.equal(await page.locator('#mode').getAttribute('aria-pressed'),'true');
        }
    }],
];

await test('display modes across application views',async t=>{
    const work=mkdtempSync(path.join(tmpdir(),'ivnc-display-mode-'));
    const server=await createServer({configFile:false,root,cacheDir:path.join(work,'vite'),logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]},server:{host:'127.0.0.1',port:0,hmr:false},plugins:[{name:'display-mode-fixture',configureServer(s){s.middlewares.use((req,res,next)=>{
        const mint=req.url.match(/^\/api\/apps\/([^/]+)\/view$/);
        if(mint){res.setHeader('Content-Type','application/json');res.end(JSON.stringify({url:'/app-view/'+mint[1]+'/'}));return}
        if(!req.url.startsWith('/__display-test')&&!req.url.startsWith('/app-view/'))return next();
        res.setHeader('Content-Type','text/html; charset=utf-8');res.end(fixture);
    })}}]});
    try{
        await server.listen();const url=`http://127.0.0.1:${server.httpServer.address().port}`;
        for(const name of (process.env.BROWSERS||'chromium,webkit').split(','))await t.test(name,async t=>{
            const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
            try{for(const [title,run] of cases)await t.test(title,async()=>{
                const context=await browser.newContext({viewport:{width:1920,height:992}}),page=await context.newPage(),errors=[];
                page.on('pageerror',e=>errors.push(e.message));
                try{await page.goto(url+'/__display-test');await page.waitForFunction(()=>!!window.fixture);await run(page,url);assert.deepEqual(errors,[])}finally{await context.close()}
            })}finally{await browser.close()}
        });
    }finally{await server.close();rmSync(work,{recursive:true,force:true})}
});
