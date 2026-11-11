// Pure event/transport regressions; no OS clipboard or running instance touched.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../web/ivnc/clipboard.js', import.meta.url), 'utf8');
const { createClipboard, pasteShortcut } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const tick = () => new Promise(resolve => setImmediate(resolve));

function setup() {
    const listeners = new Map(), sent = [], errors = [], writes = [];
    const overlay = { readOnly: true, value: '', closest: () => null };
    let active = true;
    globalThis.document = { addEventListener: (type, fn) => listeners.set(type, fn), removeEventListener: type => listeners.delete(type), hasFocus: () => active };
    Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { clipboard: { writeText: async text => writes.push(text) } } });
    const rtc = { _send_channel: { readyState: 'open', bufferedAmount: 0 }, sendDataChannelMessage: message => { sent.push(message); return true; } };
    const controller = createClipboard({ webrtc: rtc, overlay, imeTarget: () => null, isActive: () => active, getTarget: () => 3, report: error => errors.push(error) });
    controller.connected();
    controller.message('clipboard_caps', { version: 1, enabled: true, max_bytes: 16 * 1024 * 1024, chunk_bytes: 12288 });
    const event = (target = overlay) => ({ target, code: 'KeyV', key: 'v', ctrlKey: true,
        preventDefault() { this.defaultPrevented = true; }, stopPropagation() {} });
    const paste = (text, target = overlay, types = ['text/plain']) => {
        const e = { ...event(target), clipboardData: { types, getData: () => text } };
        listeners.get('paste')(e); return e;
    };
    const requests = type => sent.filter(m => m.startsWith('paste,')).map(m => JSON.parse(m.slice(6))).filter(m => !type || m.type === type);
    const ack = () => controller.message('paste_ack', { id: requests('commit').at(-1).id, status: 'applied', revision: 1 });
    return { controller, overlay, rtc, sent, errors, writes, event, paste, requests, ack, listeners, setActive: value => { active = value; } };
}

test('actual keyboard path sends no V before clipboard capture and queues later keyup', async () => {
    const t = setup();
    try {
        const inputSource = readFileSync(new URL('../web/ivnc/lib/input2.js', import.meta.url), 'utf8')
            .replace(/^import .*;\s*$/gm, '').replace('export class Input', 'class Input');
        let modalBlocked = false;
        const ctx = { navigator: { platform: 'Linux' }, window: {}, document: { fullscreenElement: null,
            documentElement: { hasAttribute: name => name === 'data-ivnc-modal-blocked' && modalBlocked } }, pasteShortcut, setTimeout, clearTimeout };
        vm.createContext(ctx); vm.runInContext(inputSource + '\nglobalThis.InputClass = Input;', ctx);
        const input = Object.assign(Object.create(ctx.InputClass.prototype), { _keyDownList: { ControlLeft: 0xffe3 }, _EVENT_MARKER: 'handled', clipboard: t.controller, send: t.controller.sendInput });
        modalBlocked = true;
        const beforeBlocked = t.sent.length;
        input._handleKeyDown(t.event());
        input._handleKeyPress(t.event());
        input._handleKeyUp(t.event());
        assert.equal(t.sent.length, beforeBlocked, 'a parent-owned modal must block remote keyboard capture');
        modalBlocked = false;
        input._handleKeyDown(t.event());
        assert.equal(t.sent.some(m => m === 'kd,118'), false);
        assert.equal(t.requests('start').length, 0);
        t.controller.sendInput('ku,65507');
        t.paste('NEW_B');
        assert.equal(t.sent.includes('ku,65507'), false);
        assert.equal(Buffer.from(t.requests('chunk')[0].data, 'base64').toString(), 'NEW_B');
        t.ack(); await tick();
        assert.equal(t.sent.at(-1), 'ku,65507');
        assert.equal(t.overlay.readOnly, true);
    } finally { t.controller.destroy(); }
});

test('rapid identical pastes and A/B pastes retain distinct operations and ordering', async () => {
    const t = setup();
    try {
        for (const text of ['A', 'A', 'B']) { t.controller.beginPaste(t.event(), 'ctrl-v'); t.paste(text); }
        for (let i = 0; i < 3; i++) { assert.equal(t.requests('commit').length, i + 1); t.ack(); await tick(); }
        assert.deepEqual(t.requests('start').map(m => m.id), [1, 2, 3]);
        assert.deepEqual(t.requests('chunk').map(m => Buffer.from(m.data, 'base64').toString()), ['A', 'A', 'B']);
        assert.deepEqual(t.errors, []);
    } finally { t.controller.destroy(); }
});

test('local rename and native form inputs are left alone', () => {
    const t = setup();
    try {
        const local = { closest: () => ({}) };
        assert.equal(t.paste('LOCAL', local).defaultPrevented, undefined);
        assert.equal(t.requests('start').length, 0);
    } finally { t.controller.destroy(); }
});

test('empty text executes once; nontext data does not paste the old selection', async () => {
    const t = setup();
    try {
        t.paste(''); assert.equal(t.requests('start')[0].size, 0);
        t.ack(); await tick();
        t.paste('', t.overlay, ['image/png']);
        assert.equal(t.requests('start').length, 1);
        assert.deepEqual(t.errors, ['text_required']);
    } finally { t.controller.destroy(); }
});

test('unicode large text is split by bytes and only committed when complete', async () => {
    const t = setup();
    try {
        const text = '中文😀\n'.repeat(5000);
        t.paste(text);
        const bytes = Buffer.concat(t.requests('chunk').map(c => Buffer.from(c.data, 'base64')));
        assert.equal(bytes.toString(), text);
        assert(t.requests('chunk').length > 1);
        assert.equal(t.requests().at(-1).type, 'commit');
        t.ack(); await tick();
    } finally { t.controller.destroy(); }
});

test('disconnect cancels buffered input and late acknowledgement cannot replay it', async () => {
    const t = setup();
    try {
        t.paste('A'); t.controller.sendInput('kd,120');
        t.controller.disconnected();
        t.ack(); await tick();
        assert.equal(t.sent.includes('kd,120'), false);
        t.controller.connected();
        assert.equal(t.controller.beginPaste(t.event(), 'ctrl-v'), true);
        assert.equal(t.errors.at(-1), 'unsupported');
    } finally { t.controller.destroy(); }
});

test('browser echoes, stale revisions and inactive views never overwrite local clipboard', async () => {
    const t = setup();
    try {
        t.controller.receive('ECHO', { revision: 1, origin: 'browser' });
        t.controller.receive('OLD', { revision: 1, origin: 'remote' });
        t.controller.receive('NEW', { revision: 2, origin: 'remote' });
        await tick(); assert.deepEqual(t.writes, ['NEW']);
        t.setActive(false);
        t.controller.receive('HIDDEN', { revision: 3, origin: 'remote' });
        await tick(); assert.deepEqual(t.writes, ['NEW']);
    } finally { t.controller.destroy(); }
});

test('shortcut variants, key repeat and explicit disabled mode', () => {
    const t = setup();
    try {
        assert.equal(pasteShortcut({code:'KeyV',metaKey:true}), 'ctrl-v');
        assert.equal(pasteShortcut({code:'KeyV',ctrlKey:true,shiftKey:true}), 'ctrl-shift-v');
        assert.equal(pasteShortcut({code:'Insert',shiftKey:true}), 'shift-insert');
        assert.equal(pasteShortcut({code:'KeyV',ctrlKey:true,altKey:true}), null);
        const repeat = { ...t.event(), repeat: true };
        t.controller.beginPaste(repeat, 'ctrl-v'); assert(repeat.defaultPrevented);
        assert(t.overlay.readOnly);
        t.controller.message('clipboard_caps', {version:1,enabled:false,max_bytes:1,chunk_bytes:1});
        assert.equal(t.controller.beginPaste(t.event(), 'ctrl-v'), false);
        assert.equal(t.paste('A').defaultPrevented, undefined);
    } finally { t.controller.destroy(); }
    assert.equal(t.listeners.size, 0);
});

test('immediate remote copy then paste waits for that copy instead of stale browser text', async () => {
    const t = setup();
    try {
        t.controller.noteCopy();
        t.controller.sendInput('kd,99');
        t.controller.beginPaste(t.event(), 'ctrl-v');
        t.paste('STALE_LOCAL');
        assert.equal(t.requests('start').length, 0);
        t.controller.receive('REMOTE_NEW', {revision:1,origin:'remote'});
        await tick();
        assert.equal(Buffer.from(t.requests('chunk')[0].data,'base64').toString(), 'REMOTE_NEW');
        t.ack(); await tick();
        assert.deepEqual(t.errors, []);
    } finally { t.controller.destroy(); }
});

test('copy queued after a paste cannot consume an earlier selection notification', async () => {
    const t = setup();
    try {
        t.paste('FIRST');
        t.controller.noteCopy(); t.controller.sendInput('kd,99');
        t.controller.beginPaste(t.event(),'ctrl-v'); t.paste('STALE');
        t.controller.receive('UNRELATED', {revision:1,origin:'remote'});
        t.ack(); await tick();
        assert.equal(t.requests('commit').length, 1);
        t.controller.receive('COPIED', {revision:2,origin:'remote'});
        await tick(); t.ack(); await tick();
        assert.equal(Buffer.from(t.requests('chunk').at(-1).data,'base64').toString(), 'COPIED');
    } finally { t.controller.destroy(); }
});

test('lost acknowledgement times out without retry and cleanup tolerates a closed channel', async context => {
    context.mock.timers.enable({apis:['setTimeout']});
    const t = setup();
    try {
        t.paste('ONCE');
        context.mock.timers.tick(30001); await tick();
        assert.deepEqual(t.errors, ['timeout']);
        assert.equal(t.requests('commit').length, 1);
        assert.equal(t.requests('cancel').length, 1);
        t.rtc._send_channel.readyState = 'closed';
        t.controller.disconnected();
        assert.doesNotThrow(() => t.controller.sendInput('kr'));
    } finally { t.controller.destroy(); context.mock.timers.reset(); }
});

test('moving focus to a local editor cancels the pending remote paste', async () => {
    const t = setup();
    try {
        t.controller.beginPaste(t.event(),'ctrl-v');
        t.controller.sendInput('ku,65507');
        t.listeners.get('focusin')({target:{closest:()=>({})}});
        assert(t.overlay.readOnly);
        assert.equal(t.requests('start').length,0);
        assert.equal(t.sent.at(-1),'kr');
        assert.equal(t.paste('LOCAL',{closest:()=>({})}).defaultPrevented,undefined);
        await tick(); assert.deepEqual(t.errors,[]);
    } finally { t.controller.destroy(); }
});

test('reconnect clears clipboard chunks and ignores callbacks from old channels and peers', () => {
    const rtcSource = readFileSync(new URL('../web/ivnc/lib/webrtc.js', import.meta.url), 'utf8')
        .replace(/^import .*;\s*$/gm, '').replace('export class WebRTCDemo', 'class WebRTCDemo');
    class Peer {
        signalingState = 'stable';
        addTransceiver() {}
        createDataChannel(label) { return { label, readyState: 'open', close() { this.readyState = 'closed'; } }; }
        close() { this.connectionState = 'closed'; }
    }
    const ctx = { RTCPeerConnection: Peer, setTimeout, base64ToString: text => Buffer.from(text, 'base64').toString() };
    vm.createContext(ctx); vm.runInContext(rtcSource + '\nglobalThis.RtcClass = WebRTCDemo;', ctx);
    const rtc = new ctx.RtcClass({ connect() {} }, {}, 1);
    let opens = 0, closes = 0;
    const errors = [], messages = [], contents = [], states = [];
    rtc.ondatachannelopen = () => opens++;
    rtc.ondatachannelclose = () => closes++;
    rtc.onerror = error => errors.push(error);
    rtc.onclipboardmessage = (type, data) => messages.push([type, data.id]);
    rtc.onclipboardcontent = (text, metadata) => contents.push([text, metadata.revision]);
    rtc._handleConnectionStateChange = state => states.push(state);
    rtc.connect();
    const oldPeer = rtc.peerConnection, oldChannel = rtc._send_channel;
    oldChannel.onopen();
    rtc.createAuxDataChannel();
    const oldAux = rtc._aux_channel;
    oldChannel.onmessage({ target: oldChannel, data: 'clipboard_start,{"revision":1}' });
    oldChannel.onmessage({ target: oldChannel, data: 'clipboard_data,T0xE' });
    rtc.reset();
    assert.equal(closes, 1);
    assert.equal(rtc.clipboardcontent.length, 0);
    assert.equal(rtc._clipboardMetadata, null);
    const channel = rtc._send_channel;
    channel.onopen();
    rtc.createAuxDataChannel();
    const auxiliary = rtc._aux_channel;
    oldChannel.onclose(); oldChannel.onerror(); oldChannel.onopen();
    oldAux.onclose(); oldAux.onerror(Error('late')); oldAux.onopen();
    oldPeer.onconnectionstatechange();
    oldPeer.ondatachannel({ target: oldPeer, channel: oldChannel });
    oldChannel.onmessage({ target: oldChannel, data: 'paste_ack,{"id":1,"status":"applied"}' });
    channel.onmessage({ target: channel, data: 'paste_ack,{"id":2,"status":"applied"}' });
    channel.onmessage({ target: channel, data: 'clipboard_start,{"revision":2}' });
    oldChannel.onmessage({ target: oldChannel, data: 'clipboard_data,T0xE' });
    channel.onmessage({ target: channel, data: 'clipboard_data,TkVX' });
    channel.onmessage({ target: channel, data: 'clipboard_finish' });
    assert.equal(rtc._send_channel, channel);
    assert.equal(rtc._aux_channel, auxiliary);
    assert.equal(opens, 2); assert.equal(closes, 1);
    assert.deepEqual(errors, []); assert.deepEqual(states, []);
    assert.deepEqual(messages, [['paste_ack', 2]]);
    assert.deepEqual(contents, [['NEW', 2]]);
});
