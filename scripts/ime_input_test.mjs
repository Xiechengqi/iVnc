// Desktop IME lifecycle and caret mapping in real browser engines. Core
// preference/button/message handlers are taken from production, media mocked.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const { createServer } = require('vite'), engines = require('playwright');
const root = fileURLToPath(new URL('../web/ivnc/', import.meta.url));
const source = readFileSync(path.join(root, 'ivnc-wr-core.js'), 'utf8');
const slice = (from, to) => {
    const start = source.indexOf(from), end = source.indexOf(to, start);
    assert(start >= 0 && end > start, from); return source.slice(start, end);
};
const fixture = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
html,body{margin:0;overflow:hidden}#app{position:relative;top:var(--ivnc-visible-top,0px);height:calc(var(--ivnc-visible-height,100dvh) - var(--ivnc-toolbar-height,0px))}
#area{position:relative;width:100%;height:100%;overflow:hidden}#stream,#overlayInput{position:absolute}#overlayInput{opacity:0;touch-action:none;border:0;padding:0}
#taskbar{position:fixed;bottom:0;height:36px;width:100%;background:#333;z-index:1000}.taskbar-pin{height:26px}
.app-streams{position:fixed;z-index:5;pointer-events:none}.app-stream{position:absolute;inset:0;display:none}.app-stream.active{display:block;pointer-events:auto}.app-stream iframe{width:100%;height:100%;border:0}.app-stream-status{display:none}
.web-terminal-modal{position:fixed;top:50px;left:50px;z-index:10000}
</style>
<div id="app"><div id="area"><video id="stream"></video><input id="overlayInput" readonly></div></div><div id="taskbar" class="taskbar pinned"></div>
<script type="module">
import {Input} from '/lib/input2.js';import {usesTouchKeyboard} from '/lib/text-input.js';
import {createViewport,normalizeDisplayMode} from '/viewport.js';import {createAppStreams} from '/app-streams.js';import {createToolbar} from '/toolbar.js';import {getLang,setLang} from '/lib/i18n.js?v=1';
window.__IVNC_APP_VIEW__=location.pathname.startsWith('/app-view/');window.__IVNC_EMBED__=window.__IVNC_APP_VIEW__&&parent!==window;
window.__ivncViewActive=!window.__IVNC_EMBED__;
const visual=Object.assign(new EventTarget(),{width:innerWidth,height:innerHeight,offsetTop:0,offsetLeft:0,scale:1});Object.defineProperty(window,'visualViewport',{configurable:true,value:visual});
${slice('const urlForKey =', 'function formatNetworkMetric(')}
let input=null,appStreams=null,displayMode='fit',displayModeButton=null,scaleLocal=true,viewportController=null;
const messages=[],errors=[],postToParent=message=>{if(window.__IVNC_EMBED__)parent.postMessage({...message,view:location.pathname},location.origin)};
const setLiveTitle=(node,key)=>{node.title=key};
const video=document.getElementById('stream'),overlay=document.getElementById('overlayInput'),area=document.getElementById('area');
Object.defineProperties(video,{videoWidth:{get:()=>1280},videoHeight:{get:()=>720}});
const toolbarController=createToolbar({element:document.getElementById('taskbar'),activateWindow:()=>true});
const pin=document.createElement('button');pin.id='pin';pin.className='taskbar-pin';pin.title='Pin';toolbarController.addTool(pin,true);
${slice('function setDisplayMode(', 'function reportViewport()')}
${slice("displayModeButton = document.createElement('button');", '// IME toggle button')}
${slice('// IME toggle button', '// File upload button')}
displayModeButton.id='mode';setDisplayMode(normalizeDisplayMode(getStringParam('display_mode','fit'))||'fit',false);
const secondary=document.createElement('button');secondary.id='secondary';secondary.className='taskbar-pin';secondary.title='Secondary';toolbarController.addTool(secondary);
const late=document.createElement('button');late.id='late-primary';late.className='taskbar-pin';late.title='Primary';toolbarController.addTool(late,true);
viewportController=createViewport({root:document.getElementById('app'),container:area,video,overlay,toolbar:document.getElementById('taskbar'),isPinned:()=>true,embedded:window.__IVNC_EMBED__,onChange:()=>{input?.resize();appStreams?.layout()}});
input=new Input(overlay,m=>{messages.push(m);return true});window.__ivncInput=input;
input.onimepreferencechange=enabled=>setImePreference(enabled,true);input.ontextinputerror=reason=>errors.push(reason);
if(!usesTouchKeyboard())input.setImeMode(imeModeActive,false);
${slice('input.onimetoggle = active => {', 'webrtc.onplaystreamrequired =')}
function connect(){input.attach();${slice('// Restore desired state, never toggle on reconnect.', '// Enable upload button')}}
if(!window.__IVNC_APP_VIEW__){appStreams=createAppStreams({viewport:area,displayMode:()=>displayMode,imeMode:()=>imeModeActive,
onImePreference:enabled=>setImePreference(enabled,true),onImeError:reason=>errors.push(reason),onChange:()=>{input.setViewActive(!appStreams?.activeId());syncImeButton()},onEdgeHover:()=>{}});
appStreams.update([{id:'shared',name:'Shared',kind:'shared',status:'running'},{id:'remote',name:'Remote',kind:'remote',status:'connected'}]);}
else addEventListener('message',event=>{if(event.origin!==location.origin||event.source!==parent)return;const message=event.data;
if(message.type==='ivnc-view-active'){window.__ivncViewActive=!!message.active;input.setViewActive(!!message.active)}
else if(message.type==='ivnc-ime-mode'&&typeof message.enabled==='boolean'&&message.view===location.pathname){setImePreference(message.enabled);input.restoreImeFocus()}});
connect();if(window.__IVNC_EMBED__)document.getElementById('taskbar').style.display='none';
window.fixture={input,messages,errors,streams:appStreams,viewport:viewportController,visual,connect,setLang,
stored:()=>localStorage.getItem(storageAppName+'_ime_mode'),key:storageAppName,
clear(){messages.length=0;errors.length=0},anchor(){return [parseFloat(input._imeTextarea.style.left),parseFloat(input._imeTextarea.style.top)]},
hidden(value){Object.defineProperty(document,'hidden',{configurable:true,value});document.dispatchEvent(new Event('visibilitychange'))},
metadata(state){input.setTextInputState({protocol:1,enabled:true,focus:'editor',cursor:[100,150,1,20],...state})},
resize(){Object.assign(visual,{width:innerWidth,height:innerHeight});visual.dispatchEvent(new Event('resize'));viewportController.layout()},
};
</script>`;

const enable = async page => { await page.locator('#keyboard-toggle').click(); assert(await page.evaluate(() => fixture.input.textInput.focused())); };
const settled = page => page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
const activate = async (page, id) => {
    await page.evaluate(id => fixture.streams.activate(id), id);
    if (id) await page.waitForFunction(() => document.querySelector('.app-stream.active iframe')?.contentWindow?.fixture?.input.inputAttached);
    await settled(page);
    return id ? page.frames().find(f => f.url().includes(`/app-view/${id}/`)) : page;
};
const cases = [
    ['default fit, fill highlight and primary tool order survive compact/wide layout', async page => {
        assert.equal(await page.locator('#mode').getAttribute('aria-pressed'), 'false');
        await page.locator('#mode').click();
        assert.equal(await page.locator('#mode').getAttribute('aria-pressed'), 'true');
        await page.waitForTimeout(200);
        const normal = await page.locator('#mode').evaluate(el => getComputedStyle(el).backgroundColor);
        await page.locator('#mode').hover(); await page.waitForTimeout(200);
        assert.equal(await page.locator('#mode').evaluate(el => getComputedStyle(el).backgroundColor), normal);
        assert.notEqual(normal, 'rgba(0, 0, 0, 0)');
        for (const width of [390, 960, 600, 960]) {
            await page.setViewportSize({width,height:720});await settled(page);
            assert.deepEqual(await page.locator('.toolbar-tools > [id]').evaluateAll(nodes => nodes.map(n => n.id)), width<=700?['pin','mode','keyboard-toggle','late-primary']:['pin','mode','keyboard-toggle','secondary','late-primary']);
        }
        await page.reload();await page.waitForFunction(()=>!!window.fixture);
        assert.equal(await page.locator('#mode').getAttribute('aria-pressed'),'true');
    }],
    ['refresh and repeated reconnections restore the desired mode without toggling it off', async page => {
        await enable(page);assert.equal(await page.evaluate(()=>fixture.stored()),'true');
        await page.reload();await page.waitForFunction(()=>fixture.input.textInput.focused());
        for(let n=0;n<2;n++)await page.evaluate(()=>{fixture.input.detach();fixture.connect()});
        assert(await page.evaluate(()=>fixture.input.textInput.enabled&&fixture.input.textInput.focused()));
        assert.equal(await page.evaluate(()=>fixture.stored()),'true');
        await page.locator('#keyboard-toggle').click();await page.reload();await page.waitForFunction(()=>!!window.fixture);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.enabled),false);
        assert.equal(await page.evaluate(()=>fixture.stored()),'false');
    }],
    ['temporary hiding and overlays cancel composition without changing saved mode', async page => {
        await enable(page);await page.evaluate(()=>{fixture.clear();const el=fixture.input._imeTextarea;el.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));fixture.hidden(true);el.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'晚到中文'}));fixture.hidden(false)});
        assert.equal(await page.evaluate(()=>fixture.stored()),'true');
        assert.equal(await page.evaluate(()=>fixture.messages.some(m=>m.startsWith('t,')||m.startsWith('ti,'))),false);
        assert(await page.evaluate(()=>fixture.input.textInput.focused()));
        await page.evaluate(()=>window.dispatchEvent(new Event('ivnc-overlay-interaction')));
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.enabled),true);
        await page.mouse.click(200,200);assert(await page.evaluate(()=>fixture.input.textInput.focused()));
    }],
    ['reconnect/focus events do not steal focus from a local popup editor', async page => {
        await enable(page);await page.evaluate(()=>{const modal=document.createElement('div');modal.className='web-terminal-modal';modal.innerHTML='<input class="allow-native-input">';document.body.appendChild(modal);modal.firstChild.focus();fixture.input.detach();fixture.connect();window.dispatchEvent(new Event('focus'))});
        assert.equal(await page.evaluate(()=>document.activeElement.className),'allow-native-input');
        assert.equal(await page.evaluate(()=>fixture.stored()),'true');
    }],
    ['shared/remote tabs inherit the desktop preference and active views restore focus', async page => {
        await enable(page);const shared=await activate(page,'shared');
        await shared.waitForFunction(()=>fixture.input.textInput.enabled&&fixture.input.textInput.focused());
        const remote=await activate(page,'remote');await remote.waitForFunction(()=>fixture.input.textInput.enabled&&fixture.input.textInput.focused());
        assert.equal(await shared.evaluate(()=>fixture.input.textInput.focused()),false);
        assert.equal(await page.evaluate(()=>fixture.stored()),'true');
        assert.equal(await remote.evaluate(()=>fixture.stored()),null);
        await activate(page,null);await page.waitForFunction(()=>fixture.input.textInput.focused());
        assert.equal(await page.locator('#keyboard-toggle').getAttribute('aria-pressed'),'true');
        await page.locator('#keyboard-toggle').click();
        await shared.waitForFunction(()=>!fixture.input.textInput.enabled);await remote.waitForFunction(()=>!fixture.input.textInput.enabled);
        await activate(page,'shared');assert.equal(await shared.evaluate(()=>fixture.input.textInput.focused()),false);
    }],
    ['active-frame ready messages do not pull focus out of a local popup', async page => {
        await enable(page);const shared=await activate(page,'shared');
        await page.evaluate(()=>{const modal=document.createElement('div');modal.className='web-terminal-modal';modal.innerHTML='<input id="local-edit" class="allow-native-input">';document.body.appendChild(modal);modal.firstChild.focus()});
        await shared.evaluate(()=>parent.postMessage({type:'ivnc-view-ready',view:location.pathname},location.origin));
        await settled(page);await settled(page);
        assert.equal(await page.evaluate(()=>document.activeElement.id),'local-edit');
        assert.equal(await shared.evaluate(()=>fixture.input.textInput.focused()),false);
    }],
    ['an embedded shortcut updates the parent preference, while stale/inactive messages are ignored', async page => {
        const shared=await activate(page,'shared');await shared.locator('#overlayInput').focus();
        await page.keyboard.press('Control+Shift+Space');
        await page.waitForFunction(()=>fixture.stored()==='true');
        const remote=await activate(page,'remote');await remote.waitForFunction(()=>fixture.input.textInput.focused());
        await shared.evaluate(()=>parent.postMessage({type:'ivnc-ime-preference',enabled:false,view:location.pathname},location.origin));
        await remote.evaluate(()=>parent.postMessage({type:'ivnc-ime-preference',enabled:false,view:'/app-view/stale/'},location.origin));
        await settled(page);assert.equal(await page.evaluate(()=>fixture.stored()),'true');
        await page.keyboard.press('Control+Shift+Space');await page.waitForFunction(()=>fixture.stored()==='false');
        await page.reload();await page.waitForFunction(()=>!!window.fixture);assert.equal(await page.evaluate(()=>fixture.input.textInput.enabled),false);
    }],
    ['caret updates follow the editor in fit and independently scaled fill modes', async page => {
        await enable(page);
        for(const mode of ['fit','fill']){
            await page.evaluate(mode=>{fixture.viewport.mode(mode);fixture.input.setDisplayState({protocol:1,revision:1,desktop:{width:2560,height:1440},encoded:{width:1280,height:720}});fixture.metadata({cursor:[200,180,1,20]})},mode);
            const a=await page.evaluate(()=>fixture.anchor());
            await page.evaluate(()=>fixture.metadata({cursor:[800,420,1,30]}));
            const b=await page.evaluate(()=>{const v=document.querySelector('video').getBoundingClientRect(),scale=Math.min(v.width/1280,v.height/720),contain=document.querySelector('video').style.objectFit==='contain',w=contain?1280*scale:v.width,h=contain?720*scale:v.height;return {actual:fixture.anchor(),expected:[v.left+(v.width-w)/2+800*w/2560,v.top+(v.height-h)/2+450*h/1440]}});
            assert(b.actual[0]>a[0]&&b.actual[1]>a[1]);for(let i=0;i<2;i++)assert(Math.abs(b.actual[i]-b.expected[i])<1);
        }
    }],
    ['viewport events preserve caret/click anchors and a new target clears stale coordinates', async page => {
        await enable(page);await page.evaluate(()=>fixture.metadata({cursor:[700,400,0,20]}));
        const caret=await page.evaluate(()=>fixture.anchor());await page.evaluate(()=>{fixture.visual.dispatchEvent(new Event('scroll'));fixture.visual.dispatchEvent(new Event('resize'))});
        assert.deepEqual(await page.evaluate(()=>fixture.anchor()),caret);
        await page.evaluate(()=>fixture.metadata({cursor:null}));await page.mouse.click(400,300);
        const click=await page.evaluate(()=>fixture.anchor());await page.evaluate(()=>fixture.resize());
        assert.deepEqual(await page.evaluate(()=>fixture.anchor()),click);
        await page.evaluate(()=>fixture.metadata({focus:'other',cursor:null}));assert.deepEqual(await page.evaluate(()=>fixture.anchor()),[12,12]);
    }],
    ['resize recalculates a stationary caret and clamps it above the reserved toolbar', async page => {
        await enable(page);await page.evaluate(()=>{fixture.viewport.mode('fill');fixture.metadata({cursor:[700,400,1,20]})});
        const before=await page.evaluate(()=>fixture.anchor());await page.setViewportSize({width:720,height:500});await page.evaluate(()=>fixture.resize());
        const after=await page.evaluate(()=>fixture.anchor());assert(after[0]<before[0]&&after[1]<before[1]);
        await page.evaluate(()=>fixture.metadata({cursor:[9999,9999,0,18]}));
        assert(await page.evaluate(()=>fixture.anchor()[1]+18<=document.getElementById('taskbar').getBoundingClientRect().top));
    }],
    ['new runtimes receive one committed edit and switching cancels pending text', async page => {
        await enable(page);await page.evaluate(()=>{fixture.metadata({commit:1});fixture.clear()});
        await page.keyboard.insertText('中文,🙂');
        const request=await page.evaluate(()=>JSON.parse(fixture.messages.find(m=>m.startsWith('ti,')).slice(3)));assert.equal(request.text,'中文,🙂');
        await page.evaluate(()=>fixture.input.setViewActive(false));
        assert(await page.evaluate(()=>fixture.messages.some(m=>m.includes('"type":"cancel"'))));
        await page.evaluate(id=>fixture.input.textInputResult({id,status:'delivery_failed'}),request.id);
        assert.deepEqual(await page.evaluate(()=>fixture.errors),[]);
        assert.equal(await page.evaluate(()=>fixture.stored()),'true');
    }],
];

await test('desktop IME preferences, caret and toolbar',async t=>{
    const work=mkdtempSync(path.join(tmpdir(),'ivnc-ime-input-'));
    const server=await createServer({configFile:false,root,cacheDir:path.join(work,'vite'),logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]},server:{host:'127.0.0.1',port:0,hmr:false},plugins:[{name:'ime-input-fixture',configureServer(s){s.middlewares.use((req,res,next)=>{
        const mint=req.url.match(/^\/api\/apps\/([^/]+)\/view$/);if(mint){res.setHeader('Content-Type','application/json');return res.end(JSON.stringify({url:'/app-view/'+mint[1]+'/'}))}
        if(!req.url.startsWith('/__ime')&&!req.url.startsWith('/app-view/'))return next();res.setHeader('Content-Type','text/html; charset=utf-8');res.end(fixture);
    })}}]});
    try{await server.listen();const url='http://127.0.0.1:'+server.httpServer.address().port+'/__ime';
        for(const name of (process.env.BROWSERS||'chromium,webkit').split(','))await t.test(name,async t=>{
            const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
            try{for(const[title,run]of cases)await t.test(title,async()=>{const context=await browser.newContext({viewport:{width:960,height:720},deviceScaleFactor:2}),page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
                try{await page.goto(url);await page.waitForFunction(()=>!!window.fixture);await run(page);assert.deepEqual(errors,[])}finally{await context.close()}
            })}finally{await browser.close()}
        });
    }finally{await server.close();rmSync(work,{recursive:true,force:true})}
});
