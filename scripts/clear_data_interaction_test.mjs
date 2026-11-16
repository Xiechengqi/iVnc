// Selective Chrome clear-data dialog in the production console (Chromium/WebKit).
// API calls are intercepted; no real application data is touched.
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
const CATEGORIES = ['cache', 'cookies', 'history', 'passwords', 'bookmarks', 'extensions', 'site_storage'];
const DEFAULT_UNCHECKED = ['extensions', 'site_storage'];
const appsPayload = {
    apps: [
        {
            id: 'builtin-chrome', name: 'Chrome', app_type: 'desktop', status: 'stopped',
            enabled: true, builtin: true, data_size_human: '12.3 MB',
            clear_categories: CATEGORIES, can_clear_data: true, can_delete: false, name_revision: 1,
        },
        {
            id: 'other-app', name: 'Other', app_type: 'background', status: 'stopped',
            enabled: true, data_size_human: '1 MB', can_clear_data: true, can_delete: true,
            name_revision: 1, url: '',
        },
    ],
    order_revision: 1,
};
const closed = page => page.waitForFunction(() => !document.querySelector('#clear-modal').classList.contains('show'));
const locked = page => page.waitForFunction(() => window.__ivncConsoleDialogState.locked);
const unlocked = page => page.waitForFunction(() => !window.__ivncConsoleDialogState.locked);
const checked = (page, id) => page.locator(`#clear-categories input[value="${id}"]`).isChecked();

const cases = [
    ['Chrome row opens the selective dialog with extensions and site storage unchecked by default', async ({ page, errors }) => {
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        assert.equal(await page.locator('#clear-categories .clear-category').count(), CATEGORIES.length);
        for (const id of CATEGORIES) assert.equal(await page.locator(`#clear-categories input[value="${id}"]`).count(), 1, id);
        for (const id of CATEGORIES.filter(id => !DEFAULT_UNCHECKED.includes(id))) assert.equal(await checked(page, id), true, id);
        for (const id of DEFAULT_UNCHECKED) assert.equal(await checked(page, id), false, id);
        assert.equal(await page.locator('#clear-confirm-btn').isDisabled(), false);
        assert.match(await page.locator('#clear-title').textContent(), /Chrome/);
        assert.deepEqual(errors, []);
    }],
    ['unchecking every category disables confirm and rechecking restores it', async ({ page }) => {
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        for (const id of CATEGORIES) if (await checked(page, id)) await page.locator(`#clear-categories input[value="${id}"]`).uncheck();
        assert.equal(await page.locator('#clear-confirm-btn').isDisabled(), true);
        await page.locator('#clear-categories input[value="cache"]').check();
        assert.equal(await page.locator('#clear-confirm-btn').isDisabled(), false);
    }],
    ['cancel, close, backdrop and Escape dismiss only the dialog without a request', async ({ page, calls }) => {
        for (const method of ['cancel', 'close', 'backdrop', 'escape']) {
            await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
            await page.locator('#clear-modal.show').waitFor();
            if (method === 'cancel') await page.locator('#clear-cancel-btn').click();
            else if (method === 'close') await page.locator('#clear-close').click();
            else if (method === 'backdrop') await page.locator('#clear-modal').click({ position: { x: 2, y: 2 } });
            else await page.keyboard.press('Escape');
            await closed(page);
            assert.equal(calls.filter(c => c.path.endsWith('/clear-data')).length, 0);
        }
    }],
    ['confirm sends only checked categories, locks while pending and reports freed size', async ({ page, calls }) => {
        let release;
        const gate = new Promise(resolve => { release = resolve; });
        await page.route('**/api/apps/builtin-chrome/clear-data', async route => {
            await gate;
            await route.fulfill({ status: 200, json: { ok: true, categories: ['cache'], full_reset: false, freed_bytes: 1234, freed_human: '1.2 KB' } });
        });
        try {
            await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
            await page.locator('#clear-modal.show').waitFor();
            for (const id of CATEGORIES.filter(id => id !== 'cache')) if (await checked(page, id)) await page.locator(`#clear-categories input[value="${id}"]`).uncheck();
            await page.locator('#clear-confirm-btn').click();
            await locked(page);
            assert.equal(await page.locator('#clear-confirm-btn').isDisabled(), true);
            assert.equal(await page.locator('#clear-reset-btn').isDisabled(), true);
            await page.locator('#clear-confirm-btn').click({ force: true });
            await page.keyboard.press('Escape');
            assert(await page.locator('#clear-modal').evaluate(el => el.classList.contains('show')));
            assert.equal(calls.filter(c => c.path.endsWith('/clear-data')).length, 1);
            const request = calls.find(c => c.path.endsWith('/clear-data'));
            assert.equal(request.method, 'POST');
            assert.deepEqual(request.body, { categories: ['cache'] });
            release(); await unlocked(page);
            await closed(page);
            assert.match(await page.locator('#toast').textContent(), /1\.2 KB/);
        } finally { release(); }
    }],
    ['default selection never submits extensions or site storage', async ({ page, calls }) => {
        await page.route('**/api/apps/builtin-chrome/clear-data', route => route.fulfill({ status: 200, json: { ok: true, freed_human: '2 KB' } }));
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        await page.locator('#clear-confirm-btn').click();
        await closed(page);
        const request = calls.find(c => c.path.endsWith('/clear-data'));
        assert.deepEqual(request.body.categories.sort(), ['bookmarks', 'cache', 'cookies', 'history', 'passwords']);
        assert(!request.body.categories.includes('extensions'));
        assert(!request.body.categories.includes('site_storage'));
    }],
    ['site storage can be opted into explicitly', async ({ page, calls }) => {
        await page.route('**/api/apps/builtin-chrome/clear-data', route => route.fulfill({ status: 200, json: { ok: true, freed_human: '2 KB' } }));
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        await page.locator('#clear-categories input[value="site_storage"]').check();
        await page.locator('#clear-confirm-btn').click();
        await closed(page);
        const request = calls.find(c => c.path.endsWith('/clear-data'));
        assert(request.body.categories.includes('site_storage'));
    }],
    ['full reset needs a second click and sends the full_reset flag', async ({ page, calls }) => {
        await page.route('**/api/apps/builtin-chrome/clear-data', route => route.fulfill({ status: 200, json: { ok: true, categories: [], full_reset: true, freed_bytes: 2048, freed_human: '2.0 KB' } }));
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        const reset = page.locator('#clear-reset-btn');
        await reset.click();
        assert.match(await reset.textContent(), /again|再次/i);
        assert.equal(calls.filter(c => c.path.endsWith('/clear-data')).length, 0);
        await reset.click();
        await closed(page);
        const request = calls.find(c => c.path.endsWith('/clear-data'));
        assert.deepEqual(request.body, { full_reset: true });
    }],
    ['a failed request keeps the dialog open, unlocks and allows retry', async ({ page, calls }) => {
        let mode = 'fail';
        await page.route('**/api/apps/builtin-chrome/clear-data', route => mode === 'fail'
            ? route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'boom' }) })
            : route.fulfill({ status: 200, json: { ok: true, freed_human: '3 KB' } }));
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        await page.locator('#clear-confirm-btn').click();
        await unlocked(page);
        assert(await page.locator('#clear-modal').evaluate(el => el.classList.contains('show')));
        assert.equal(await page.locator('#clear-confirm-btn').isDisabled(), false);
        assert.equal(await page.locator('#clear-reset-btn').isDisabled(), false);
        assert.equal(calls.filter(c => c.path.endsWith('/clear-data')).length, 1);
        mode = 'ok';
        await page.locator('#clear-confirm-btn').click();
        await closed(page);
        assert.equal(calls.filter(c => c.path.endsWith('/clear-data')).length, 2);
    }],
    ['server error text is rendered as text, not markup', async ({ page }) => {
        await page.route('**/api/apps/builtin-chrome/clear-data', route => route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: '<img src=x onerror=alert(1)>' }) }));
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        await page.locator('#clear-confirm-btn').click();
        await unlocked(page);
        assert.equal(await page.locator('#toast img').count(), 0);
    }],
    ['non-Chrome apps keep the original confirm flow and post no body', async ({ page }) => {
        page.once('dialog', dialog => dialog.accept());
        const nextRequest = page.waitForRequest(request => new URL(request.url()).pathname === '/api/apps/other-app/clear-data');
        await page.locator('tr[data-app-id="other-app"] .btn-clear').click();
        const request = await nextRequest;
        assert.equal(request.method(), 'POST');
        assert.equal(request.postData(), null);
    }],
    ['category labels follow the console language toggle', async ({ page }) => {
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        const before = await page.locator('#clear-categories .clear-category-name').first().textContent();
        await page.locator('#clear-cancel-btn').click();
        await closed(page);
        await page.locator('#lang-toggle').click();
        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').click();
        await page.locator('#clear-modal.show').waitFor();
        const after = await page.locator('#clear-categories .clear-category-name').first().textContent();
        assert.notEqual(before, after);
    }],
];

await test('clear data interaction', async t => {
    const work = mkdtempSync(path.join(tmpdir(), 'ivnc-clear-data-test-'));
    const server = await createServer({
        configFile: false, root, cacheDir: path.join(work, 'vite'), logLevel: 'error',
        optimizeDeps: { noDiscovery: true, include: [] },
        server: { host: '127.0.0.1', port: 0, hmr: false },
        plugins: [{ name: 'clear-data-fixture', configureServer(s) {
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
                    const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, locale: 'zh-CN' });
                    const page = await context.newPage(), errors = [], calls = [];
                    context.setDefaultTimeout(7000);
                    page.on('pageerror', error => errors.push(error.message));
                    page.on('request', request => {
                        const u = new URL(request.url());
                        if (u.pathname.endsWith('/clear-data')) calls.push({ path: u.pathname, method: request.method(), body: request.postDataJSON() });
                    });
                    await page.route('**/api/**', route => {
                        const u = new URL(route.request().url());
                        if (u.pathname === '/api/apps') return route.fulfill({ json: appsPayload });
                        if (u.pathname === '/api/capabilities') return route.fulfill({ json: { apps: [] } });
                        return route.fulfill({ json: {} });
                    });
                    await page.route('**/health', route => route.fulfill({ json: { ok: true } }));
                    try {
                        await page.goto(url);
                        await page.locator('#nav-apps').click();
                        await page.locator('tr[data-app-id="builtin-chrome"] .btn-clear').waitFor();
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
