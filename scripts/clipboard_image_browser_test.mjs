// Local production clipboard UI regressions in Chromium and WebKit.
import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { readFileSync } from 'node:fs';
import { networkInterfaces } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const engines = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('playwright');
const root = fileURLToPath(new URL('../', import.meta.url));
const html = readFileSync(path.join(root, 'scripts/fixtures/clipboard-ui.html'), 'utf8');
const allowed = new Set(['/clipboard.js', '/clipboard-ui.js', '/overlay-manager.js', '/lib/i18n.js']);
const ip = Object.values(networkInterfaces()).flat().find(a => a.family === 'IPv4' && !a.internal)?.address;
assert(ip, 'A non-loopback IPv4 address is required');

async function visit(context, url, narrow = false) {
    context.setDefaultTimeout(10000);
    const page = await context.newPage();
    await page.goto(url);
    await page.waitForFunction(() => window.ready);
    if (narrow) await page.addStyleTag({ content: '#remote,#local{width:calc(100vw - 40px)}' });
    await page.locator('#remote').click();
    return page;
}

await test('image clipboard browser compatibility', async t => {
    const server = http.createServer((req, res) => {
        const url = new URL(req.url, 'http://localhost');
        if (allowed.has(url.pathname)) {
            res.setHeader('Content-Type', 'text/javascript');
            res.end(readFileSync(path.join(root, 'web/ivnc', url.pathname)));
        } else if (url.pathname === '/') {
            res.setHeader('Content-Type', 'text/html');
            res.end(html);
        } else {
            res.writeHead(404); res.end();
        }
    });
    await new Promise(resolve => server.listen(0, '0.0.0.0', resolve));
    const port = server.address().port;
    try {
        for (const name of (process.env.BROWSERS || 'chromium,webkit').split(',')) {
            await t.test(name, async t => {
                const browser = await engines[name].launch({
                    headless: true,
                    ...(name === 'chromium' ? { executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome' } : {}),
                });
                try {
                    await t.test('HTTP image preview, native menu, download and narrow viewport', async () => {
                        const context = await browser.newContext({ viewport: { width: 375, height: 720 }, acceptDownloads: true });
                        try {
                            const page = await visit(context, `http://${ip}:${port}/`, true);
                            assert.equal(await page.evaluate(() => isSecureContext), false);
                            await page.evaluate(() => receiveImage());
                            const image = page.locator('.clipboard-image');
                            await image.waitFor({ state: 'visible' });
                            await page.waitForFunction(() => document.querySelector('.clipboard-image').naturalWidth === 3);
                            assert.equal(await page.locator('.clipboard-copy').isVisible(), false);
                            const layout = await page.evaluate(() => ({ width: document.documentElement.scrollWidth, viewport: innerWidth }));
                            assert(layout.width <= layout.viewport);
                            const download = page.waitForEvent('download');
                            // A real click must survive the focus-triggered state update in WebKit.
                            await page.locator('.clipboard-download').click();
                            const result = await download;
                            const bytes = readFileSync(await result.path());
                            assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
                            assert.equal(bytes.readUInt32BE(16), 3);
                            assert.equal(bytes.readUInt32BE(20), 2);
                            await page.evaluate(() => document.addEventListener('contextmenu', event =>
                                queueMicrotask(() => window.menuBlocked = event.defaultPrevented), { capture: true }));
                            await image.click({ button: 'right' });
                            await page.waitForFunction(() => window.menuBlocked === false);
                        } finally { await context.close(); }
                    });

                    await t.test('trusted image retry calls the real Clipboard API', async t => {
                        const context = await browser.newContext();
                        try {
                            const page = await visit(context, `http://127.0.0.1:${port}/`);
                            await page.evaluate(() => {
                                window.actualImageWrite = navigator.clipboard.write.bind(navigator.clipboard);
                                navigator.clipboard.write = () => Promise.reject(new DOMException('Gesture required', 'NotAllowedError'));
                            });
                            await page.evaluate(() => receiveImage());
                            await page.locator('.clipboard-image').waitFor({ state: 'visible' });
                            await page.evaluate(() => {
                                window.writeCalls = [];
                                navigator.clipboard.write = items => {
                                    writeCalls.push({ trusted: navigator.userActivation.isActive, types: items[0].types });
                                    return actualImageWrite(items);
                                };
                            });
                            await page.locator('.clipboard-copy').click();
                            await page.waitForFunction(() => document.querySelector('.clipboard-assist-message').textContent.includes('Copied to'));
                            assert.deepEqual(await page.evaluate(() => writeCalls), [{ trusted: true, types: ['image/png'] }]);
                            await t.test('independent native paste preserves original pixels', {
                                // A minimal page calling write() directly has the same empty PNG
                                // pasteboard in Playwright's Linux headless WebKit. Keep the native
                                // OS assertion in Chromium; do not fake a WebKit paste event.
                                skip: name === 'webkit' && process.platform === 'linux'
                                    ? 'Linux headless WebKit exposes no PNG in native paste; actual Safari hardware remains unverified.'
                                    : false,
                            }, async () => {
                                await page.locator('#local').focus();
                                await page.keyboard.press(process.platform === 'darwin' ? 'Meta+v' : 'Control+v');
                                await page.waitForFunction(() => window.pastedImage);
                                assert.deepEqual(await page.evaluate(() => pastedImage), {
                                    width: 3, height: 2, pixel: [51, 85, 170, 255],
                                });
                            });
                        } finally { await context.close(); }
                    });

                    await t.test('typed failures and disabled sync do not expose invalid actions', async () => {
                        const context = await browser.newContext();
                        try {
                            const page = await visit(context, `http://${ip}:${port}/`);
                            let revision = 0;
                            for (const status of ['disabled', 'format', 'timeout', 'limit', 'image_limit', 'invalid_image', 'invalid_content', 'read_failed', 'busy']) {
                                await page.evaluate(data => controller.message('clipboard_status', data), { revision: ++revision, status, image: true });
                                await page.locator('.clipboard-assist').waitFor({ state: 'visible' });
                                assert.equal(await page.locator('.clipboard-copy').isVisible(), false);
                                assert.equal(await page.locator('.clipboard-download').isVisible(), false);
                                assert((await page.locator('.clipboard-assist-message').textContent()).length > 10);
                            }
                        } finally { await context.close(); }
                    });
                } finally { await browser.close(); }
            });
        }
    } finally { await new Promise(resolve => server.close(resolve)); }
});
