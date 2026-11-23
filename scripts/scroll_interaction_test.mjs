// Local-only browser coverage of the production input and preference modules.
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
const fixture = `<!doctype html><style>html,body{margin:0;overflow:hidden}video,#overlayInput{position:absolute;inset:0;width:100%;height:100%;padding:0;border:0}#overlayInput{opacity:0;touch-action:none}iframe{width:200px;height:120px;position:fixed;right:0;bottom:0}</style>
<video id="stream"></video><input id="overlayInput" readonly>
<script type="module">
import {Input} from '/lib/input2.js';
import {createGestureNavigation} from '/gesture-navigation.mjs';
import {getScrollSpeed,setScrollSpeed} from '/scroll-preference.mjs';
const video=document.querySelector('video'),el=document.querySelector('input'),messages=[],switches=[];
Object.defineProperties(video,{videoWidth:{get:()=>1280},videoHeight:{get:()=>720}});
const held=[];let hold=false;
const input=new Input(el,(message,isCurrent)=>{if(hold){held.push({message,isCurrent});return Promise.resolve(true)}messages.push(message);return true});input.attach();input.setInputCapabilities({scroll_pixels:1});
const nav=createGestureNavigation({canHandle:()=>input._viewActive, navigate:d=>{switches.push(d);input.resetScroll()}});
input.gestureNavigation=()=>nav;
window.fixture={input,el,messages,switches,nav,getScrollSpeed,setScrollSpeed,hold(value){hold=value},release(){hold=false;for(const item of held.splice(0))if(!item.isCurrent||item.isCurrent())messages.push(item.message)},
clear(){input.resetScroll();nav.reset();messages.length=0;switches.length=0},
wheel(x,y,mode=0,extras={}){el.dispatchEvent(new WheelEvent('wheel',{deltaX:x,deltaY:y,deltaMode:mode,clientX:60,clientY:60,bubbles:true,cancelable:true,...extras}))},
touch(type,points){const touches=points.map((p,i)=>new Touch({identifier:p.id??i,target:el,clientX:p.x,clientY:p.y}));el.dispatchEvent(new TouchEvent(type,{bubbles:true,cancelable:true,touches:type==='touchend'||type==='touchcancel'?[]:touches,targetTouches:type==='touchend'||type==='touchcancel'?[]:touches,changedTouches:touches}))},
frame(){const frame=document.createElement('iframe');frame.src='/__scroll-child';document.body.appendChild(frame)},
hidden(value){Object.defineProperty(document,'hidden',{configurable:true,value});document.dispatchEvent(new Event('visibilitychange'))}};
</script>`;
const wheelMessages = view => view.evaluate(() => fixture.messages.filter(m => /^(i,\d+,)?w2?,/.test(m)));
const settle = view => view.waitForTimeout(30);
const cases = [
    ['pixels, line/page units and subpixels keep their normalized distance', async page => {
        await page.evaluate(() => { fixture.wheel(0, .25); fixture.wheel(0, 1.5); }); await settle(page);
        assert.deepEqual(await wheelMessages(page), ['w2,0,1.734375']);
        await page.evaluate(() => { fixture.clear(); fixture.wheel(0, 2, 1); }); await settle(page);
        assert.deepEqual(await wheelMessages(page), ['w2,0,48']);
        await page.evaluate(() => { fixture.clear(); fixture.wheel(0, .5, 2); }); await settle(page);
        assert.deepEqual(await wheelMessages(page), ['w2,0,360']);
    }],
    ['ambiguous motion returns its reserved deltas before vertical scrolling', async page => {
        await page.evaluate(() => { fixture.wheel(4,4); fixture.wheel(4,50); }); await settle(page);
        const total=(await wheelMessages(page))[0].split(',').slice(1).map(Number);
        assert(Math.abs(total[0]-8)<.047 && Math.abs(total[1]-54)<.047);
        await page.evaluate(() => { fixture.clear(); fixture.wheel(2,2); }); await page.waitForTimeout(110);
        assert.deepEqual(await wheelMessages(page), ['w2,1.96875,1.96875']);
    }],
    ['speed updates affect scrolling but leave horizontal navigation distance unchanged', async page => {
        for (const speed of [.25,4]) {
            await page.evaluate(speed => { fixture.clear(); fixture.setScrollSpeed(speed); fixture.wheel(0,100); }, speed); await settle(page);
            assert(Math.abs(Number((await wheelMessages(page))[0].split(',')[2])-speed*100)<.047);
            await page.evaluate(() => { fixture.clear(); fixture.wheel(35,0); fixture.wheel(35,0); fixture.wheel(35,0); }); await settle(page);
            assert.deepEqual(await wheelMessages(page), []);
            assert.deepEqual(await page.evaluate(() => fixture.switches), [1]);
        }
    }],
    ['two-finger motion accumulates one-pixel and fractional deltas on both axes', async page => {
        await page.evaluate(() => { fixture.input.gestureNavigation=null; for(let i=0;i<8;i++)fixture.input._handleTwoFingerScroll(.25,-1); }); await settle(page);
        const total=(await wheelMessages(page))[0].split(',').slice(1).map(Number);
        assert(Math.abs(total[0]+2)<.047 && Math.abs(total[1]-8)<.047);
    }],
    ['a one-finger movement under the threshold sends no button until a stationary tap clicks the start point', async page => {
        await page.evaluate(() => { fixture.clear(); fixture.touch('touchstart',[{x:80,y:80}]); fixture.touch('touchmove',[{x:86,y:84}]); });
        assert.deepEqual(await page.evaluate(() => fixture.messages), []);
        await page.evaluate(() => fixture.touch('touchend',[{x:86,y:84}]));
        await page.waitForTimeout(40);
        const held=await page.evaluate(() => fixture.messages);
        assert.deepEqual(held.filter(m => m.startsWith('m,')).map(m => m.split(',').slice(1,4).join(',')), ['80,80,1','80,80,0']);
        assert.deepEqual(await wheelMessages(page), []);
    }],
    ['a one-finger vertical swipe sends only a wheel at the touch gain and does not click', async page => {
        await page.evaluate(() => { fixture.clear(); fixture.touch('touchstart',[{x:100,y:100}]); fixture.touch('touchmove',[{x:104,y:148}]); fixture.touch('touchend',[{x:104,y:148}]); });
        await settle(page);
        const messages=await page.evaluate(() => fixture.messages);
        assert.equal(messages.filter(m => /^m2?,/.test(m)).length, 0);
        const total=(await wheelMessages(page))[0].split(',').slice(1).map(Number);
        assert(Math.abs(total[0]+10)<.047 && Math.abs(total[1]+120)<.047);
        assert.deepEqual(await page.evaluate(() => fixture.switches), []);
    }],
    ['a one-finger horizontal swipe still drags and a second finger leaves one-finger scrolling', async page => {
        await page.evaluate(() => { fixture.clear(); fixture.touch('touchstart',[{x:40,y:40}]); fixture.touch('touchmove',[{x:80,y:46}]); fixture.touch('touchend',[{x:80,y:46}]); });
        await settle(page);
        const drag=await page.evaluate(() => fixture.messages);
        assert(drag.some(m => /^m,.*,1,0$/.test(m)));
        assert.deepEqual(await wheelMessages(page), []);
        await page.evaluate(() => { fixture.clear(); fixture.touch('touchstart',[{id:1,x:100,y:80}]); fixture.touch('touchmove',[{id:1,x:100,y:120}]); fixture.touch('touchstart',[{id:1,x:100,y:120},{id:2,x:160,y:120}]); fixture.touch('touchmove',[{id:1,x:100,y:150},{id:2,x:160,y:150}]); fixture.touch('touchend',[{id:1,x:100,y:150},{id:2,x:160,y:150}]); });
        await settle(page);
        const messages=await page.evaluate(() => fixture.messages);
        assert.equal(messages.filter(m => /^m2?,/.test(m)).length, 0);
        const total=(await wheelMessages(page))[0].split(',').slice(1).map(Number);
        assert(Math.abs(total[0])<.047 && Math.abs(total[1]+130)<.047);
    }],
    ['two-finger scrolling and horizontal switching stay on the existing gesture', async page => {
        await page.evaluate(() => { fixture.clear(); fixture.touch('touchstart',[{id:1,x:90,y:90},{id:2,x:150,y:90}]); fixture.touch('touchmove',[{id:1,x:90,y:114},{id:2,x:150,y:114}]); fixture.touch('touchend',[{id:1,x:90,y:114},{id:2,x:150,y:114}]); });
        await settle(page);
        const scrolled=(await wheelMessages(page))[0].split(',').slice(1).map(Number);
        assert(Math.abs(scrolled[0])<.047 && Math.abs(scrolled[1]+24)<.047);
        await page.evaluate(() => { fixture.clear(); fixture.touch('touchstart',[{id:1,x:40,y:200},{id:2,x:100,y:200}]); fixture.touch('touchmove',[{id:1,x:58,y:200},{id:2,x:118,y:200}]); fixture.touch('touchmove',[{id:1,x:168,y:204},{id:2,x:228,y:204}]); fixture.touch('touchend',[{id:1,x:168,y:204},{id:2,x:228,y:204}]); });
        await settle(page);
        assert.deepEqual(await wheelMessages(page), []);
        assert.deepEqual(await page.evaluate(() => fixture.switches), [-1]);
        await page.evaluate(() => { fixture.clear(); fixture.input.setTrackpadMode(true); fixture.touch('touchstart',[{x:100,y:100}]); fixture.touch('touchmove',[{x:104,y:148}]); fixture.touch('touchend',[{x:104,y:148}]); });
        await settle(page);
        const trackpad=await page.evaluate(() => fixture.messages);
        assert.equal(trackpad.filter(m => /^m2?,/.test(m)).length, 0);
        const scaled=(await wheelMessages(page))[0].split(',').slice(1).map(Number);
        assert(Math.abs(scaled[0]+10)<.047 && Math.abs(scaled[1]+120)<.047);
        await page.evaluate(() => fixture.input.setTrackpadMode(false));
    }],
    ['a queued wheel is sent before pointer moves, buttons and modifier release', async page => {
        await page.evaluate(() => { fixture.wheel(0,10); fixture.input.send('ku,65507'); });
        const messages=await page.evaluate(() => fixture.messages);
        assert(Math.abs(Number(messages[0].split(',')[2])-10)<.047); assert.equal(messages[1],'ku,65507');
    }],
    ['view, overlay, geometry, blur, page hide and detach cancel pending wheel input', async page => {
        for (const action of ['view','overlay','geometry','blur','hide','pagehide','detach']) {
            await page.evaluate(action => {
                fixture.clear(); fixture.wheel(0,100);
                const i=fixture.input;
                if(action==='view')i.setViewActive(false);
                if(action==='overlay')window.dispatchEvent(new Event('ivnc-overlay-interaction'));
                if(action==='geometry')i.setDisplayState(null);
                if(action==='blur')window.dispatchEvent(new Event('blur'));
                if(action==='hide')fixture.hidden(true);
                if(action==='pagehide')window.dispatchEvent(new PageTransitionEvent('pagehide'));
                if(action==='detach')i.detach();
            }, action); await settle(page);
            assert.deepEqual(await wheelMessages(page), [], action);
            await page.evaluate(() => {fixture.hidden(false);fixture.input.setViewActive(true);if(!fixture.input.inputAttached){fixture.input.attach();fixture.input.setInputCapabilities({scroll_pixels:1})}});
        }
    }],
    ['legacy fallback retains fractions and capability negotiation restores precise input', async page => {
        await page.evaluate(() => { fixture.input.setInputCapabilities(null); for(let i=0;i<48;i++)fixture.wheel(0,.25); }); await settle(page);
        assert.deepEqual(await wheelMessages(page), ['w,0,1']);
        await page.evaluate(() => { fixture.messages.length=0; fixture.wheel(0,.25); }); await settle(page);
        assert.deepEqual(await wheelMessages(page), []);
        await page.evaluate(() => fixture.input.setInputCapabilities({scroll_pixels:1})); await settle(page);
        assert.deepEqual(await wheelMessages(page), ['w2,0,0.234375']);
    }],
    ['scroll already held in an ordered paste queue cannot reach a later local window', async page => {
        await page.evaluate(()=>{fixture.hold(true);fixture.wheel(0,120);fixture.input.scrollInput.flush()});
        assert.deepEqual(await wheelMessages(page),[]);
        await page.evaluate(()=>{fixture.input.resetScroll();fixture.release()});
        assert.deepEqual(await wheelMessages(page),[]);
        await page.evaluate(()=>{fixture.wheel(0,120);fixture.input.scrollInput.flush()});
        assert.deepEqual(await wheelMessages(page),['w2,0,120']);
    }],
    ['preferences reach existing/new iframes, reload and another tab on the same site', async page => {
        await page.evaluate(() => fixture.frame());
        const frame = page.frames().find(f => f !== page.mainFrame());
        await frame.waitForFunction(() => !!window.fixture);
        await page.evaluate(() => fixture.setScrollSpeed(2.5));
        assert.equal(await frame.evaluate(() => fixture.getScrollSpeed()), 2.5);
        await frame.evaluate(() => fixture.wheel(0,10)); await settle(frame);
        assert.deepEqual(await wheelMessages(frame), ['w2,0,24.984375']);
        const other=await page.context().newPage();
        try {
            await other.goto(page.url()); await other.waitForFunction(() => !!window.fixture);
            assert.equal(await other.evaluate(() => fixture.getScrollSpeed()), 2.5);
            await other.evaluate(() => fixture.setScrollSpeed(.5));
            await page.waitForFunction(() => fixture.getScrollSpeed()===.5);
            assert.equal(await frame.evaluate(() => fixture.getScrollSpeed()), .5);
        } finally { await other.close(); }
        await page.reload(); await page.waitForFunction(() => !!window.fixture);
        assert.equal(await page.evaluate(() => fixture.getScrollSpeed()), .5);
        await page.evaluate(() => fixture.frame());
        await page.frames().find(f=>f!==page.mainFrame()).waitForFunction(() => window.fixture?.getScrollSpeed()===.5);
    }],
    ['blocked storage keeps live parent/iframe preferences functional', async page => {
        await page.evaluate(() => {
            Object.defineProperty(window,'localStorage',{configurable:true,get(){throw Error('blocked')}});
            fixture.setScrollSpeed(3); fixture.frame();
        });
        const frame = page.frames().find(f=>f!==page.mainFrame()); await frame.waitForFunction(()=>!!window.fixture);
        assert.equal(await frame.evaluate(()=>fixture.getScrollSpeed()),3);
        await frame.evaluate(()=>fixture.setScrollSpeed(2));
        assert.equal(await page.evaluate(()=>fixture.getScrollSpeed()),2);
    }],
    ['malformed stored values restore the normal default and separate browsers stay independent', async page => {
        await page.evaluate(()=>localStorage.setItem('ivnc.scroll-speed.v1','Infinity'));
        await page.reload();await page.waitForFunction(()=>!!window.fixture);
        assert.equal(await page.evaluate(()=>fixture.getScrollSpeed()),1);
        await page.evaluate(()=>fixture.setScrollSpeed(4));
        const context=await page.context().browser().newContext();
        try {
            const other=await context.newPage();await other.goto(page.url());await other.waitForFunction(()=>!!window.fixture);
            assert.equal(await other.evaluate(()=>fixture.getScrollSpeed()),1);
        }finally{await context.close()}
        assert.equal(await page.evaluate(()=>fixture.getScrollSpeed()),4);
    }],
];

await test('scroll interactions', async t => {
    const work=mkdtempSync(path.join(tmpdir(),'ivnc-scroll-test-'));
    const server=await createServer({root,configFile:false,cacheDir:path.join(work,'vite'),logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]},server:{host:'127.0.0.1',port:0,hmr:false},plugins:[{name:'scroll-fixture',configureServer(server){server.middlewares.use((req,res,next)=>{if(!req.url.startsWith('/__scroll'))return next();res.setHeader('Content-Type','text/html');res.end(fixture)})}}]});
    try {
        await server.listen(); const url=`http://127.0.0.1:${server.httpServer.address().port}/__scroll`;
        for(const name of (process.env.BROWSERS||'chromium,webkit').split(',')) await t.test(name,async t=>{
            const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
            try {for(const [title,run] of cases) await t.test(title,async()=>{
                const context=await browser.newContext({viewport:{width:1280,height:720},deviceScaleFactor:2});
                const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));context.setDefaultTimeout(7000);
                try{await page.goto(url);await page.waitForFunction(()=>!!window.fixture);await run(page);assert.deepEqual(errors,[])}finally{await context.close()}
            })}finally{await browser.close()}
        });
    }finally{await server.close();rmSync(work,{recursive:true,force:true})}
});
