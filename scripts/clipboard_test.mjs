// Pure event/transport regressions; no OS clipboard or running instance touched.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../web/ivnc/clipboard.js', import.meta.url), 'utf8');
const { createClipboard, pasteShortcut } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const tick = () => new Promise(resolve => setImmediate(resolve));

function setup(options = {}) {
    const listeners = new Map(), sent = [], errors = [], writes = [], states = [], diagnostics = [];
    const overlay = { readOnly: true, value: '', closest: () => null };
    let active = true;
    globalThis.isSecureContext = options.secure ?? true;
    globalThis.document = { activeElement: overlay, visibilityState: 'visible',
        documentElement: { hasAttribute: () => false },
        addEventListener: (type, fn) => listeners.set(type, fn), removeEventListener: type => listeners.delete(type), hasFocus: () => active };
    Object.defineProperty(globalThis, 'navigator', { configurable: true, value: {
        permissions: options.permissions, clipboard: { writeText: options.write || (async text => writes.push(text)) } } });
    const rtc = { _send_channel: { readyState: 'open', bufferedAmount: 0 }, sendDataChannelMessage: message => { sent.push(message); return true; } };
    const controller = createClipboard({ webrtc: rtc, overlay, imeTarget: () => null, isActive: () => active, getTarget: () => 3,
        report: error => errors.push(error), onCopyState: state => states.push(state), diagnostic: detail => diagnostics.push(detail) });
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
    const assist = { closest: selector => selector.includes('.clipboard-assist') || selector.includes('.allow-native-input') ? {} : null };
    const focus = target => { document.activeElement = target; listeners.get('focusin')({target}); };
    return { controller, overlay, assist, focus, rtc, sent, errors, writes, event, paste, requests, ack, listeners,
        states, diagnostics, state: () => states.at(-1), setActive: value => { active = value; } };
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

test('asynchronous PNG capture keeps later input and the next paste in gesture order', async () => {
    const t = setup(); let finish;
    try {
        t.controller.message('clipboard_caps', {version:1,enabled:true,max_bytes:16777216,chunk_bytes:12288,mime_types:['text/plain','text/html','image/png']});
        const png = Uint8Array.from([137,80,78,71,13,10,26,10]);
        t.controller.beginPaste(t.event(), 'ctrl-v');
        const capture = t.listeners.get('paste')({...t.event(),clipboardData:{types:['Files'],items:[{kind:'file',type:'image/png',getAsFile:()=>({size:png.length,arrayBuffer:()=>new Promise(resolve=>{finish=resolve})})}]}});
        t.controller.sendInput('ku,65507'); t.paste('NEXT');
        assert.equal(t.requests('start').length,0);
        assert(!t.sent.includes('ku,65507'));
        finish(png.buffer); await capture; await tick();
        assert.deepEqual(t.requests('start')[0].formats,[{mime:'image/png',size:8}]);
        assert.deepEqual([...Buffer.from(t.requests('chunk')[0].data,'base64')],[...png]);
        t.ack(); await tick();
        assert.equal(t.requests('start').length,2);
        assert(t.sent.indexOf('ku,65507')<t.sent.findIndex(m=>m.startsWith('paste,')&&JSON.parse(m.slice(6)).type==='start'&&JSON.parse(m.slice(6)).id===2));
        assert.equal(Buffer.from(t.requests('chunk')[1].data,'base64').toString(),'NEXT');
        t.ack(); await tick(); assert.deepEqual(t.errors,[]);
    } finally { t.controller.destroy(); }
});

test('MIME clipboard decoding rejects duplicate formats, mismatched lengths and oversized chunk streams', () => {
    const rtcSource = readFileSync(new URL('../web/ivnc/lib/webrtc.js', import.meta.url), 'utf8')
        .replace(/^import .*;\s*$/gm, '').replace('export class WebRTCDemo', 'class WebRTCDemo');
    const ctx = {atob,Uint8Array};
    vm.createContext(ctx);vm.runInContext(rtcSource+'\nglobalThis.RtcClass=WebRTCDemo;',ctx);
    const records=[],errors=[];
    const rtc=Object.assign(Object.create(ctx.RtcClass.prototype),{clipboardcontent:[],_clipboardEncodedLength:0,
        _setDebug:m=>errors.push(m),onclipboardformats:(r,m)=>records.push({r,m})});
    const send=data=>rtc._onPeerDataChannelMessage({data});
    const start=formats=>send('clipboard_start,'+JSON.stringify({formats,revision:1,origin:'remote'}));
    start([{mime:'text/plain',size:1},{mime:'text/html',size:8}]);
    send('clipboard_data,'+btoa('A<b>A</b>'));send('clipboard_finish');
    assert.equal(records.length,1);assert.equal(new TextDecoder().decode(records[0].r[1].data),'<b>A</b>');
    for(const formats of [[{mime:'text/plain',size:1},{mime:'text/plain',size:0}],[{mime:'text/plain',size:2}],[{mime:'text/uri-list',size:1}]]){
        start(formats);send('clipboard_data,QQ==');send('clipboard_finish');
    }
    start([{mime:'text/plain',size:1}]);
    send('clipboard_data,'+'A'.repeat(Math.ceil(16777216/3)*4+4));
    assert.equal(rtc.clipboardcontent.length,0,'overflow must be discarded before joining/decoding');
    send('clipboard_finish');
    assert.equal(records.length,1);assert.equal(errors.length,4);
    start([{mime:'text/plain',size:1}]);send('clipboard_data,QQ==');send('clipboard_finish');
    assert.equal(records.length,2,'a rejected transfer must not poison the next revision');
});

const deniedWrite = async () => { throw new DOMException('Write denied', 'NotAllowedError'); };
const receiveCopy = (t, text = 'REMOTE', revision = 1) => t.controller.receive(text, {revision, origin:'remote'});

test('HTTP and missing APIs offer manual copy without trying an unavailable API', async () => {
    for (const mode of ['http', 'missing']) {
        const t = setup({secure: mode !== 'http'});
        try {
            if (mode === 'missing') delete navigator.clipboard;
            receiveCopy(t); await tick();
            assert.equal(t.state().reason, mode === 'http' ? 'copy_insecure' : 'copy_unsupported');
            assert.equal(t.state().canRetry, false);
            assert.deepEqual(t.writes, []); assert.deepEqual(t.errors, []);
            t.focus(t.assist);
            assert.equal(t.controller.copySnapshot().text, 'REMOTE');
            assert.equal(t.controller.retryCopy(), false);
        } finally { t.controller.destroy(); }
    }
});

test('unsupported permission queries and ambiguous write rejection are not labelled permission denial', async () => {
    for (const query of [() => { throw Error('Unsupported permission'); }, async () => { throw Error('Unsupported permission'); }]) {
        const t = setup({permissions:{query}, write:deniedWrite});
        try {
            receiveCopy(t); await tick();
            assert.equal(t.state().reason, 'copy_blocked'); assert.equal(t.state().canRetry, true);
            assert.equal(t.diagnostics[0].error, 'NotAllowedError');
            assert.equal(JSON.stringify(t.diagnostics).includes('REMOTE'), false);
        } finally { t.controller.destroy(); }
    }
});

test('confirmed permission denial is distinguished and permission changes do not replay writes', async () => {
    const handlers = new Map();
    const status = {state:'denied', addEventListener:(n,f)=>handlers.set(n,f), removeEventListener:n=>handlers.delete(n)};
    const t = setup({permissions:{query:async () => status}});
    try {
        await tick(); receiveCopy(t); await tick();
        assert.equal(t.state().reason, 'copy_permission'); assert.deepEqual(t.writes, []);
        status.state = 'granted'; handlers.get('change')(); await tick();
        assert.equal(t.state().reason, 'copy_blocked'); assert.deepEqual(t.writes, []);
        t.focus(t.assist); assert.equal(t.controller.retryCopy(), true);
        assert.deepEqual(t.writes, ['REMOTE'], 'API is invoked inside the click call stack');
        await tick(); assert.equal(t.state().status, 'copied');
    } finally { t.controller.destroy(); }
    assert.equal(handlers.size, 0);
});

test('only a recognized forbidden policy is reported as a policy restriction', async () => {
    for (const known of [true, false]) {
        const t = setup();
        try {
            document.featurePolicy = {features:() => known ? ['clipboard-write'] : [], allowsFeature:() => false};
            receiveCopy(t); await tick();
            if (known) { assert.equal(t.state().reason, 'copy_policy'); assert.deepEqual(t.writes, []); }
            else assert.deepEqual(t.writes, ['REMOTE']);
        } finally { t.controller.destroy(); }
    }
});

test('retry uses the captured text synchronously, without resending remote copy or cut', async () => {
    const t = setup({write:deniedWrite});
    try {
        t.controller.noteCopy(); t.controller.sendInput('kd,120'); receiveCopy(t); await tick();
        t.focus(t.assist);
        const before = [...t.sent];
        navigator.clipboard.writeText = text => {t.writes.push(text); return Promise.resolve();};
        assert(t.controller.retryCopy()); assert.deepEqual(t.writes, ['REMOTE']);
        assert.deepEqual(t.sent, before);
        assert.equal(t.controller.retryCopy(), false);
        await tick(); assert.equal(t.state().status, 'copied');
        assert.equal(t.controller.copySnapshot(), null);
    } finally { t.controller.destroy(); }
});

test('failed local write still permits immediate remote copy/paste with the current text', async () => {
    const t = setup({write:deniedWrite});
    try {
        t.controller.noteCopy(); t.controller.beginPaste(t.event(), 'ctrl-v'); t.paste('OLD_LOCAL');
        receiveCopy(t, 'NEW_REMOTE'); await tick();
        assert.equal(Buffer.from(t.requests('chunk')[0].data,'base64').toString(), 'NEW_REMOTE');
        t.ack(); await tick(); assert.deepEqual(t.errors, []);
        assert.equal(t.state().reason, 'copy_blocked');
    } finally { t.controller.destroy(); }
});

test('menu copies have a same-context fallback; external copy after blur always wins', async () => {
    const t = setup({write:deniedWrite});
    try {
        receiveCopy(t, 'MENU_REMOTE'); await tick();
        t.paste('OLD_LOCAL'); await tick();
        assert.equal(Buffer.from(t.requests('chunk')[0].data,'base64').toString(), 'MENU_REMOTE');
        t.ack(); await tick();
        t.controller.suspend();
        assert.equal(t.controller.copySnapshot().text, 'MENU_REMOTE', 'manual offer can survive window blur');
        t.paste('EXTERNAL_NEW'); await tick();
        assert.equal(Buffer.from(t.requests('chunk').at(-1).data,'base64').toString(), 'EXTERNAL_NEW');
        assert.equal(t.state(), null);
        t.ack(); await tick();
    } finally { t.controller.destroy(); }
});

test('assist focus preserves the offer, other local editors clear it and cannot receive automatic writes', async () => {
    const t = setup({write:deniedWrite});
    try {
        receiveCopy(t); await tick(); t.focus(t.assist);
        receiveCopy(t, 'UNRELATED', 2); await tick();
        assert.equal(t.controller.copySnapshot().text, 'REMOTE', 'manual text stays stable');
        t.focus({closest:selector=>selector.includes('.allow-native-input') ? {} : null});
        assert.equal(t.state(), null);
        receiveCopy(t, 'WHILE_EDITING', 3); await tick();
        assert.equal(t.state(), null); assert.equal(t.diagnostics.length, 1);
    } finally { t.controller.destroy(); }
});

test('writes serialize and old completion cannot clear a newer manual offer', async () => {
    const pending = [];
    const t = setup({write:text=>new Promise((resolve,reject)=>pending.push({text,resolve,reject}))});
    try {
        receiveCopy(t, 'A', 1); await tick();
        receiveCopy(t, 'B', 2); receiveCopy(t, 'C', 3); await tick();
        assert.deepEqual(pending.map(p=>p.text), ['A']);
        pending[0].resolve(); await tick();
        assert.deepEqual(pending.map(p=>p.text), ['A','C']);
        pending[1].reject(new DOMException('Denied','NotAllowedError')); await tick();
        assert.equal(t.controller.copySnapshot().text, 'C');
        assert.equal(t.state().status, 'ready');
    } finally { t.controller.destroy(); }
});

test('view cancellation drops queued writes and ignores in-flight completion', async () => {
    let finish;
    const t = setup({write:text=>{t.writes.push(text);return new Promise(resolve=>{finish=resolve});}});
    try {
        receiveCopy(t, 'A', 1); await tick(); receiveCopy(t, 'B', 2);
        t.controller.cancel(); finish(); await tick();
        assert.deepEqual(t.writes, ['A']); assert.equal(t.state(), null);
        assert.equal(t.controller.copySnapshot(), null);
    } finally { t.controller.destroy(); }
});

test('new local paste invalidates an automatic copy that has not started', async () => {
    const t = setup();
    try {
        receiveCopy(t); t.controller.suspend(); t.paste('LOCAL_NEW'); await tick();
        assert.deepEqual(t.writes, []);
        assert.equal(Buffer.from(t.requests('chunk')[0].data,'base64').toString(), 'LOCAL_NEW');
        t.ack(); await tick();
    } finally { t.controller.destroy(); }
});

test('empty and Unicode text are preserved in the manual snapshot; oversized text is rejected', async () => {
    const t = setup({secure:false});
    try {
        receiveCopy(t, '', 1); await tick(); assert.equal(t.controller.copySnapshot().text, '');
        const large = '中文😀\n'.repeat(12000);
        receiveCopy(t, large, 2); await tick(); assert.equal(t.controller.copySnapshot().text, large);
        receiveCopy(t, 'x'.repeat(16*1024*1024+1), 3); await tick();
        assert.deepEqual(t.errors, ['limit']);
    } finally { t.controller.destroy(); }
});

test('copy timeout reports a remote failure only when a paste awaits it, not for a bare Ctrl+C interrupt', async context => {
    context.mock.timers.enable({apis:['setTimeout']});
    try {
        for (const pasteWaiting of [false, true]) {
            const t = setup();
            try {
                t.controller.noteCopy();
                if (pasteWaiting) t.paste('STALE');
                context.mock.timers.tick(30001); await tick();
                assert.deepEqual(t.errors, pasteWaiting ? ['remote_copy_timeout'] : []);
                assert.deepEqual(t.writes, []); assert.equal(t.requests('commit').length, 0);
            } finally { t.controller.destroy(); }
        }
    } finally { context.mock.timers.reset(); }
});

test('destroy ignores late permission resolution and late write rejection', async () => {
    let permissionResolve, writeReject;
    const t = setup({permissions:{query:()=>new Promise(resolve=>{permissionResolve=resolve})},
        write:()=>new Promise((_,reject)=>{writeReject=reject})});
    receiveCopy(t); await tick(); t.controller.destroy();
    const count = t.states.length;
    permissionResolve({addEventListener:()=>assert.fail('listener after destroy')});
    writeReject(new Error('Late failure')); await tick();
    assert.equal(t.states.length, count); assert.deepEqual(t.errors, []);
});

test('permission-dialog blur retains a manual offer but never resumes an automatic write', async () => {
    let rejectWrite;
    const t = setup({write:()=>new Promise((_,reject)=>{rejectWrite=reject})});
    try {
        receiveCopy(t); await tick(); t.controller.suspend();
        rejectWrite(new DOMException('Denied','NotAllowedError')); await tick();
        assert.equal(t.state().reason, 'copy_paused');
        assert.equal(t.controller.copySnapshot().text, 'REMOTE');
        assert.deepEqual(t.errors, []);
    } finally { t.controller.destroy(); }
});

test('disabling clipboard sync clears the offer and queued write', async () => {
    const t = setup();
    try {
        receiveCopy(t);
        t.controller.message('clipboard_caps',{version:1,enabled:false,max_bytes:16777216,chunk_bytes:12288});
        await tick(); assert.deepEqual(t.writes, []);
        assert.equal(t.state(), null); assert.equal(t.controller.retryCopy(), false);
        assert.equal(t.controller.beginPaste(t.event(),'ctrl-v'),false);
    } finally { t.controller.destroy(); }
});

test('an unknown clipboard API failure uses a neutral explanation', async () => {
    const t = setup({write:async()=>{throw new Error('OS failure');}});
    try {
        receiveCopy(t); await tick();
        assert.equal(t.state().reason, 'copy_failed');
        assert.equal(t.state().canRetry, true);
    } finally { t.controller.destroy(); }
});
