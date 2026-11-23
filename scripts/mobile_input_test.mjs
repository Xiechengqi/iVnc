// Production input/viewport/app-stream modules in touch-enabled Chromium/WebKit.
// OS keyboard appearance itself additionally needs a physical mobile device.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const { createServer } = require('vite'), engines = require('playwright');
const root = fileURLToPath(new URL('../web/ivnc/', import.meta.url));
const fixture = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
html,body{margin:0;overflow:hidden}#app{position:relative;top:var(--ivnc-visible-top,0px);height:calc(var(--ivnc-visible-height,100dvh) - var(--ivnc-toolbar-height,0px))}
#area{position:relative;width:100%;height:100%;overflow:hidden}#stream,#overlayInput{position:absolute}#overlayInput{opacity:0;touch-action:none;border:0;padding:0}
#toolbar{position:fixed;bottom:var(--ivnc-keyboard-inset,0px);height:36px;width:100%;background:#333}button{height:36px}
.app-streams{position:fixed;z-index:5;pointer-events:none;inset:auto}.app-stream{position:absolute;inset:0;display:none}.app-stream.active{display:block;pointer-events:auto}.app-stream iframe{width:100%;height:100%;border:0}.app-stream-status{display:none}
</style><div id="app"><div id="area"><video id="stream"></video><input id="overlayInput" readonly></div></div><div id="toolbar"><button id="keyboard-toggle">Keyboard</button></div>
<script type="module">
import {Input} from '/lib/input2.js';import {createViewport} from '/viewport.js';import {createAppStreams} from '/app-streams.js';
const embedded=location.pathname.startsWith('/__mobile-frame/');
const visual=Object.assign(new EventTarget(),{width:innerWidth,height:innerHeight,offsetTop:0,offsetLeft:0,scale:1});
Object.defineProperty(window,'visualViewport',{configurable:true,value:visual});
const messages=[],focusTrace=[],video=document.querySelector('video'),overlay=document.querySelector('#overlayInput'),button=document.querySelector('button');
Object.defineProperties(video,{videoWidth:{get:()=>1280},videoHeight:{get:()=>720}});
const input=new Input(overlay,m=>messages.push(m));window.__ivncInput=input;input.attach();
let streams=null;input.setViewActive(!embedded);
const viewport=createViewport({root:document.querySelector('#app'),container:document.querySelector('#area'),video,overlay,toolbar:document.querySelector('#toolbar'),embedded,isPinned:()=>true,onChange:()=>{input.resize();streams?.layout()}});
input._imeTextarea.addEventListener('focus',()=>focusTrace.push(navigator.userActivation?.isActive));
input.onimetoggle=()=>button.setAttribute('aria-pressed',String(input.textInput.focused()));
let wasActive;button.addEventListener('pointerdown',()=>{wasActive=streams?.imeState()??input.textInput.focused()});button.addEventListener('click',()=>{const desired=wasActive===undefined?undefined:!wasActive;wasActive=undefined;if(streams?.toggleIme(desired))return;input.toggleImeMode(desired)});
if(!embedded){streams=createAppStreams({viewport:document.querySelector('#area'),onChange:()=>input.setViewActive(!streams?.activeId()),onEdgeHover:()=>{}});streams.update([{id:'a',name:'Stream A',kind:'shared',status:'running'}]);}
else window.addEventListener('message',e=>{if(e.source===parent&&e.data?.type==='ivnc-view-active')input.setViewActive(e.data.active)});
window.fixture={input,messages,focusTrace,viewport,visual,streams,clear(){messages.length=0},resize(height,top=0,scale=1){Object.assign(visual,{height,offsetTop:top,scale});visual.dispatchEvent(new Event('resize'));viewport.layout()},bounds(){return ['app','area','toolbar'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return {id,top:r.top,bottom:r.bottom,height:r.height}})}};
</script>`;

const open = async page => {
    await page.locator('#keyboard-toggle').tap();
    assert(await page.evaluate(() => fixture.input.textInput.focused()));
    await page.evaluate(() => fixture.clear());
};
const messages = page => page.evaluate(() => fixture.messages.filter(m => /^(t,|kd,|ku,)/.test(m)));
const composition = (page, text) => page.evaluate(text => {
    const el = fixture.input._imeTextarea;
    el.dispatchEvent(new CompositionEvent('compositionstart', { bubbles:true, data:'' }));
    el.value = '\u200b' + text;
    el.dispatchEvent(new InputEvent('input', { bubbles:true, data:text, inputType:'insertCompositionText', isComposing:true }));
    el.dispatchEvent(new CompositionEvent('compositionend', { bubbles:true, data:text }));
    el.value = '\u200b' + text;
    el.dispatchEvent(new InputEvent('input', { bubbles:true, data:text, inputType:'insertText' }));
}, text);

const cases = [
    ['button synchronously focuses the editable proxy and toggles closed', async page => {
        assert.equal(await page.evaluate(() => fixture.input.textInput.focused()), false);
        await open(page);
        assert.deepEqual(await page.evaluate(() => fixture.focusTrace), [true]);
        await page.locator('#keyboard-toggle').tap();
        assert.equal(await page.evaluate(() => fixture.input.textInput.focused()), false);
    }],
    ['English, numbers, punctuation and Unicode are sent intact once', async page => {
        await open(page); await page.keyboard.insertText('Ab12,中文🙂');
        assert.deepEqual(await messages(page), ['t,Ab12,中文🙂']);
    }],
    ['physical printable keys use native text events without duplicate keysyms', async page => {
        await open(page); await page.keyboard.type('abc 123');
        assert.deepEqual(await messages(page), [...'abc 123'].map(c => 't,' + c));
    }],
    ['IME stays focused after a mouse click on the remote picture', async page => {
        await open(page); await page.mouse.click(190,380);
        assert(await page.evaluate(()=>fixture.input.textInput.focused()));
        await page.keyboard.insertText('click中文');
        assert.deepEqual(await messages(page),['t,click中文']);
    }],
    ['legacy editors type ASCII with balanced Shift and preserve Unicode fallback', async page => {
        await open(page);
        await page.evaluate(()=>fixture.input.setTextInputState({protocol:1,enabled:false,focus:'legacy'}));
        await page.keyboard.insertText('A');await page.keyboard.insertText('?');await page.keyboard.insertText('a');await page.keyboard.insertText('中文🙂');
        assert.deepEqual(await messages(page),['kd,65505','kd,65','ku,65','ku,65505','kd,65505','kd,63','ku,63','ku,65505','kd,97','ku,97','t,中文🙂']);
    }],
    ['Chinese composition and trailing WebKit input commit exactly once', async page => {
        await open(page); await composition(page, '中文🙂');
        assert.deepEqual(await messages(page), ['t,中文🙂']);
        await page.waitForTimeout(5); await composition(page, '中文🙂');
        assert.deepEqual(await messages(page), ['t,中文🙂', 't,中文🙂']);
    }],
    ['cancelled composition commits no text', async page => {
        await open(page); await composition(page, ''); assert.deepEqual(await messages(page), []);
    }],
    ['Backspace, Delete and Enter produce one balanced key pair', async page => {
        await open(page);
        for (const key of ['Backspace','Delete','Enter']) await page.keyboard.press(key);
        assert.deepEqual(await messages(page), ['kd,65288','ku,65288','kd,65535','ku,65535','kd,65293','ku,65293']);
    }],
    ['mobile beforeinput-only deletion and newline work without keydown', async page => {
        await open(page);
        await page.evaluate(() => {
            const el=fixture.input._imeTextarea;
            for (const inputType of ['deleteContentBackward','insertLineBreak']) {
                el.dispatchEvent(new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType}));
                el.dispatchEvent(new InputEvent('input',{bubbles:true,inputType}));
            }
        });
        assert.deepEqual(await messages(page), ['kd,65288','ku,65288','kd,65293','ku,65293']);
    }],
    ['non-cancelable input fallback forwards deletion and text with null data', async page => {
        await open(page); await page.evaluate(() => {
            const el=fixture.input._imeTextarea;
            el.dispatchEvent(new InputEvent('beforeinput',{bubbles:true,inputType:'deleteContentBackward'}));
            el.value='';el.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'deleteContentBackward'}));
            el.value='\u200breplacement';el.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertReplacementText'}));
        });
        assert.deepEqual(await messages(page), ['kd,65288','ku,65288','t,replacement']);
    }],
    ['composition keydown remains native and Enter does not prematurely submit', async page => {
        await open(page); const prevented=await page.evaluate(() => {
            const el=fixture.input._imeTextarea;
            el.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));
            const e=new KeyboardEvent('keydown',{bubbles:true,cancelable:true,key:'Enter',code:'Enter',isComposing:true});
            el.dispatchEvent(e);return e.defaultPrevented;
        });
        assert.equal(prevented,false);assert.deepEqual(await messages(page),[]);
    }],
    ['shortcuts still go through the physical keyboard path', async page => {
        await open(page); await page.keyboard.press('Control+a');
        const m=await messages(page);assert(m.includes('kd,65507'));assert(m.includes('kd,97'));assert(!m.some(m=>m.startsWith('t,')));
    }],
    ['unknown remote editors do not open the keyboard for ordinary clicks', async page => {
        await page.touchscreen.tap(190,380);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        assert((await page.evaluate(()=>fixture.messages)).some(m=>m.startsWith('m,')));
    }],
    ['known editor taps only move the remote cursor; the button opens the reported keyboard layout', async page => {
        await page.evaluate(()=>fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a',purpose:6}));
        await page.touchscreen.tap(190,380);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        assert.deepEqual(await page.evaluate(()=>fixture.focusTrace),[]);
        await open(page);
        assert.equal(await page.evaluate(()=>fixture.input._imeTextarea.inputMode),'email');
    }],
    ['late editor metadata after a tap never opens the keyboard, even with user activation', async page => {
        await page.touchscreen.tap(190,380);
        await page.evaluate(()=>{
            if (!navigator.userActivation.isActive) throw Error('The tap must still have user activation');
            fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a',purpose:3});
        });
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        assert.deepEqual(await page.evaluate(()=>fixture.focusTrace),[]);
        await open(page);
        assert.equal(await page.evaluate(()=>fixture.input._imeTextarea.inputMode),'numeric');
    }],
    ['blurred or manually dismissed mobile input stays closed through touch, mouse and metadata', async page => {
        for (const dismiss of ['blur','button']) {
            await open(page);
            if(dismiss==='blur') await page.evaluate(()=>fixture.input._imeTextarea.blur());
            else await page.locator('#keyboard-toggle').tap();
            assert.equal(await page.evaluate(()=>fixture.input.textInput.enabled),false);
            await page.touchscreen.tap(190,380);
            await page.mouse.click(190,380);
            await page.evaluate(()=>fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a'}));
            assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        }
    }],
    ['app reactivation, reconnect and refresh require another keyboard-button click', async page => {
        await open(page);
        await page.evaluate(()=>{fixture.input.setViewActive(false);fixture.input.setViewActive(true)});
        await page.touchscreen.tap(190,380);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        await open(page);
        await page.evaluate(()=>{fixture.input.detach();fixture.input.attach();fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a'})});
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        await open(page);
        await page.reload();await page.waitForFunction(()=>!!window.fixture);
        await page.touchscreen.tap(190,380);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
    }],
    ['iPad desktop identity with a mouse still requires the manual keyboard button', async page => {
        await page.evaluate(()=>{
            Object.defineProperties(navigator,{
                userAgent:{configurable:true,value:'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15'},
                platform:{configurable:true,value:'MacIntel'},maxTouchPoints:{configurable:true,value:5},
            });
            fixture.input.setTextInputState({protocol:1,enabled:true,focus:'ipad',purpose:3});
        });
        await page.touchscreen.tap(190,380);await page.mouse.click(190,380);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        await open(page);
        await page.touchscreen.tap(190,380);await page.mouse.click(190,380);
        assert(await page.evaluate(()=>fixture.input.textInput.focused()));
        assert.equal(await page.evaluate(()=>fixture.input._imeTextarea.inputMode),'numeric');
        await page.locator('#keyboard-toggle').tap();
        await page.touchscreen.tap(190,380);await page.mouse.click(190,380);
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
    }],
    ['application switch invalidates composition and late input', async page => {
        await open(page); await page.evaluate(()=>{
            const el=fixture.input._imeTextarea;el.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));
            fixture.input.setViewActive(false);
            el.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'stale'}));
            el.dispatchEvent(new InputEvent('input',{bubbles:true,data:'stale',inputType:'insertText'}));
        });assert.deepEqual(await messages(page),[]);
    }],
    ['cancelled composition cannot leak into a still-focused editor', async page => {
        await open(page);await page.evaluate(()=>{
            const el=fixture.input._imeTextarea;
            el.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));
            fixture.input.textInput.cancel();
            el.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'stale'}));
            el.dispatchEvent(new InputEvent('input',{bubbles:true,data:'stale',inputType:'insertText'}));
        });
        assert.deepEqual(await messages(page),[]);
        await page.keyboard.type('new');
        assert.deepEqual(await messages(page),['t,n','t,e','t,w']);
    }],
    ['closing a known remote editor dismisses the keyboard without disabling legacy manual entry', async page => {
        await page.evaluate(()=>fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a'}));
        await open(page);
        await page.evaluate(()=>fixture.input.setTextInputState({protocol:1,enabled:false,focus:'a'}));
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        await open(page);await page.keyboard.type('a');
        assert.deepEqual(await messages(page),['kd,97','ku,97']);
    }],
    ['inactive views cannot steal focus from a late remote editor update', async page => {
        await page.evaluate(()=>{
            fixture.input.setViewActive(false);
            fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a'});
            fixture.input.toggleImeMode(true);
        });
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
    }],
    ['embedded stream keyboard uses the original toolbar gesture', async page => {
        await page.evaluate(()=>fixture.streams.activate('a'));
        await page.waitForFunction(()=>fixture.streams.frames()[0]?.contentWindow?.fixture?.input._viewActive);
        const child=page.frames().find(f=>f.url().includes('/__mobile-frame/'));
        await child.evaluate(()=>fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a',purpose:6}));
        await child.locator('#overlayInput').tap();
        assert.equal(await child.evaluate(()=>fixture.input.textInput.focused()),false);
        await page.locator('#keyboard-toggle').tap();
        assert(await child.evaluate(()=>fixture.input.textInput.focused()));
        assert.deepEqual(await child.evaluate(()=>fixture.focusTrace),[true]);
        await page.keyboard.insertText('stream🙂');
        assert.deepEqual(await child.evaluate(()=>fixture.messages.filter(m=>m.startsWith('t,'))),['t,stream🙂']);
        assert.deepEqual(await messages(page),[]);
        await page.evaluate(()=>fixture.streams.activate(null));
        assert.equal(await child.evaluate(()=>fixture.input.textInput.focused()),false);
    }],
    ['an open keyboard keeps the full picture and only lifts a covered caret', async page => {
        await open(page);await page.evaluate(()=>fixture.resize(480,20));
        const openBounds=await page.evaluate(()=>fixture.bounds());
        assert.equal(openBounds[0].top,20);assert.equal(openBounds[1].height,788);assert.equal(openBounds[2].bottom,500);
        await page.evaluate(()=>{
            fixture.viewport.mode('fill');
            fixture.viewport.caret([100,40,2,18],{width:1280,height:720},true);
            fixture.viewport.layout();
        });
        assert.equal((await page.evaluate(()=>document.querySelector('#stream').getBoundingClientRect().top)),20);
        await page.evaluate(()=>{
            fixture.viewport.caret([100,600,2,18],{width:1280,height:720},true);
            fixture.viewport.layout();
        });
        const lifted=await page.evaluate(()=>{const value=document.querySelector('#area').style.transform;return !value||value==='none'?0:new DOMMatrixReadOnly(value).m42});
        assert(lifted<0);
        await page.evaluate(()=>fixture.streams.activate('a'));
        const aligned=await page.evaluate(()=>{
            const area=document.querySelector('#area'),layer=document.querySelector('.app-streams');
            const value=area.style.transform;
            const shift=!value||value==='none'?0:new DOMMatrixReadOnly(value).m42;
            const expected=area.parentElement.getBoundingClientRect().top+shift;
            return {layer:layer.getBoundingClientRect().top,expected,delta:layer.getBoundingClientRect().top-expected};
        });
        assert(Math.abs(aligned.delta)<1, JSON.stringify(aligned));
        await page.evaluate(()=>fixture.resize(844));
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        assert.equal((await page.evaluate(()=>fixture.bounds()))[1].height,808);
        assert.equal((await page.evaluate(()=>document.querySelector('#stream').getBoundingClientRect().top)),0);
    }],
    ['pinch zoom does not masquerade as software keyboard occlusion', async page => {
        await page.evaluate(()=>fixture.resize(400,20,2));
        assert.equal((await page.evaluate(()=>fixture.bounds()))[1].height,808);
    }],
    ['pinch zoom with an open keyboard preserves the full picture', async page => {
        await open(page);await page.evaluate(()=>fixture.resize(480,20));
        const before=await page.evaluate(()=>fixture.bounds());
        await page.evaluate(()=>fixture.resize(230,40,2));
        assert.deepEqual(await page.evaluate(()=>fixture.bounds()),before);
        assert(await page.evaluate(()=>fixture.input.textInput.focused()));
    }],
    ['resizing while zoomed keeps the picture full height and the toolbar at the keyboard', async page => {
        await open(page);await page.evaluate(()=>fixture.resize(480,20));
        await page.evaluate(()=>fixture.resize(230,40,2));
        await page.setViewportSize({width:390,height:700});
        await page.waitForFunction(()=>document.querySelector('#area').getBoundingClientRect().height===644);
        const b=await page.evaluate(()=>fixture.bounds());
        assert.equal(b[0].top,20);assert.equal(b[1].height,644);assert.equal(b[2].bottom,356);
        assert(await page.evaluate(()=>fixture.input.textInput.focused()));
        await page.evaluate(()=>fixture.resize(700));
        assert.equal((await page.evaluate(()=>fixture.bounds()))[1].height,664);
    }],
    ['fill keeps the full picture and lifts only a caret that the keyboard would cover', async page => {
        await open(page);await page.evaluate(()=>{
            fixture.resize(450);fixture.viewport.mode('fill');
            fixture.viewport.caret([1100,40,2,18],{width:1280,height:720},true);fixture.viewport.layout();
        });
        const scroll=await page.evaluate(()=>({x:document.querySelector('#area').scrollLeft,y:document.querySelector('#area').scrollTop}));
        assert.deepEqual(scroll,{x:0,y:0});
        const high=await page.evaluate(()=>document.querySelector('#stream').getBoundingClientRect().top);
        assert.equal(high,0);
        await page.evaluate(()=>{
            fixture.viewport.caret([1100,650,2,18],{width:1280,height:720},true);fixture.viewport.layout();
        });
        const picture=await page.evaluate(()=>{
            const v=document.querySelector('#stream').getBoundingClientRect(),c=document.querySelector('#area').getBoundingClientRect(),bar=document.querySelector('#toolbar').getBoundingClientRect();
            const value=document.querySelector('#area').style.transform;
            const lift=!value||value==='none'?0:new DOMMatrixReadOnly(value).m42;
            return {width:v.width,area:c.height,top:lift,bottom:c.height+lift,toolbar:bar.top};
        });
        assert.equal(picture.width,picture.area?picture.width:0);
        assert.equal(picture.width,390);
        assert(picture.top<0);
        assert(picture.bottom>picture.toolbar);
    }],
    ['overlay interaction cancels editing and disconnect destroys the proxy', async page => {
        await open(page); await page.evaluate(()=>window.dispatchEvent(new Event('ivnc-overlay-interaction')));
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        await page.evaluate(()=>{
            fixture.input.setTextInputState({protocol:1,enabled:true,focus:'a',purpose:6});
            fixture.input.detach();fixture.input.attach();
        });
        assert.equal(await page.evaluate(()=>fixture.input.textInput.remote),null);
        assert.equal(await page.evaluate(()=>fixture.input._imeTextarea.inputMode),'text');
        assert.equal(await page.evaluate(()=>fixture.input.textInput.focused()),false);
        await page.evaluate(()=>fixture.input.destroy());
        assert.equal(await page.locator('.ivnc-text-input').count(),0);
    }],
];

await test('mobile keyboard input',async t=>{
    const work=mkdtempSync(path.join(tmpdir(),'ivnc-mobile-input-'));
    const server=await createServer({configFile:false,root,cacheDir:path.join(work,'vite'),logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]},server:{host:'127.0.0.1',port:0,hmr:false},plugins:[{name:'mobile-input-fixture',configureServer(s){s.middlewares.use((req,res,next)=>{
        if(req.url.startsWith('/api/apps/a/view')){res.setHeader('Content-Type','application/json');return res.end(JSON.stringify({url:'/__mobile-frame/a/'}));}
        if(!req.url.startsWith('/__mobile'))return next();res.setHeader('Content-Type','text/html; charset=utf-8');res.end(fixture);
    })}}]});
    try{
        await server.listen();const url=`http://127.0.0.1:${server.httpServer.address().port}/__mobile-input`;
        for(const name of (process.env.BROWSERS||'chromium,webkit').split(','))await t.test(name,async t=>{
            const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
            try{for(const [title,run]of cases)await t.test(title,async()=>{
                const context=await browser.newContext({...engines.devices['iPhone 13'],viewport:{width:390,height:844},reducedMotion:'reduce'}),page=await context.newPage(),errors=[];
                page.on('pageerror',e=>errors.push(e.message));
                try{await page.goto(url);await page.waitForFunction(()=>!!window.fixture);await run(page);assert.deepEqual(errors,[]);}finally{await context.close();}
            });}finally{await browser.close();}
        });
    }finally{await server.close();rmSync(work,{recursive:true,force:true});}
});
