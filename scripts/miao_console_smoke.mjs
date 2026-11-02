// Run through miao_smoke.py --browser after npm ci in web/ivnc.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { openSync, closeSync, readFileSync } from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';

const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');

const [origin, root] = process.argv.slice(2);
const server = net.createServer();
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
await new Promise(resolve => server.close(resolve));
const log = openSync(path.join(root, 'browser.log'), 'a', 0o600);
const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu',
    '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
    '--password-store=basic', '--use-mock-keychain',
    '--ignore-certificate-errors', `--remote-debugging-port=${port}`,
    `--user-data-dir=${path.join(root, 'browser')}`, 'about:blank'
], { stdio: ['ignore', log, log] });
closeSync(log);
let socket;
const pending = new Map();
let id = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => {
    const key = ++id;
    const timer = setTimeout(() => { pending.delete(key); reject(Error(`CDP timeout: ${method}`)); }, 15000);
    pending.set(key, { resolve, reject, timer });
    socket.send(JSON.stringify({ id: key, method, params }));
});

try {
    let tabs;
    for (let attempt = 0; attempt < 100; attempt++) {
        try { tabs = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); break; }
        catch { await delay(100); }
    }
    assert(tabs?.[0]?.webSocketDebuggerUrl, 'isolated Chrome did not start');
    socket = new WebSocket(tabs[0].webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    socket.onmessage = event => {
        const result = JSON.parse(event.data);
        if (process.env.MIAO_SMOKE_DEBUG && ['Network.loadingFailed', 'Network.responseReceived', 'Fetch.authRequired'].includes(result.method)) {
            console.error(result.method, JSON.stringify(result.params));
        }
        const item = pending.get(result.id);
        if (!item) return;
        pending.delete(result.id);
        clearTimeout(item.timer);
        if (result.error) item.reject(Error(result.error.message));
        else item.resolve(result.result);
    };
    socket.onclose = () => {
        for (const item of pending.values()) {
            clearTimeout(item.timer);
            item.reject(Error('CDP socket closed'));
        }
        pending.clear();
    };
    await call('Page.enable');
    await call('Network.enable');
    await call('Network.setExtraHTTPHeaders', { headers: { Authorization: process.env.MIAO_SMOKE_AUTH } });
    await delay(1000); // The initial about:blank navigation can race the first CDP navigation.
    let navigation;
    for (let attempt = 0; attempt < 3; attempt++) {
        navigation = await call('Page.navigate', { url: `${origin}/console` });
        if (!navigation.errorText) break;
        await delay(500);
    }
    if (navigation.errorText) {
        console.error(readFileSync(path.join(root, 'browser.log'), 'utf8').slice(-3000));
        throw Error(JSON.stringify(navigation));
    }
    // Return a concrete assertion result from the actual shipped console and embedded panel.
    await delay(500);
    const result = await call('Runtime.evaluate', { awaitPromise: true, returnByValue: true,
        expression: `(async () => {
            const wait = async predicate => {
                for (let i = 0; i < 100; i++) {
                    if (predicate()) return;
                    await new Promise(resolve => setTimeout(resolve, 100));
                }
                throw Error('UI condition timed out');
            };
            await wait(() => document.querySelector('.nav-item[data-section="apps"]'));
            document.querySelector('.nav-item[data-section="apps"]').click();
            await wait(() => Array.from(document.querySelectorAll('#app-list tr')).some(row => row.querySelector('strong')?.textContent === 'Miao'));
            const row = Array.from(document.querySelectorAll('#app-list tr')).find(row => row.querySelector('strong')?.textContent === 'Miao');
            if (row.querySelector('.btn-clear,.btn-delete')) throw Error('protected actions are exposed');
            if (!row.querySelector('.btn-stop')) throw Error('Miao is not displayed as running');
            const edit = row.querySelector('.btn-edit');
            if (edit.disabled) throw Error('cannot edit startup preference while running');
            edit.click();
            await wait(() => document.querySelector('#modal').classList.contains('show'));
            if (document.querySelector('#miao-config').hidden || getComputedStyle(document.querySelector('#background-config')).display !== 'none') throw Error('wrong managed settings form');
            if (!document.querySelector('#f-autostart').checked) throw Error('autostart default is missing');
            document.querySelector('#f-autostart').checked = false;
            document.querySelector('#modal-save').click();
            await wait(() => !document.querySelector('#modal').classList.contains('show'));
            const saved = await (await fetch('/api/apps/builtin-miao')).json();
            if (saved.autostart || saved.status !== 'running') throw Error('startup preference was not saved independently of running state');
            const frame = document.createElement('iframe');
            frame.src = '/proxy/';
            document.body.appendChild(frame);
            await wait(() => frame.contentDocument?.querySelector('.topbar'));
            if (frame.contentDocument.querySelector('.auth-page')) throw Error('passwordless panel is showing a login form');
            const actions = Array.from(frame.contentDocument.querySelectorAll('.topbar button'));
            if (actions.some(button => /修改密码|退出登录|Change password|Sign out/.test(button.getAttribute('aria-label') || ''))) throw Error('password actions are exposed in passwordless mode');
            if (!frame.contentWindow.__ivncProxyPanelRewrite) throw Error('panel path adapter did not run');
            if (frame.contentWindow.WebSocket.OPEN !== 1 || frame.contentWindow.WebSocket.CONNECTING !== 0) throw Error('WebSocket constants were lost');
            const auth = await (await frame.contentWindow.fetch('/api/auth/status')).json();
            if (!auth.data.authenticated || auth.data.password_required) throw Error('embedded panel did not create a passwordless session');
            return { runningEdit: true, savedPreference: true, protectedActions: true, iframe: true, panelAuth: true };
        })()` });
    if (result.exceptionDetails) {
        const page = await call('Runtime.evaluate', { expression: 'location.href + "\\n" + document.body?.innerText?.slice(0, 400)', returnByValue: true });
        throw Error(JSON.stringify({ exception: result.exceptionDetails, page: page.result?.value }));
    }
    assert.deepEqual(result.result.value, { runningEdit: true, savedPreference: true, protectedActions: true, iframe: true, panelAuth: true });
    console.log('PASS Chrome UI: running Miao settings, startup preference, protected actions and authenticated iframe');
} finally {
    if (socket?.readyState === 1) {
        try { await call('Browser.close'); } catch { /* Chrome may close before acknowledging. */ }
        socket.close();
    }
    if (browser.exitCode === null) {
        await Promise.race([new Promise(resolve => browser.once('exit', resolve)), delay(5000)]);
        if (browser.exitCode === null) browser.kill('SIGKILL');
    }
    for (const entry of pending.values()) clearTimeout(entry.timer);
}
