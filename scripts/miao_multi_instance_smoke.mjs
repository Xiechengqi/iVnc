// Local only: two real isolated iVnc/Miao instances in one browser cookie jar.
// Started by miao_smoke.py --browser or --multi-instance-only.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';

const require = createRequire(new URL('../web/ivnc/package.json', import.meta.url));
const engines = require('playwright');
const origins = process.argv.slice(2);
assert.equal(origins.length, 2);
const credentials = Buffer.from(process.env.MIAO_SMOKE_AUTH.replace(/^Basic /i, ''), 'base64').toString();
const separator = credentials.indexOf(':');
assert(separator > 0);

async function status(frame) {
    return frame.evaluate(async () => {
        const response = await fetch('/api/auth/status');
        if (!response.ok) throw Error(`Miao status returned ${response.status}`);
        return (await response.json()).data;
    });
}

for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) {
    const browser = await engines[name].launch({ headless: true,
        ...(name === 'chromium' && process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
    try {
        const context = await browser.newContext({ locale: 'zh-CN' });
        context.setDefaultTimeout(15000);
        const pages = [], frames = [], failures = [], restarting = new Set();
        // Model cookies retained from before the upgrade; the panel must enter
        // without asking the user to clear cookies or reset proxy settings.
        await context.addCookies([{ name: 'miao_session_legacy', value: 'obsolete',
            url: origins[0], httpOnly: true, sameSite: 'Strict' }]);
        for (const origin of origins) {
            const page = await context.newPage();
            page.on('pageerror', error => failures.push(error.message));
            page.on('response', response => {
                if (!restarting.has(page) && new URL(response.url()).pathname.startsWith('/proxy/') && response.status() === 401)
                    failures.push(`unexpected Miao 401: ${response.url()}`);
            });
            pages.push(page);
            await page.goto(origin + '/auth/login?next=%2Fconsole');
            await page.locator('#username').fill(credentials.slice(0, separator));
            await page.locator('#password').fill(credentials.slice(separator + 1));
            await page.locator('#submit').click();
            await page.waitForURL(origin + '/console');
            await page.locator('#nav-apps').waitFor();
            await page.evaluate(() => {
                const frame = document.createElement('iframe');
                frame.id = 'miao-multi-test';
                frame.src = '/proxy/';
                frame.style.cssText = 'position:fixed;inset:0;width:100%;height:100%;z-index:99999;background:white';
                document.body.appendChild(frame);
            });
            const frame = await (await page.locator('#miao-multi-test').elementHandle()).contentFrame();
            await frame.locator('.topbar').waitFor();
            await frame.evaluate(() => {
                window.__miaoAuthFailures = 0;
                window.addEventListener('miao-auth-expired', () => window.__miaoAuthFailures++);
                new MutationObserver(() => {
                    if (document.querySelector('.auth-page')) window.__miaoAuthFailures++;
                }).observe(document.getElementById('root'), { childList: true, subtree: true });
            });
            frames.push(frame);
        }
        const sessions = await Promise.all(frames.map(status));
        assert(sessions.every(session => session.authenticated && session.password_required === false));
        const cookies = (await context.cookies()).filter(cookie => cookie.name.startsWith('ivnc_miao_'));
        assert.equal(cookies.length, 2, 'each published port needs its own Miao cookie');
        assert.notEqual(cookies[0].name.split('miao_session_')[0], cookies[1].name.split('miao_session_')[0]);
        assert(cookies.every(cookie => cookie.httpOnly && cookie.sameSite === 'Strict' && !cookie.secure));
        console.log(`PASS ${name}: two real Miao iframes share a browser and retain separate port-scoped sessions`);

        // Observe more than two actual 15-second AuthGate polling intervals.
        // Recheck both sessions throughout, without mocking timers or APIs.
        for (let round = 0; round < 8; round++) {
            await delay(4000);
            for (let i = 0; i < frames.length; i++) {
                const current = await status(frames[i]);
                assert(current.authenticated);
                assert.equal(current.csrf, sessions[i].csrf);
                assert.equal(await frames[i].evaluate(() => window.__miaoAuthFailures), 0);
                assert.equal(await frames[i].locator('.auth-page').count(), 0);
            }
        }
        console.log(`PASS ${name}: both sessions survive repeated live auth polling without sign-in flicker`);

        await frames[0].goto(origins[0] + '/proxy/');
        await frames[0].locator('.topbar').waitFor();
        assert.equal((await status(frames[0])).csrf, sessions[0].csrf);
        assert.equal((await status(frames[1])).csrf, sessions[1].csrf);
        // Restarting intentionally invalidates this panel's old session. Any
        // old in-flight request may return 401; the other panel must stay valid.
        restarting.add(pages[0]);
        await pages[0].evaluate(async () => {
            await window.__ivncAuthReady;
            const response = await fetch('/api/proxy/restart', { method: 'POST' });
            if (!response.ok) throw Error(`Miao restart returned ${response.status}`);
        });
        await frames[0].goto(origins[0] + '/proxy/');
        await frames[0].locator('.topbar').waitFor();
        assert((await status(frames[0])).authenticated);
        assert.notEqual((await status(frames[0])).csrf, sessions[0].csrf);
        assert.equal((await status(frames[1])).csrf, sessions[1].csrf);
        assert.equal(await frames[1].evaluate(() => window.__miaoAuthFailures), 0);
        assert.deepEqual(failures, []);
        console.log(`PASS ${name}: refreshing or restarting one panel leaves the other panel's session intact`);
        await context.close();
    } finally {
        await browser.close();
    }
}
