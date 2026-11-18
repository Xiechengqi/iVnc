// Local browser regression against the production application form. Fixtures
// model persistence and deliberately delay discovery/preview responses.
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
const closed = page => page.waitForFunction(() => !document.querySelector('#modal').classList.contains('show'));
const writes = calls => calls.filter(c => ['POST', 'PUT'].includes(c.method) && !/\/(chrome-preview|desktop-presets)$/.test(c.path));
const baseConfig = () => ({ mode: 'generated', profile_dir: null, crash_dir: null, debug_port: null });
function preview(body) {
    const id = body.id || body.creation_id, config = body.chrome_config;
    const effective = { profile_dir: config.profile_dir || `/ivnc/apps/${id}/profile`, crash_dir: config.crash_dir || `/ivnc/apps/${id}/crash`, debug_port: config.debug_port };
    return { id, chrome_config: config, chrome_effective: effective,
        exec_command: `google-chrome --user-data-dir=${effective.profile_dir} --crash-dumps-dir=${effective.crash_dir}${config.debug_port ? ` --remote-debugging-port=${config.debug_port}` : ''}${body.proxy_url ? ` --proxy-server=${body.proxy_url}` : ''}` };
}
function initialApps() {
    return [
        { id: 'builtin-chrome', name: 'Chrome', desktop_preset: 'chrome', chrome_config: { ...baseConfig(), profile_dir: '/ivnc/apps/builtin-chrome/profile', crash_dir: '/ivnc/apps/builtin-chrome/profile/Crash Reports', debug_port: 9222 }, proxy_configurable: true, proxy_url: null },
        { id: 'custom', name: 'Custom', desktop_preset: 'custom', exec_command: 'custom-app --flag' },
    ].map(a => ({ app_type: 'desktop', status: 'stopped', name_revision: 0, data_size_human: '0 B', env_vars: null, desktop_launch: {}, ...a,
        exec_command: a.chrome_config ? preview({ id: a.id, chrome_config: a.chrome_config }).exec_command : a.exec_command }));
}
async function add(page) {
    await page.locator('#add-app-btn').click();
    await page.locator('#f-app-type').selectOption('desktop', { force: true });
    await page.locator('#f-name').fill('New Chrome');
}
async function selectChrome(page) {
    await page.locator('#f-desktop-preset + button').click();
    await page.locator('#f-desktop-preset-listbox [role="option"]').nth(1).click();
    await page.waitForFunction(() => document.querySelector('#f-exec').value.includes('--user-data-dir='));
}
async function edit(page, id = 'builtin-chrome') {
    await page.locator(`tr[data-app-id="${id}"] .btn-edit`).click();
    await page.locator('#modal.show:not(.loading)').waitFor();
}

const cases = [
    ['new Chrome has automatic paths, empty optional fields and a read-only preview', async ({ page }) => {
        await add(page);
        assert.equal(await page.locator('#chrome-settings').isVisible(), false);
        await selectChrome(page);
        assert.equal(await page.locator('#f-exec').getAttribute('readonly'), '');
        for (const id of ['f-chrome-profile', 'f-chrome-crash', 'f-chrome-port', 'f-proxy-url']) assert.equal(await page.locator(`#${id}`).inputValue(), '');
        assert.match(await page.locator('#chrome-profile-effective').textContent(), /\/apps\/[0-9a-f-]{36}\/profile$/);
        assert.equal(await page.locator('#desktop-launch-advanced').isVisible(), false);
    }],
    ['draft identity survives previews, renaming and the first save', async ({ page, calls }) => {
        await add(page); await selectChrome(page);
        const initial = await page.locator('#chrome-profile-effective').textContent();
        await page.locator('#f-name').fill('Renamed before save');
        await page.locator('#f-proxy-url').fill('socks5://localhost:1080');
        await page.waitForFunction(() => document.querySelector('#f-exec').value.includes('--proxy-server='));
        assert.equal(await page.locator('#chrome-profile-effective').textContent(), initial);
        await page.locator('#modal-save').click(); await closed(page);
        const write = writes(calls)[0];
        assert.equal(initial, `/ivnc/apps/${write.body.creation_id}/profile`);
        assert.equal(write.body.desktop_preset, 'chrome'); assert(!('exec_command' in write.body));
        await add(page); await selectChrome(page);
        assert.notEqual(await page.locator('#chrome-profile-effective').textContent(), initial);
    }],
    ['built-in values are editable and preview follows all four fields', async ({ page }) => {
        await edit(page);
        assert.equal(await page.locator('#f-desktop-preset').isDisabled(), true);
        assert.equal(await page.locator('#f-chrome-port').inputValue(), '9222');
        await page.locator('#f-chrome-profile').fill('/data/profiles/browser a');
        await page.locator('#f-chrome-crash').fill('/data/crashes/browser a');
        await page.locator('#f-chrome-port').fill('9333');
        await page.locator('#f-proxy-url').fill('http://localhost:8080');
        await page.waitForFunction(() => document.querySelector('#f-exec').value.includes('--proxy-server=http://localhost:8080'));
        const command = await page.locator('#f-exec').inputValue();
        assert(command.includes('/data/profiles/browser a')); assert(command.includes('/data/crashes/browser a')); assert(command.includes('--remote-debugging-port=9333'));
    }],
    ['installation updates keep an open dropdown and its option nodes intact', async ({ page, state }) => {
        state.status = 'missing'; await add(page);
        await page.waitForFunction(() => document.querySelector('#f-desktop-preset').options[1].textContent.includes('未安装'));
        await page.locator('#f-desktop-preset + button').click();
        await page.evaluate(() => { window.savedOption = document.querySelector('#f-desktop-preset-listbox').children[1]; });
        state.status = 'installed';
        await page.locator('#f-env').fill('PATH=/new/path');
        await page.waitForFunction(() => document.querySelector('#f-desktop-preset').options[1].textContent.includes('已安装'));
        assert(await page.evaluate(() => window.savedOption === document.querySelector('#f-desktop-preset-listbox').children[1]));
        assert.equal(await page.locator('#f-desktop-preset-listbox').isVisible(), true);
        await page.locator('#f-desktop-preset-listbox [role="option"]').nth(1).click();
        assert.equal(await page.locator('#f-desktop-preset').inputValue(), 'chrome');
    }],
    ['manual commands persist exactly while inactive fields do not block saving', async ({ page, calls }) => {
        await edit(page); await page.locator('#f-chrome-port').fill('invalid');
        await page.locator('#f-proxy-url').fill('invalid inactive proxy');
        await page.locator('#f-chrome-manual').check();
        assert.equal(await page.locator('#f-chrome-profile').isDisabled(), true);
        const command = "custom-chrome --my-flag\n# keep whitespace  \n";
        await page.locator('#f-exec').fill(command);
        await page.locator('#modal-save').click(); await closed(page);
        const body = writes(calls)[0].body;
        assert.equal(body.exec_command, command); assert.equal(body.chrome_config.mode, 'manual');
        await page.reload(); await page.locator('#nav-apps').click(); await edit(page);
        assert.equal(await page.locator('#f-chrome-manual').isChecked(), true);
        assert.equal(await page.locator('#f-exec').inputValue(), command);
    }],
    ['restoring generated mode confirms replacement and retains configuration', async ({ page }) => {
        await edit(page); await page.locator('#f-chrome-manual').check();
        await page.locator('#f-exec').fill('custom-browser');
        page.once('dialog', dialog => dialog.dismiss()); await page.locator('#f-chrome-manual').click();
        assert.equal(await page.locator('#f-chrome-manual').isChecked(), true);
        page.once('dialog', dialog => dialog.accept()); await page.locator('#f-chrome-manual').uncheck();
        await page.waitForFunction(() => document.querySelector('#f-exec').value.includes('--remote-debugging-port=9222'));
        assert.equal(await page.locator('#f-chrome-profile').isDisabled(), false);
        assert.equal(await page.locator('#f-exec').getAttribute('readonly'), '');
    }],
    ['invalid active port or path prevents a write', async ({ page, calls }) => {
        await add(page); await selectChrome(page);
        for (const value of ['0', '65536', '12x']) {
            await page.locator('#f-chrome-port').fill(value); await page.locator('#modal-save').click();
            assert(await page.locator('#err-f-chrome-port').textContent());
        }
        await page.locator('#f-chrome-port').fill(''); await page.locator('#f-chrome-profile').fill('/data/../other');
        await page.locator('#modal-save').click(); assert(await page.locator('#err-f-chrome-profile').textContent());
        assert.equal(writes(calls).length, 0);
    }],
    ['late previews cannot replace a newer configuration or another dialog', async ({ page, state }) => {
        await add(page); await selectChrome(page); state.delay = true;
        await page.locator('#f-chrome-profile').fill('/data/old');
        await page.waitForFunction(() => window.previewOldStarted === true);
        await page.locator('#f-chrome-profile').fill('/data/new');
        await page.waitForFunction(() => document.querySelector('#f-exec').value.includes('/data/new'));
        await page.locator('#modal-cancel').click(); await edit(page, 'custom');
        await new Promise(resolve => setTimeout(resolve, 650));
        assert.equal(await page.locator('#f-exec').inputValue(), 'custom-app --flag');
        assert.equal(await page.locator('#chrome-settings').isVisible(), false);
        assert.equal(await page.locator('#f-exec').getAttribute('readonly'), null);
    }],
    ['preview refresh alone does not mark a running Chrome as changed', async ({ page, apps }) => {
        apps.get('builtin-chrome').status = 'running';
        await edit(page); await page.locator('#modal-save').click(); await closed(page);
    }],
    ['configuration edits on a running Chrome require a restart', async ({ page, apps, calls }) => {
        apps.get('builtin-chrome').status = 'running';
        await edit(page); await page.locator('#f-chrome-port').fill('9333');
        await page.locator('#modal-save').click(); await page.locator('#restart-notice:not([hidden])').waitFor();
        assert.equal(writes(calls).filter(c => c.path.endsWith('/restart')).length, 0);
    }],
    ['a missing Chrome can be configured and saved without starting it', async ({ page, calls, state }) => {
        state.status = 'missing'; await add(page); await selectChrome(page);
        await page.locator('#modal-save').click(); await closed(page);
        assert.equal(writes(calls).length, 1);
        assert.equal(writes(calls)[0].path, '/api/apps');
    }],
    ['narrow forms keep the command and configuration inside the modal', async ({ page }) => {
        await page.setViewportSize({ width: 320, height: 740 });
        await add(page); await selectChrome(page);
        for (const lang of ['zh', 'en']) {
            if (lang === 'en') {
                await page.locator('#modal-cancel').click();
                await page.locator('#lang-toggle').click();
                await add(page); await selectChrome(page);
            }
            await page.screenshot({ path: `/tmp/ivnc-chrome-form-mobile-${lang}.png` });
            const overflow = await page.locator('#modal .modal').evaluate(node => ({
                width: node.clientWidth, scroll: node.scrollWidth,
                children: [...node.querySelectorAll('*')].filter(child => child.getBoundingClientRect().right > node.getBoundingClientRect().right + 1)
                    .map(child => ({ id: child.id, class: child.className, width: child.getBoundingClientRect().width })),
            }));
            assert(overflow.scroll <= overflow.width + 1, `${lang}: ${JSON.stringify(overflow)}`);
            const bounds = await page.evaluate(() => {
                const rect = id => document.getElementById(id).getBoundingClientRect().toJSON();
                return { checkbox: rect('f-chrome-manual'), manual: rect('chrome-manual-control'),
                    autostart: rect('autostart-control'), label: rect('label-exec'), command: rect('f-exec') };
            });
            assert(bounds.checkbox.width >= 12 && bounds.checkbox.width <= 18, `${lang}: compact checkbox width`);
            assert(bounds.checkbox.height >= 12 && bounds.checkbox.height <= 18, `${lang}: compact checkbox height`);
            assert(bounds.manual.height <= 24, `${lang}: manual label stays on one line`);
            assert(bounds.label.bottom <= Math.min(bounds.manual.top, bounds.autostart.top), `${lang}: options below the label`);
            assert(Math.max(bounds.manual.bottom, bounds.autostart.bottom) <= bounds.command.top, `${lang}: options above the command`);
        }
    }],
];

await test('Chrome desktop preset interaction', async t => {
    const work = mkdtempSync(path.join(tmpdir(), 'ivnc-chrome-ui-'));
    const server = await createServer({ configFile: false, root, cacheDir: path.join(work, 'vite'), logLevel: 'error',
        optimizeDeps: { noDiscovery: true, include: [] }, server: { host: '127.0.0.1', port: 0, hmr: false },
        plugins: [{ name: 'chrome-fixture', configureServer(s) { s.middlewares.use((req, res, next) => {
            if (req.url === '/console') { res.setHeader('Content-Type', 'text/html'); return res.end(readFileSync(path.join(root, 'console.html'))); } next();
        }); } }] });
    try {
        await server.listen(); const url = `http://127.0.0.1:${server.httpServer.address().port}/console`;
        for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) await t.test(name, async t => {
            const browser = await engines[name].launch({ headless: true, ...(name === 'chromium' && process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
            try { for (const [title, run] of cases) await t.test(title, async () => {
                const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: 'zh-CN' });
                const page = await context.newPage(), calls = [], errors = [], state = { status: 'installed', delay: false };
                const apps = new Map(initialApps().map(a => [a.id, a])); context.setDefaultTimeout(7000);
                page.on('pageerror', error => errors.push(error.message));
                await page.route('**/api/**', async route => {
                    const request = route.request(), pathname = new URL(request.url()).pathname;
                    const body = request.method() === 'GET' ? null : request.postDataJSON();
                    calls.push({ path: pathname, method: request.method(), body });
                    const respond = data => route.fulfill({ json: data }).catch(() => {});
                    if (pathname === '/api/apps/desktop-presets') return respond({ presets: [{ id: 'chrome', installation: { status: state.status } }] });
                    if (pathname === '/api/apps/chrome-preview') {
                        if (state.delay && body.chrome_config.profile_dir === '/data/old') {
                            await page.evaluate(() => { window.previewOldStarted = true; });
                            await new Promise(resolve => setTimeout(resolve, 600));
                        }
                        return respond(preview(body));
                    }
                    if (pathname.endsWith('/sharing')) return respond({ enabled: false, password_set: false });
                    if (pathname === '/api/apps') {
                        if (request.method() === 'GET') return respond({ apps: [...apps.values()], order_revision: 1 });
                        const app = { ...body, id: body.creation_id, name_revision: 0, status: 'stopped', ...(body.desktop_preset === 'chrome' && body.chrome_config.mode === 'generated' ? preview({ ...body, id: body.creation_id }) : {}) };
                        apps.set(app.id, app); return respond({ ok: true, app });
                    }
                    const id = pathname.split('/')[3];
                    if (apps.has(id) && pathname === `/api/apps/${id}`) {
                        if (request.method() === 'PUT') {
                            const app = { ...apps.get(id), ...body };
                            if (app.chrome_config?.mode === 'generated') Object.assign(app, preview({ ...app, id }));
                            apps.set(id, app); return respond({ ok: true, app });
                        }
                        return respond(apps.get(id));
                    }
                    if (pathname === '/api/capabilities') return respond({ apps: [] });
                    return respond({});
                });
                await page.route('**/health', route => route.fulfill({ json: { ok: true } }));
                try { await page.goto(url); await page.locator('#nav-apps').click(); await page.locator('tr[data-app-id="builtin-chrome"]').waitFor();
                    await run({ page, calls, state, apps }); assert.deepEqual(errors, []);
                } finally { await context.close(); }
            }); } finally { await browser.close(); }
        });
    } finally { await server.close(); rmSync(work, { recursive: true, force: true }); }
});
