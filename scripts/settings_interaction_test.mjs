// Production settings controller, console, actions and modal input guard in
// Chromium/WebKit. Privileged operations are intercepted; no service restarts or shutdowns.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const { createServer } = require('vite'), engines = require('playwright');
const { WebSocketServer } = require('ws');
const root = fileURLToPath(new URL('../web/ivnc/', import.meta.url));
const source = readFileSync(path.join(root, 'ivnc-wr-core.js'), 'utf8');
const slice = (from, to) => source.slice(source.indexOf(from), source.indexOf(to, source.indexOf(from)));
// Run the actual desktop controller without starting a media session.
const fixture = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>body{margin:0} ${slice('\t.web-terminal-modal {', '\t.connect-page {')}</style>
<button id="underlay" style="position:fixed;inset:0">Application</button>
<script type="module">
import {t,onLangChange} from '/lib/i18n.js?v=1';
import {installOverlayGuard,minimizeOtherOverlays,createConsoleShield,releaseOverlayInput} from '/overlay-manager.js';
${slice('const MAIN_I18N =', 'function InitUI()')}
${slice('function getBasePath()', 'function createWebTerminalController()')}
${slice('function createConsoleModalController(', 'function createProxyModalController()')}
const controller=createConsoleModalController(new URLSearchParams(location.search).get('console')!=='0');
controller.show();window.fixture={controller,clicks:0};
document.getElementById('underlay').onclick=()=>fixture.clicks++;
</script>`;
const wait = (frame, fn) => frame.waitForFunction(fn);
const openPassword = async frame => { await frame.locator('#settings-password').click(); await frame.locator('#pwd-new').waitFor(); };
const openUpdate = async frame => { await frame.locator('#settings-update').click(); await frame.locator('#update-ok').waitFor(); };
const openShutdown = async frame => { await frame.locator('#settings-shutdown').click(); await frame.locator('#shutdown-ok').waitFor(); };
const closed = frame => wait(frame, () => !document.querySelector('.settings-dialog-bg'));
const locked = frame => wait(frame, () => window.__ivncConsoleDialogState.locked);
const unlocked = frame => wait(frame, () => !window.__ivncConsoleDialogState.locked);

const cases = [
    ['desktop header keeps navigation and exposes all actions without requests', async ({ page, frame, calls }) => {
        assert.equal(await frame.locator('.nav-item').count(), 4);
        assert(await frame.locator('#settings-password').isVisible());
        assert(await frame.locator('#settings-update').isVisible());
        assert(await frame.locator('#settings-shutdown').isVisible());
        assert(await frame.locator('#signout-btn').isVisible());
        const positions = await frame.evaluate(() => ['.nav-list','#settings-password','#settings-update','#settings-shutdown','#lang-toggle','#signout-btn'].map(s => {
            const r=document.querySelector(s).getBoundingClientRect(); return {left:r.left,right:r.right,top:r.top};
        }));
        assert(positions.every((r,i) => i===0 || r.left>=positions[i-1].right));
        assert.equal(await page.locator('.web-terminal-title span').textContent(), '设置');
        assert.equal(calls.filter(c => c.method==='POST').length, 0);
    }],
    ['minimal settings needs no console page or management requests', async ({ page, url, calls }) => {
        const count=calls.length;
        await page.goto(url+'?console=0');
        const frame=page.frameLocator('#settings-frame');
        await frame.locator('#settings-password').waitFor();
        assert((await page.locator('#settings-frame').getAttribute('src')).endsWith('/settings.html'));
        await frame.locator('#settings-update').click(); await frame.locator('#update-cancel').click();
        await openShutdown(frame); await frame.locator('#shutdown-cancel').click();
        assert(!calls.some(c=>c.url==='/api/shutdown'));
        assert(!calls.slice(count).some(c => ['/console','/console.css','/console.js'].includes(c.url) || c.url.startsWith('/api/console/') || c.url.startsWith('/api/apps')));
        assert.equal(await frame.locator('.nav-item').count(), 0);
    }],
    ['320px to desktop layouts and both languages fit without overlapping', async ({ page, frame }) => {
        for (const width of [320,375,768,1280]) {
            await page.setViewportSize({width,height:720});
            for(let language=0;language<2;language++) {
                await frame.locator('#lang-toggle').click();
                const b=await frame.evaluate(() => {
                    const nav=document.querySelector('.nav-list').getBoundingClientRect(), a=document.querySelector('#settings-actions').getBoundingClientRect();
                    return {navRight:nav.right,actionsLeft:a.left,right:a.right,width:innerWidth,scroll:document.documentElement.scrollWidth,
                        buttons:[...document.querySelectorAll('.settings-action')].map(n=>({height:n.getBoundingClientRect().height,label:n.getAttribute('aria-label')}))};
                });
                assert(b.right<=b.width+1 && b.scroll<=b.width && b.navRight<=b.actionsLeft,JSON.stringify(b));
                assert(b.buttons.every(n=>n.label && (width>760 || n.height>=44)));
            }
        }
    }],
    ['advanced settings loads on demand, preserves drafts and saves with CSRF', async ({ page, frame, calls }) => {
        let saved={enabled:false,ip_allowlist:[],client_ip:'192.168.1.20',authentication_enabled:true,matches:false};
        await page.route('**/api/settings/access',route=>{
            if(route.request().method()==='PUT') saved={...saved,...route.request().postDataJSON(),matches:true};
            return route.fulfill({json:saved});
        });
        assert(!calls.some(c=>c.url==='/api/settings/access'));
        await frame.locator('#nav-advanced').click();
        await frame.locator('#access-rules:enabled').waitFor();
        assert.equal(await frame.locator('#access-client-ip').textContent(),'192.168.1.20');
        await frame.locator('#access-enabled').check();
        await frame.locator('#access-rules').fill('192.168.1.0/24\n2001:db8::/64');
        await frame.locator('#nav-overview').click();await frame.locator('#nav-advanced').click();
        await frame.locator('#lang-toggle').click();
        assert.equal(await frame.locator('#access-rules').inputValue(),'192.168.1.0/24\n2001:db8::/64');
        assert.equal(await frame.locator('#access-title').textContent(),'Passwordless sign-in by IP');
        await frame.locator('#access-save').click();
        await frame.locator('#access-message').filter({hasText:'Saved and applied'}).waitFor();
        const request=calls.find(c=>c.url==='/api/settings/access'&&c.method==='PUT');
        assert.equal(request.csrf,'fixture-csrf');
        assert.deepEqual(request.body,{enabled:true,ip_allowlist:['192.168.1.0/24','2001:db8::/64']});
        assert(await frame.locator('#access-save').isDisabled());
    }],
    ['advanced settings reports validation lines and failed saves preserve the draft', async ({ page, frame }) => {
        await page.route('**/api/settings/access',route=>route.fulfill(route.request().method()==='GET'
            ?{json:{enabled:false,ip_allowlist:[],client_ip:'192.168.1.20',authentication_enabled:true,matches:false}}
            :{status:400,json:{error:'invalid_ip_rule',line:3}}));
        await frame.locator('#nav-advanced').click();await frame.locator('#access-rules:enabled').waitFor();
        await frame.locator('#access-enabled').check();
        await frame.locator('#access-rules').fill('192.168.1.20\n\ninvalid');
        await frame.locator('#access-save').click();
        await frame.locator('#access-message').filter({hasText:'第 3 行'}).waitFor();
        assert.equal(await frame.locator('#access-rules').inputValue(),'192.168.1.20\n\ninvalid');
        await page.route('**/api/settings/access',route=>route.fulfill({status:500,json:{error:'access_save_failed'}}));
        await frame.locator('#access-rules').fill('192.168.1.20');await frame.locator('#access-save').click();
        await frame.locator('#access-message').filter({hasText:'原有白名单继续有效'}).waitFor();
        assert.equal(await frame.locator('#access-rules').inputValue(),'192.168.1.20');
        assert(await frame.locator('#access-enabled').isChecked());
        assert.equal(await frame.locator('#access-save').isDisabled(),false);
    }],
    ['minimal settings shares advanced controls without console management requests', async ({ page, url, calls }) => {
        await page.route('**/api/settings/access',route=>route.fulfill({json:{enabled:false,ip_allowlist:[],client_ip:'::1',authentication_enabled:true,matches:false}}));
        const first=calls.length;
        await page.goto(url+'?console=0');const frame=page.frameLocator('#settings-frame');
        await frame.locator('#nav-advanced').click();await frame.locator('#access-rules:enabled').waitFor();
        assert.equal(await frame.locator('#access-client-ip').textContent(),'::1');
        assert(!calls.slice(first).some(c=>c.url.startsWith('/api/console/')||c.url.startsWith('/api/apps')));
        for(const width of [320,375,768,1280]) {
            await page.setViewportSize({width,height:720});
            assert.equal(await frame.locator('#access-rules').evaluate(el=>document.documentElement.scrollWidth>innerWidth),false);
        }
    }],
    ['scroll speed is immediate, local and independent of failed IP settings', async ({ page, frame, calls }) => {
        await page.route('**/api/settings/access', route => route.fulfill({status:503,json:{error:'network'}}));
        await frame.locator('#nav-advanced').click();
        const slider = frame.locator('#scroll-speed');
        assert.equal(await slider.inputValue(), '1');
        assert.equal(await frame.locator('#scroll-value').textContent(), '1.0×');
        assert(await frame.locator('#scroll-reset').isDisabled());
        await slider.focus(); await slider.press('End');
        for (let i=0;i<6;i++) await slider.press('ArrowLeft');
        assert.equal(await frame.locator('#scroll-value').textContent(), '2.5×');
        assert.equal(await frame.evaluate(() => localStorage.getItem('ivnc.scroll-speed.v1')), '2.5');
        assert(await frame.locator('#access-save').isDisabled());
        assert(!calls.some(c => c.url==='/api/settings/access' && c.method==='PUT'));
        await page.reload();
        frame=page.frameLocator('#settings-frame');
        await frame.locator('#nav-advanced').click();
        assert.equal(await frame.locator('#scroll-speed').inputValue(), '2.5');
        await frame.locator('#lang-toggle').click();
        assert.equal(await frame.locator('#scroll-title').textContent(), 'Scroll speed');
        assert.match(await frame.locator('#scroll-hint').textContent(), /this browser only/);
        await frame.locator('#scroll-reset').click();
        assert.equal(await frame.locator('#scroll-speed').inputValue(), '1');
    }],
    ['scroll speed synchronizes across settings tabs without overwriting an IP draft', async ({ page, frame, context, url }) => {
        await page.route('**/api/settings/access', route => route.fulfill({json:{enabled:false,ip_allowlist:[],authentication_enabled:true}}));
        await frame.locator('#nav-advanced').click(); await frame.locator('#access-rules:enabled').waitFor();
        await frame.locator('#access-enabled').check(); await frame.locator('#access-rules').fill('192.168.1.0/24');
        const other=await context.newPage();
        try {
            await other.route('**/auth/session',route=>route.fulfill({json:{enabled:true,authenticated:true,kind:'session',csrf:'fixture-csrf'}}));
            await other.goto(new URL('/settings.html',url).href); await other.locator('#nav-advanced').click();
            await other.locator('#scroll-speed').focus(); await other.locator('#scroll-speed').press('End');
            await frame.locator('#scroll-value').filter({hasText:'4.0×'}).waitFor();
            assert.equal(await frame.locator('#access-rules').inputValue(), '192.168.1.0/24');
            assert.equal(await frame.locator('#access-save').isDisabled(), false);
            for (const width of [320,375,768,1280]) {
                await page.setViewportSize({width,height:720});
                assert.equal(await frame.locator('#scroll-speed').evaluate(el=>document.documentElement.scrollWidth>innerWidth),false);
            }
        } finally { await other.close(); }
    }],
    ['password validation and failed submission keep native input and CSRF', async ({ page, frame, calls }) => {
        await page.route('**/api/change-password', route => route.fulfill({status:403,json:{error:'<img src=x onerror=alert(1)>'}}));
        await openPassword(frame);
        await frame.locator('#pwd-new').fill('abc'); await frame.locator('#pwd-confirm').fill('abc');
        await frame.locator('#pwd-ok').click(); assert.match(await frame.locator('#pwd-msg').textContent(),/4/);
        await frame.locator('#pwd-new').fill('password'); await frame.locator('#pwd-confirm').fill('different');
        await frame.locator('#pwd-ok').click(); assert.match(await frame.locator('#pwd-msg').textContent(),/不一致/);
        assert.equal(calls.filter(c=>c.url==='/api/change-password').length,0);
        await frame.locator('#pwd-new').fill('输入🙂password'); await frame.locator('#pwd-confirm').fill('输入🙂password');
        await frame.locator('#pwd-confirm').press('Enter');
        await wait(frame,()=>document.querySelector('#pwd-msg').textContent.includes('<img'));
        assert.equal(await frame.locator('#pwd-msg img').count(),0);
        const request=calls.find(c=>c.url==='/api/change-password');
        assert.equal(request.csrf,'fixture-csrf'); assert.equal(request.body.new_password,'输入🙂password');
        assert.equal(await frame.locator('#pwd-ok').isDisabled(),false);
    }],
    ['Escape, backdrop and parent clicks dismiss only the child and restore focus', async ({ page, frame }) => {
        for (const method of ['escape','backdrop','parent']) {
            await openPassword(frame);
            await frame.locator('#pwd-new').evaluate(el=>el.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',isComposing:true,bubbles:true,cancelable:true})));
            assert.equal(await frame.locator('#pwd-new').count(),1);
            if(method==='escape') await frame.locator('#pwd-new').press('Escape');
            else if(method==='backdrop') await frame.locator('.settings-dialog-bg').click({position:{x:2,y:2}});
            else await page.mouse.click(2,2);
            await closed(frame);
            assert(await page.locator('.web-settings-modal').isVisible());
            assert(await frame.locator('#settings-password').evaluate(el=>document.activeElement===el));
            assert.equal(await page.evaluate(()=>fixture.clicks),0);
            assert.equal(await page.evaluate(()=>document.documentElement.hasAttribute('data-ivnc-modal-blocked')),false);
        }
    }],
    ['password request locks dismissal, minimizing and duplicate submission', async ({ page, frame, calls }) => {
        let release;
        const gate=new Promise(resolve=>{release=resolve});
        await page.route('**/api/change-password',async route=>{await gate;await route.fulfill({status:500,json:{error:'test failure'}})});
        try {
            await openPassword(frame); await frame.locator('#pwd-new').fill('test-password'); await frame.locator('#pwd-confirm').fill('test-password');
            await frame.locator('#pwd-ok').click(); await locked(frame);
            await frame.evaluate(()=>{document.querySelector('form.settings-dialog').requestSubmit();window.__ivncConsoleDialogState.dismiss(window.__ivncConsoleDialogState.id)});
            await frame.evaluate(()=>document.querySelector('#add-app-btn').click());
            await page.mouse.click(2,2); await page.keyboard.press('Escape');
            await page.evaluate(()=>fixture.controller.minimize());
            assert(await page.locator('.web-settings-modal').isVisible());
            assert(await page.locator('#web-console-minimize').isDisabled());
            assert(await frame.locator('#pwd-cancel').isDisabled());
            assert.equal(calls.filter(c=>c.url==='/api/change-password').length,1);
            release(); await unlocked(frame);
            assert.equal(await page.locator('#web-console-minimize').isDisabled(),false);
            await frame.locator('#pwd-cancel').click(); await closed(frame);
        } finally { release(); }
    }],
    ['restart defaults to no upgrade and HTTP failure unlocks the dialog', async ({ page, frame, calls }) => {
        await page.route('**/api/restart',route=>route.fulfill({status:500,json:{error:'restart refused'}}));
        await openUpdate(frame);
        assert.equal(await frame.locator('#upgrade-latest').isChecked(),false);
        assert.equal(calls.filter(c=>c.url==='/api/restart').length,0);
        await frame.locator('#update-ok').click();
        await wait(frame,()=>document.querySelector('#update-logs').textContent.includes('restart refused'));
        await unlocked(frame);
        assert.equal(calls.filter(c=>c.url==='/api/restart').length,1);
        assert.equal(calls.find(c=>c.url==='/api/restart').csrf,'fixture-csrf');
        await frame.locator('#update-cancel').click(); await closed(frame);
    }],
    ['shutdown cancellation never sends a request and dismisses only the confirmation', async ({ page, frame, calls }) => {
        for (const method of ['cancel','escape','backdrop','parent']) {
            await openShutdown(frame);
            assert(await frame.locator('#shutdown-cancel').evaluate(el=>document.activeElement===el));
            assert.match(await frame.locator('.shutdown-dialog').textContent(),/保存应用运行状态/);
            if(method==='cancel') await frame.locator('#shutdown-cancel').click();
            else if(method==='escape') await frame.locator('#shutdown-cancel').press('Escape');
            else if(method==='backdrop') await frame.locator('.shutdown-overlay').click({position:{x:2,y:2}});
            else await page.mouse.click(2,2);
            await closed(frame);
            assert(await page.locator('.web-settings-modal').isVisible());
            assert(await frame.locator('#settings-shutdown').evaluate(el=>document.activeElement===el));
            assert.equal(calls.filter(c=>c.url==='/api/shutdown').length,0);
        }
        await frame.locator('#lang-toggle').click(); await openShutdown(frame);
        assert.equal(await frame.locator('#settings-dialog-title').textContent(),'Shut down');
        assert.equal(await frame.locator('#shutdown-ok').textContent(),'Confirm shutdown');
    }],
    ['shutdown confirms once, locks pending operations and never polls for a restart', async ({ page, frame, calls }) => {
        let release;const gate=new Promise(resolve=>{release=resolve});
        await page.route('**/api/shutdown',async route=>{await gate;await route.fulfill({status:202,json:{ok:true,mode:'shutdown'}})});
        try {
            await openShutdown(frame);
            assert.equal(calls.filter(c=>c.url==='/api/shutdown').length,0);
            await frame.locator('#shutdown-ok').click();await locked(frame);
            await frame.evaluate(()=>document.querySelector('form.shutdown-dialog').requestSubmit());
            await page.mouse.click(2,2);await page.keyboard.press('Escape');
            await page.evaluate(()=>fixture.controller.minimize());
            assert(await page.locator('.web-settings-modal').isVisible());
            assert(await frame.locator('#shutdown-cancel').isDisabled());
            assert.equal(calls.filter(c=>c.url==='/api/shutdown').length,1);
            assert.equal(calls.find(c=>c.url==='/api/shutdown').csrf,'fixture-csrf');
            assert.equal(calls.find(c=>c.url==='/api/shutdown').method,'POST');
            release();await unlocked(frame);
            assert.match(await frame.locator('#shutdown-msg').textContent(),/请求已接受/);
            assert(await frame.locator('#shutdown-ok').isHidden());
            await frame.evaluate(()=>document.querySelector('form.shutdown-dialog').requestSubmit());
            assert.equal(calls.filter(c=>c.url==='/api/shutdown').length,1);
            assert.equal(calls.filter(c=>['/health','/api/restart','/api/upgrade/ws'].includes(c.url)).length,0);
            await frame.locator('#shutdown-cancel').click();await closed(frame);
        } finally { release(); }
    }],
    ['shutdown HTTP and connection failures unlock without claiming success', async ({ page, frame, calls }) => {
        for (const mode of ['http','connection']) {
            await page.route('**/api/shutdown',route=>mode==='http'?route.fulfill({status:503,json:{error:'<b>shutdown unavailable</b>'}}):route.abort('failed'));
            await openShutdown(frame);await frame.locator('#shutdown-ok').click();
            await wait(frame,()=>!window.__ivncConsoleDialogState.locked && !!document.querySelector('#shutdown-msg').textContent);
            assert.match(await frame.locator('#shutdown-msg').textContent(),mode==='http'?/shutdown unavailable/:/无法确认关机结果/);
            assert.equal(await frame.locator('#shutdown-msg b').count(),0);
            assert.equal(await frame.locator('#shutdown-msg').evaluate(el=>el.classList.contains('success')),false);
            assert.equal(await frame.locator('#shutdown-ok').isDisabled(),false);
            await frame.locator('#shutdown-cancel').click();await closed(frame);
        }
        assert.equal(calls.filter(c=>c.url==='/api/shutdown').length,2);
    }],
    ['shutdown requires a complete and valid acknowledgment before displaying acceptance', async ({ page, frame }) => {
        for (const reply of [
            {status:200,json:{ok:true,mode:'shutdown'}},
            {status:202,json:{ok:false}},
            {status:202,contentType:'application/json',body:'{"ok":true'},
        ]) {
            await page.route('**/api/shutdown',route=>route.fulfill(reply));
            await openShutdown(frame);await frame.locator('#shutdown-ok').click();await unlocked(frame);
            assert.match(await frame.locator('#shutdown-msg').textContent(),/无法确认关机结果/);
            assert.equal(await frame.locator('#shutdown-msg').evaluate(el=>el.classList.contains('success')),false);
            assert.equal(await frame.locator('#shutdown-ok').isDisabled(),false);
            await frame.locator('#shutdown-cancel').click();await closed(frame);
        }
    }],
    ['shutdown timeout and page teardown release the request and modal shield', async ({ page, frame }) => {
        await page.route('**/api/shutdown',()=>{});
        await page.clock.install();await openShutdown(frame);await frame.locator('#shutdown-ok').click();await locked(frame);
        await page.clock.runFor(15001);await unlocked(frame);
        assert.match(await frame.locator('#shutdown-msg').textContent(),/无法确认关机结果/);
        await frame.locator('#shutdown-cancel').click();await closed(frame);
        await openShutdown(frame);await frame.locator('#shutdown-ok').click();await locked(frame);
        await frame.evaluate(()=>window.dispatchEvent(new PageTransitionEvent('pagehide',{persisted:false})));
        assert.equal(await frame.locator('.settings-dialog-bg').count(),0);
        assert.equal(await page.evaluate(()=>document.documentElement.hasAttribute('data-ivnc-modal-blocked')),false);
    }],
    ['shutdown response body remains covered by timeout and page teardown', async ({ page, frame }) => {
        await frame.evaluate(()=>{
            const previous=window.fetch;
            window.fetch=(url,options)=>{
                if(url!=='/api/shutdown')return previous(url,options);
                return Promise.resolve(new Response(new ReadableStream({
                    start(controller){options.signal.addEventListener('abort',()=>{window.shutdownBodyAborted=true;controller.error(new DOMException('Aborted','AbortError'))},{once:true})},
                }),{status:202,headers:{'Content-Type':'application/json'}}));
            };
        });
        await page.clock.install();await openShutdown(frame);await frame.locator('#shutdown-ok').click();await locked(frame);
        await page.clock.runFor(15001);await unlocked(frame);
        assert.match(await frame.locator('#shutdown-msg').textContent(),/无法确认关机结果/);
        assert.equal(await frame.evaluate(()=>window.shutdownBodyAborted),true);
        await frame.locator('#shutdown-cancel').click();await closed(frame);
        await frame.evaluate(()=>{window.shutdownBodyAborted=false});
        await openShutdown(frame);await frame.locator('#shutdown-ok').click();await locked(frame);
        await frame.evaluate(()=>window.dispatchEvent(new PageTransitionEvent('pagehide',{persisted:false})));
        assert.equal(await frame.evaluate(()=>window.shutdownBodyAborted),true);
        assert.equal(await frame.locator('.settings-dialog-bg').count(),0);
    }],
    ['upgrade starts only after confirmation and final restart error is not success', async ({ frame, calls, handleUpgrade }) => {
        let socket,opened=0;
        handleUpgrade(ws=>{socket=ws;opened++});
        await openUpdate(frame); await frame.locator('#upgrade-latest').check();
        assert.match(await frame.locator('#update-ok').textContent(),/升级/); assert.equal(opened,0);
        await frame.locator('#update-ok').click(); await locked(frame);
        await wait(frame,()=>document.querySelector('#update-info').hidden);
        for(let attempt=0;!socket&&attempt<100;attempt++) await new Promise(resolve=>setTimeout(resolve,20));
        assert(socket,await frame.locator('#update-logs').textContent());
        socket.send(JSON.stringify({step:10,total_steps:10,level:'info',message:'restarting'}));
        socket.send(JSON.stringify({step:10,total_steps:10,level:'error',message:'restart failed'}));
        socket.close(1000,'fixture');
        await unlocked(frame);
        assert.equal(opened,1); assert.match(await frame.locator('#update-logs').textContent(),/restart failed/);
        assert.equal(calls.filter(c=>c.url==='/health').length,0);
        await frame.locator('#update-cancel').click(); await closed(frame);
    }],
    ['upgrade proxy controls appear on demand and inactive custom input cannot block restart', async ({ page, frame, calls }) => {
        await openUpdate(frame);
        assert.equal(await frame.locator('#upgrade-use-proxy').isVisible(),false);
        await frame.locator('#upgrade-latest').check();
        assert(await frame.locator('#upgrade-use-proxy').isVisible());
        assert.equal(await frame.locator('#upgrade-use-proxy').isChecked(),false);
        assert.equal(await frame.locator('#upgrade-proxy').isVisible(),false);
        await frame.locator('#upgrade-use-proxy').check();
        assert.equal(await frame.locator('#upgrade-proxy').inputValue(),'https://sina.dev/');
        assert.equal(await frame.locator('#upgrade-custom-proxy').isVisible(),false);
        await frame.locator('#upgrade-proxy').selectOption('custom');
        assert(await frame.locator('#upgrade-custom-proxy').isVisible());
        assert.equal(await frame.locator('#upgrade-custom-proxy').evaluate(el=>el.required),true);
        await frame.locator('#upgrade-use-proxy').uncheck();
        assert(await frame.locator('#upgrade-custom-proxy').isDisabled());
        await frame.locator('#upgrade-use-proxy').check();
        await frame.locator('#upgrade-latest').uncheck();
        assert(await frame.locator('#upgrade-custom-proxy').isDisabled());
        assert.equal(calls.filter(c=>c.method==='WS'||c.url==='/api/restart').length,0);
        await page.route('**/api/restart',route=>route.fulfill({status:500,json:{error:'restart-only'}}));
        await frame.locator('#update-ok').click(); await unlocked(frame);
        assert.match(await frame.locator('#update-logs').textContent(),/restart-only/);
        assert.equal(calls.filter(c=>c.url==='/api/restart').length,1);
        assert.equal(calls.filter(c=>c.method==='WS').length,0);
    }],
    ['upgrade uses the chosen proxy and safely encodes normalized custom addresses', async ({ frame, calls, handleUpgrade }) => {
        for (const [choice,address,expected] of [
            ['https://sina.dev/',null,'https://sina.dev/'],
            ['https://httptunnel.top/gh/',null,'https://httptunnel.top/gh/'],
            ['custom',' proxy.example:8443/gh ','https://proxy.example:8443/gh/'],
            ['custom','http://127.0.0.1:8080/代理/','http://127.0.0.1:8080/%E4%BB%A3%E7%90%86/'],
        ]) {
            let requestUrl;
            handleUpgrade((ws,request)=>{
                requestUrl=new URL(request.url,'http://localhost');
                ws.send(JSON.stringify({step:3,total_steps:10,level:'error',message:'fixture download stopped'})); ws.close(1000);
            });
            await openUpdate(frame); await frame.locator('#upgrade-latest').check(); await frame.locator('#upgrade-use-proxy').check();
            await frame.locator('#upgrade-proxy').selectOption(choice);
            if(address) await frame.locator('#upgrade-custom-proxy').fill(address);
            await frame.locator('#update-ok').click();
            await wait(frame,()=>document.querySelector('#update-logs').textContent.includes('fixture download stopped')); await unlocked(frame);
            assert.equal(requestUrl.searchParams.get('github_proxy'),expected);
            await frame.locator('#update-cancel').click(); await closed(frame);
        }
        assert.equal(calls.filter(c=>c.method==='WS').length,4);
        assert.equal(calls.filter(c=>c.url==='/api/restart').length,0);
    }],
    ['invalid custom proxies stay editable and never start an upgrade', async ({ frame, calls }) => {
        await openUpdate(frame); await frame.locator('#upgrade-latest').check(); await frame.locator('#upgrade-use-proxy').check();
        await frame.locator('#upgrade-proxy').selectOption('custom');
        for(const value of ['ftp://proxy.example/','https://user:password@proxy.example/','https://proxy.example/?token=secret','https://proxy.example/#part','https://proxy.example/some path','https://proxy.example/'+'代'.repeat(230)]) {
            await frame.locator('#upgrade-custom-proxy').fill(value); await frame.locator('#update-ok').click();
            assert(await frame.locator('#upgrade-proxy-message').isVisible());
            assert.match(await frame.locator('#upgrade-proxy-message').textContent(),/HTTP\(S\)/);
            await unlocked(frame); assert(await frame.locator('#update-info').isVisible());
        }
        assert.equal(calls.filter(c=>c.method==='WS'||c.url==='/api/restart').length,0);
        await frame.locator('#upgrade-use-proxy').uncheck();
        assert.equal(await frame.locator('#upgrade-proxy-message').isVisible(),false);
    }],
    ['unchecked proxy uses the existing automatic route even after entering a custom proxy', async ({ frame, handleUpgrade }) => {
        let requestUrl;
        handleUpgrade((ws,request)=>{
            requestUrl=new URL(request.url,'http://localhost');
            ws.send(JSON.stringify({step:3,total_steps:10,level:'error',message:'automatic route'})); ws.close(1000);
        });
        await openUpdate(frame); await frame.locator('#upgrade-latest').check(); await frame.locator('#upgrade-use-proxy').check();
        await frame.locator('#upgrade-proxy').selectOption('custom'); await frame.locator('#upgrade-custom-proxy').fill('https://proxy.example/');
        await frame.locator('#upgrade-use-proxy').uncheck(); await frame.locator('#update-ok').click();
        await wait(frame,()=>document.querySelector('#update-logs').textContent.includes('automatic route')); await unlocked(frame);
        assert.equal(requestUrl.search,'');
    }],
    ['upgrade proxy form fits narrow screens and both settings pages', async ({ page, frame, url }) => {
        for(const minimal of [false,true]) {
            if(minimal) {
                await page.goto(url+'?console=0'); await page.frameLocator('#settings-frame').locator('#settings-update').waitFor();
                frame=page.frames().find(f=>new URL(f.url()).pathname==='/settings.html');
            }
            if(await frame.locator('#lang-toggle').textContent()==='中文') await frame.locator('#lang-toggle').click();
            await page.setViewportSize({width:320,height:600});
            await openUpdate(frame); await frame.locator('#upgrade-latest').check(); await frame.locator('#upgrade-use-proxy').check();
            await frame.locator('#upgrade-proxy').selectOption('custom');
            const bounds=await frame.evaluate(()=>{
                const dialog=document.querySelector('.update-dialog');
                return {width:dialog.clientWidth,scroll:dialog.scrollWidth};
            });
            assert(bounds.scroll<=bounds.width,JSON.stringify(bounds));
            await frame.locator('#update-ok').scrollIntoViewIfNeeded();
            const button=await frame.locator('#update-ok').boundingBox();
            assert(button.x>=0 && button.x+button.width<=320 && button.y>=0 && button.y+button.height<=600,JSON.stringify(button));
            await frame.locator('#update-cancel').click(); await closed(frame);
            await frame.locator('#lang-toggle').click(); await openUpdate(frame); await frame.locator('#upgrade-latest').check();
            assert.match(await frame.locator('#upgrade-proxy-options').textContent(),/Use a GitHub proxy/);
            await frame.locator('#update-cancel').click(); await closed(frame);
        }
    }],
    ['an incomplete upgrade socket cannot be reported as successful', async ({ frame, calls, handleUpgrade }) => {
        handleUpgrade(ws=>{ws.send(JSON.stringify({step:2,total_steps:10,level:'info',message:'downloading'}));ws.close(1000)});
        await openUpdate(frame); await frame.locator('#upgrade-latest').check(); await frame.locator('#update-ok').click();
        await unlocked(frame);
        assert.match(await frame.locator('#update-logs').textContent(),/无法确认/);
        assert.equal(calls.filter(c=>c.url==='/health').length,0);
    }],
    ['a stalled upgrade unlocks after its inactivity timeout', async ({ page, frame, handleUpgrade }) => {
        handleUpgrade(()=>{});
        await page.clock.install();
        await openUpdate(frame); await frame.locator('#upgrade-latest').check(); await frame.locator('#update-ok').click();
        await locked(frame); await page.clock.runFor(120001); await unlocked(frame);
        assert.match(await frame.locator('#update-logs').textContent(),/无法确认/);
        assert.equal(await page.locator('#web-console-minimize').isDisabled(),false);
        await frame.locator('#update-cancel').click(); await closed(frame);
    }],
    ['successful upgrade tolerates a recovered warning and reloads the main page', async ({ page, frame, calls, handleUpgrade }) => {
        handleUpgrade(ws=>{
            for(const entry of [{step:0,level:'error',message:'state save warning'},{step:1,level:'success',message:'continuing'},
                {step:10,level:'info',message:'restarting'},{step:10,level:'success',message:'restart triggered'}]) ws.send(JSON.stringify({...entry,total_steps:10}));
            ws.close(1000);
        });
        await openUpdate(frame); await frame.locator('#upgrade-latest').check();
        const navigation=page.waitForEvent('framenavigated',{predicate:f=>f===page.mainFrame()});
        await frame.locator('#update-ok').click(); await navigation;
        await page.frameLocator('#settings-frame').locator('#settings-update').waitFor();
        assert(calls.some(c=>c.url==='/health'));
    }],
    ['software keyboard and pinch keep settings within the visible work area', async ({ page, frame }) => {
        await page.setViewportSize({width:375,height:800}); await openPassword(frame);
        await page.evaluate(()=>{
            fixture.originalViewport=window.visualViewport;fixture.layoutEvents=0;
            window.addEventListener('ivnc-overlay-layout',()=>fixture.layoutEvents++);
            const visual=Object.assign(new EventTarget(),{height:360,offsetTop:10,scale:1});
            Object.defineProperty(window,'visualViewport',{configurable:true,value:visual});fixture.originalViewport.dispatchEvent(new Event('resize'));
        });
        const bounds=await page.locator('.web-settings-modal').boundingBox();
        assert(bounds.y>=10 && bounds.y+bounds.height<=370,JSON.stringify(bounds));
        await frame.locator('#pwd-ok').scrollIntoViewIfNeeded();
        const button=await frame.locator('#pwd-ok').boundingBox();
        assert(button.y>=10 && button.y+button.height<=370,JSON.stringify(button));
        await page.evaluate(()=>{Object.assign(visualViewport,{height:150,offsetTop:30,scale:2});fixture.originalViewport.dispatchEvent(new Event('scroll'))});
        assert.deepEqual(await page.locator('.web-settings-modal').boundingBox(),bounds);
        assert.equal(await page.evaluate(()=>fixture.layoutEvents),0,'keyboard layout must not cancel terminal dragging');
    }],
    ['disabled authentication hides account actions while preserving service controls', async ({ page, url, state }) => {
        state.enabled=false; await page.goto(url+'?console=0');
        const frame=page.frameLocator('#settings-frame'); await frame.locator('#settings-update').waitFor();
        assert.equal(await frame.locator('#settings-password').isVisible(),false);
        assert.equal(await frame.locator('#signout-btn').isVisible(),false);
        await frame.locator('#settings-update').click(); await frame.locator('#update-cancel').click();
        assert(await frame.locator('#settings-shutdown').isVisible());
        await openShutdown(frame);await frame.locator('#shutdown-cancel').click();
    }],
    ['password success immediately redirects the top document and other tabs', async ({ page, frame, context, url }) => {
        const other=await context.newPage();
        await other.route('**/auth/session',route=>route.fulfill({json:{enabled:true,authenticated:true,kind:'session',csrf:'fixture-csrf'}}));
        await other.goto(new URL('/settings.html',url).href);await other.locator('#settings-password').waitFor();
        await page.route('**/api/change-password',route=>route.fulfill({json:{ok:true}}));
        await openPassword(frame); await frame.locator('#pwd-new').fill('new-password');await frame.locator('#pwd-confirm').fill('new-password');
        await frame.locator('#pwd-ok').click(); await page.waitForURL(/\/auth\/login/);await other.waitForURL(/\/auth\/login/);
    }],
    ['leaving a locked operation cleans up its socket and modal shield', async ({ page, frame, handleUpgrade }) => {
        handleUpgrade(()=>{});
        await openUpdate(frame);await frame.locator('#upgrade-latest').check();await frame.locator('#update-ok').click();await locked(frame);
        await frame.evaluate(()=>window.dispatchEvent(new PageTransitionEvent('pagehide',{persisted:false})));
        assert.equal(await frame.locator('.settings-dialog-bg').count(),0);
        assert.equal(await page.evaluate(()=>document.documentElement.hasAttribute('data-ivnc-modal-blocked')),false);
        assert.equal(await page.locator('#web-console-minimize').isDisabled(),false);
    }],
];

await test('settings actions',async t=>{
    const work=mkdtempSync(path.join(tmpdir(),'ivnc-settings-test-'));
    const server=await createServer({configFile:false,root,cacheDir:path.join(work,'vite'),logLevel:'error',optimizeDeps:{noDiscovery:true,include:[]},server:{host:'127.0.0.1',port:0,hmr:false},plugins:[{name:'settings-fixture',configureServer(s){s.middlewares.use((req,res,next)=>{
        const url=req.url.split('?')[0];
        if(url==='/__settings'||url==='/console'||url==='/auth/login') {
            res.setHeader('Content-Type','text/html; charset=utf-8');
            return res.end(url==='/__settings'?fixture:url==='/console'?readFileSync(path.join(root,'console.html'),'utf8'):'<p>Sign in</p>');
        }
        next();
    })}}]});
    const sockets=new WebSocketServer({noServer:true});
    let upgradeHandler=()=>{};
    try {
        await server.listen();const url=`http://127.0.0.1:${server.httpServer.address().port}/__settings`;
        server.httpServer.on('upgrade',(request,socket,head)=>{
            if(new URL(request.url,'http://localhost').pathname!=='/api/upgrade/ws')return;
            sockets.handleUpgrade(request,socket,head,ws=>{ws.on('error',()=>{});upgradeHandler(ws,request)});
        });
        for(const name of (process.env.BROWSERS||'chromium,webkit').split(',')) await t.test(name,async t=>{
            const browser=await engines[name].launch({headless:true,...(name==='chromium'&&process.env.CHROME_BIN?{executablePath:process.env.CHROME_BIN}:{})});
            try { for(const [title,run] of cases) await t.test(title,async()=>{
                const context=await browser.newContext({viewport:{width:1280,height:800},locale:'zh-CN'});
                const page=await context.newPage(),errors=[],calls=[],state={enabled:true};
                context.setDefaultTimeout(7000);
                page.on('pageerror',error=>errors.push(error.message));
                page.on('websocket',socket=>calls.push({url:new URL(socket.url()).pathname,method:'WS'}));
                page.on('request',request=>{const u=new URL(request.url());if(u.pathname.startsWith('/api/')||u.pathname.startsWith('/console')||u.pathname==='/health')calls.push({url:u.pathname,method:request.method(),csrf:request.headers()['x-ivnc-csrf'],body:request.postDataJSON()})});
                await page.route('**/auth/session',route=>route.fulfill({json:{enabled:state.enabled,authenticated:true,kind:state.enabled?'session':'anonymous',csrf:'fixture-csrf'}}));
                await page.route('**/api/**',route=>route.fulfill({json:{apps:[],metrics:{},version:{},display:{},settings:{}}}));
                await page.route('**/health',route=>route.fulfill({json:{ok:true}}));
                try {
                    await page.goto(url);
                    await page.frameLocator('#settings-frame').locator('#settings-password').waitFor();
                    const child=page.frames().find(f=>new URL(f.url()).pathname==='/console');
                    upgradeHandler=()=>{throw Error('Upgrade started without confirmation')};
                    await run({page,frame:child,context,calls,state,url,handleUpgrade:handler=>{upgradeHandler=handler}});
                    assert.deepEqual(errors,[]);
                } catch(error) { error.message += `\nBrowser errors: ${JSON.stringify(errors)}\nRequests: ${JSON.stringify(calls.map(c=>[c.method,c.url]))}`;throw error; }
                finally { await context.close(); }
            }); } finally { await browser.close(); }
        });
    } finally { for(const socket of sockets.clients)socket.terminate();sockets.close();await server.close();rmSync(work,{recursive:true,force:true}); }
});
