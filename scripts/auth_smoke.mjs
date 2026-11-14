// Real form login, Cookie-only refreshes and live socket revocation over HTTP
// and HTTPS, using isolated iVnc homes. Requires a TLS-enabled ivnc and Playwright.
// IVNC_BIN=target/debug/ivnc node scripts/auth_smoke.mjs
// IVNC_TEST_HTTP_HOST=<local LAN IP> also exercises an untrusted HTTP origin.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, openSync, closeSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import http from 'node:http';
import https from 'node:https';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';
const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const engines = require('playwright'), { WebSocket } = require('ws');
const root = path.resolve(new URL('..', import.meta.url).pathname);
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-auth-'));
const binary = path.resolve(process.env.IVNC_BIN || path.join(root, 'target/debug/ivnc'));
const username = 'session-test', originalPassword = 'test-session-secret';
let passed = 0;
const pass = name => { passed++; console.log(`PASS ${name}`); };
const servers = [], browsers = [], sockets = [];
async function freePort() {
    const s = net.createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r));
    const p = s.address().port; await new Promise(r => s.close(r)); return p;
}
async function waitFor(name, check, timeout = 30000) {
    const end = Date.now() + timeout; let last;
    while (Date.now() < end) { try { const value = await check(); if (value) return value; } catch (e) { last = e; } await delay(50); }
    throw Error(`Timed out: ${name}${last ? ` (${last.message})` : ''}`);
}
function request(server, target, { method = 'GET', headers = {}, body } = {}) {
    const payload = body === undefined ? undefined : JSON.stringify(body);
    return new Promise((resolve, reject) => {
        const req = (server.tls ? https : http).request(server.origin + target, {
            method, rejectUnauthorized: false, headers: { ...(payload ? { 'Content-Type': 'application/json' } : {}), ...headers }, timeout: 10000,
        }, res => {
            let data = ''; res.setEncoding('utf8'); res.on('data', s => data += s);
            res.on('end', () => { let json; try { json = JSON.parse(data); } catch {} resolve({ status: res.statusCode, headers: res.headers, body: data, json }); });
        });
        req.on('error', reject); req.on('timeout', () => req.destroy(Error('HTTP timeout'))); req.end(payload);
    });
}
async function startServer(name, { tls = false, idle = 600, maximum = 3600, enabled = true } = {}) {
    const dir = path.join(work, name); mkdirSync(dir);
    const env = { ...process.env, IVNC_HOME: path.join(dir, 'home'), XDG_RUNTIME_DIR: path.join(dir, 'run'),
        XDG_CONFIG_HOME: path.join(dir, 'config'), XDG_DATA_HOME: path.join(dir, 'data'), XDG_CACHE_HOME: path.join(dir, 'cache'), IVNC_MIAO_PORT: String(await freePort()) };
    for (const key of ['IVNC_HOME','XDG_RUNTIME_DIR','XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_CACHE_HOME']) mkdirSync(env[key], { mode: 0o700 });
    const port = await freePort(), config = path.join(dir, 'ivnc.toml');
    writeFileSync(config, `[http]\nport=${port}\nbasic_auth_enabled=${enabled}\nbasic_auth_user=${JSON.stringify(username)}\nbasic_auth_password=${JSON.stringify(originalPassword)}\nauth_session_idle_seconds=${idle}\nauth_session_max_seconds=${maximum}\n[audio]\nenabled=false\nsample_rate=48000\nchannels=2\nbitrate=128000\n`, { mode: 0o600 });
    const host = tls ? '127.0.0.1' : (process.env.IVNC_TEST_HTTP_HOST || '127.0.0.1');
    const server = { tls, port, dir, config, env, password: originalPassword, origin: `${tls ? 'https' : 'http'}://${host}:${port}` };
    servers.push(server);
    await launchServer(server); return server;
}
async function launchServer(server) {
    const fd = openSync(path.join(server.dir, 'server.log'), 'a', 0o600);
    server.child = spawn(binary, ['--config', server.config, '--proxy-enabled', 'false', '--proxy-autostart', 'false', '--width', '800', '--height', '600', ...(server.tls ? ['--tls'] : [])], { cwd: root, env: server.env, stdio: ['ignore', fd, fd] });
    closeSync(fd);
    await waitFor('server health', async () => {
        assert.equal(server.child.exitCode, null, 'isolated server exited');
        return (await request(server, '/health')).status === 200;
    });
}
async function stopServer(server) {
    if (server.child?.exitCode === null) {
        server.child.kill('SIGINT');
        await waitFor('server shutdown', () => server.child.exitCode !== null || server.child.signalCode !== null, 20000)
            .catch(() => server.child.kill('SIGKILL'));
    }
}
const basic = server => ({ Authorization: 'Basic ' + Buffer.from(`${username}:${server.password}`).toString('base64') });
async function loginApi(server) {
    const result = await request(server, '/auth/login', { method: 'POST', headers: { Origin: server.origin }, body: { username, password: server.password } });
    assert.equal(result.status, 200);
    return { Cookie: result.headers['set-cookie'][0].split(';', 1)[0], Origin: server.origin, 'X-IVNC-CSRF': result.json.csrf };
}
async function terminal(server, credentials) {
    const socket = new WebSocket(server.origin.replace(/^http/, 'ws') + '/terminal/ws', { headers: credentials, rejectUnauthorized: false });
    sockets.push(socket);
    await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(Error('terminal ready timeout')), 10000);
        socket.on('error', reject);
        socket.on('message', data => { if (JSON.parse(data).type === 'ready') { clearTimeout(timer); resolve(); } });
    });
    return socket;
}
async function refusedSocket(server, credentials, status) {
    const socket = new WebSocket(server.origin.replace(/^http/, 'ws') + '/terminal/ws', { headers: credentials, rejectUnauthorized: false });
    await new Promise((resolve, reject) => {
        socket.once('unexpected-response', (_, response) => { response.resume(); socket.terminate(); assert.equal(response.statusCode, status); resolve(); });
        socket.on('error', () => {}); socket.once('open', () => { socket.close(); reject(Error('unauthorized socket opened')); });
    });
}
async function formLogin(page, server, next = '/') {
    await page.goto(server.origin + '/auth/login?next=' + encodeURIComponent(next));
    await page.locator('#username').fill(username); await page.locator('#password').fill(server.password);
    await page.locator('#submit').click();
    await page.waitForURL(server.origin + next);
    await page.locator(next === '/console' ? '#nav-overview' : '#taskbar').waitFor();
}
try {
    const plain = await startServer('http'), secure = await startServer('https', { tls: true });
    for (const server of [plain, secure]) {
        const tag = server.tls ? 'HTTPS' : 'HTTP';
        const unauth = await request(server, '/api/apps'); assert.equal(unauth.status, 401); assert(unauth.headers['www-authenticate']);
        const insecureBrowser = await request(server, '/api/apps', { headers: { 'User-Agent': 'Mozilla/5.0 (iPhone) AppleWebKit/605.1.15', Accept: '*/*' } });
        assert.equal(insecureBrowser.status, 401); assert(!insecureBrowser.headers['www-authenticate']);
        assert.equal(insecureBrowser.headers['x-ivnc-auth'], 'required');
        const nav = await request(server, '/console', { headers: { Accept: 'text/html' } }); assert.equal(nav.status, 303); assert(nav.headers.location.startsWith('/auth/login?next=')); assert(!nav.headers['www-authenticate']);
        const valid = await request(server, '/api/apps', { headers: basic(server) }); assert.equal(valid.status, 200); assert(!valid.headers['set-cookie']);
        assert.equal((await request(server, '/auth/login', { method: 'POST', headers: { Origin: 'http://foreign.test' }, body: { username, password: server.password } })).status, 403);
        assert.equal((await request(server, '/sw.js')).status, 200);
        assert.match((await request(server, '/sw.js')).body, /No fetch interception/);
        assert.equal((await request(server, '/manifest.json')).status, 200);
        pass(`${tag}: browser navigation, explicit Basic compatibility, login origin and shipped public assets`);
        const credentials = await loginApi(server), cookieOnly = { Cookie: credentials.Cookie };
        assert.equal((await request(server, '/api/apps', { headers: cookieOnly })).status, 200);
        assert.equal((await request(server, '/api/console/keyframe', { method: 'POST', headers: { ...cookieOnly, Origin: server.origin } })).status, 403);
        assert.equal((await request(server, '/api/console/keyframe', { method: 'POST', headers: { ...credentials, Origin: 'http://foreign.test' } })).status, 403);
        assert.equal((await request(server, '/api/console/keyframe', { method: 'POST', headers: credentials })).status, 200);
        await refusedSocket(server, cookieOnly, 403); await refusedSocket(server, { Origin: server.origin }, 401);
        const socket = await terminal(server, credentials);
        assert.equal((await request(server, '/auth/logout', { method: 'POST', headers: credentials })).status, 200);
        await waitFor('logout closes terminal shell', () => socket.readyState === WebSocket.CLOSED);
        assert.equal((await request(server, '/api/apps', { headers: cookieOnly })).status, 401);
        assert.equal((await request(server, '/api/upgrade/ws', { headers: cookieOnly })).status, 401);
        pass(`${tag}: Cookie-only APIs, CSRF, socket origins, terminal revocation and upgrade authentication`);
    }
    for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) {
        const browser = await engines[name].launch({ ...(name === 'chromium' ? { executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] } : {}) }); browsers.push(browser);
        for (const server of [plain, secure]) {
            const tag = `${name} ${server.tls ? 'HTTPS' : 'HTTP'}`;
            const context = await browser.newContext({ ignoreHTTPSErrors: true, locale: 'zh-CN', viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
            const page = await context.newPage(), pageErrors = [], requests = [];
            page.on('pageerror', error => pageErrors.push(error.message));
            page.on('request', req => { if (req.url().startsWith(server.origin)) requests.push(req.headers()); });
            await page.goto(server.origin + '/'); await page.locator('#login-form').waitFor();
            assert(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth));
            await page.screenshot({ path: path.join(work, `${name}-${server.tls ? 'https' : 'http'}-login.png`) });
            await page.locator('#username').fill(username); await page.locator('#password').fill('wrong'); await page.locator('#submit').click();
            await page.locator('#error').filter({ hasText: '用户名或密码不正确' }).waitFor();
            await page.locator('#password').fill(server.password); await page.locator('#submit').click();
            await page.waitForURL(server.origin + '/'); await page.locator('#taskbar').waitFor();
            const cookie = (await context.cookies()).find(c => c.name.startsWith('ivnc_session_'));
            assert(cookie?.httpOnly); assert.equal(cookie.secure, server.tls); assert.equal(cookie.sameSite, 'Lax');
            assert(!await page.evaluate(() => document.cookie.includes('ivnc_session_')));
            assert.equal((await page.evaluate(async () => (await (await fetch('/auth/session')).json()).kind)), 'session');
            pass(`${tag}: real login form, mobile layout, errors and HttpOnly Cookie attributes`);
            for (let i = 0; i < 10; i++) {
                await page.reload(); await page.locator('#taskbar').waitFor(); assert.equal(new URL(page.url()).pathname, '/');
                assert.equal(await page.evaluate(async () => (await fetch('/api/apps')).status), 200);
            }
            assert(requests.every(headers => !headers.authorization));
            assert(!pageErrors.length, pageErrors.join('\n'));
            pass(`${tag}: ten refreshes with Cookie authentication and no injected Basic headers`);
            await context.setOffline(true);
            assert.equal(await page.evaluate(async () => {
                window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true }));
                await new Promise(resolve => setTimeout(resolve, 150));
                return window.__ivncAuthRedirecting || false;
            }), false);
            await context.setOffline(false);
            await page.evaluate(() => {
                window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true }));
                window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true }));
            });
            assert.equal(new URL(page.url()).pathname, '/');
            const other = await context.newPage(); await other.goto(server.origin + '/console'); await other.locator('#nav-overview').waitFor();
            await other.screenshot({ path: path.join(work, `${name}-${server.tls ? 'https' : 'http'}-console.png`) });
            const oldCookie = `${cookie.name}=${cookie.value}`;
            await other.locator('#signout-btn').click();
            await other.waitForURL(/\/auth\/login/); await page.waitForURL(/\/auth\/login/);
            assert.equal((await request(server, '/api/apps', { headers: { Cookie: oldCookie } })).status, 401);
            const marker = (await context.cookies()).find(c => c.name.startsWith('ivnc_session_'));
            assert.equal((await request(server, '/api/apps', { headers: { ...basic(server), Cookie: `${marker.name}=${marker.value}` } })).status, 401);
            pass(`${tag}: background restore, multiple tabs, logout and cached Basic fallback protection`);
            await context.close();
        }
        const context = await browser.newContext({ locale: 'zh-CN' }), page = await context.newPage();
        await page.route('**/auth/login', async route => {
            const response = await route.fetch(), headers = response.headers();
            delete headers['set-cookie'];
            // route.fetch shares the context's cookie jar; remove its cookies
            // too, so this represents a browser actually refusing storage.
            await context.clearCookies();
            await route.fulfill({ response, headers });
        });
        await page.goto(plain.origin + '/auth/login?next=%2Fconsole');
        await page.locator('#username').fill(username); await page.locator('#password').fill(plain.password);
        await page.locator('#submit').click();
        await page.locator('#error').filter({ hasText: '请允许此站点使用 Cookie' }).waitFor();
        assert.equal(new URL(page.url()).pathname, '/auth/login');
        await page.unroute('**/auth/login');
        await page.goto(plain.origin + '/auth/login?next=%2F%2Fforeign.test');
        await page.locator('#username').fill(username); await page.locator('#password').fill(plain.password);
        await page.locator('#submit').click();
        await page.waitForURL(plain.origin + '/'); await page.locator('#taskbar').waitFor();
        for (const next of ['/\\foreign.test', 'https://foreign.test', '/auth/login', '/login.html']) {
            await page.goto(plain.origin + '/auth/login?next=' + encodeURIComponent(next));
            await page.waitForURL(plain.origin + '/'); await page.locator('#taskbar').waitFor();
        }
        await page.goto(plain.origin + '/auth/login?next=' + encodeURIComponent('/console?from=login#apps'));
        await page.waitForURL(plain.origin + '/console?from=login#apps'); await page.locator('#nav-overview').waitFor();
        pass(`${name}: blocked Cookies have an actionable error; unsafe return URLs remain on this instance`);
        await context.close();
        await browser.close();
    }
    // Prove media revocation independently of the signaling socket's lifetime.
    const browser = await engines.chromium.launch({ executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] }); browsers.push(browser);
    const context = await browser.newContext({ locale: 'zh-CN' }), page = await context.newPage();
    await formLogin(page, plain);
    await page.waitForFunction(() => window.__ivncWebRTC?._send_channel?.readyState === 'open', null, { timeout: 60000 });
    await page.evaluate(() => { window.__ivncWebRTC.signaling.ondisconnect = null; window.__ivncWebRTC.signaling.destroy(); });
    assert.equal(await page.evaluate(() => window.__ivncWebRTC._send_channel.readyState), 'open');
    const c = (await context.cookies()).find(c => c.name.startsWith('ivnc_session_'));
    const auth = await request(plain, '/auth/session', { headers: { Cookie: `${c.name}=${c.value}` } });
    await request(plain, '/auth/logout', { method: 'POST', headers: { Cookie: `${c.name}=${c.value}`, Origin: plain.origin, 'X-IVNC-CSRF': auth.json.csrf } });
    await waitFor('media revoked after signaling already ended', async () => (await request(plain, '/health')).json.connections === 0);
    pass('live media/input session is revoked even after its signaling socket closed');
    await formLogin(page, plain);
    const viewCookie = (await context.cookies()).find(c => c.name.startsWith('ivnc_session_'));
    const viewStatus = await request(plain, '/auth/session', { headers: { Cookie: `${viewCookie.name}=${viewCookie.value}` } });
    const viewCredentials = { Cookie: `${viewCookie.name}=${viewCookie.value}`, Origin: plain.origin, 'X-IVNC-CSRF': viewStatus.json.csrf };
    const app = await request(plain, '/api/apps', { method: 'POST', headers: viewCredentials, body: { name: 'Authenticated view', app_type: 'desktop', exec_command: 'sleep 600' } });
    assert.equal(app.status, 201); const appPath = `/api/apps/${app.json.app.id}`;
    assert.equal((await request(plain, appPath + '/sharing', { method: 'PUT', headers: viewCredentials, body: { enabled: true, password: 'view-sharing-secret' } })).status, 200);
    assert.equal((await request(plain, appPath + '/start', { method: 'POST', headers: viewCredentials })).status, 200);
    const view = await request(plain, appPath + '/view', { method: 'POST', headers: viewCredentials });
    assert.equal(view.status, 200);
    await page.locator(`.toolbar-list [data-key="app:${app.json.app.id}"]`).waitFor();
    await page.locator(`.toolbar-list [data-key="app:${app.json.app.id}"]`).click();
    await page.waitForFunction(() => document.querySelector('.app-stream iframe')?.contentWindow?.__ivncWebRTC?._send_channel?.readyState === 'open', null, { timeout: 60000 });
    const bridge = new WebSocket(plain.origin.replace(/^http/, 'ws') + view.json.url + 'webrtc', { headers: viewCredentials }); sockets.push(bridge);
    await new Promise((resolve, reject) => { bridge.once('open', resolve); bridge.once('error', reject); });
    await request(plain, '/auth/logout', { method: 'POST', headers: viewCredentials });
    await waitFor('application gateway revocation', () => bridge.readyState === WebSocket.CLOSED);
    assert.equal((await request(plain, view.json.url + 'ui-config', { headers: viewCredentials })).status, 401);
    await request(plain, appPath + '/stop', { method: 'POST', headers: basic(plain) });
    pass('Cookie login reaches a real shared-app iframe and revokes its independent gateway socket');
    await context.close(); await browser.close();
    const credentials = await loginApi(plain), socket = await terminal(plain, credentials);
    assert.equal((await request(plain, '/api/change-password', { method: 'POST', headers: credentials, body: { new_password: 'new-test-session-secret' } })).status, 200);
    await waitFor('password change closes old terminal', () => socket.readyState === WebSocket.CLOSED);
    assert.equal((await request(plain, '/api/apps', { headers: credentials })).status, 401);
    assert.equal((await request(plain, '/api/apps', { headers: basic(plain) })).status, 401);
    plain.password = 'new-test-session-secret'; const newCredentials = await loginApi(plain);
    assert.equal((await request(plain, '/api/apps', { headers: newCredentials })).status, 200);
    await stopServer(plain); await launchServer(plain); plain.password = originalPassword;
    assert.equal((await request(plain, '/api/apps', { headers: newCredentials })).status, 401);
    assert.equal((await request(plain, '/api/apps', { headers: basic(plain) })).status, 200);
    pass('password changes revoke sessions/Basic sockets; process restart ends in-memory sessions');
    const short = await startServer('expiry', { idle: 1, maximum: 2 }), expiring = await loginApi(short);
    const expiringSocket = await terminal(short, expiring);
    await waitFor('idle expiration closes an otherwise live terminal', () => expiringSocket.readyState === WebSocket.CLOSED, 5000);
    assert.equal((await request(short, '/api/apps', { headers: expiring })).status, 401);
    pass('idle expiration closes live sockets without another HTTP request');
    const open = await startServer('disabled', { enabled: false });
    assert.equal((await request(open, '/api/apps')).status, 200);
    const noAuthSocket = await terminal(open, {}); noAuthSocket.close();
    pass('authentication-disabled installations retain API and terminal access');
    console.log(`${passed} authentication smoke checks passed. Artifacts: ${work}`);
} catch (error) {
    let screenshot = 0;
    for (const browser of browsers) for (const context of browser.contexts()) for (const page of context.pages()) {
        await page.screenshot({ path: path.join(work, `failure-${screenshot++}.png`) }).catch(() => {});
        console.error('Failed page:', new URL(page.url()).pathname, await page.locator('#error').textContent().catch(() => ''));
    }
    console.error(`Authentication smoke failed. Artifacts: ${work}`); throw error;
} finally {
    for (const socket of sockets) socket.terminate();
    for (const browser of browsers) await browser.close().catch(() => {});
    for (const server of servers) await stopServer(server);
    if (process.env.CLEAN_TMP === '1') rmSync(work, { recursive: true, force: true });
}
