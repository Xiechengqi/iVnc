// Native input in Chromium and WebKit, using the production toolbar modules.
// npm --prefix web/ivnc exec -- playwright install chromium webkit
// npm --prefix web/ivnc run test:toolbar
// CHROME_BIN can select an installed Chromium; BROWSERS can limit the engines.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const { createServer } = require('vite');
const engines = require('playwright');
const root = fileURLToPath(new URL('../web/ivnc/', import.meta.url));
const source = readFileSync(path.join(root, 'ivnc-wr-core.js'), 'utf8');
const styles = source.slice(source.indexOf('\t.taskbar {'), source.indexOf('\t.web-terminal-modal {'));
const fixture = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>${styles} #desktop {position:fixed;inset:0 0 36px;background:#123}</style>
<div id="desktop"></div><div id="taskbar" class="taskbar pinned"></div>
<script type="module">
import { createToolbar } from '/toolbar.js';
import { createAppStreams } from '/app-streams.js';
import { setLang } from '/lib/i18n.js?v=1';
setLang('zh');
const data = { order_revision:1, ordered_ids:['alpha','beta','shared','remote'], windows:[
    {id:1, window_key:'00000000-0000-4000-8000-000000000001', managed_app_id:'alpha', managed_app_name:'应用 Alpha', name_revision:0, can_rename:true, title:'Alpha', app_id:'test', focused:true},
    {id:2, window_key:'00000000-0000-4000-8000-000000000002', managed_app_id:'beta', managed_app_name:'应用 Beta', name_revision:0, can_rename:true, title:'Beta', app_id:'test', focused:false},
    {id:3, window_key:'00000000-0000-4000-8000-000000000003', title:'Unmanaged', app_id:'test', focused:false}
] };
const calls = [], pointerIds = [];
let streams;
const toolbar = createToolbar({element:document.querySelector('#taskbar'),
    canReorder:new URLSearchParams(location.search).get('reorder') !== '0', canRename:true,
    activateWindow:(id,key) => {
        calls.push({id,key}); streams.activate(null);
        data.windows.forEach(w => w.focused = w.window_key === key); render();
    }
});
function render() { toolbar.update(data, streams.tabs(), !!streams.activeId()); }
streams = createAppStreams({viewport:document.querySelector('#desktop'),onChange:render,onEdgeHover:()=>{}});
streams.update([
    {id:'shared',name:'共享应用',name_revision:0,can_rename:true,kind:'shared',status:'running'},
    {id:'remote',name:'远程应用',name_revision:0,can_rename:true,kind:'remote',status:'connected'}
]);
render();
document.addEventListener('pointerdown', e => pointerIds.push(e.pointerId), true);
window.fixture = {data,calls,toolbar,streams,render,pointerIds};
</script>`;
const embedded = `<!doctype html><div id="app" tabindex="0">Embedded application</div><script>
addEventListener('message', e => {
    if(e.origin !== location.origin || e.source !== parent) return;
    if(e.data.type === 'ivnc-view-active') {
        document.documentElement.dataset.active = String(e.data.active);
        if(e.data.active) document.querySelector('#app').focus();
    }
});
parent.postMessage({type:'ivnc-view-ready'}, location.origin);
parent.postMessage({type:'ivnc-view-state',state:'connected'}, location.origin);
</script>`;

const row = (page, id = 'beta') => page.locator(`.toolbar-list [data-app-id="${id}"]`);
const point = async (locator, edge = false) => {
    await locator.scrollIntoViewIfNeeded();
    const rect = await locator.boundingBox();
    assert(rect, 'tab is visible');
    return { x: rect.x + (edge ? 3 : Math.min(20, rect.width / 2)), y: rect.y + rect.height / 2 };
};
const click = async (page, id = 'beta') => {
    const p = await point(row(page, id).locator('.toolbar-label'));
    await page.mouse.click(p.x, p.y);
};
const press = async (page, id = 'beta') => {
    const p = await point(row(page, id).locator('.toolbar-label'));
    await page.mouse.move(p.x, p.y); await page.mouse.down(); return p;
};
const calls = page => page.evaluate(() => fixture.calls.map(c => c.id));
const ids = page => page.locator('.toolbar-list .taskbar-item').evaluateAll(nodes => nodes.map(n => n.dataset.appId));
const saved = page => page.waitForFunction(() => document.querySelector('.toolbar-list').getAttribute('aria-busy') === 'false');
const drag = async (page, padding = false) => {
    const tab = row(page, 'alpha');
    const start = await point(padding ? tab : tab.locator('.toolbar-label'), padding);
    const end = await row(page, 'beta').boundingBox();
    await page.mouse.move(start.x, start.y); await page.mouse.down();
    await page.mouse.move(start.x + 8, start.y);
    await page.mouse.move(end.x + end.width - 2, start.y);
    return start;
};

const cases = [
    ['native title, padding and unmanaged tab clicks', async page => {
        await click(page);
        const edge = await point(row(page, 'alpha'), true); await page.mouse.click(edge.x, edge.y);
        const unmanaged = page.locator('.toolbar-list [data-key$="000003"] .toolbar-label');
        const p = await point(unmanaged); await page.mouse.click(p.x, p.y);
        assert.deepEqual(await calls(page), [2,1,3]);
    }],
    ['text nodes survive same-state refresh and rename during a press', async page => {
        // Exercise text hit testing too, independently of decorative pointer-events.
        await row(page).locator('.toolbar-label').evaluate(n => n.style.pointerEvents = 'auto');
        await page.evaluate(() => {
            window.savedRow = document.querySelector('[data-app-id="beta"]');
            window.savedText = savedRow.querySelector('.toolbar-label').firstChild;
        });
        await press(page);
        await page.evaluate(() => {
            fixture.render();
            const app = fixture.data.windows[1]; app.managed_app_name = '新应用名称'; app.name_revision++;
            fixture.render();
        });
        assert.equal(await row(page).locator('.toolbar-label').textContent(), '应用 Beta');
        await page.mouse.up();
        assert.deepEqual(await calls(page), [2]);
        assert.equal(await row(page).locator('.toolbar-label').textContent(), '新应用名称');
        assert(await page.evaluate(() => savedRow === document.querySelector('[data-app-id="beta"]') && savedText === savedRow.querySelector('.toolbar-label').firstChild));
    }],
    ['non-sortable tabs remain clickable during background refresh', async page => {
        await press(page);
        await page.evaluate(() => { fixture.data.windows[1].managed_app_name = '更新名称'; fixture.data.windows[1].name_revision++; fixture.render(); });
        await page.mouse.up();
        assert.deepEqual(await calls(page), [2]);
        assert.equal(await row(page).locator('.toolbar-label').textContent(), '更新名称');
    }, '?reorder=0'],
    ['background reordering waits for the click and activates the original key', async page => {
        const before = await ids(page); await press(page);
        await page.evaluate(() => { fixture.data.ordered_ids.reverse(); fixture.data.order_revision++; fixture.render(); });
        assert.deepEqual(await ids(page), before);
        await page.mouse.up();
        assert.deepEqual(await calls(page), [2]);
        assert.deepEqual((await ids(page)).slice(0,4), ['remote','shared','beta','alpha']);
    }],
    ['a removed and reinserted tab cannot complete a stale click', async page => {
        await press(page);
        await page.evaluate(() => {
            const removed = fixture.data.windows.splice(1,1)[0]; fixture.render();
            fixture.data.windows.push(removed); fixture.render();
        });
        await page.mouse.up();
        assert.deepEqual(await calls(page), []);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }],
    ['sub-threshold pointer movement remains one ordinary click', async page => {
        const p = await press(page); await page.mouse.move(p.x + 3, p.y + 1); await page.mouse.up();
        assert.deepEqual(await calls(page), [2]);
        assert.equal(await page.locator('.toolbar-drag-ghost').count(), 0);
    }],
    ['vertical pointer movement inside a tab does not start sorting', async (page, api) => {
        const p = await press(page); await page.mouse.move(p.x + 1, p.y - 8);
        assert.equal(await page.locator('.toolbar-drag-ghost').count(), 0);
        await page.mouse.up();
        assert.deepEqual(await calls(page), [2]); assert.equal(api.orders.length, 0);
    }],
    ['row drag persists once and the immediate next click is accepted', async (page, api) => {
        await drag(page);
        assert.deepEqual((await ids(page)).slice(0,2), ['beta','alpha']);
        assert.equal(api.orders.length, 0);
        await page.mouse.up(); await saved(page);
        assert.equal(api.orders.length, 1); assert.deepEqual(api.orders[0].ordered_ids.slice(0,2), ['beta','alpha']);
        assert.deepEqual(await calls(page), []);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }],
    ['tabs have no handle and dragging their padding also sorts', async (page, api) => {
        assert.equal(await page.locator('.taskbar-item button').count(), 0);
        assert.equal(await row(page).evaluate(n => getComputedStyle(n).cursor), 'grab');
        await drag(page, true); await page.mouse.up();
        await saved(page);
        assert.equal(api.orders.length, 1); assert.deepEqual(await calls(page), []);
        const p = await point(row(page), true); await page.mouse.click(p.x,p.y);
        assert.deepEqual(await calls(page), [2]);
    }],
    ['trailing tab padding can drag left with a preview before release', async (page, api) => {
        const a = await row(page,'alpha').boundingBox(), b = await row(page).boundingBox();
        const y = b.y + b.height / 2;
        await page.mouse.move(b.x + b.width - 3,y); await page.mouse.down();
        await page.mouse.move(b.x + b.width - 11,y); await page.mouse.move(a.x + 3,y);
        assert.deepEqual((await ids(page)).slice(0,2), ['beta','alpha']);
        assert.equal(api.orders.length, 0);
        await page.mouse.up(); await saved(page);
        assert.equal(api.orders.length, 1); assert.deepEqual(await calls(page), []);
    }],
    ['left and right arrows reorder the focused tab and preserve focus', async (page, api) => {
        await row(page).focus(); await page.keyboard.press('ArrowLeft'); await saved(page);
        assert.deepEqual((await ids(page)).slice(0,2), ['beta','alpha']);
        assert(await row(page).evaluate(n => document.activeElement === n));
        await page.keyboard.press('ArrowRight'); await saved(page);
        assert.deepEqual((await ids(page)).slice(0,2), ['alpha','beta']);
        assert(await row(page).evaluate(n => document.activeElement === n));
        assert.equal(api.orders.length, 2); assert.deepEqual(await calls(page), []);
    }],
    ['read-only tabs reject mouse and keyboard sorting while clicks still work', async (page, api) => {
        await drag(page); await page.mouse.up();
        assert.equal(api.orders.length, 0);
        assert.deepEqual((await ids(page)).slice(0,2), ['alpha','beta']);
        await row(page).focus(); await page.keyboard.press('ArrowLeft');
        assert.equal(api.orders.length, 0);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }, '?reorder=0'],
    ['Escape cancels a drag without swallowing the next click', async (page, api) => {
        await drag(page); await page.keyboard.press('Escape'); await page.mouse.up();
        assert.equal(api.orders.length, 0); assert.deepEqual(await calls(page), []);
        assert.equal(await page.locator('.toolbar-drag-ghost').count(), 0);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }],
    ['losing pointer capture cancels sorting and releases the toolbar', async (page, api) => {
        const start = await drag(page);
        await page.evaluate(() => {
            const viewport = document.querySelector('.toolbar-viewport');
            viewport.releasePointerCapture(fixture.pointerIds.at(-1));
        });
        await page.mouse.move(start.x + 10,start.y);
        await page.waitForFunction(() => !fixture.toolbar.interacting());
        await page.mouse.up();
        assert.equal(api.orders.length, 0); assert.deepEqual(await calls(page), []);
        assert.equal(await page.locator('.toolbar-drag-ghost').count(), 0);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }],
    ['releasing outside the toolbar cancels sorting and activation', async (page, api) => {
        await press(page); await page.mouse.move(20,20); await page.mouse.up();
        assert.equal(api.orders.length, 0); assert.deepEqual(await calls(page), []);
        assert.equal(await page.locator('.toolbar-drag-ghost').count(), 0);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }],
    ['pointer cancellation is scoped to the active pointer', async page => {
        await press(page);
        await page.evaluate(() => document.dispatchEvent(new PointerEvent('pointercancel', {pointerId:999,bubbles:true})));
        assert(await page.evaluate(() => fixture.toolbar.interacting()));
        await page.evaluate(() => document.dispatchEvent(new PointerEvent('pointercancel', {pointerId:fixture.pointerIds.at(-1),bubbles:true})));
        assert.equal(await page.evaluate(() => fixture.toolbar.interacting()), false);
        await page.mouse.up(); assert.deepEqual(await calls(page), []);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }],
    ['Enter, Space, right-click rename and F2 retain their local behavior', async page => {
        await row(page).focus(); await page.keyboard.press('Enter'); await page.keyboard.press('Space');
        assert.deepEqual(await calls(page), [2,2]);
        const p = await point(row(page).locator('.toolbar-label'));
        await page.mouse.click(p.x,p.y,{button:'right'});
        assert.equal(await page.locator('.toolbar-rename').isVisible(), true);
        await page.keyboard.press('Escape'); await row(page).focus(); await page.keyboard.press('F2');
        assert.equal(await page.locator('.toolbar-rename').isVisible(), true);
        assert.deepEqual(await calls(page), [2,2]);
        await page.keyboard.press('Escape'); await click(page); assert.deepEqual(await calls(page), [2,2,2]);
    }],
    ['touch tap activates once without initiating sorting', async (page, api) => {
        const p = await point(row(page).locator('.toolbar-label')); await page.touchscreen.tap(p.x,p.y);
        assert.deepEqual(await calls(page), [2]); assert.equal(api.orders.length, 0);
    }],
    ['shared, remote and local tabs switch focus while retaining their iframes', async page => {
        await click(page, 'shared');
        await page.waitForFunction(() => document.querySelector('.app-stream.active iframe')?.contentDocument?.documentElement.dataset.active === 'true');
        await page.evaluate(() => window.sharedFrame = document.querySelector('.app-stream.active iframe'));
        await click(page, 'remote');
        await page.waitForFunction(() => fixture.streams.activeId() === 'remote' && document.querySelector('.app-stream.active iframe')?.contentDocument?.documentElement.dataset.active === 'true');
        await page.evaluate(() => window.remoteFrame = document.querySelector('.app-stream.active iframe'));
        await press(page, 'shared');
        await page.evaluate(() => { fixture.data.ordered_ids.reverse(); fixture.data.order_revision++; fixture.render(); });
        await page.mouse.up();
        await page.waitForFunction(() => fixture.streams.activeId() === 'shared' && document.activeElement === sharedFrame);
        assert(await page.evaluate(() => document.querySelector('.app-stream.active iframe') === sharedFrame && remoteFrame.isConnected));
        await click(page);
        await page.waitForFunction(() => fixture.streams.activeId() === null && !document.querySelector('.app-stream.active'));
        assert.deepEqual(await calls(page), [2]);
        assert(await row(page).evaluate(n => n.classList.contains('focused')));
        assert(await page.evaluate(() => sharedFrame.isConnected && remoteFrame.isConnected));
    }],
    ['a non-sortable press released inside an iframe cannot hold the toolbar', async page => {
        await click(page,'shared');
        await page.waitForFunction(() => document.querySelector('.app-stream.active iframe')?.contentDocument?.documentElement.dataset.active === 'true');
        await press(page);
        await page.mouse.move(200,200); await page.mouse.up();
        assert.equal(await page.evaluate(() => fixture.toolbar.interacting()), false);
        assert.deepEqual(await calls(page), []);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }, '?reorder=0'],
    ['blur and destruction release pending input and deferred rendering', async page => {
        await press(page);
        await page.evaluate(() => { fixture.data.ordered_ids.reverse(); fixture.data.order_revision++; fixture.render(); window.dispatchEvent(new Event('blur')); });
        assert.equal(await page.evaluate(() => fixture.toolbar.interacting()), false);
        await page.mouse.up(); assert.deepEqual(await calls(page), []);
        await press(page);
        await page.evaluate(() => { fixture.render(); fixture.toolbar.destroy(); });
        await page.mouse.up();
        assert.equal(await page.locator('#taskbar').count(), 0);
        assert.deepEqual(await calls(page), []);
    }],
    ['a missing native click cannot leave deferred rendering stuck', async page => {
        await page.evaluate(() => document.addEventListener('click', e => e.stopImmediatePropagation(), {capture:true,once:true}));
        await press(page);
        await page.evaluate(() => { fixture.data.ordered_ids.reverse(); fixture.data.order_revision++; fixture.render(); });
        await page.mouse.up();
        await page.waitForFunction(() => !fixture.toolbar.interacting());
        assert.deepEqual((await ids(page)).slice(0,4), ['remote','shared','beta','alpha']);
        assert.deepEqual(await calls(page), []);
        await click(page); assert.deepEqual(await calls(page), [2]);
    }],
];

await test('toolbar native browser input', async t => {
    const work = mkdtempSync(path.join(tmpdir(), 'ivnc-toolbar-input-'));
    const server = await createServer({configFile:false,root,cacheDir:path.join(work,'vite'),logLevel:'error',
        optimizeDeps:{noDiscovery:true,include:[]},server:{host:'127.0.0.1',port:0,hmr:false},
        plugins:[{name:'toolbar-fixture',configureServer(s) {
            s.middlewares.use((req,res,next) => {
                const url = new URL(req.url,'http://localhost');
                const body = url.pathname === '/__toolbar-test' ? fixture : url.pathname.startsWith('/app-view/') ? embedded : null;
                if (body === null) return next();
                res.setHeader('Content-Type','text/html; charset=utf-8'); res.end(body);
            });
        }}]});
    try {
        await server.listen(); const base = `http://127.0.0.1:${server.httpServer.address().port}`;
        for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) await t.test(name, async t => {
            assert(['chromium','webkit'].includes(name), `unknown browser: ${name}`);
            const browser = await engines[name].launch({headless:true,
                ...(name === 'chromium' && process.env.CHROME_BIN ? {executablePath:process.env.CHROME_BIN} : {})});
            try {
                for (const [title,run,query=''] of cases) await t.test(title, async () => {
                    const context = await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:2,hasTouch:true});
                    const page = await context.newPage(), errors = [], api = {orders:[]};
                    page.on('pageerror', error => errors.push(error.message));
                    await page.route('**/api/**', async route => {
                        const request = route.request(), pathname = new URL(request.url()).pathname;
                        let body = {};
                        if (pathname === '/api/apps/order' && request.method() === 'PUT') {
                            const input = request.postDataJSON(); api.orders.push(input);
                            body = {ordered_ids:input.ordered_ids,order_revision:input.base_revision+1};
                        } else if (pathname.endsWith('/view')) body = {url:`/app-view/${pathname.split('/')[3]}/`};
                        await route.fulfill({json:body});
                    });
                    try {
                        await page.goto(`${base}/__toolbar-test${query}`);
                        await page.waitForFunction(() => !!window.fixture);
                        await run(page,api);
                        assert.deepEqual(errors, []);
                    } finally { await context.close(); }
                });
            } finally { await browser.close(); }
        });
    } finally { await server.close(); rmSync(work,{recursive:true,force:true}); }
});
