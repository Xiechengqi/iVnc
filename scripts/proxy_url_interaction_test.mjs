// Built-in Chrome proxy URL field in the production console (Chromium/WebKit).
// API calls are intercepted; no real application is modified or restarted.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const { createServer } = require('vite'), engines = require('playwright');
const root = fileURLToPath(new URL('../web/ivnc/', import.meta.url));
const chrome = {
    id: 'builtin-chrome', name: 'Chrome', app_type: 'desktop', status: 'stopped',
    enabled: true, builtin: true, data_size_human: '12 MB', proxy_configurable: true,
    proxy_url: null, exec_command: 'google-chrome', env_vars: {}, desktop_launch: {},
    can_clear_data: true, can_delete: false, name_revision: 0,
};
const other = {
    id: 'other', name: 'Other', app_type: 'desktop', status: 'stopped', enabled: true,
    data_size_human: '1 MB', exec_command: 'other-browser', env_vars: {}, desktop_launch: {},
    can_clear_data: true, can_delete: true, name_revision: 0,
};
let state;
const appsPayload = () => ({ apps: [chrome, other], order_revision: 1 });

const cases = [
    ['Chrome edit dialog exposes an empty proxy field', async ({ page }) => {
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-edit').click();
        await page.locator('#modal.show').waitFor();
        assert.equal(await page.locator('#proxy-url-line').isVisible(), true);
        assert.equal(await page.locator('#f-proxy-url').inputValue(), '');
        assert.match(await page.locator('#label-proxy-url').textContent(), /代理|Proxy/);
    }],
    ['non-configurable desktop apps hide the proxy field', async ({ page }) => {
        await page.locator('tr[data-app-id="other"] .btn-edit').click();
        await page.locator('#modal.show').waitFor();
        assert.equal(await page.locator('#proxy-url-line').isVisible(), false);
    }],
    ['saving a proxy URL submits it and regenerates through PUT', async ({ page, calls }) => {
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-edit').click();
        await page.locator('#modal.show').waitFor();
        await page.locator('#f-proxy-url').fill('socks5://127.0.0.1:1080');
        await page.locator('#modal-save').click();
        await page.waitForFunction(() => !document.querySelector('#modal').classList.contains('show'));
        const put = calls.find(c => c.method === 'PUT' && c.path === '/api/apps/builtin-chrome');
        assert(put, 'a PUT was sent');
        assert.equal(put.body.proxy_url, 'socks5://127.0.0.1:1080');
    }],
    ['clearing the proxy URL submits null', async ({ page, calls }) => {
        state.proxy = 'socks5://127.0.0.1:1080';
        await page.reload();
        await page.locator('#nav-apps').click();
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-edit').click();
        await page.locator('#modal.show').waitFor();
        assert.equal(await page.locator('#f-proxy-url').inputValue(), 'socks5://127.0.0.1:1080');
        await page.locator('#f-proxy-url').fill('');
        await page.locator('#modal-save').click();
        await page.waitForFunction(() => !document.querySelector('#modal').classList.contains('show'));
        const put = calls.find(c => c.method === 'PUT' && c.path === '/api/apps/builtin-chrome');
        assert(put, 'a PUT was sent');
        assert.equal(put.body.proxy_url, null);
    }],
    ['an invalid proxy URL is rejected without a request', async ({ page, calls }) => {
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-edit').click();
        await page.locator('#modal.show').waitFor();
        await page.locator('#f-proxy-url').fill('not a url');
        await page.locator('#modal-save').click();
        await page.waitForFunction(() => document.querySelector('#err-f-proxy-url').textContent.length > 0);
        assert.equal(calls.filter(c => c.method === 'PUT').length, 0);
        assert.equal(await page.locator('#modal.show').count(), 1);
    }],
    ['a configured proxy from the server is shown in the field', async ({ page }) => {
        state.proxy = 'socks5://proxy.example:1080';
        await page.reload();
        await page.locator('#nav-apps').click();
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-edit').click();
        await page.locator('#modal.show').waitFor();
        assert.equal(await page.locator('#f-proxy-url').inputValue(), 'socks5://proxy.example:1080');
    }],
];

await test('proxy url interaction', async t => {
    const work = mkdtempSync(path.join(tmpdir(), 'ivnc-proxy-url-test-'));
    const server = await createServer({
        configFile: false, root, cacheDir: path.join(work, 'vite'), logLevel: 'error',
        optimizeDeps: { noDiscovery: true, include: [] },
        server: { host: '127.0.0.1', port: 0, hmr: false },
        plugins: [{ name: 'proxy-url-fixture', configureServer(s) {
            s.middlewares.use((req, res, next) => {
                if (req.url.split('?')[0] === '/console') {
                    res.setHeader('Content-Type', 'text/html; charset=utf-8');
                    return res.end(readFileSync(path.join(root, 'console.html'), 'utf8'));
                }
                next();
            });
        } }],
    });
    try {
        await server.listen();
        const url = `http://127.0.0.1:${server.httpServer.address().port}/console`;
        for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) await t.test(name, async t => {
            const browser = await engines[name].launch({ headless: true, ...(name === 'chromium' && process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
            try {
                for (const [title, run] of cases) await t.test(title, async () => {
                    state = { proxy: null };
                    const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, locale: 'zh-CN' });
                    const page = await context.newPage(), errors = [], calls = [];
                    context.setDefaultTimeout(7000);
                    page.on('pageerror', error => errors.push(error.message));
                    page.on('request', request => {
                        const u = new URL(request.url());
                        if (u.pathname === '/api/apps/builtin-chrome' && !['GET'].includes(request.method()))
                            calls.push({ path: u.pathname, method: request.method(), body: request.postDataJSON() });
                    });
                    await page.route('**/api/**', route => {
                        const u = new URL(route.request().url());
                        if (u.pathname === '/api/apps') return route.fulfill({ json: appsPayload() });
                        if (u.pathname === '/api/apps/builtin-chrome') {
                            const app = { ...chrome, proxy_url: state.proxy };
                            return route.fulfill({ json: app });
                        }
                        if (u.pathname === '/api/capabilities') return route.fulfill({ json: { apps: [] } });
                        if (route.request().method() === 'PUT') return route.fulfill({ json: { ok: true, app: chrome } });
                        return route.fulfill({ json: {} });
                    });
                    await page.route('**/health', route => route.fulfill({ json: { ok: true } }));
                    try {
                        await page.goto(url);
                        await page.locator('#nav-apps').click();
                        await page.locator('tr[data-app-id="builtin-chrome"] .btn-edit').waitFor();
                        await run({ page, calls, errors });
                        assert.deepEqual(errors, []);
                    } catch (error) {
                        error.message += `\nBrowser errors: ${JSON.stringify(errors)}\nRequests: ${JSON.stringify(calls)}`;
                        throw error;
                    } finally { await context.close(); }
                });
            } finally { await browser.close(); }
        });
    } finally { await server.close(); rmSync(work, { recursive: true, force: true }); }
});
