// Local Chromium/WebKit regression against the production console and API-shaped
// fixtures. Faults deliberately occur before and after persistence.
import test from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const { createServer } = require('vite'), engines = require('playwright');
const root = fileURLToPath(new URL('../web/ivnc/', import.meta.url));
const writes = calls => calls.filter(c => c.method !== 'GET' && !/\/(desktop-presets|chrome-preview)$/.test(c.path));
const idle = page => page.waitForFunction(() => !document.querySelector('#modal-save').disabled);
const closed = page => page.waitForFunction(() => !document.querySelector('#modal').classList.contains('show'));
async function add(page, type = 'desktop') {
    await page.locator('#add-app-btn').click();
    // The styled select dispatches the native change event after a selection.
    await page.locator('#f-app-type').selectOption(type, { force: true });
    await page.locator('#f-name').fill('New desktop');
    if (type === 'desktop') await page.locator('#f-exec').fill('example-desktop');
}
async function enable(page, password = 'sharing-test-password') {
    await page.locator('#f-share-enabled').check();
    await page.locator('#f-share-password').fill(password);
}
async function saved(page) {
    await page.waitForFunction(() => !!document.querySelector('#f-share-url').value && !document.querySelector('#modal-save').disabled);
}
const cases = [
    ['suspended desktop, shared and background processes show a live warning with stop/restart available', async ({ page, apps, seed }) => {
        const fixtures = [
            seed({ name: 'Suspended desktop', status: 'running', window_ready: false }),
            seed({ name: 'Suspended shared app', status: 'starting', window_ready: null }),
            seed({ name: 'Suspended service', app_type: 'background', status: 'running' }),
        ];
        for (const app of fixtures) app.launch_error = { code: 'app_process_stopped', error: 'Application process is suspended.' };
        await page.locator('#nav-apps').click();
        for (const app of fixtures) {
            const row = page.locator(`tr[data-app-id="${app.id}"]`);
            await row.locator('.status-warning').waitFor();
            assert.match(await row.textContent(), /进程已暂停/);
            assert.match(await row.locator('.launch-error').textContent(), /恢复进程或重新启动/);
            assert(await row.locator('.btn-stop').isVisible());
            assert(await row.locator('.btn-restart').isVisible());
        }
        await page.evaluate(async () => (await import('/lib/i18n.js?v=1')).setLang('en'));
        await page.getByText('Process suspended', { exact: true }).first().waitFor();
        for (const app of fixtures) assert.match(await page.locator(`tr[data-app-id="${app.id}"]`).textContent(), /Process suspended/);
        for (const app of fixtures) apps.set(app.id, { ...app, launch_error: null, status: 'running', window_ready: true });
        await page.locator('#nav-apps').click();
        await page.waitForFunction(() => !document.querySelector('#app-list .launch-error'));
        assert.equal(await page.locator('#app-list .status-warning').count(), 0);
        assert.match(await page.locator(`tr[data-app-id="${fixtures[0].id}"]`).textContent(), /Window available/);
    }],
    ['new desktop shows sharing off by default; switching types preserves the draft only within this dialog', async ({ page, calls }) => {
        await add(page);
        assert(await page.locator('#sharing-section').isVisible());
        assert.equal(await page.locator('#f-share-enabled').isChecked(), false);
        await page.locator('#f-share-enabled').check();
        const password = await page.locator('#f-share-password').inputValue();
        assert.equal(password.length, 24);
        assert.equal(await page.locator('#f-share-url').inputValue(), '');
        assert.match(await page.locator('#f-share-url').getAttribute('placeholder'), /保存后/);
        assert(await page.locator('#share-url-copy').isDisabled());
        await page.locator('#f-app-type').selectOption('background', { force: true });
        assert.equal(await page.locator('#sharing-section').isVisible(), false);
        await page.locator('#f-app-type').selectOption('desktop', { force: true });
        assert.equal(await page.locator('#f-share-password').inputValue(), password);
        assert.equal(calls.filter(c => c.path.endsWith('/sharing')).length, 0);
        await page.locator('#modal-cancel').click(); await add(page);
        assert.equal(await page.locator('#f-share-enabled').isChecked(), false);
        assert.equal(await page.locator('#f-share-password').inputValue(), '');
        assert.equal(await page.locator('#f-share-password').getAttribute('placeholder'), '');
    }],
    ['ordinary creation closes without a sharing write or immediate autostart', async ({ page, calls, apps }) => {
        await add(page); await page.locator('#f-autostart').check();
        await page.locator('#modal-save').click(); await closed(page);
        assert.deepEqual(writes(calls).map(c => [c.method, c.path]), [['POST', '/api/apps']]);
        assert.equal([...apps.values()][0].autostart, true);
    }],
    ['first save enables sharing and shows the real URL without reopening', async ({ page, calls, apps, policies, url }) => {
        await add(page); await enable(page); await page.locator('#modal-save').click(); await saved(page);
        const [app] = apps.values();
        assert.equal(policies.get(app.id).password, 'sharing-test-password');
        assert.equal(await page.locator('#f-share-url').inputValue(), `${url}/remote/v1/apps/${app.id}`);
        assert.equal(await page.locator('#share-url-copy').isDisabled(), false);
        assert.equal(await page.locator('#f-exec').isDisabled(), false);
        assert.equal(await page.locator('#f-desktop-compat').isDisabled(), false);
        assert.equal(await page.locator('#modal-cancel').textContent(), '关闭');
        assert.deepEqual(writes(calls).map(c => [c.method, c.path]), [['POST', '/api/apps'], ['PUT', `/api/apps/${app.id}/sharing`]]);
        assert.match(writes(calls)[0].body.creation_id, /^[0-9a-f-]{36}$/);
        assert(!writes(calls)[0].body.sharing);
    }],
    ['password validation runs before creation and counts UTF-8 bytes', async ({ page, calls }) => {
        await add(page); await enable(page);
        for (const password of ['', '1234567', '中'.repeat(86)]) {
            await page.locator('#f-share-password').fill(password);
            await page.locator('#modal-save').click();
            assert(await page.locator('#err-f-share-password').textContent());
            assert.equal(writes(calls).length, 0);
        }
        await page.locator('#f-share-password').fill('中'.repeat(85));
        await page.locator('#modal-save').click(); await saved(page);
        assert.equal(writes(calls).length, 2);
    }],
    ['a sharing failure keeps the created ID; retry never adds a duplicate', async ({ page, calls, apps, mode }) => {
        mode.shareFail = true;
        await add(page); await enable(page); await page.locator('#modal-save').click();
        await page.waitForFunction(() => document.querySelector('#err-f-share-enabled').textContent.includes('应用已添加'));
        await idle(page);
        assert.equal(apps.size, 1);
        assert(await page.locator('#f-app-type').isDisabled());
        assert.equal(await page.locator('#f-share-enabled').isChecked(), true);
        assert.equal(await page.locator('#f-share-password').inputValue(), 'sharing-test-password');
        mode.shareFail = false;
        await page.locator('#modal-save').click(); await saved(page);
        assert.equal(apps.size, 1);
        assert.equal(writes(calls).filter(c => c.method === 'POST').length, 1);
        assert.equal(writes(calls).filter(c => c.path.endsWith('/sharing')).length, 2);
    }],
    ['a lost creation response is reconciled by ID before sharing', async ({ page, calls, apps, mode }) => {
        mode.createDrop = true;
        await add(page); await enable(page); await page.locator('#modal-save').click(); await saved(page);
        assert.equal(apps.size, 1);
        assert.equal(writes(calls).filter(c => c.method === 'POST').length, 1);
    }],
    ['an uncertain result freezes the draft; retry uses the same ID and creates only once', async ({ page, calls, apps, mode }) => {
        mode.createDrop = true; mode.appReadFail = true;
        await add(page); await enable(page); await page.locator('#modal-save').click();
        await page.waitForFunction(() => document.querySelector('#modal-error').textContent.includes('暂无法确认'));
        await idle(page);
        assert(await page.locator('#f-exec').isDisabled());
        assert(await page.locator('#f-app-type').isDisabled());
        await page.locator('#modal-save').focus(); await page.keyboard.press('Tab');
        assert(await page.locator('#modal-close').evaluate(el => document.activeElement === el));
        await page.keyboard.press('Tab');
        // Informational tooltips remain focusable even in a disabled fieldset.
        assert(await page.evaluate(() => document.querySelector('#modal').contains(document.activeElement)
            && !document.activeElement.matches(':disabled')));
        mode.appReadFail = false; mode.createDrop = false;
        await page.locator('#modal-save').click(); await saved(page);
        const posts = writes(calls).filter(c => c.method === 'POST');
        assert.equal(posts.length, 2);
        assert.equal(posts[0].body.creation_id, posts[1].body.creation_id);
        assert.equal(apps.size, 1);
    }],
    ['a name conflict never adopts or shares another client’s app', async ({ page, apps, calls, seed }) => {
        const existing = seed({ name: 'New desktop' });
        await add(page); await enable(page); await page.locator('#modal-save').click();
        await page.waitForFunction(() => document.querySelector('#modal-error').textContent.includes('已存在'));
        await idle(page);
        assert.equal(await page.locator('#f-name').isDisabled(), false);
        assert.equal(apps.size, 1);
        assert.equal(apps.get(existing.id).exec_command, 'existing-command');
        assert.equal(writes(calls).filter(c => c.path.endsWith('/sharing')).length, 0);
    }],
    ['a lost sharing response is read back without rewriting the password', async ({ page, calls, mode }) => {
        mode.shareDrop = true;
        await add(page); await enable(page); await page.locator('#modal-save').click(); await saved(page);
        await page.locator('#modal-save').click(); await closed(page);
        assert.equal(writes(calls).filter(c => c.path.endsWith('/sharing')).length, 1);
    }],
    ['a successful sharing write remains saved if its follow-up read fails', async ({ page, calls, mode }) => {
        mode.shareReadFail = true;
        await add(page); await enable(page); await page.locator('#modal-save').click(); await saved(page);
        assert.equal(await page.locator('#share-url-copy').isDisabled(), false);
        await page.locator('#modal-save').click(); await closed(page);
        assert.equal(writes(calls).filter(c => c.path.endsWith('/sharing')).length, 1);
    }],
    ['closing a pending add cannot write its response into the next draft', async ({ page, calls, mode }) => {
        let release; mode.createGate = new Promise(resolve => { release = resolve; });
        try {
            await add(page); await enable(page); await page.locator('#modal-save').click();
            await page.waitForFunction(() => document.querySelector('#modal-save').disabled);
            await page.locator('#modal-close').click(); await add(page);
            await page.locator('#f-name').fill('Next draft');
            release();
            await page.waitForFunction(() => document.querySelector('#app-list').textContent.includes('New desktop'));
            // Wait for the old operation's sharing read, not a fixed timeout.
            await page.waitForFunction(() => document.querySelector('#toast').textContent.includes('已添加'));
            assert.equal(await page.locator('#f-name').inputValue(), 'Next draft');
            assert.equal(await page.locator('#f-share-enabled').isChecked(), false);
            assert.equal(await page.locator('#f-share-password').inputValue(), '');
            assert.equal(writes(calls).filter(c => c.path.endsWith('/sharing')).length, 1);
        } finally { release(); }
    }],
    ['reopening during the sharing write waits for the entire submitted operation', async ({ page, apps, mode }) => {
        let release; mode.shareGate = new Promise(resolve => { release = resolve; });
        try {
            await add(page); await enable(page); await page.locator('#modal-save').click();
            await page.waitForFunction(() => document.querySelector('#modal-title').textContent === '编辑应用');
            await page.locator('#modal-close').click();
            // The list may refresh independently while the sharing request is pending.
            await page.locator('#nav-apps').click();
            await page.locator(`#app-list tr[data-app-id="${[...apps.keys()][0]}"] .btn-edit`).click();
            await page.waitForFunction(() => document.querySelector('#modal').classList.contains('loading'));
            release();
            await saved(page);
            assert.equal(await page.locator('#f-share-enabled').isChecked(), true);
        } finally { release(); }
    }],
    ['legacy hash-only passwords stay valid and loading state never leaks into a new draft', async ({ page, seed, policies }) => {
        const app = seed({ name: 'Legacy desktop' });
        policies.set(app.id, { enabled: false, password_set: true, password_viewable: false, password: null });
        await page.locator('#nav-apps').click();
        await page.locator(`tr[data-app-id="${app.id}"] .btn-edit`).click();
        await page.locator('#f-share-enabled').waitFor(); await idle(page);
        await page.waitForFunction(() => !document.querySelector('#f-share-enabled').disabled);
        assert.match(await page.locator('#f-share-password').getAttribute('placeholder'), /无法查看/);
        await page.locator('#f-share-enabled').check();
        assert.equal(await page.locator('#f-share-password').inputValue(), '');
        await page.locator('#modal-save').click(); await closed(page);
        assert.equal(policies.get(app.id).enabled, true);
        await add(page); await page.locator('#f-share-enabled').check();
        assert.equal((await page.locator('#f-share-password').inputValue()).length, 24);
        assert.equal(await page.locator('#f-share-password').getAttribute('placeholder'), '');
    }],
    ['mobile layout and both languages keep the pending URL and controls usable', async ({ page }) => {
        await page.setViewportSize({ width: 375, height: 720 });
        await add(page); await enable(page);
        await page.evaluate(async () => (await import('/lib/i18n.js?v=1')).setLang('en'));
        assert.equal(await page.locator('#f-share-url').getAttribute('placeholder'), 'Generated after saving');
        assert.match(await page.locator('#help-share-password').textContent(), /UTF-8 bytes/);
        await page.evaluate(async () => (await import('/lib/i18n.js?v=1')).setLang('zh'));
        assert.match(await page.locator('#f-share-url').getAttribute('placeholder'), /保存后/);
        const width = await page.evaluate(() => ({ window: innerWidth, document: document.documentElement.scrollWidth }));
        assert(width.document <= width.window, JSON.stringify(width));
    }],
];

await test('desktop app sharing', async t => {
    const work = mkdtempSync(path.join(tmpdir(), 'ivnc-app-sharing-'));
    const server = await createServer({ configFile: false, root, cacheDir: path.join(work, 'vite'), logLevel: 'error',
        optimizeDeps: { noDiscovery: true, include: [] }, server: { host: '127.0.0.1', port: 0, hmr: false } });
    try {
        await server.listen();
        const url = `http://127.0.0.1:${server.httpServer.address().port}`;
        for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) await t.test(name, async t => {
            const browser = await engines[name].launch({ headless: true,
                ...(name === 'chromium' && process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
            try { for (const [title, run] of cases) await t.test(title, async () => {
                const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1280, height: 800 } });
                const page = await context.newPage(), apps = new Map(), policies = new Map(), calls = [], mode = {}, errors = [];
                context.setDefaultTimeout(8000);
                page.on('pageerror', error => errors.push(error.message));
                const seed = props => {
                    const app = { id: randomUUID(), name: 'Existing desktop', app_type: 'desktop', exec_command: 'existing-command',
                        status: 'stopped', name_revision: 0, data_size_human: '0 B', autostart: false, ...props };
                    apps.set(app.id, app); return app;
                };
                await page.route('**/auth/session', route => route.fulfill({ json: { enabled: false, authenticated: true, kind: 'anonymous' } }));
                await page.route('**/api/**', async route => {
                    const request = route.request(), p = new URL(request.url()).pathname, method = request.method();
                    const body = request.postDataJSON(); calls.push({ path: p, method, body });
                    const reply = (json, status = 200) => route.fulfill({ json, status });
                    if (p === '/api/apps' && method === 'GET') return reply({ apps: [...apps.values()] });
                    if (p === '/api/apps' && method === 'POST') {
                        if (mode.createGate) await mode.createGate;
                        if (apps.has(body.creation_id)) return reply({ error: 'already created', code: 'app_creation_exists' }, 409);
                        if ([...apps.values()].some(a => a.name === body.name)) return reply({ error: 'duplicate name', code: 'app_name_exists', params: { name: body.name } }, 409);
                        const app = seed({ ...body, id: body.creation_id || randomUUID() });
                        if (mode.createDrop) return route.abort('failed');
                        return reply({ ok: true, app }, 201);
                    }
                    const match = p.match(/^\/api\/apps\/([^/]+)(\/sharing)?$/);
                    if (match) {
                        const [, id, sharing] = match, app = apps.get(id);
                        if (!sharing && method === 'GET' && mode.appReadFail) return route.abort('failed');
                        if (!app) return reply({ error: 'not found', code: 'app_not_found' }, 404);
                        if (!sharing && method === 'GET') return reply(app);
                        if (!sharing && method === 'PUT') { apps.set(id, { ...app, ...body }); return reply({ ok: true, id, name: app.name, name_revision: 0 }); }
                        const old = policies.get(id) || { enabled: false, password_set: false, password_viewable: false, password: null };
                        if (method === 'PUT') {
                            if (mode.shareGate) await mode.shareGate;
                            if (mode.shareFail) return reply({ error: 'sharing unavailable' }, 500);
                            policies.set(id, { ...old, enabled: body.enabled, ...(body.password ? { password: body.password, password_set: true, password_viewable: true } : {}) });
                            if (mode.shareDrop) return route.abort('failed');
                            return reply({ ok: true, restart_required: false, revoked: old.enabled });
                        }
                        if (mode.shareReadFail) return route.abort('failed');
                        return reply({ ...old, share_path: `/remote/v1/apps/${id}`, restart_required: false });
                    }
                    return reply({ apps: [], metrics: {}, version: {}, display: {}, settings: {} });
                });
                try {
                    await page.goto(url + '/console.html');
                    await page.locator('#nav-apps').click();
                    await run({ page, calls, apps, policies, mode, seed, url });
                    assert.deepEqual(errors, []);
                } finally { await context.close(); }
            }); } finally { await browser.close(); }
        });
    } finally { await server.close(); rmSync(work, { recursive: true, force: true }); }
});
