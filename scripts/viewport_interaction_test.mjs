// Real layout and native input in Chromium/WebKit, using production modules.
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
// Exercise the desktop's actual CSS, including the root height calculation.
// An independent inset-only fixture can hide stale --ivnc-visible-height bugs.
const desktopStyles = readFileSync(path.join(root, 'ivnc-wr-core.js'), 'utf8')
    .match(/function InitUI\(\) \{[\s\S]*?style\.textContent = `([\s\S]*?)`;/)[1];
const fixture = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
${desktopStyles}
#toolbar{position:fixed;bottom:var(--ivnc-keyboard-inset,0px);height:36px;width:100%;background:#333}
</style>
<div id="app"><div class="video-container"><video id="stream"></video><div id="overlayInput"></div></div></div><div id="toolbar" class="taskbar pinned"></div>
<script type="module">
import { createViewport } from '/viewport.js';
import { createToolbarVisibility } from '/toolbar-visibility.js';
import { Input } from '/lib/input2.js';
const root=document.querySelector('#app'),container=document.querySelector('.video-container'),video=document.querySelector('video'),overlay=document.querySelector('#overlayInput'),toolbar=document.querySelector('#toolbar');
let width=1280,height=720,pinned=true,pendingFrame;
Object.defineProperties(video,{videoWidth:{get:()=>width},videoHeight:{get:()=>height}});
video.requestVideoFrameCallback=cb=>{pendingFrame=cb};
const messages=[];
const input=new Input(overlay,m=>messages.push(m));
let viewport,interacting=false;
const visibility=createToolbarVisibility({element:toolbar,pinned,onChange:()=>viewport?.layout(),interacting:()=>interacting});
viewport=createViewport({root,container,video,overlay,toolbar,isToolbarReserved:()=>visibility.isReserved(),embedded:false,onChange:()=>input.resize()});
input.attach();
input.setDisplayState({protocol:1,revision:1,desktop:{width:1920,height:1080},encoded:{width:1280,height:720}});
window.fixture={input,viewport,visibility,messages,container,video,overlay,
 pin(value){pinned=value;visibility.setPinned(value);viewport.layout()},
 interact(value){interacting=value},
 frame(w,h){width=w;height=h;video.dispatchEvent(new Event('resize'));pendingFrame?.()},
 clear(){messages.length=0},
 bounds(){const r=n=>{const b=n.getBoundingClientRect();return {left:b.left,top:b.top,bottom:b.bottom,width:b.width,height:b.height}};return {video:r(video),input:r(overlay),area:r(container),bar:r(toolbar),scroll:[container.scrollLeft,container.scrollTop]}}
};
</script>`;

const pointer = page => page.evaluate(() => fixture.messages.filter(m => /^(i,|m,|m2,|w,)/.test(m)));
const bounds = page => page.evaluate(() => fixture.bounds());
const nextLayout = page => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
const hiddenToolbar = page => page.waitForFunction(()=>!fixture.visibility.isReserved());
const shownToolbar = page => page.waitForFunction(()=>new DOMMatrixReadOnly(getComputedStyle(document.querySelector('#toolbar')).transform).m42===0);
const cases = [
    ['proportional fit, measured toolbar and portrait reflow', async page => {
        let b=await bounds(page); assert.equal(b.area.height,764); assert.equal(b.video.width,1280); assert.equal(b.video.height,720); assert.equal(b.video.top,22); assert.deepEqual(b.video,b.input);
        await page.setViewportSize({width:500,height:1000}); await nextLayout(page);
        b=await bounds(page); assert.equal(b.video.width,500); assert.equal(b.video.height,281.25); assert.equal(b.area.height,964);
        await page.evaluate(()=>{document.querySelector('#toolbar').style.height='52px';fixture.viewport.layout()});
        assert.equal((await bounds(page)).area.height,948);
        await page.evaluate(()=>fixture.pin(false));await hiddenToolbar(page);assert.equal((await bounds(page)).area.height,1000);
    }],
    ['resizing while zoomed keeps the work area aligned with toolbar pin and measured height', async page => {
        // Both engines exercise the same zoomed VisualViewport state. The real
        // Chromium pinch/resize sequence is also covered by viewport_smoke.
        await page.evaluate(()=>{
            const visual=Object.assign(new EventTarget(),{height:innerHeight/1.5,offsetTop:0,scale:1.5});
            Object.defineProperty(window,'visualViewport',{configurable:true,value:visual});
            fixture.viewport.layout();
        });
        for(const height of [650,900]) {
            await page.setViewportSize({width:1024,height});await nextLayout(page);
            let b=await bounds(page);
            assert.equal(b.area.bottom,b.bar.top);assert.equal(b.area.height,height-36);
            assert(b.video.bottom<=b.bar.top);assert.deepEqual(b.video,b.input);
            await page.evaluate(()=>fixture.pin(false));await hiddenToolbar(page);assert.equal((await bounds(page)).area.height,height);
            await page.evaluate(()=>fixture.pin(true));await shownToolbar(page);
            b=await bounds(page);assert.equal(b.area.bottom,b.bar.top);
        }
        await page.evaluate(()=>document.querySelector('#toolbar').style.height='52px');await nextLayout(page);
        const b=await bounds(page);assert.equal(b.area.bottom,b.bar.top);assert.equal(b.area.height,848);
    }],
    ['native mouse uses desktop coordinates across a reduced encoded stream', async page => {
        await page.mouse.move(640,382);await page.mouse.down();await page.mouse.up();
        assert((await pointer(page)).includes('i,1,m,960,540,1,0'));
        await page.setViewportSize({width:640,height:800});await nextLayout(page);
        const b=await bounds(page);await page.mouse.click(b.video.left+160,b.video.top+90);
        assert((await pointer(page)).includes('i,1,m,480,270,1,0'));
    }],
    ['blank areas ignore clicks/wheels and an outside release ends the drag', async page => {
        await page.mouse.move(10,5);await page.mouse.click(10,5);await page.mouse.wheel(0,100);
        assert.deepEqual(await pointer(page),[]);
        await page.mouse.move(640,382);await page.mouse.down();await page.mouse.move(640,5);await page.mouse.up();
        const messages=await pointer(page);assert(messages.some(m=>m.endsWith(',1,0')));assert(messages.at(-1).endsWith(',0,0'));
    }],
    ['encoder-only changes preserve an in-progress drag and its release', async page => {
        await page.mouse.move(640,382);await page.mouse.down();
        await page.evaluate(()=>fixture.input.setDisplayState({protocol:1,revision:1,desktop:{width:1920,height:1080},encoded:{width:640,height:360}}));
        await page.mouse.move(660,392);await page.mouse.up();
        let messages=await pointer(page);
        assert(messages.at(-1).startsWith('i,1,m,') && messages.at(-1).endsWith(',0,0'));
        await page.evaluate(()=>fixture.frame(640,360));await nextLayout(page);
        await page.mouse.click(640,382);messages=await pointer(page);
        assert(messages.includes('i,1,m,960,540,1,0'));
    }],
    ['original-size scrolling maps to the content and preserves local mode', async page => {
        await page.setViewportSize({width:500,height:400});await nextLayout(page);
        await page.evaluate(()=>{fixture.viewport.mode('native');fixture.container.scrollTop=320;fixture.container.scrollLeft=390});await nextLayout(page);
        const b=await bounds(page);assert.equal(b.video.width,1280);assert.deepEqual(b.video,b.input);assert.deepEqual(b.scroll,[390,320]);
        await page.mouse.click(250,180);assert((await pointer(page)).includes('i,1,m,960,750,1,0'));
        await page.evaluate(()=>fixture.viewport.mode('fit'));assert.deepEqual((await bounds(page)).scroll,[0,0]);
    }],
    ['reapplying original size preserves scrolling and bottom controls stay above the toolbar', async page => {
        await page.setViewportSize({width:500,height:400});await nextLayout(page);
        await page.evaluate(()=>{fixture.viewport.mode('native');fixture.container.scrollLeft=200;fixture.container.scrollTop=200});await nextLayout(page);
        await page.evaluate(()=>fixture.viewport.mode('native'));
        assert.deepEqual((await bounds(page)).scroll,[200,200]);
        await page.evaluate(()=>fixture.container.scrollTop=fixture.container.scrollHeight);await nextLayout(page);
        let b=await bounds(page);assert(b.video.bottom<=b.bar.top+1);assert.deepEqual(b.video,b.input);
        await page.mouse.click(250,b.video.bottom-4);
        assert((await pointer(page)).includes('i,1,m,675,1074,1,0'));
    }],
    ['original-size bottom anchoring survives a taller toolbar and a smaller work area', async page => {
        await page.setViewportSize({width:500,height:400});await nextLayout(page);
        await page.evaluate(()=>{fixture.viewport.mode('native');fixture.container.scrollTop=fixture.container.scrollHeight});await nextLayout(page);
        await page.evaluate(()=>document.querySelector('#toolbar').style.height='52px');await nextLayout(page);
        let b=await bounds(page);assert(b.video.bottom<=b.bar.top+1);assert.equal(b.area.bottom,b.bar.top);
        await page.setViewportSize({width:500,height:340});await nextLayout(page);
        b=await bounds(page);assert(b.video.bottom<=b.bar.top+1);assert.equal(b.area.bottom,b.bar.top);
        // Away from the edge, reflow must not unexpectedly scroll to the bottom.
        await page.evaluate(()=>fixture.container.scrollTop=100);await nextLayout(page);
        await page.evaluate(()=>document.querySelector('#toolbar').style.height='70px');await nextLayout(page);
        assert.equal((await bounds(page)).scroll[1],100);
    }],
    ['both modes reserve space throughout temporary toolbar reveal and hiding transitions', async page => {
        await page.setViewportSize({width:500,height:400});await nextLayout(page);
        await page.evaluate(()=>fixture.pin(false));await hiddenToolbar(page);
        for(const mode of ['fit','native']) {
            await page.evaluate(mode=>{fixture.viewport.mode(mode);fixture.container.scrollTop=fixture.container.scrollHeight;fixture.visibility.show()},mode);
            // Sample every frame: the toolbar must not cover the video or
            // its input surface, even while the toolbar is moving.
            const samples=await page.evaluate(async()=>{
                const samples=[];
                for(let i=0;i<12;i++){await new Promise(requestAnimationFrame);samples.push(fixture.bounds())}
                fixture.visibility.hide({immediate:true});
                for(let i=0;i<18;i++){await new Promise(requestAnimationFrame);samples.push(fixture.bounds())}
                return samples;
            });
            for(const b of samples){assert(b.area.bottom<=b.bar.top+1,JSON.stringify(b));assert(b.video.bottom<=b.bar.top+1,JSON.stringify(b));assert.deepEqual(b.video,b.input)}
            await hiddenToolbar(page);assert.equal((await bounds(page)).area.height,400);
        }
    }],
    ['rapid reveal, cancelled transitions and interaction keep toolbar reservation consistent', async page => {
        await page.evaluate(()=>fixture.pin(false));await hiddenToolbar(page);
        await page.evaluate(()=>{for(let i=0;i<5;i++){fixture.visibility.show();fixture.visibility.hide({immediate:true})}fixture.visibility.show()});
        await nextLayout(page);assert.equal((await bounds(page)).area.height,764);
        await page.evaluate(()=>{fixture.interact(true);fixture.visibility.hide({immediate:true})});
        assert(await page.evaluate(()=>fixture.visibility.isReserved()));
        await page.evaluate(()=>{fixture.interact(false);fixture.visibility.hide({immediate:true});document.querySelector('#toolbar').style.transition='none'});
        await hiddenToolbar(page);assert.equal((await bounds(page)).area.height,800);
        await page.evaluate(()=>{fixture.pin(true);fixture.visibility.hide({immediate:true})});
        assert.equal((await bounds(page)).area.height,764);
    }],
    ['reduced motion hides without transition events or a stale work area', async page => {
        await page.emulateMedia({reducedMotion:'reduce'});
        await page.evaluate(()=>fixture.pin(false));await hiddenToolbar(page);
        await page.evaluate(()=>fixture.visibility.show());assert.equal((await bounds(page)).area.height,764);
        await page.evaluate(()=>fixture.visibility.hide({immediate:true}));await hiddenToolbar(page);
        assert.equal((await bounds(page)).area.height,800);
    }],
    ['touch and pen share coordinates; pen cancellation releases buttons', async page => {
        await page.touchscreen.tap(640,382);
        assert((await pointer(page)).includes('i,1,m,960,540,1,0'));
        await page.evaluate(()=>{fixture.clear();const e=fixture.overlay;for(const type of ['pointerdown','pointerup'])e.dispatchEvent(new PointerEvent(type,{pointerType:'pen',button:0,clientX:640,clientY:382,bubbles:true}));});
        let messages=await pointer(page);assert(messages.includes('i,1,m,960,540,1,0'));assert(messages.at(-1).endsWith(',0,0'));
        await page.evaluate(()=>{const e=fixture.overlay;e.dispatchEvent(new PointerEvent('pointerdown',{pointerType:'pen',button:0,clientX:640,clientY:382,bubbles:true}));e.dispatchEvent(new PointerEvent('pointercancel',{pointerType:'pen',button:-1,clientX:640,clientY:382,bubbles:true}));});
        assert((await pointer(page)).at(-1).endsWith(',0,0'));
    }],
    ['geometry revision waits for a new frame and reconnect clears a pending transition', async page => {
        await page.evaluate(()=>{fixture.input.setDisplayState({protocol:1,revision:2,desktop:{width:1600,height:1000},encoded:{width:1280,height:800}});fixture.clear()});
        await page.mouse.click(640,382);assert.deepEqual(await pointer(page),[]);
        await page.evaluate(()=>fixture.frame(1280,800));await nextLayout(page);
        const b=await bounds(page);await page.mouse.click(b.video.left+b.video.width/2,b.video.top+b.video.height/2);
        const press=(await pointer(page)).find(m=>m.endsWith(',1,0')).split(',').map(Number);assert(Math.abs(press[3]-800)<=1 && Math.abs(press[4]-500)<=1, JSON.stringify(press));
        await page.evaluate(()=>{fixture.input.setDisplayState({protocol:1,revision:3,desktop:{width:1920,height:1080},encoded:{width:1920,height:1080}});fixture.input.setDisplayState(null);fixture.clear()});
        await page.mouse.click(640,382);assert((await pointer(page)).some(m=>m.startsWith('m,')));
    }],
    ['old servers use encoded coordinates and relative motion retains fractions', async page => {
        await page.evaluate(()=>{fixture.input.setDisplayState(null);fixture.clear()});
        await page.mouse.click(640,382);assert((await pointer(page)).includes('m,640,360,1,0'));
        const total=await page.evaluate(()=>{const i=fixture.input;i._relativeRemainder={x:0,y:0};fixture.viewport.mode('fit');return Array.from({length:20},()=>i._relativeDelta(.1,-.1)).reduce((a,p)=>[a[0]+p[0],a[1]+p[1]],[0,0])});
        assert.deepEqual(total,[2,-2]);
    }],
];

await test('independent viewport browser input', async t => {
    const work=mkdtempSync(path.join(tmpdir(),'ivnc-viewport-input-'));
    const server=await createServer({configFile:false,root,cacheDir:path.join(work,'vite'),logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]},server:{host:'127.0.0.1',port:0,hmr:false},plugins:[{name:'viewport-fixture',configureServer(s){s.middlewares.use((req,res,next)=>{if(!req.url.startsWith('/__viewport-test'))return next();res.setHeader('Content-Type','text/html; charset=utf-8');res.end(fixture)})}}]});
    try {
        await server.listen();const url=`http://127.0.0.1:${server.httpServer.address().port}/__viewport-test`;
        for(const name of (process.env.BROWSERS||'chromium,webkit').split(','))await t.test(name,async t=>{
            const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
            try {for(const [title,run] of cases)await t.test(title,async()=>{
                const context=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:2,hasTouch:true}),page=await context.newPage(),errors=[];
                page.on('pageerror',e=>errors.push(e.message));
                try {await page.goto(url);await page.waitForFunction(()=>!!window.fixture);await run(page);assert.deepEqual(errors,[]);}finally{await context.close()}
            })}finally{await browser.close()}
        });
    }finally{await server.close();rmSync(work,{recursive:true,force:true})}
});
