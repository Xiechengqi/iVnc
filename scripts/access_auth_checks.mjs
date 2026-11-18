// Local-only extension of auth_smoke.mjs: real IP login, proxy trust, revocation
// and settings UI. Uses the parent's isolated homes and process cleanup.
import assert from 'node:assert/strict';
import path from 'node:path';
import { readFileSync, statSync, writeFileSync, renameSync, mkdirSync, rmSync } from 'node:fs';

export async function runAccessChecks({ startServer, stopServer, launchServer, request, loginApi, basic, terminal, refusedSocket, waitFor, pass, engines, browsers, work }) {
    const update = (server, headers, enabled, ip_allowlist) => request(server, '/api/settings/access', {
        method: 'PUT', headers, body: { enabled, ip_allowlist },
    });
    const auto = (server, headers = {}, manual = false) => request(server, '/auth/auto-login', {
        method: 'POST', headers: { Origin: server.origin, ...headers }, body: { manual },
    });
    const credentials = (server, response) => {
        assert.equal(response.status, 200, response.body);
        return { Cookie: response.headers['set-cookie'][0].split(';', 1)[0], Origin: server.origin, 'X-IVNC-CSRF': response.json.csrf };
    };
    const plain = await startServer('access-http'), secure = await startServer('access-https', { tls: true });
    for (const server of [plain, secure]) {
        const tag = server.tls ? 'HTTPS' : 'HTTP', admin = await loginApi(server);
        const initial = await request(server, '/api/settings/access', { headers: admin });
        const ip = initial.json.client_ip; assert(ip);
        assert.equal(initial.json.enabled, false);
        assert.deepEqual(initial.json.ip_allowlist, []);
        assert.equal((await request(server, '/api/settings/access')).status, 401);
        assert.equal((await auto(server)).status, 403);
        assert.equal((await update(server, { Cookie: admin.Cookie, Origin: server.origin }, true, [ip])).status, 403);
        assert.equal((await update(server, { Cookie: admin.Cookie, Origin: server.origin, Referer: server.origin + '/proxy/' }, true, [ip])).status, 403, 'Miao referer cannot bypass settings CSRF');
        assert.equal((await update(server, { ...admin, Origin: 'http://foreign.test' }, true, [ip])).status, 403);
        const invalid = await update(server, admin, true, [ip, '', 'invalid']);
        assert.equal(invalid.status, 400); assert.equal(invalid.json.line, 3);
        assert.equal((await request(server, '/api/settings/access', { method: 'PUT', headers: admin, body: {} })).status, 422);
        assert.equal((await request(server, '/api/settings/access', { headers: admin })).json.enabled, false);
        assert.equal((await update(server, admin, true, [ip, ip])).status, 200);
        const status = await request(server, '/auth/session');
        assert.equal(status.json.authenticated, false); assert.equal(status.json.auto_login_available, true);
        const originMissing = await request(server, '/auth/auto-login', { method: 'POST', body: {} });
        assert.equal(originMissing.status, 403);
        assert.equal((await auto(server, { Origin: 'http://foreign.test' })).status, 403);
        const response = await auto(server), session = credentials(server, response);
        assert.match(response.headers['set-cookie'][0], /HttpOnly; SameSite=Lax/);
        assert.equal(response.headers['set-cookie'][0].includes('; Secure'), server.tls);
        const state = await request(server, '/auth/session', { headers: session });
        assert.equal(state.json.kind, 'session'); assert.equal(state.json.method, 'ip_whitelist');
        assert.equal((await request(server, '/api/apps', { headers: session })).status, 200);
        assert.equal((await auto(server, session)).headers['set-cookie'], undefined, 'valid sessions are reused');
        assert.equal((await auto(server, { Cookie: session.Cookie + '; ' + session.Cookie })).status, 400);
        await refusedSocket(server, { Cookie: session.Cookie }, 403);
        const socket = await terminal(server, session);
        assert.equal((await update(server, admin, true, ['198.51.100.0/24'])).status, 200);
        await waitFor('IP policy revokes an open terminal', () => socket.readyState === 3);
        assert.equal((await request(server, '/api/apps', { headers: session })).status, 401);
        assert.equal((await request(server, '/api/apps', { headers: admin })).status, 200, 'password sessions survive policy changes');
        assert.equal((await auto(server, { 'X-Forwarded-For': '198.51.100.20', 'X-Real-IP': '198.51.100.20' })).status, 403);
        assert.equal((await update(server, admin, true, [ip])).status, 200);
        const beforeRestart = credentials(server, await auto(server));
        const stored = JSON.parse(readFileSync(path.join(server.env.IVNC_HOME, 'access.json'), 'utf8'));
        assert.deepEqual(stored, { enabled: true, ip_allowlist: [ip] });
        assert.equal(statSync(path.join(server.env.IVNC_HOME, 'access.json')).mode & 0o777, 0o600);
        await stopServer(server); await launchServer(server);
        assert.equal((await request(server, '/api/apps', { headers: beforeRestart })).status, 401);
        assert.equal((await request(server, '/auth/session')).json.auto_login_available, true);
        pass(`${tag}: IP login, CSRF/Origin, Cookie reuse, spoof rejection, live revocation and restart persistence`);
    }

    const proxy = await startServer('access-proxy', { host: '127.0.0.1', trustedProxies: ['127.0.0.1', '10.0.0.0/24'] });
    assert.equal((await update(proxy, basic(proxy), true, ['192.168.1.20'])).status, 200);
    assert.equal((await auto(proxy)).status, 403);
    const forwarded = { 'X-Forwarded-For': '192.168.1.20, 10.0.0.2' };
    const proxiedSession = credentials(proxy, await auto(proxy, forwarded));
    assert.equal((await request(proxy, '/api/apps', { headers: { ...proxiedSession, ...forwarded } })).status, 200);
    assert.equal((await request(proxy, '/api/apps', { headers: { ...proxiedSession, 'X-Forwarded-For': '192.168.1.21' } })).status, 401);
    for (const value of ['192.168.1.20, 172.16.0.2', 'invalid', '192.168.1.20,']) {
        assert.equal((await auto(proxy, { 'X-Forwarded-For': value })).status, 403);
    }
    assert.equal((await request(proxy, '/api/apps', { headers: { ...proxiedSession, ...forwarded } })).status, 200);
    pass('trusted proxy chain resolves from the right, fails closed and binds sessions to the resolved IP');

    for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) {
        const browser = await engines[name].launch({ ...(name === 'chromium' ? { executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] } : {}) });
        browsers.push(browser);
        for (const server of [plain, secure]) {
            const context = await browser.newContext({ ignoreHTTPSErrors: true, locale: 'zh-CN', viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
            const page = await context.newPage(), errors = [], passwordRequests = [];
            page.on('pageerror', error => errors.push(error.message));
            page.on('request', req => { if (new URL(req.url()).pathname === '/auth/login' && req.method() === 'POST') passwordRequests.push(req); });
            await page.goto(server.origin + '/console?from=ip-login#advanced');
            await page.waitForURL(server.origin + '/console?from=ip-login#advanced');
            await page.locator('#nav-advanced').click();
            await page.locator('#access-rules:enabled').waitFor();
            await page.screenshot({ path: path.join(work, `${name}-${server.tls ? 'https' : 'http'}-advanced-settings.png`) });
            assert.equal(await page.evaluate(async () => (await (await fetch('/auth/session')).json()).method), 'ip_whitelist');
            assert.equal(passwordRequests.length, 0);
            await page.reload(); await page.locator('#nav-overview').waitFor();
            await page.locator('#signout-btn').click(); await page.waitForURL(/\/auth\/login/);
            await page.locator('#login-form').waitFor();
            assert(await page.locator('#ip-login').isVisible());
            await page.reload(); await page.locator('#login-form').waitFor();
            assert.equal(new URL(page.url()).pathname, '/auth/login');
            const marker = (await context.cookies()).find(c => c.name.startsWith('ivnc_session_'));
            assert.equal((await auto(server, { Cookie: `${marker.name}=${marker.value}` })).status, 409);
            await page.locator('#password').fill('visibility-fixture');
            await page.locator('#password').evaluate(el => { el.focus(); el.setSelectionRange(3, 7); });
            await page.locator('#password-toggle').click();
            assert.equal(await page.locator('#password').getAttribute('type'), 'text');
            assert.equal(await page.locator('#password-toggle').getAttribute('aria-pressed'), 'true');
            await page.waitForFunction(() => document.querySelector('#password').selectionStart === 3 && document.querySelector('#password').selectionEnd === 7);
            assert.deepEqual(await page.locator('#password').evaluate(el => [el.selectionStart, el.selectionEnd]), [3, 7]);
            await page.locator('#password-toggle').press('Enter');
            assert.equal(await page.locator('#password').getAttribute('type'), 'password');
            assert.equal(await page.locator('#password').inputValue(), 'visibility-fixture');
            assert.equal(passwordRequests.length, 0, 'eye button must not submit the form');
            await page.locator('#language').click();
            assert.equal(await page.locator('#password-toggle').getAttribute('aria-label'), 'Show password');
            await page.screenshot({ path: path.join(work, `${name}-${server.tls ? 'https' : 'http'}-password-eye.png`) });
            await page.locator('#ip-login').click();
            await page.waitForURL(server.origin + '/console?from=ip-login#advanced');
            await page.locator('#nav-overview').waitFor();
            await page.locator('#nav-advanced').click();await page.locator('#access-rules:enabled').waitFor();
            await page.locator('#access-enabled').uncheck();await page.locator('#access-save').click();
            await page.waitForURL(/\/auth\/login/);await page.locator('#login-form').waitFor();
            const admin = await loginApi(server);
            const saved = await request(server, '/api/settings/access', { headers: admin });
            assert.equal(saved.json.enabled, false);
            assert.equal((await update(server, admin, true, saved.json.ip_allowlist)).status, 200);
            assert.deepEqual(errors, []);
            await context.close();
            pass(`${name} ${server.tls ? 'HTTPS' : 'HTTP'}: automatic entry, logout/re-entry, password visibility and settings self-revocation`);

            const blocked = await browser.newContext({ ignoreHTTPSErrors: true, locale: 'zh-CN' });
            const blockedPage = await blocked.newPage(); let attempts = 0;
            await blockedPage.route('**/auth/auto-login', async route => {
                attempts++;
                const response = await route.fetch(), headers = response.headers();
                delete headers['set-cookie']; await blocked.clearCookies();
                await route.fulfill({ response, headers });
            });
            await blockedPage.goto(server.origin + '/');
            await blockedPage.locator('#error').filter({ hasText: '请允许此站点使用 Cookie' }).waitFor();
            assert.equal(new URL(blockedPage.url()).pathname, '/auth/login');
            assert.equal(attempts, 1);
            assert.equal(await blockedPage.evaluate(() => window.__ivncAuthReady), false);
            assert.equal(attempts, 1, 'failed automatic entry must not create a retry loop');
            await blockedPage.locator('#ip-login').click();
            await blockedPage.locator('#error').filter({ hasText: '请允许此站点使用 Cookie' }).waitFor();
            assert.equal(attempts, 2);
            await blocked.close();
            pass(`${name} ${server.tls ? 'HTTPS' : 'HTTP'}: blocked IP-login Cookies show an error without repeated automatic entry`);

            const legacy = await browser.newContext({ ignoreHTTPSErrors: true, extraHTTPHeaders: basic(server) });
            const legacyPage = await legacy.newPage();await legacyPage.goto(server.origin + '/auth/login?next=%2Fconsole');
            await legacyPage.waitForURL(server.origin + '/console');await legacyPage.locator('#nav-overview').waitFor();
            assert.equal(await legacyPage.evaluate(async () => (await (await fetch('/auth/session')).json()).method), 'ip_whitelist');
            await legacy.close();
            pass(`${name} ${server.tls ? 'HTTPS' : 'HTTP'}: cached Basic credentials do not prevent IP-session bootstrap`);
        }
        await browser.close();
    }

    // Revoke actual media after the signaling socket is gone, using an IP session.
    const mediaBrowser = await engines.chromium.launch({ executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    browsers.push(mediaBrowser);
    const page = await mediaBrowser.newPage();
    await page.goto(plain.origin + '/');
    await page.waitForFunction(() => window.__ivncWebRTC?._send_channel?.readyState === 'open', null, { timeout: 60000 });
    await page.evaluate(() => { window.__ivncWebRTC.signaling.ondisconnect = null; window.__ivncWebRTC.signaling.destroy(); });
    const admin = await loginApi(plain);
    assert.equal((await update(plain, admin, false, [])).status, 200);
    await waitFor('IP policy revokes media with no signaling socket', async () => (await request(plain, '/health')).json.connections === 0);
    await mediaBrowser.close();
    pass('IP allowlist removal revokes real media/input independently of the signaling socket');

    // Exercise the settings API with console management disabled over LAN HTTP.
    const minimal = await startServer('access-minimal', { consoleEnabled: false });
    const minimalAdmin = await loginApi(minimal);
    const currentIp = (await request(minimal, '/api/settings/access', { headers: minimalAdmin })).json.client_ip;
    assert.equal((await update(minimal, minimalAdmin, true, [currentIp])).status, 200);
    const browser = await engines.chromium.launch({ executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    browsers.push(browser);
    const minimalPage = await browser.newPage();
    await minimalPage.goto(minimal.origin + '/settings.html');
    await minimalPage.locator('#nav-advanced').click();await minimalPage.locator('#access-rules:enabled').waitFor();
    assert.equal(await minimalPage.locator('#access-client-ip').textContent(), currentIp);
    await browser.close();
    pass('console-disabled settings exposes the same authenticated IP controls');

    const durable = path.join(secure.env.IVNC_HOME, 'access.json');
    const backup = durable + '.backup'; renameSync(durable, backup); mkdirSync(durable);
    const secureAdmin = await loginApi(secure);
    assert.equal((await update(secure, secureAdmin, false, [])).status, 500);
    assert.equal((await request(secure, '/auth/session')).json.auto_login_available, true);
    rmSync(durable, { recursive: true });renameSync(backup, durable);
    await stopServer(secure);writeFileSync(durable, '{invalid');await launchServer(secure);
    assert.equal((await request(secure, '/auth/session')).json.auto_login_available, false);
    assert.equal((await request(secure, '/api/apps', { headers: basic(secure) })).status, 200);
    pass('failed saves keep the active policy; corrupt settings disable IP login and retain password access');
}
