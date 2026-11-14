// Drives a real headless Chrome through an iVnc application view and asserts
// that WebRTC media, the input data channel and revocation all work end to end.
// Usage: node scripts/remote_view_smoke.mjs <view-url> <workdir> [flags]
//   --no-input  skip the upload and the mouse click
//   --audio     also require application audio
//   --embed     open the view as a stream tab does: a same-origin iframe with
//               ?embed=1 inside a parent page, and assert that the view hides
//               its own taskbar and connection indicator, hides playback UI
//               after the first frame, and
//               reports readiness and its state to the parent over postMessage
// REMOTE_VIEW_EXPECT_REVOKE=1 waits until the server ends the view.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { openSync, closeSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { createRequire } from 'node:module';
import { setTimeout as delay } from 'node:timers/promises';

const { WebSocket } = createRequire(new URL('../web/ivnc/package.json', import.meta.url))('ws');

const [viewUrl, root, ...flags] = process.argv.slice(2);
assert(viewUrl && root, 'usage: remote_view_smoke.mjs <view-url> <workdir> [flags]');
const embed = flags.includes('--embed');
const targetUrl = new URL(viewUrl);
const credentials = targetUrl.username
    ? { username: decodeURIComponent(targetUrl.username), password: decodeURIComponent(targetUrl.password) }
    : null;
targetUrl.username = ''; targetUrl.password = ''; targetUrl.search = '';
const pagePath = embed ? '/ui-config' : targetUrl.pathname;
// Authenticate through the same form as users. URL credentials and injected
// Basic headers cannot exercise the browser's Cookie session or iframe sharing.
const launchUrl = new URL(targetUrl);
launchUrl.pathname = credentials ? '/auth/login' : pagePath;
if (credentials) launchUrl.searchParams.set('next', pagePath);

const server = net.createServer();
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
await new Promise(resolve => server.close(resolve));
const log = openSync(path.join(root, 'browser.log'), 'a', 0o600);
const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu',
    '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
    '--ignore-certificate-errors', '--autoplay-policy=no-user-gesture-required',
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${path.join(root, 'browser')}`,
    // Navigate on launch and attach only to this page, ignoring extension targets.
    '--window-size=1280,800', launchUrl.href
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
// Evaluates in the top-level page.
const evaluateTop = async (expression) => {
    const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
};
// Evaluates in the view: the page itself, or the embedded same-origin iframe,
// whose window and document shadow the parent's.
const evaluate = (expression) => evaluateTop(embed
    ? `((frame) => ((window, document) => (${expression}))(frame.contentWindow, frame.contentWindow.document))(document.getElementById('ivnc-view'))`
    : expression);
const waitFor = async (name, predicate, timeoutMs = 60000) => {
    const deadline = Date.now() + timeoutMs;
    let error;
    while (Date.now() < deadline) {
        try {
            const value = await predicate();
            if (value) return value;
        } catch (e) { error = e; /* the view may still be loading */ }
        await delay(250);
    }
    throw Error(`timed out waiting for ${name}${error ? `: ${error.message}` : ''}`);
};
const parentMessages = (type) => evaluateTop(
    `window.__viewMessages.filter(m => m && m.type === ${JSON.stringify(type)})`);

try {
    let page;
    for (let attempt = 0; attempt < 100; attempt++) {
        try {
            const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
            page = targets.find(t => {
                if (t.type !== 'page') return false;
                const url = new URL(t.url);
                return url.origin === targetUrl.origin &&
                    [launchUrl.pathname, pagePath].includes(url.pathname);
            });
            if (page?.webSocketDebuggerUrl) break;
        } catch { /* Chrome not ready yet */ }
        await delay(100);
    }
    assert(page?.webSocketDebuggerUrl, 'headless Chrome did not start');
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

    if (credentials) {
        await waitFor('login form', () => evaluateTop(
            `location.pathname === '/auth/login' && !!document.getElementById('login-form') && !!window.__ivncAuthReady`));
        await evaluateTop(`(() => {
            document.getElementById('username').value = ${JSON.stringify(credentials.username)};
            document.getElementById('password').value = ${JSON.stringify(credentials.password)};
            document.getElementById('login-form').requestSubmit();
            return true;
        })()`);
        await waitFor('authenticated destination', () => evaluateTop(
            `location.pathname === ${JSON.stringify(pagePath)} && document.readyState === 'complete'`));
    }

    if (embed) {
        // Chrome can advertise the startup target URL while its active
        // document is still about:blank (also readyState=complete). Wait for
        // the real same-origin page before inserting a relative iframe URL.
        await waitFor('parent page', () => evaluateTop(
            `location.origin === ${JSON.stringify(targetUrl.origin)} && location.pathname === ${JSON.stringify(pagePath)} && document.readyState === 'complete'`), 60000);
        // Like app-streams.js: collect same-origin messages from the view, then
        // open it full-size so page coordinates equal view coordinates.
        await evaluateTop(`(() => {
            window.__viewMessages = [];
            window.addEventListener('message', (event) => {
                const frame = document.getElementById('ivnc-view');
                if (event.origin === location.origin && event.source === frame?.contentWindow) {
                    window.__viewMessages.push(event.data);
                }
            });
            document.body.replaceChildren();
            document.body.style.margin = '0';
            const frame = document.createElement('iframe');
            frame.id = 'ivnc-view';
            frame.allow = 'autoplay; clipboard-read; clipboard-write';
            frame.style.cssText = 'position:fixed;inset:0;width:100%;height:100%;border:0';
            frame.src = ${JSON.stringify(targetUrl.pathname + '?embed=1')};
            document.body.appendChild(frame);
            return true;
        })()`);
    }

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
    assert.equal(remotePort, Number(targetUrl.port || (targetUrl.protocol === 'https:' ? 443 : 80)));
    console.log('PASS media-uses-view-origin');

    // 2. Video frames must actually flow through the relay.
    await waitFor('video dimensions', () => evaluate(
        `(() => { const v = document.querySelector('video'); return v && v.videoWidth > 0 ? v.videoWidth : null; })()`));
    const videoBytes = () => evaluate(`(async () => {
        let bytes = 0;
        for (const r of (await window.__ivncWebRTC.peerConnection.getStats()).values())
            if (r.type === 'inbound-rtp' && r.kind === 'video') bytes += r.bytesReceived;
        return bytes;
    })()`);
    const bytes0 = await videoBytes();
    await delay(2000);
    const bytes1 = await videoBytes();
    assert(bytes1 > bytes0, `video bytes did not increase (${bytes0} -> ${bytes1})`);
    console.log(`PASS video-flowing bytes=${bytes1}`);
    if (flags.includes('--audio')) {
        await waitFor('application audio', () => evaluate(`(async () => {
            return [...(await window.__ivncWebRTC.peerConnection.getStats()).values()]
                .some(r => r.type === 'inbound-rtp' && r.kind === 'audio' && r.bytesReceived > 0);
        })()`));
        console.log('PASS application-audio-received');
    }

    // 3. Embedded views leave navigation to the parent. Their playback UI is
    //    hidden only after a real frame, so autoplay/codec errors stay actionable.
    if (embed) {
        assert.equal(await evaluate(`window.__IVNC_EMBED__`), true);
        const hidden = await evaluate(`({
            taskbar: !document.getElementById('taskbar')?.isConnected,
            trigger: !document.querySelector('.taskbar-trigger'),
            connIndicator: !document.getElementById('conn-indicator')?.isConnected,
            statusDisplay: document.getElementById('status-display')?.classList.contains('hidden'),
            noWindowOverlay: !document.querySelector('.no-window-overlay'),
        })`);
        assert.deepEqual(hidden, { taskbar: true, trigger: true, connIndicator: true, statusDisplay: true, noWindowOverlay: true });
        console.log('PASS embed-hides-taskbar-indicator-status');
        const viewPath = targetUrl.pathname;
        await waitFor('ready message posted to the parent', async () =>
            (await parentMessages('ivnc-view-ready')).some(m => m.view === viewPath), 15000);
        console.log('PASS embed-posts-view-ready');
        await waitFor('connected state posted to the parent', async () =>
            (await parentMessages('ivnc-view-state')).some(m => m.state === 'connected' && m.view === viewPath), 15000);
        console.log('PASS embed-posts-view-state');
        // Select the tab, as the parent does before input.
        await evaluateTop(`document.getElementById('ivnc-view').contentWindow.postMessage({type: 'ivnc-view-active', active: true}, location.origin)`);
        await waitFor('view activation', () => evaluate(`window.__ivncViewActive === true`), 10000);
        console.log('PASS embed-view-active');
    }

    // 4. Every connection may control: upload and click through the relay.
    await waitFor('data channel', () => evaluate(`window.__ivncWebRTC._send_channel?.readyState === 'open'`));
    console.log('PASS datachannel-open');
    await waitFor('authoritative source display',()=>evaluate('window.__ivncDisplayState?.protocol===1'));
    const desktop=await evaluate('JSON.stringify(window.__ivncDisplayState.desktop)');
    await call('Emulation.setDeviceMetricsOverride',{width:720,height:1100,deviceScaleFactor:2,mobile:false});
    await waitFor('portrait local fit',()=>evaluate(`(() => {const v=document.querySelector('video').getBoundingClientRect(),a=document.querySelector('.video-container').getBoundingClientRect();return a.width===innerWidth && v.width<=a.width+1 && v.height<=a.height+1 && Math.abs(v.width/v.height-document.querySelector('video').videoWidth/document.querySelector('video').videoHeight)<.01})()`));
    assert.equal(await evaluate('JSON.stringify(window.__ivncDisplayState.desktop)'),desktop);
    await evaluate(`window.__ivncWebRTC.sendDataChannelMessage('r,3840x2160')`);
    await delay(300);
    assert.equal(await evaluate('JSON.stringify(window.__ivncDisplayState.desktop)'),desktop);
    console.log('PASS remote-source-geometry-stable-on-portrait-dpr-and-legacy-resize');
    if (!flags.includes('--no-input')) {
        // The calling script checks that the file arrived in the app's Downloads.
        const uploadName = process.env.REMOTE_VIEW_UPLOAD_NAME || 'smoke-upload.txt';
        await evaluate(`(() => {
            const w = window.__ivncWebRTC, bytes = new TextEncoder().encode('iVnc upload 中文');
            const name = ${JSON.stringify(uploadName)};
            w.sendDataChannelMessage('FILE_UPLOAD_START:' + name + ':' + bytes.length);
            w.sendDataChannelMessage(new Uint8Array([1, ...bytes]));
            w.sendDataChannelMessage('FILE_UPLOAD_END:' + name);
            return true;
        })()`);
        console.log('PASS upload-sent');

        // A real mouse click must travel browser -> A -> B -> worker compositor.
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
        if (embed) {
            await waitFor('ended state posted to the parent', async () =>
                (await parentMessages('ivnc-view-state')).some(m => m.state === 'ended'), 15000);
            console.log('PASS embed-posts-ended');
        }
    }

    const shot = await call('Page.captureScreenshot', { format: 'png' });
    writeFileSync(path.join(root, 'view.png'), Buffer.from(shot.data, 'base64'));
    console.log('PASS screenshot-saved');
} finally {
    browser.kill('SIGKILL');
}
