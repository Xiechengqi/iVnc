// Real Chromium gestures against an isolated iVnc instance. Includes the
// database/API, production toolbar, console notifications and real GTK peers.
// IVNC_BIN=/path/to/ivnc node scripts/toolbar_smoke.mjs
// KEEP_TMP=1 preserves logs and screenshots. Needs google-chrome, gtk3-demo, ws.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-toolbar-'));
const binary = process.env.IVNC_BIN || path.join(root, 'target/release/ivnc');
const freePort = async () => {
    const server = net.createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve));
    return port;
};
const port = await freePort(), debugPort = await freePort(), appDebugPort = await freePort();
const base = `http://127.0.0.1:${port}`;
const env = { ...process.env, IVNC_HOME: path.join(work, 'ivnc'),
    XDG_CONFIG_HOME: path.join(work, 'config'), XDG_CACHE_HOME: path.join(work, 'cache'), XDG_DATA_HOME: path.join(work, 'data'),
    XDG_RUNTIME_DIR: path.join(work, 'run'), IVNC_CHROME_DEBUG_PORT: String(appDebugPort) };
for (const dir of [env.IVNC_HOME, env.XDG_CONFIG_HOME, env.XDG_CACHE_HOME, env.XDG_DATA_HOME, env.XDG_RUNTIME_DIR]) mkdirSync(dir, { mode: 0o700 });
const launch = (command, args, logfile, extra = {}) => {
    const log = openSync(path.join(work, logfile), 'a', 0o600);
    const child = spawn(command, args, { stdio: ['ignore', log, log], ...extra });
    closeSync(log);
    return child;
};
let server, browser, socket;
let passed = 0;
const pass = name => { passed++; console.log(`PASS ${name}`); };
const waitFor = async (name, predicate, timeout = 20000) => {
    const until = Date.now() + timeout;
    let error;
    while (Date.now() < until) {
        try { const value = await predicate(); if (value) return value; } catch (e) { error = e; }
        await delay(80);
    }
    throw Error(`Timeout: ${name}${error ? ` (${error.message})` : ''}`);
};
const api = async (url, method = 'GET', body, expected = 200) => {
    const response = await fetch(base + url, { method, headers: { 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body) });
    const data = await response.json();
    assert.equal(response.status, expected, `${method} ${url}: ${JSON.stringify(data)}`);
    return data;
};
const pending = new Map(); let seq = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++seq;
    const timer = setTimeout(() => { pending.delete(id); reject(Error(`CDP timeout: ${method}`)); }, 20000);
    pending.set(id, { resolve, reject, timer }); socket.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression) => {
    const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
};
let mouseDown = false;
const mouse = (type, x, y) => {
    if (type === 'mousePressed') mouseDown = true;
    if (type === 'mouseReleased') mouseDown = false;
    return call('Input.dispatchMouseEvent', { type, x, y,
        button: type === 'mouseReleased' || mouseDown ? 'left' : 'none',
        buttons: mouseDown ? 1 : 0, clickCount: type === 'mouseMoved' ? 0 : 1 });
};
const key = async (name, code = name) => {
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: name, code });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code });
};
const firstPoint = () => evaluate(`(() => {
    const viewport = document.querySelector('.toolbar-viewport'); viewport.scrollLeft = 0;
    const n = document.querySelector('.toolbar-list .toolbar-grip'); const r = n.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
})()`);
const toolbarIds = () => evaluate(`[...new Set([...document.querySelectorAll('.toolbar-list .taskbar-item')].map(n => n.dataset.appId))]`);
const stop = async child => {
    if (!child || child.exitCode !== null || child.signalCode) return;
    child.kill('SIGTERM');
    for (let i = 0; i < 100 && child.exitCode === null && !child.signalCode; i++) await delay(100);
    if (child.exitCode === null && !child.signalCode) child.kill('SIGKILL');
};
try {
    server = launch(binary, ['--http-port', String(port), '--basic-auth-enabled', 'false', '--proxy-enabled', 'false',
        '--proxy-autostart', 'false', '--width', '800', '--height', '600'], 'server.log', { env, cwd: root });
    await waitFor('server ready', () => api('/api/apps'), 30000);
    const initial = await api('/api/apps/order');
    const ids = [];
    let hidden;
    for (let i = 0; i < 30; i++) {
        const result = await api('/api/apps', 'POST', { name: `Toolbar ${String(i).padStart(2, '0')} 中文 long application title`, app_type: 'desktop', exec_command: 'sleep 600' }, 201);
        ids.push(result.app.id);
        assert.equal(result.app.sort_order, initial.ordered_ids.length + i + (i > 2 ? 1 : 0));
        if (i === 2) hidden = (await api('/api/apps', 'POST', { name: 'Hidden CLI', app_type: 'cli', cli_binary_path: '/bin/true' }, 201)).app.id;
    }
    const before = await api('/api/apps/order');
    assert.equal(before.ordered_ids[initial.ordered_ids.length + 3], hidden);
    await api('/api/apps/order', 'PUT', { ordered_ids: [ids[0], ids[0]], base_revision: before.order_revision }, 400);
    await api('/api/apps/order', 'PUT', { ordered_ids: [ids[1], 'missing'], base_revision: before.order_revision }, 400);
    await api('/api/apps/order', 'PUT', { ordered_ids: ids, base_revision: -1 }, 400);
    await api('/api/apps/order', 'PUT', { ordered_ids: ids }, 400);
    assert.deepEqual(await api('/api/apps/order'), before);
    pass('API validates order atomically; new apps append');

    browser = launch(process.env.CHROME_BIN || 'google-chrome', ['--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu',
        '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--autoplay-policy=no-user-gesture-required',
        `--remote-debugging-port=${debugPort}`, `--user-data-dir=${path.join(work, 'browser')}`, '--window-size=1280,800', base], 'browser.log');
    const page = await waitFor('browser target', async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(t => t.type === 'page'));
    socket = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    socket.onmessage = ({ data }) => {
        const result = JSON.parse(data), item = pending.get(result.id);
        if (!item) return;
        pending.delete(result.id); clearTimeout(item.timer);
        if (result.error) item.reject(Error(result.error.message)); else item.resolve(result.result);
    };
    await call('Runtime.enable'); await call('Page.enable');
    await waitFor('production toolbar and data channel', () => evaluate(`!!document.querySelector('.ivnc-toolbar') && window.__ivncWebRTC?._send_channel?.readyState === 'open'`), 60000);
    const rows = (await api('/api/apps')).apps;
    const fake = { windows: rows.filter(a => ids.slice(0, 20).includes(a.id)).map((a, i) => ({ id: i, managed_app_id: a.id, title: a.name, app_id: 'test.class', focused: false })),
        apps: rows.filter(a => ids.slice(20).includes(a.id)).map((a, i) => ({ id: a.id, name: a.name, kind: i % 2 ? 'shared' : 'remote', status: i % 2 ? 'starting' : 'connecting' })), ...before };
    await evaluate(`(() => {
        window.__synthetic = ${JSON.stringify(fake)};
        const rtc = window.__ivncWebRTC, update = rtc.ontaskbarupdate;
        rtc.ontaskbarupdate = data => {
            window.__actual = data;
            if (!window.__synthetic) return update(data);
            if (!window.__freezeOrder && Array.isArray(data.ordered_ids)) {
                window.__synthetic.ordered_ids = data.ordered_ids; window.__synthetic.order_revision = data.order_revision;
            }
            update(window.__synthetic);
        };
        rtc.ontaskbarupdate(window.__synthetic);
        window.__messages = [];
        const send = rtc.sendDataChannelMessage.bind(rtc);
        rtc.sendDataChannelMessage = message => { window.__messages.push(message); return send(message); };
        window.__fetch = window.fetch;
        window.fetch = (url, options) => {
            if (url === '/api/apps/order' && options?.method === 'PUT') {
                window.__putCount = (window.__putCount || 0) + 1;
                if (window.__failOrder) return Promise.reject(Error('simulated offline'));
                if (window.__delayOrder) return new Promise(resolve => setTimeout(resolve, 300)).then(() => window.__fetch(url, options));
            }
            return window.__fetch(url, options);
        };
        return true;
    })()`);
    assert.deepEqual(await toolbarIds(), ids);
    pass('ordinary, shared and remote tabs share one application order');

    for (const count of [0, 1, 10, 30]) {
        await evaluate(`window.__synthetic = {...${JSON.stringify(fake)}, windows:${JSON.stringify(fake.windows)}.slice(0,Math.min(${count},20)), apps:${JSON.stringify(fake.apps)}.slice(0,Math.max(0,${count}-20))}; window.__ivncWebRTC.ontaskbarupdate(window.__synthetic)`);
        assert.equal(await evaluate(`document.querySelectorAll('.toolbar-list .taskbar-item').length`), count);
    }
    pass('zero, one, ten and thirty applications update without stale tabs');

    for (const width of [320, 375, 768, 1280, 1920]) {
        await call('Emulation.setDeviceMetricsOverride', { width, height: 800, deviceScaleFactor: width === 375 ? 2 : 1, mobile: false });
        await delay(150);
        for (const lang of ['zh', 'en']) {
            // Language is stored through the same storage event as another tab.
            await evaluate(`window.dispatchEvent(new StorageEvent('storage', {key:'ivnc.lang', newValue:${JSON.stringify(lang)}}))`);
            await delay(80);
            const bounds = await evaluate(`(() => {
                const r = s => { const b = document.querySelector(s).getBoundingClientRect(); return {left:b.left,right:b.right,width:b.width}; };
                return {tools:r('.toolbar-tools'),apps:r('.toolbar-apps'),status:r('.toolbar-status'),bar:r('.ivnc-toolbar'),width:innerWidth,scroll:document.documentElement.scrollWidth};
            })()`);
            assert(bounds.tools.right <= bounds.apps.left + 1, JSON.stringify(bounds));
            assert(bounds.apps.right <= bounds.status.left + 1, JSON.stringify(bounds));
            assert(bounds.status.right <= bounds.width + 1, JSON.stringify(bounds));
            assert(bounds.apps.width > 40 && bounds.scroll <= bounds.width + 1, JSON.stringify(bounds));
        }
    }
    pass('five widths, two languages and HiDPI: no overlap or page overflow');
    await call('Emulation.setDeviceMetricsOverride', { width: 320, height: 800, deviceScaleFactor: 1, mobile: false });
    await delay(100);
    await evaluate(`document.querySelector('.toolbar-more').click()`);
    assert(await evaluate(`!document.querySelector('.toolbar-more-menu').hidden && document.querySelector('.toolbar-more-menu #console-btn') !== null`));
    await evaluate(`document.querySelector('.toolbar-more').click(); document.querySelector('#network-quality').click()`);
    assert(await evaluate(`!document.querySelector('.toolbar-info').hidden && document.querySelector('.toolbar-info').textContent.length > 0`));
    await evaluate(`document.querySelector('#network-quality').click()`);
    await evaluate(`document.querySelector('#network-quality').focus()`); await key('Enter');
    assert(await evaluate(`!document.querySelector('.toolbar-info').hidden`));
    await key('Escape');
    assert(await evaluate(`document.querySelector('.toolbar-info').hidden`));
    await evaluate(`document.querySelector('.toolbar-more').click(); document.querySelector('#console-btn').click()`);
    assert(await evaluate(`document.querySelector('.toolbar-more-menu').hidden`));
    await waitFor('compact console tool', () => evaluate(`document.querySelector('.web-console-modal iframe')?.contentDocument?.querySelector('[data-section="apps"]')`));
    await evaluate(`document.querySelector('#web-console-minimize').click()`);
    pass('compact tools open and dismiss their menu; network details support click and keyboard');

    await call('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
    await delay(100);
    await evaluate(`document.querySelector('.toolbar-scroll:last-child').click()`);
    await waitFor('scroll button', () => evaluate(`document.querySelector('.toolbar-viewport').scrollLeft > 100`));
    const preserved = await evaluate(`(() => {
        const v = document.querySelector('.toolbar-viewport'); v.scrollLeft = 700;
        window.__node = document.querySelector('.toolbar-list .taskbar-item');
        const before = v.scrollLeft; window.__ivncWebRTC.ontaskbarupdate(window.__synthetic);
        return before === v.scrollLeft && window.__node === document.querySelector('.toolbar-list .taskbar-item');
    })()`);
    assert(preserved);
    pass('overflow scroll controls and state refresh preserve nodes and scroll position');

    // Keep the real console iframe alive, then verify the reorder reaches it
    // before its five-second periodic refresh.
    await evaluate(`document.querySelector('#console-btn').click()`);
    await waitFor('console iframe', () => evaluate(`document.querySelector('.web-console-modal iframe')?.contentDocument?.querySelector('[data-section="apps"]')`));
    await evaluate(`document.querySelector('.web-console-modal iframe').contentDocument.querySelector('[data-section="apps"]').click()`);
    await waitFor('console rows', () => evaluate(`document.querySelector('.web-console-modal iframe').contentDocument.querySelectorAll('#app-list tr[data-app-id]').length === ${rows.length}`));
    await evaluate(`document.querySelector('#web-console-minimize').click(); window.__messages = []`);
    const start = await firstPoint();
    const target = await evaluate(`(() => { const r = document.querySelectorAll('.toolbar-list .taskbar-item')[1].getBoundingClientRect(); return {x:r.right-5,y:r.top+14}; })()`);
    await evaluate(`window.__trace = []; ['pointerdown','pointermove','pointerup','pointercancel','lostpointercapture','blur'].forEach(type => window.addEventListener(type, event => window.__trace.push({type, target:event.target.className, x:event.clientX, y:event.clientY}), true))`);
    await mouse('mouseMoved', start.x, start.y); await mouse('mousePressed', start.x, start.y);
    await mouse('mouseMoved', start.x + 8, start.y);
    await mouse('mouseMoved', target.x, target.y);
    await mouse('mouseReleased', target.x, target.y);
    await waitFor('saved mouse reorder', async () => (await api('/api/apps/order')).ordered_ids.indexOf(ids[1]) < (await api('/api/apps/order')).ordered_ids.indexOf(ids[0]));
    await waitFor('immediate console reorder', () => evaluate(`(() => {const rows=[...document.querySelector('.web-console-modal iframe').contentDocument.querySelectorAll('#app-list tr')].map(n=>n.dataset.appId); return rows.indexOf(${JSON.stringify(ids[1])}) < rows.indexOf(${JSON.stringify(ids[0])});})()`), 2000);
    const afterDrag = await api('/api/apps/order');
    assert.equal(afterDrag.ordered_ids.indexOf(hidden), before.ordered_ids.indexOf(hidden));
    assert.equal(await evaluate(`window.__messages.filter(m => /^(focus|kd|ku|m|m2|cw),/.test(m)).length`), 0);
    pass('real mouse drag persists, preserves hidden apps, updates console immediately and sends no remote input');

    // Arrow key reorder must stay local to the toolbar, even with the main
    // desktop's keyboard capture listener installed on window.
    await waitFor('save complete', () => evaluate(`!document.querySelector('.toolbar-grip').disabled`));
    await evaluate(`document.querySelector('.toolbar-grip').focus(); window.__messages = []`);
    await key('ArrowRight');
    await waitFor('keyboard reorder', async () => (await api('/api/apps/order')).order_revision > afterDrag.order_revision);
    assert.equal(await evaluate(`window.__messages.filter(m => /^(kd|ku),/.test(m)).length`), 0);
    pass('keyboard reorder is accessible and does not reach the remote desktop');

    await waitFor('keyboard save complete', () => evaluate(`!document.querySelector('.toolbar-grip').disabled`));
    const beforeFailure = await api('/api/apps/order');
    await evaluate(`window.__failOrder = true; document.querySelector('.toolbar-grip').focus()`);
    await key('ArrowRight');
    await waitFor('failure notice', () => evaluate(`!document.querySelector('.toolbar-notice').hidden && !document.querySelector('.toolbar-grip').disabled`));
    assert.deepEqual(await api('/api/apps/order'), beforeFailure);
    assert.deepEqual(await toolbarIds(), beforeFailure.ordered_ids.filter(id => ids.includes(id)));
    await evaluate(`window.__failOrder = false; window.__freezeOrder = true`);
    const external = await api('/api/apps/order', 'PUT', { ordered_ids: ids.slice().reverse(), base_revision: beforeFailure.order_revision });
    await evaluate(`document.querySelector('.toolbar-grip').focus()`); await key('ArrowRight');
    await waitFor('conflict recovery', () => evaluate(`!document.querySelector('.toolbar-grip').disabled && document.querySelector('.toolbar-notice').textContent.includes('changed')`));
    assert.deepEqual(await api('/api/apps/order'), external);
    assert.deepEqual(await toolbarIds(), external.ordered_ids.filter(id => ids.includes(id)));
    await evaluate(`window.__freezeOrder = false`);
    pass('failed saves roll back; stale versions recover without overwriting another client');

    // Holding near the right edge scrolls the application viewport while the
    // status area stays fixed. Escape cancels the uncommitted order.
    const edgeStart = await firstPoint();
    const edge = await evaluate(`(() => {const r=document.querySelector('.toolbar-viewport').getBoundingClientRect();return {x:r.right-2,y:r.top+16};})()`);
    const beforeCancel = await api('/api/apps/order');
    await mouse('mouseMoved', edgeStart.x, edgeStart.y); await mouse('mousePressed', edgeStart.x, edgeStart.y);
    await mouse('mouseMoved', edge.x, edge.y); await delay(700);
    assert(await evaluate(`document.querySelector('.toolbar-viewport').scrollLeft > 150`));
    await key('Escape'); await mouse('mouseReleased', edge.x, edge.y);
    assert.deepEqual(await api('/api/apps/order'), beforeCancel);
    assert.equal(await evaluate(`document.querySelectorAll('.toolbar-drag-ghost').length`), 0);
    pass('drag edge autoscroll and Escape cancellation');

    const disappearStart = await firstPoint();
    const beforeDisappear = await api('/api/apps/order');
    await mouse('mouseMoved', disappearStart.x, disappearStart.y); await mouse('mousePressed', disappearStart.x, disappearStart.y);
    await mouse('mouseMoved', disappearStart.x + 8, disappearStart.y);
    assert.equal(await evaluate(`document.querySelectorAll('.toolbar-drag-ghost').length`), 1);
    await evaluate(`(() => {
        window.__savedSynthetic = window.__synthetic;
        const id = document.querySelector('.toolbar-list .dragging').dataset.appId;
        window.__synthetic = {...window.__synthetic, windows:window.__synthetic.windows.filter(w=>w.managed_app_id!==id), apps:window.__synthetic.apps.filter(a=>a.id!==id)};
        window.__ivncWebRTC.ontaskbarupdate(window.__synthetic);
    })()`);
    await mouse('mouseReleased', disappearStart.x + 8, disappearStart.y);
    assert.equal(await evaluate(`document.querySelectorAll('.toolbar-drag-ghost').length`), 0);
    assert.deepEqual(await api('/api/apps/order'), beforeDisappear);
    await evaluate(`window.__synthetic = window.__savedSynthetic; window.__ivncWebRTC.ontaskbarupdate(window.__synthetic)`);
    pass('an application disappearing during drag cancels without saving');

    const beforeBusy = await api('/api/apps/order');
    await evaluate(`window.__delayOrder = true; window.__putCount = 0; document.querySelector('.toolbar-grip').focus()`);
    await key('ArrowRight'); await key('ArrowRight');
    await waitFor('serialized save', () => evaluate(`!document.querySelector('.toolbar-grip').disabled`));
    assert.equal(await evaluate(`window.__putCount`), 1);
    assert.equal((await api('/api/apps/order')).order_revision, beforeBusy.order_revision + 1);
    await evaluate(`window.__delayOrder = false`);
    pass('rapid repeated gestures issue one in-flight save');

    await call('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 });
    const touchStart = await firstPoint();
    const touchTarget = await evaluate(`(() => {const r=document.querySelectorAll('.toolbar-list .taskbar-item')[1].getBoundingClientRect();return {x:r.right-6,y:r.top+14};})()`);
    const beforeTouch = await api('/api/apps/order');
    const touch = (type, point) => call('Input.dispatchTouchEvent', { type, touchPoints: point ? [{...point,id:1,radiusX:1,radiusY:1,force:1}] : [] });
    await touch('touchStart', touchStart);
    await touch('touchMove', {x:touchStart.x+8,y:touchStart.y});
    await touch('touchMove', touchTarget);
    await touch('touchEnd');
    await waitFor('touch reorder save', async () => (await api('/api/apps/order')).order_revision > beforeTouch.order_revision);
    await waitFor('touch save complete', () => evaluate(`!document.querySelector('.toolbar-grip').disabled`));
    pass('real touch pointer drag through the handle saves order');
    const afterTouch = await api('/api/apps/order');
    const swipeStart = await evaluate(`(() => {
        document.querySelector('.toolbar-viewport').scrollLeft = 0;
        const r=document.querySelectorAll('.toolbar-label')[1].getBoundingClientRect();
        return {x:r.left+Math.min(r.width/2,30),y:r.top+r.height/2};
    })()`);
    await touch('touchStart', swipeStart);
    for (const offset of [20, 60, 100, 150]) {
        await touch('touchMove', {x:swipeStart.x-offset,y:swipeStart.y}); await delay(30);
    }
    await touch('touchEnd');
    await waitFor('native touch scrolling', () => evaluate(`document.querySelector('.toolbar-viewport').scrollLeft > 30`));
    assert.deepEqual(await api('/api/apps/order'), afterTouch);
    await call('Emulation.setTouchEmulationEnabled', { enabled: false });
    pass('touch swipes on labels scroll naturally without changing order');

    const shot = await call('Page.captureScreenshot', { format: 'png' });
    writeFileSync(path.join(work, 'toolbar.png'), Buffer.from(shot.data, 'base64'));
    // Restore actual broadcasts and exercise PID -> application identity with
    // two different managed GTK processes, rather than desktop class guessing.
    await evaluate(`window.__synthetic = null; window.__ivncWebRTC.ontaskbarupdate(window.__actual)`);
    const real = [];
    for (const label of ['Real GTK A', 'Real GTK B']) {
        const app = (await api('/api/apps', 'POST', { name: label, app_type: 'desktop', exec_command: label.endsWith('A') ? 'gtk3-demo & gtk3-demo & wait' : 'gtk3-demo' }, 201)).app;
        real.push(app.id); await api(`/api/apps/${app.id}/start`, 'POST');
    }
    await waitFor('real peer identities', () => evaluate(`(() => { const ids=[...document.querySelectorAll('.toolbar-list .taskbar-item')].map(n=>n.dataset.appId);return ${JSON.stringify(real)}.every(id=>ids.includes(id));})()`), 30000);
    const realOrder = await api('/api/apps/order');
    await api('/api/apps/order', 'PUT', { ordered_ids: real.slice().reverse(), base_revision: realOrder.order_revision });
    await waitFor('real peer reorder broadcast', () => evaluate(`(() => { const ids=[...document.querySelectorAll('.toolbar-list .taskbar-item')].map(n=>n.dataset.appId);return ids.indexOf(${JSON.stringify(real[1])})<ids.indexOf(${JSON.stringify(real[0])});})()`));
    assert.equal(await evaluate(`document.querySelectorAll('.toolbar-list [data-app-id="${real[0]}"]').length`), 2);
    pass('real GTK peer identity, multi-window grouping and order broadcast');
    await api(`/api/apps/${real[1]}/sharing`, 'PUT', { enabled: true, password: 'toolbar-smoke-password' });
    await api(`/api/apps/${real[1]}/restart`, 'POST');
    await waitFor('shared stream tab', () => evaluate(`!!document.querySelector('.toolbar-list [data-key="app:${real[1]}"]')`), 30000);
    await evaluate(`document.querySelector('.toolbar-list [data-key="app:${real[1]}"]').click()`);
    await waitFor('shared iframe connected', () => evaluate(`document.querySelector('.app-stream iframe')?.contentWindow?.__ivncWebRTC?._send_channel?.readyState === 'open'`), 60000);
    await evaluate(`window.__frame = document.querySelector('.app-stream iframe'); window.__frameUrl = window.__frame.src`);
    const liveOrder = await api('/api/apps/order');
    await api('/api/apps/order', 'PUT', { ordered_ids: real, base_revision: liveOrder.order_revision });
    await waitFor('shared reorder', () => evaluate(`(() => { const ids=[...document.querySelectorAll('.toolbar-list .taskbar-item')].map(n=>n.dataset.appId);return ids.indexOf(${JSON.stringify(real[0])})<ids.indexOf(${JSON.stringify(real[1])});})()`));
    assert(await evaluate(`window.__frame === document.querySelector('.app-stream iframe') && window.__frameUrl === window.__frame.src && window.__frame.contentWindow.__ivncWebRTC._send_channel.readyState === 'open' && document.querySelector('.toolbar-list [data-key="app:${real[1]}"]').classList.contains('focused')`));
    pass('sorting a live shared app keeps the iframe, connection and active selection');
    for (const id of real) await api(`/api/apps/${id}/stop`, 'POST');

    const final = await api('/api/apps/order');
    await stop(server);
    server = launch(binary, ['--http-port', String(port), '--basic-auth-enabled', 'false', '--proxy-enabled', 'false', '--proxy-autostart', 'false', '--width', '800', '--height', '600'], 'server.log', { env, cwd: root });
    await waitFor('restart preserves order', async () => { const order=await api('/api/apps/order'); assert.deepEqual(order, final); return true; }, 30000);
    pass('service restart preserves order and revision');
    console.log(`RESULT ${passed} passed; artifacts: ${work}`);
} catch (error) {
    if (socket?.readyState === WebSocket.OPEN) {
        console.log('DEBUG', JSON.stringify(await evaluate(`({trace:window.__trace, messages:window.__messages?.filter(m=>/^(focus|kd|ku|m),/.test(m)), ids:[...document.querySelectorAll('.toolbar-list .taskbar-item')].map(n=>n.dataset.appId), notice:document.querySelector('.toolbar-notice')?.textContent})`).catch(() => null)));
        const shot=await call('Page.captureScreenshot',{format:'png'}).catch(()=>null);
        if(shot)writeFileSync(path.join(work,'failure.png'),Buffer.from(shot.data,'base64'));
    }
    throw error;
} finally {
    socket?.close();
    for (const item of pending.values()) { clearTimeout(item.timer); item.reject(Error('test ended')); }
    pending.clear();
    await stop(browser); await stop(server);
    if (process.env.KEEP_TMP !== '1') rmSync(work, { recursive: true, force: true });
    else console.log(`Artifacts retained: ${work}`);
}
