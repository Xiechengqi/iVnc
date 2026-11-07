// Drives a real headless Chrome through an iVnc application view and asserts
// that WebRTC media, the input data channel and revocation all work end to end.
// Usage: node scripts/remote_view_smoke.mjs <view-url> <workdir> [--no-input]
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { openSync, closeSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';

const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');

const [viewUrl, root, ...flags] = process.argv.slice(2);
assert(viewUrl && root, 'usage: remote_view_smoke.mjs <view-url> <workdir>');
const targetUrl = new URL(viewUrl);
targetUrl.username = ''; targetUrl.password = ''; targetUrl.search = '';

const server = net.createServer();
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
await new Promise(resolve => server.close(resolve));
const log = openSync(path.join(root, 'browser.log'), 'a', 0o600);
const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu',
    '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
    '--ignore-certificate-errors', `--remote-debugging-port=${port}`,
    `--user-data-dir=${path.join(root, 'browser')}`,
    // Navigate on launch and attach only to this page, ignoring extension targets.
    '--window-size=1280,800', viewUrl
], { stdio: ['ignore', log, log] });
closeSync(log);

let socket;
const pending = new Map();
let id = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => {
    const key = ++id;
    const timer = setTimeout(() => { pending.delete(key); reject(Error(`CDP timeout: ${method}`)); }, 20000);
    pending.set(key, { resolve, reject, timer });
    socket.send(JSON.stringify({ id: key, method, params }));
});
const evaluate = async (expression) => {
    const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
};
const waitFor = async (name, predicate, timeoutMs = 60000) => {
    const deadline = Date.now() + timeoutMs;
    let last;
    while (Date.now() < deadline) {
        last = await predicate();
        if (last) return last;
        await delay(250);
    }
    throw Error(`timed out waiting for ${name}`);
};

try {
    let page;
    for (let attempt = 0; attempt < 100; attempt++) {
        try {
            const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
            page = targets.find(t => {
                if (t.type !== 'page') return false;
                const url = new URL(t.url);
                return url.origin === targetUrl.origin && url.pathname === targetUrl.pathname;
            });
            if (page?.webSocketDebuggerUrl) break;
        } catch { /* Chrome not ready yet */ }
        await delay(100);
    }
    assert(page?.webSocketDebuggerUrl, 'isolated Chrome did not start');
    socket = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    socket.onmessage = event => {
        const result = JSON.parse(event.data);
        const item = pending.get(result.id);
        if (!item) return;
        pending.delete(result.id);
        clearTimeout(item.timer);
        if (result.error) item.reject(Error(result.error.message));
        else item.resolve(result.result);
    };
    socket.onclose = () => {
        for (const item of pending.values()) { clearTimeout(item.timer); item.reject(Error('CDP socket closed')); }
        pending.clear();
    };
    await call('Runtime.enable');
    await call('Page.enable');

    // 1. Signaling + WebRTC must reach connected state.
    await waitFor('WebRTC connection', () => evaluate(
        `window.__ivncWebRTC && window.__ivncWebRTC.peerConnection?.connectionState`));
    const state = await waitFor('connected state', () => evaluate(
        `window.__ivncWebRTC.peerConnection.connectionState === 'connected' ? true : null`));
    assert.equal(state, true);
    console.log('PASS webrtc-connected');
    const remotePort = await evaluate(`(async () => {
        const stats = await window.__ivncWebRTC.peerConnection.getStats();
        const pair = [...stats.values()].find(r => r.type === 'candidate-pair' && r.state === 'succeeded');
        return stats.get(pair?.remoteCandidateId)?.port;
    })()`);
    assert.equal(remotePort, Number(new URL(viewUrl).port || (viewUrl.startsWith('https:') ? 443 : 80)));
    console.log('PASS media-uses-view-origin');

    // 2. Video frames must actually flow through the relay.
    await waitFor('video dimensions', () => evaluate(
        `(() => { const v = document.querySelector('video'); return v && v.videoWidth > 0 ? v.videoWidth : null; })()`));
    const bytes0 = await evaluate(`(async () => {
        let bytes = 0;
        for (const r of (await window.__ivncWebRTC.peerConnection.getStats()).values())
            if (r.type === 'inbound-rtp' && r.kind === 'video') bytes += r.bytesReceived;
        return bytes;
    })()`);
    await delay(2000);
    const bytes1 = await evaluate(`(async () => {
        let bytes = 0;
        for (const r of (await window.__ivncWebRTC.peerConnection.getStats()).values())
            if (r.type === 'inbound-rtp' && r.kind === 'video') bytes += r.bytesReceived;
        return bytes;
    })()`);
    assert(bytes1 > bytes0, `video bytes did not increase (${bytes0} -> ${bytes1})`);
    console.log(`PASS video-flowing bytes=${bytes1}`);
    if (flags.includes('--audio')) {
        await waitFor('application audio', () => evaluate(`(async () => {
            return [...(await window.__ivncWebRTC.peerConnection.getStats()).values()]
                .some(r => r.type === 'inbound-rtp' && r.kind === 'audio' && r.bytesReceived > 0);
        })()`));
        console.log('PASS application-audio-received');
    }

    // 3. A second viewer cannot bypass the UI to take over, resize or upload.
    await waitFor('data channel', () => evaluate(`window.__ivncWebRTC._send_channel?.readyState === 'open'`));
    if (flags.includes('--viewer') || flags.includes('--owner')) {
        await delay(1500);
        assert.equal(await evaluate(`window.__ivncCanControl`), false);
        if (flags.includes('--owner')) {
            await evaluate(`window.__ivncWebRTC.signaling._ws_conn.send(JSON.stringify({ivnc_control:'takeover'}))`);
            await waitFor('owner takeover', () => evaluate(`window.__ivncCanControl === true`));
            console.log('PASS owner-takeover');
        } else {
            const before = await evaluate(`document.querySelector('video').videoWidth`);
            await evaluate(`(() => {
                const w = window.__ivncWebRTC;
                w.signaling._ws_conn.send(JSON.stringify({ivnc_control:'takeover'}));
                w._send_channel.send('r,310x240');
                w._send_channel.send('FILE_UPLOAD_START:viewer-forbidden.txt:4');
                w._send_channel.send(new Uint8Array([1, 116, 101, 115, 116]));
                w._send_channel.send('FILE_UPLOAD_END:viewer-forbidden.txt');
            })()`);
            await delay(1500);
            assert.equal(await evaluate(`window.__ivncCanControl`), false);
            assert.equal(await evaluate(`document.querySelector('video').videoWidth`), before);
            assert.equal(await evaluate(`fetch(location.pathname + 'app/files').then(r=>r.json()).then(r=>r.files.some(f=>f.name==='viewer-forbidden.txt'))`), false);
            console.log('PASS viewer-cannot-takeover-resize-upload');
        }
    } else {
        await waitFor('control granted', () => evaluate(`window.__ivncCanControl === true`), 15000);
        console.log('PASS datachannel-control');
        await evaluate(`(() => {
            const w = window.__ivncWebRTC, bytes = new TextEncoder().encode('iVnc upload 中文');
            w.sendDataChannelMessage('FILE_UPLOAD_START:smoke-upload.txt:' + bytes.length);
            w.sendDataChannelMessage(new Uint8Array([1, ...bytes]));
            w.sendDataChannelMessage('FILE_UPLOAD_END:smoke-upload.txt');
        })()`);
        await waitFor('uploaded file', () => evaluate(`fetch(location.pathname + 'app/files/smoke-upload.txt').then(r=>r.ok?r.text():'').then(s=>s==='iVnc upload 中文')`));
        console.log('PASS upload-download-roundtrip');
    }

    if (!flags.includes('--no-input') && !flags.includes('--viewer')) {
        // 4. A real mouse click must travel browser -> A -> B -> worker compositor.
        const point = await evaluate(`(() => {
            const v = document.querySelector('video').getBoundingClientRect();
            return { x: Math.round(v.left + v.width / 2), y: Math.round(v.top + v.height / 2) };
        })()`);
        for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
            await call('Input.dispatchMouseEvent', {
                type, x: point.x, y: point.y,
                button: type === 'mouseMoved' ? 'none' : 'left',
                buttons: type === 'mouseReleased' ? 0 : 1,
                clickCount: type === 'mouseMoved' ? 0 : 1,
            });
        }
        console.log('PASS input-click-sent');
    }

    // 5. Wait for revocation: B disables sharing; the view must be force-disconnected.
    if (process.env.REMOTE_VIEW_EXPECT_REVOKE === '1') {
        await waitFor('revocation disconnect', () => evaluate(
            `window.__FORCE_DISCONNECTED__ === true ? true : null`), 120000);
        console.log('PASS revocation-disconnect');
    }

    const shot = await call('Page.captureScreenshot', { format: 'png' });
    writeFileSync(path.join(root, 'view.png'), Buffer.from(shot.data, 'base64'));
    console.log('PASS screenshot-saved');
} finally {
    browser.kill('SIGKILL');
}
