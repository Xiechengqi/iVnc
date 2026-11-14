import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { createVideoPlayback } from '../web/ivnc/video-playback.mjs';
import { prepareVideoOffer } from '../web/ivnc/video-capabilities.mjs';

class Video extends EventTarget {
    paused = true; readyState = 0; videoWidth = 0; videoHeight = 0; srcObject = null;
    mode = 'normal';
    play() {
        if (this.mode === 'blocked') return Promise.reject(new DOMException('blocked', 'NotAllowedError'));
        if (this.mode === 'pending') return new Promise(() => {});
        this.paused = false;
        return Promise.resolve();
    }
    frame() { this.paused = false; this.readyState = 4; this.videoWidth = 640; this.videoHeight = 480; this.dispatchEvent(new Event('playing')); }
}
function setup(videoStats = { bytesReceived: 0, framesDecoded: 0 }) {
    const video = new Video(), doc = new EventTarget(), changes = [], diagnostics = [];
    doc.hidden = false;
    let clock = 0, keyframes = 0;
    const player = createVideoPlayback({ video, documentRef: doc, timeoutMs: 40, intervalMs: 5,
        now: () => clock, onChange: s => changes.push(s.state), diagnostic: s => diagnostics.push(s),
        getStats: async () => ({ video: videoStats }), requestKeyframe: () => keyframes++,
    });
    return { video, doc, player, changes, diagnostics, time: n => { clock = n; }, keyframes: () => keyframes };
}

test('transport success waits for a displayed frame; idle video stays healthy', async () => {
    const t = setup();
    try {
        t.player.attach({}); t.player.connection('connected');
        assert.equal(t.player.state(), 'waiting');
        t.video.frame();
        assert.equal(t.player.state(), 'playing');
        t.time(10000); await delay(20);
        assert.equal(t.player.state(), 'playing');
    } finally { t.player.destroy(); }
});

test('autoplay denial remains actionable after connecting and recovers on a user play', async () => {
    const t = setup();
    try {
        t.video.mode = 'blocked'; t.player.connection('connected'); t.player.attach({});
        await delay(10);
        assert.equal(t.player.state(), 'blocked');
        t.time(10000); await delay(15);
        assert.equal(t.player.state(), 'blocked');
        t.video.mode = 'normal'; await t.player.play(); t.video.frame();
        assert.equal(t.player.state(), 'playing');
        assert(t.changes.includes('blocked'));
    } finally { t.player.destroy(); }
});

for (const [name, stats, expected] of [
    ['no video packets', { bytesReceived: 0, framesDecoded: 0 }, 'no-data'],
    ['packets without decoded frames', { bytesReceived: 1000, framesDecoded: 0 }, 'decode-error'],
    ['decoded frames without display', { bytesReceived: 1000, framesDecoded: 5 }, 'playback-error'],
]) test(`first-frame timeout distinguishes ${name}`, async () => {
    const t = setup(stats);
    try {
        t.video.mode = 'pending'; t.player.attach({}); t.player.connection('connected');
        await delay(10); t.time(50); await delay(15);
        assert.equal(t.player.state(), expected);
        assert.equal(t.keyframes(), 1);
        t.video.frame(); assert.equal(t.player.state(), 'playing');
    } finally { t.player.destroy(); }
});

test('hidden views do not fail and activation resumes a browser-paused stream', async () => {
    const t = setup();
    try {
        t.player.attach({}); t.player.connection('connected'); t.video.frame();
        t.player.setActive(false); t.video.paused = true; t.video.dispatchEvent(new Event('pause'));
        t.time(10000); await delay(15);
        assert.equal(t.player.state(), 'playing');
        t.player.setActive(true); await delay(1); t.video.frame();
        assert.equal(t.player.state(), 'playing'); assert(t.keyframes() > 0);
        t.player.reset(); t.time(20000); await delay(15);
        assert.equal(t.player.state(), 'connecting');
        assert.equal(t.video.srcObject, null);
    } finally { t.player.destroy(); }
});

test('late play rejection from a replaced stream cannot block the new stream', async () => {
    const t = setup();
    try {
        let reject;
        t.video.play = () => new Promise((_, r) => { reject = r; });
        t.player.attach({ old: true }); const oldReject = reject;
        t.video.play = () => { t.video.paused = false; return Promise.resolve(); };
        t.player.attach({ newer: true }); t.player.connection('connected'); t.video.frame();
        oldReject(new DOMException('old', 'NotAllowedError')); await delay(5);
        assert.equal(t.player.state(), 'playing');
        t.player.destroy(); t.video.dispatchEvent(new Event('error'));
        assert.equal(t.player.state(), 'playing');
    } finally { t.player.destroy(); }
});

const offer = { type: 'offer', sdp: 'v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 98\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1;profile-level-id=42e01f\r\na=rtpmap:98 VP8/90000\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=fmtp:111 useinbandfec=1\r\n' };
test('receive level is raised only after a positive WebRTC-specific capability probe', async () => {
    const probes = [];
    const upgraded = await prepareVideoOffer(offer, { decodingInfo: async config => { probes.push(config); return { supported: true }; } });
    assert(upgraded.sdp.includes('profile-level-id=42e034'));
    assert(upgraded.sdp.includes('a=fmtp:111 useinbandfec=1'));
    assert.equal(probes[0].type, 'webrtc');
    assert.equal(probes[0].video.width, 4096);
    assert.deepEqual(await prepareVideoOffer(offer, {}), offer);
    assert.deepEqual(await prepareVideoOffer(offer, { decodingInfo: async () => { throw Error('unsupported API'); } }), offer);
    assert.deepEqual(await prepareVideoOffer(offer, { decodingInfo: async () => ({supported: false}) }), offer);
    assert.equal(offer.sdp.includes('42e01f'), true, 'the original offer remains available if setLocalDescription refuses changes');
});

test('missing Safari frame counters do not establish a decoder error', async () => {
    const t = setup({ bytesReceived: 1000 });
    try {
        t.video.mode = 'pending'; t.player.attach({}); t.player.connection('connected');
        await delay(10); t.time(50); await delay(15);
        assert.equal(t.player.state(), 'playback-error');
    } finally { t.player.destroy(); }
});

test('old rendered events cannot clear a terminal connection or format error', async () => {
    const t = setup();
    try {
        t.player.attach({}); t.player.connection('connected'); t.video.frame();
        for (const state of ['unsupported', 'connection-error']) {
            t.player.fail(state, 'regression');
            t.video.frame(); t.video.dispatchEvent(new Event('error')); t.video.dispatchEvent(new Event('pause'));
            await delay(10);
            assert.equal(t.player.state(), state);
            assert.equal(t.player.firstFrame(), false);
        }
    } finally { t.player.destroy(); }
});

test('transient transport loss recovers and late rejection of an older play attempt is ignored', async () => {
    const t = setup();
    try {
        let reject;
        t.video.play = () => new Promise((_, r) => { reject = r; });
        t.player.attach({}); t.player.connection('connected');
        t.video.play = () => { t.video.paused = false; return Promise.resolve(); };
        await t.player.play(); t.video.frame();
        reject(new DOMException('old attempt', 'NotAllowedError')); await delay(1);
        assert.equal(t.player.state(), 'playing');
        t.player.connection('disconnected');
        assert.equal(t.player.state(), 'connection-error');
        t.player.connection('connected');
        assert.equal(t.player.state(), 'playing');
    } finally { t.player.destroy(); }
});

test('capability probes ignore unoffered, non-H264 and inactive fmtp lines', async () => {
    let probes = 0;
    const capabilities = { decodingInfo: async () => { probes++; return { supported: true }; } };
    for (const sdp of [offer.sdp.replace('96 98', '98'), offer.sdp.replace('H264/90000', 'rtx/90000'),
        offer.sdp.replace('a=recvonly', 'a=inactive'), offer.sdp.replace('packetization-mode=1', 'packetization-mode=0')]) {
        assert.deepEqual(await prepareVideoOffer({ type: 'offer', sdp }, capabilities), { type: 'offer', sdp });
    }
    assert.equal(probes, 0);
    const medium = await prepareVideoOffer(offer, { decodingInfo: async config => ({ supported: config.video.width === 1920 }) });
    assert(medium.sdp.includes('profile-level-id=42e02a'));
});

test('replaced signaling sockets and disposed retry timers cannot renegotiate a new peer', async () => {
    const source = readFileSync(new URL('../web/ivnc/lib/signaling.js', import.meta.url), 'utf8')
        .replace(/^import .*;\s*$/gm, '').replace('export class WebRTCDemoSignaling', 'class WebRTCDemoSignaling');
    class Socket extends EventTarget {
        close() { this.closed = true; }
        send(message) { (this.messages ||= []).push(message); }
    }
    const timers = new Map(); let sequence = 0;
    let authentication = Promise.resolve(true);
    const context = { WebSocket: Socket, setInterval, clearInterval, console, window: {},
        checkAuthentication: () => authentication,
        setTimeout: fn => { const id = ++sequence; timers.set(id, fn); return id; }, clearTimeout: id => timers.delete(id) };
    vm.createContext(context); vm.runInContext(source + '\nglobalThis.Signaling=WebRTCDemoSignaling;', context);
    const signal = new context.Signaling('ws://localhost/webrtc');
    let ready = 0; signal.onready = () => ready++;
    const message = () => Object.assign(new Event('message'), { data: 'SESSION_OK' });
    signal.connect(); const old = signal._ws_conn;
    signal.connect(); let current = signal._ws_conn;
    assert(old.closed);
    old.dispatchEvent(new Event('open')); old.dispatchEvent(message());
    assert.equal(ready, 0); assert.equal(old.messages, undefined);
    current.dispatchEvent(message()); assert.equal(ready, 1);
    // A delayed authentication response must not let a previous socket's
    // close/error callback disconnect or schedule retries for its replacement.
    let resolveAuthentication;
    authentication = new Promise(resolve => { resolveAuthentication = resolve; });
    const closing = signal._onServerClose({ code: 1006 }), failing = signal._onServerError();
    let disconnects = 0; signal.ondisconnect = () => disconnects++;
    signal.connect(); current = signal._ws_conn; signal.state = 'connected';
    resolveAuthentication(true); await Promise.all([closing, failing]);
    assert.equal(disconnects, 0); assert.equal(signal.state, 'connected');
    assert.equal(signal.retry_count, 0); assert.equal(timers.size, 0);
    current.dispatchEvent(Object.assign(new Event('message'), { data: "ERROR peer '0' not found" }));
    assert.equal(timers.size, 1);
    signal.destroy(); signal.connect(); current.dispatchEvent(message());
    assert.equal(timers.size, 0); assert.equal(signal._ws_conn, null); assert.equal(ready, 1);
});
