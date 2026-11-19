// Local production UI and real Wayland Chrome applications, including A -> B.
// Optional LEGACY_IVNC_BIN exercises a peer without preview support.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { closeSync, mkdirSync, mkdtempSync, openSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const { chromium } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('playwright');
const binary = path.resolve(process.env.IVNC_BIN || path.join(root, 'target/debug/ivnc'));
const work = mkdtempSync(path.join(tmpdir(), 'ivnc-grid-smoke-'));
const chrome = process.env.CHROME_BIN || '/usr/bin/google-chrome';
const firstColors = ['#bd2836', '#297d3a', '#3044b8', '#a13095'];
const secondColors = ['#204e8f', '#b7397e', '#dfa321', '#22733b'];
const apps = [], managers = [], channels = new Map(), loads = new Map(), keys = [], keyups = [];
const password = 'multi-app-view-smoke-only';
const listen = server => new Promise((resolve, reject) => server.once('error', reject).listen(0, '127.0.0.1', resolve));
const close = server => new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });

async function wait(label, predicate, timeout = 25000) {
    const deadline = Date.now() + timeout;
    let last;
    while (Date.now() < deadline) {
        try { const value = await predicate(); if (value) return value; }
        catch (error) { last = error; }
        await delay(60);
    }
    throw Error(`${label}: ${last?.message || 'timed out'}`);
}

async function api(base, route, method = 'GET', body, expected = null) {
    const response = await fetch(base + route, { method, headers: { 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(6000) });
    const result = await response.json();
    assert(expected === null ? response.ok : response.status === expected, JSON.stringify(result));
    return result;
}

const fixture = createServer((request, response) => {
    const url = new URL(request.url, 'http://localhost');
    const id = url.searchParams.get('id');
    if (url.pathname === '/events') {
        response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
        response.write(': ready\n\n');
        channels.set(id, response);
        response.on('close', () => { if (channels.get(id) === response) channels.delete(id); });
    } else if (url.pathname === '/keydown' || url.pathname === '/keyup') {
        (url.pathname === '/keydown' ? keys : keyups).push([id, url.searchParams.get('key')]);
        response.writeHead(204).end();
    } else if (url.pathname === '/loaded') {
        loads.set(id, Number(url.searchParams.get('visits')));
        response.writeHead(204).end();
    } else {
        const index = Number(id) % firstColors.length;
        response.writeHead(200, { 'Content-Type': 'text/html' });
        response.end(`<!doctype html><title>App ${id}</title><style>html{height:100%;background:${firstColors[index]}}</style>
          <script>
            const id = new URL(location.href).searchParams.get('id');
            const db = indexedDB.open('multi-app-view-smoke', 1);
            db.onupgradeneeded = () => db.result.createObjectStore('profile');
            db.onsuccess = () => {
              // Report only after Chromium has flushed the fixture data to disk.
              const tx = db.result.transaction('profile', 'readwrite', { durability: 'strict' });
              const store = tx.objectStore('profile');
              const previous = store.get('visits');
              previous.onsuccess = () => {
                const visits = (previous.result || 0) + 1;
                store.put(visits, 'visits');
                tx.oncomplete = () => fetch('/loaded?id=' + id + '&visits=' + visits);
              };
            };
            const events = new EventSource('/events?id=' + id);
            events.onmessage = e => { document.documentElement.style.background = e.data; };
            for (const type of ['keydown', 'keyup']) {
              window.addEventListener(type, e => fetch('/' + type + '?id=' + id + '&key=' + encodeURIComponent(e.key)));
            }
          </script>`);
    }
});

async function manager(name, executable = binary) {
    const reservation = createServer();
    await listen(reservation);
    const port = reservation.address().port;
    await close(reservation);
    const env = { ...process.env };
    for (const key of ['DISPLAY', 'WAYLAND_DISPLAY', 'PULSE_SERVER', 'DBUS_SESSION_BUS_ADDRESS']) delete env[key];
    for (const key of ['HOME', 'IVNC_HOME', 'XDG_RUNTIME_DIR', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME', 'TMPDIR']) {
        env[key] = path.join(work, name, key.toLowerCase());
        mkdirSync(env[key], { recursive: true, mode: 0o700 });
    }
    const config = path.join(work, name, 'ivnc.toml');
    writeFileSync(config, `[http]\nhost="127.0.0.1"\nport=${port}\nbasic_auth_enabled=false\n[audio]\nenabled=false\nsample_rate=48000\nchannels=2\nbitrate=128000\n[shell_profile]\napps=false\nterminal=false\n`);
    const log = openSync(path.join(work, name, 'ivnc.log'), 'a', 0o600);
    const child = spawn(path.resolve(executable), ['--config', config, '--width', '1280', '--height', '720',
        '--proxy-enabled', 'false', '--proxy-autostart', 'false'], { cwd: root, env, detached: true, stdio: ['ignore', log, log] });
    closeSync(log);
    managers.push(child);
    const base = `http://127.0.0.1:${port}`;
    await wait(name + ' server', () => api(base, '/api/apps'));
    return { base, env };
}

const quote = text => "'" + text.replaceAll("'", "'\\''") + "'";
async function desktop(base, number, { independent = true, sharing = false } = {}) {
    const command = [chrome, '--user-data-dir=' + path.join(work, 'chrome-' + number), '--ozone-platform=wayland',
        '--test-type', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--no-proxy-server',
        `--app=http://127.0.0.1:${fixture.address().port}/?id=${number}`];
    if (process.getuid() === 0) command.push('--no-sandbox', '--disable-setuid-sandbox');
    const { app } = await api(base, '/api/apps', 'POST', { name: 'Chrome ' + number, app_type: 'desktop',
        exec_command: 'exec ' + command.map(quote).join(' '), independent_runtime: independent });
    apps.push({ base, id: app.id });
    if (sharing) await api(base, `/api/apps/${app.id}/sharing`, 'PUT', { enabled: true, password });
    await api(base, `/api/apps/${app.id}/start`, 'POST');
    await wait('Chrome fixture ' + number, () => channels.get(String(number)) && loads.get(String(number)));
    return app.id;
}

async function remote(base, source, id, name) {
    const { app } = await api(base, '/api/apps', 'POST', { app_type: 'remote', name, password,
        url: `${source}/remote/v1/apps/${id}` });
    apps.push({ base, id: app.id });
    await api(base, `/api/apps/${app.id}/start`, 'POST');
    await wait(name + ' connected', async () => (await api(base, `/api/apps/${app.id}`)).remote_state === 'connected');
    return app.id;
}

const tile = (page, index) => page.locator(`.app-stream.preview[data-slot="${index}"]`);
const openGroup = page => page.locator('.toolbar-multi-app').click();
const editor = page => page.locator('.multi-app-dialog');
const editGroup = page => page.locator('.multi-app-heading button').first().click();
const applyGroup = page => editor(page).locator('button[type=submit]').click();
const appTab = (page, id) => page.locator(`#taskbar [data-app-id="${id}"]`);
const activeFrame = page => page.frameLocator('.app-stream.active iframe');

async function colors(page, palette, selector = '.app-stream.preview') {
    return await page.evaluate(({ expected, selector }) => {
        const tiles = [...document.querySelectorAll(selector)];
        if (tiles.length !== expected.length) return false;
        return tiles.every(tile => {
            const frame = tile.querySelector('iframe:not([hidden])');
            const video = frame?.contentDocument?.querySelector('video');
            if (!video?.videoWidth || video.readyState < 2) return false;
            const canvas = frame.contentDocument.createElement('canvas');
            canvas.width = canvas.height = 1;
            const ctx = canvas.getContext('2d');
            ctx.drawImage(video, video.videoWidth / 2, video.videoHeight / 2, 1, 1, 0, 0, 1, 1);
            const pixel = ctx.getImageData(0, 0, 1, 1).data;
            const slot = selector === '.app-stream.active' ? 0 : Number(tile.dataset.slot);
            return expected[slot].slice(1).match(/../g)
                .every((part, index) => Math.abs(parseInt(part, 16) - pixel[index]) <= 20);
        });
    }, { expected: palette, selector }).catch(() => false);
}

async function layout(page, expected, narrow = false) {
    const boxes = await Promise.all(expected.map((_, index) => tile(page, index).boundingBox()));
    const bar = await page.locator('#taskbar').boundingBox();
    assert(boxes.every(rect => rect && rect.width > 0 && rect.height > 0 && rect.y + rect.height <= bar.y + 1));
    assert.equal(await page.locator('.app-stream.preview .app-preview-open span').count(), expected.length);
    for (const [index, name] of expected.entries()) assert.equal(await tile(page, index).locator('.app-preview-open span').textContent(), name);
    if (!narrow && expected.length === 3) {
        assert.equal(boxes[0].y, boxes[1].y);
        assert(Math.abs(boxes[2].x - (boxes[0].x + boxes[1].x) / 2) <= 1);
        assert(Math.abs(boxes[2].width - boxes[0].width) <= 1);
    }
    if (narrow) assert(boxes[0].width > boxes[1].width && boxes[0].height > boxes[1].height);
}

let browser;
try {
    await listen(fixture);
    const A = await manager('A');
    const B = await manager('B');
    const first = await desktop(A.base, 0, { independent: false });
    await api(A.base, `/api/apps/${first}/view`, 'POST', undefined, 409);
    browser = await chromium.launch({ executablePath: chrome, headless: true, env: A.env });
    const consolePage = await browser.newPage();
    await consolePage.goto(`${A.base}/console.html`);
    await consolePage.locator('#nav-apps').click();
    await consolePage.locator(`tr[data-app-id="${first}"] .btn-edit`).click();
    await consolePage.locator('#f-independent-runtime').check();
    await consolePage.locator('#modal-save').click();
    await consolePage.locator('#restart-notice').waitFor({ state: 'visible' });
    assert.equal((await api(A.base, `/api/apps/${first}`)).sharing.restart_required, true);
    await consolePage.locator('#restart-now').click();
    await wait('same Chrome profile after moving to its worker', () => loads.get('0') >= 2 && channels.has('0'));
    const policy = await api(A.base, `/api/apps/${first}/sharing`);
    assert(!policy.enabled && !policy.restart_required);
    await consolePage.close();
    const denied = await api(A.base, `/remote/v1/apps/${first}/authenticate`, 'POST', { password }, 401);
    assert.equal(denied.code, 'auth_invalid');
    const second = await desktop(A.base, 1), third = await desktop(A.base, 2);
    const origin = await desktop(B.base, 3, { sharing: true });
    const fourth = await remote(A.base, B.base, origin, 'Remote Chrome');
    let old;
    if (process.env.LEGACY_IVNC_BIN) {
        const legacy = await manager('legacy', process.env.LEGACY_IVNC_BIN);
        const source = await desktop(legacy.base, 4, { independent: false, sharing: true });
        old = await remote(A.base, legacy.base, source, 'Legacy Chrome');
    }
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    await page.goto(A.base);
    await page.waitForFunction(id => !!document.querySelector(`[data-app-id="${id}"]`), fourth);
    await openGroup(page);
    await editor(page).locator('select').nth(0).selectOption(first);
    await editor(page).locator('select').nth(1).selectOption(second);
    if (old) assert.equal(await editor(page).locator(`option[value="${old}"]`).count(), 0);
    await applyGroup(page);
    const names = ['Chrome 0', 'Chrome 1', 'Chrome 2', 'Remote Chrome'];
    await wait('two Chrome streams', () => colors(page, firstColors.slice(0, 2)));
    await layout(page, names.slice(0, 2));
    for (const [index, id] of [third, fourth].entries()) {
        await editGroup(page);
        await editor(page).locator('button').filter({ hasText: /添加位置|Add slot/ }).click();
        await editor(page).locator('select').nth(index + 2).selectOption(id);
        await applyGroup(page);
        await wait(`${index + 3} Chrome streams`, () => colors(page, firstColors.slice(0, index + 3)));
        await layout(page, names.slice(0, index + 3));
    }
    console.log('PASS real 2, 3 and 4 app layouts, including A -> B');
    assert(await page.locator('.app-stream.preview iframe').evaluateAll(frames => frames.every(frame =>
        frame.inert && frame.tabIndex === -1 && frame.contentWindow.__ivncViewActive === false
        && frame.contentWindow.__ivncWebRTC._audioMuted)));
    await tile(page, 1).locator('button').focus();
    await page.keyboard.press('q');
    assert.equal(keys.length, 0);
    await page.evaluate(() => {
        window.savedFrame = document.querySelector('.app-stream.preview[data-slot="1"] iframe');
        window.savedPeer = savedFrame.contentWindow.__ivncWebRTC.peerConnection;
    });
    await tile(page, 1).locator('button').click();
    await page.waitForFunction(() => window.savedFrame.parentElement.classList.contains('active')
        && window.savedFrame.contentWindow.__ivncViewActive);
    assert(await page.evaluate(() => savedFrame.contentWindow.__ivncWebRTC.peerConnection === savedPeer));
    await activeFrame(page).locator('#overlayInput').click();
    await page.keyboard.press('q');
    await wait('input reaches only the selected Chrome', () => keys.some(([id, key]) => id === '1' && key === 'q'));
    assert(keys.every(([id]) => id === '1'));
    await openGroup(page);
    assert(await page.evaluate(() => savedFrame.contentWindow.__ivncWebRTC.peerConnection === savedPeer));
    console.log('PASS readonly preview, live connection reuse and input routed to the selected application');

    const control = await browser.newPage({ viewport: { width: 1200, height: 750 } });
    await control.goto(A.base);
    assert.equal(await control.locator('.app-stream.preview').count(), 0);
    await appTab(control, first).click();
    await wait('second page controls first app', () => colors(control, [firstColors[0]], '.app-stream.active'));
    await activeFrame(control).locator('#overlayInput').click();
    const firstControl = activeFrame(control).locator('#app');
    await firstControl.evaluate(() => {
        if (!window.__ivncWebRTC.sendDataChannelMessage('kd,65505')) throw Error('Control channel is closed');
    });
    await wait('modifier held on the controlling page', () => keys.some(([id, key]) => id === '0' && key === 'Shift'));
    const previewUrl = await tile(page, 0).locator('iframe').getAttribute('src');
    await tile(page, 1).locator('button').click();
    await page.waitForFunction(() => document.querySelectorAll('.app-streams iframe').length === 1
        && window.savedFrame === document.querySelector('.app-stream.active iframe'), undefined, { timeout: 25000 });
    await wait('released token no longer opens', async () => (await fetch(new URL(previewUrl, A.base))).status === 410);
    await delay(300);
    assert(!keyups.some(([id, key]) => id === '0' && key === 'Shift'), 'A preview release reset another viewer’s input');
    await firstControl.evaluate(() => window.__ivncWebRTC.sendDataChannelMessage('ku,65505'));
    await wait('explicit release reaches Chrome', () => keyups.some(([id, key]) => id === '0' && key === 'Shift'));
    await control.close();
    await openGroup(page);
    await wait('released previews reopen', () => colors(page, firstColors));
    assert.notEqual(await tile(page, 0).locator('iframe').getAttribute('src'), previewUrl);
    console.log('PASS preview-only connections close after the grace period without resetting another page’s keys');

    const thirdVisits = loads.get('2');
    await api(A.base, `/api/apps/${third}/stop`, 'POST');
    await wait('stopped app leaves a placeholder in slot three', async () => await tile(page, 2).locator('iframe').count() === 0
        && await tile(page, 2).locator('button').isDisabled());
    await api(A.base, `/api/apps/${third}/start`, 'POST');
    await wait('restarted app takes its previous slot', async () => loads.get('2') > thirdVisits && await colors(page, firstColors));
    console.log('PASS a stopped application retains its slot and reconnects after restart');

    const start = performance.now();
    for (const [index, color] of secondColors.entries()) channels.get(String(index)).write(`data: ${color}\n\n`);
    await wait('all four tiles update independently', () => colors(page, secondColors));
    console.log(`PASS four independent 1280×720 updates in ${Math.round(performance.now() - start)} ms`);

    await page.setViewportSize({ width: 390, height: 710 });
    await wait('narrow layout', async () => (await tile(page, 0).boundingBox())?.width > (await tile(page, 1).boundingBox())?.width);
    await layout(page, names, true);
    await page.reload();
    await openGroup(page);
    await wait('group restored with fresh views', () => colors(page, secondColors));
    await layout(page, names, true);
    const revoke = await api(B.base, `/api/apps/${origin}/sharing`, 'PUT', { enabled: false });
    assert(revoke.revoked && !revoke.restart_required);
    await wait('revoked view ends in its existing cell', async () => {
        const message = await tile(page, 3).locator('.app-stream-status').textContent();
        return /撤销|重新输入|revoked|password/i.test(message || '');
    });
    assert.equal(await page.locator('.app-stream.preview').count(), 4);
    assert(await api(B.base, `/api/apps/${origin}`).then(app => app.status === 'running'));
    console.log('PASS narrow layout, saved group and isolated remote revocation');

    await editGroup(page);
    for (let remaining = 4; remaining > 1; remaining--) {
        await editor(page).getByRole('button', { name: /移除|Remove/ }).last().click();
    }
    await editor(page).getByRole('button', { name: /打开应用|Open application/ }).click();
    await wait('editing down to one app opens it directly', () => colors(page, [secondColors[0]], '.app-stream.active'));
    await openGroup(page);
    assert.equal(await editor(page).locator('select').count(), 2);
    assert(await editor(page).locator('button[type=submit]').isDisabled());
    await page.keyboard.press('Escape');
    console.log('PASS reducing a group to one app restores its single-window controls');

    if (old) {
        await appTab(page, old).click();
        await wait('older remote source still plays in single view', () => colors(page, [firstColors[0]], '.app-stream.active'));
        await activeFrame(page).locator('#overlayInput').click();
        await page.keyboard.press('z');
        await wait('older remote source still accepts input', () => keys.some(([id, key]) => id === '4' && key === 'z'));
        console.log('PASS an older remote source stays usable in single-window mode');
    }
} catch (error) {
    console.error('Multi-app view smoke logs:', work);
    process.env.KEEP_TMP = '1';
    throw error;
} finally {
    await browser?.close();
    for (const { base, id } of apps.reverse()) await api(base, `/api/apps/${id}/stop`, 'POST').catch(() => {});
    for (const child of managers) {
        if (child.exitCode !== null || child.signalCode !== null) continue;
        const exited = new Promise(resolve => child.once('exit', resolve));
        try { process.kill(-child.pid, 'SIGTERM'); } catch (_) {}
        await Promise.race([exited, delay(5000, undefined, { ref: false })]);
        if (child.exitCode === null && child.signalCode === null) {
            try { process.kill(-child.pid, 'SIGKILL'); } catch (_) {}
            await exited;
        }
    }
    for (const stream of channels.values()) stream.end();
    await close(fixture);
    if (process.env.KEEP_TMP !== '1') rmSync(work, { recursive: true, force: true });
}
