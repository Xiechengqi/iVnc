// Clipboard contents and the user's paste intent travel as one operation.
// This controller owns its listeners, pending gestures and connection lifetime.
const MAX_BYTES = 16 * 1024 * 1024;
const CHUNK_BYTES = 12 * 1024;
const NATIVE_INPUT = '.taskbar, .allow-native-input, .web-terminal-modal';

export function pasteShortcut(event) {
    if (event.altKey) return null;
    if (event.code === 'KeyV' && (event.ctrlKey || event.metaKey)) {
        return event.shiftKey ? 'ctrl-shift-v' : 'ctrl-v';
    }
    return event.code === 'Insert' && event.shiftKey && !event.ctrlKey && !event.metaKey ? 'shift-insert' : null;
}

export function createClipboard({ webrtc, overlay, imeTarget, isActive, getTarget, report }) {
    let disposed = false, epoch = 0, sequence = 0, caps = null;
    let queue = [], capture = null, current = null, pumping = false, queuedBytes = 0;
    let remoteRevision = 0, writeGeneration = 0, writer = Promise.resolve();
    let remoteCopy = null;
    const copies = new Set();
    const open = () => webrtc._send_channel?.readyState === 'open';
    const send = message => {
        if (!open()) throw Error('disconnected');
        webrtc.sendDataChannelMessage(message);
    };
    const command = message => send('paste,' + JSON.stringify(message));
    const remoteTarget = target => !!target && !target.closest?.(NATIVE_INPUT)
        && (target === overlay || target === imeTarget());
    const restore = item => {
        if (!item?.element) return;
        item.element.readOnly = item.wasReadOnly;
        item.element.value = '';
    };
    const fail = reason => { if (!disposed) report(reason); };

    function cancel(notify = true) {
        ++epoch; ++writeGeneration;
        if (current && open()) { try { command({ type: 'cancel', id: current.id }); } catch (_) {} }
        if (capture) { clearTimeout(capture.timer); restore(capture); }
        current?.reject?.(Error('cancelled'));
        for (const copy of copies) { clearTimeout(copy.timer); copy.reject(Error('cancelled')); }
        copies.clear(); remoteCopy = null;
        capture = current = null;
        queue = []; queuedBytes = 0; pumping = false;
        if (notify && open()) { try { send('kr'); } catch (_) {} }
    }

    async function pump() {
        if (pumping || disposed) return;
        const generation = epoch;
        pumping = true;
        try {
            while (queue.length && generation === epoch) {
                const item = queue[0];
                if (item.kind === 'paste' && !item.bytes) break;
                queue.shift();
                if (item.kind === 'input') { send(item.message); continue; }
                if (item.kind === 'copy') { startCopy(item.copy); continue; }
                if (!isActive()) { cancel(); break; }
                current = item;
                const acknowledgement = new Promise((resolve, reject) => { item.resolve = resolve; item.reject = reject; });
                // Attach immediately, including while a large transfer is being sent.
                acknowledgement.catch(() => {});
                const timer = setTimeout(() => item.reject(Error('timeout')), 30000);
                try {
                    if (item.copy) {
                        // Ctrl+C/X was forwarded to the remote application. Its
                        // selection response can arrive after the local paste
                        // event, which still contains the previous local value.
                        const text = await item.copy.promise;
                        if (generation !== epoch) throw Error('cancelled');
                        const bytes = new TextEncoder().encode(text);
                        queuedBytes += bytes.length - item.bytes.length;
                        if (queuedBytes > MAX_BYTES) throw Error('limit');
                        item.bytes = bytes;
                    }
                    command({ type: 'start', id: item.id, size: item.bytes.length, shortcut: item.shortcut, target: item.target });
                    const chunkSize = Math.min(CHUNK_BYTES, caps.chunk_bytes);
                    for (let offset = 0; offset < item.bytes.length; offset += chunkSize) {
                        while (webrtc._send_channel?.bufferedAmount > 256 * 1024) {
                            await new Promise(resolve => setTimeout(resolve, 4));
                            if (generation !== epoch || !open() || Date.now() - item.started > 30000) throw Error('cancelled');
                        }
                        if (generation !== epoch || !isActive()) throw Error('cancelled');
                        const bytes = item.bytes.subarray(offset, offset + chunkSize);
                        command({ type: 'chunk', id: item.id, data: btoa(String.fromCharCode(...bytes)) });
                    }
                    if (generation !== epoch) throw Error('cancelled');
                    command({ type: 'commit', id: item.id });
                    await acknowledgement;
                } finally { clearTimeout(timer); }
                queuedBytes -= item.bytes.length;
                current = null;
            }
        } catch (error) {
            if (generation === epoch) { fail(error.message); cancel(); }
        } finally {
            if (generation === epoch) pumping = false;
        }
    }

    function newCapture(shortcut, element) {
        if (capture) {
            clearTimeout(capture.timer); restore(capture);
            queue = queue.filter(item => item !== capture);
        }
        const item = { kind: 'paste', id: ++sequence, shortcut, target: getTarget() ?? null,
            element, wasReadOnly: element.readOnly, started: Date.now(), copy: remoteCopy };
        element.readOnly = false;
        element.value = '';
        capture = item;
        queue.push(item);
        ++writeGeneration;
        // A missing native event must not silently paste an old remote selection.
        item.timer = setTimeout(() => {
            if (capture !== item) return;
            restore(item); capture = null;
            queue = queue.filter(entry => entry !== item);
            fail('clipboard_unavailable'); pump();
        }, 1500);
        return item;
    }

    function available() {
        if (disposed || !isActive() || !open()) { fail('disconnected'); return false; }
        if (caps?.version !== 1) { fail('unsupported'); return false; }
        if (queue.length >= 64 || queuedBytes >= MAX_BYTES) { fail('limit'); return false; }
        return true;
    }

    function beginPaste(event, shortcut) {
        if (caps?.enabled === false) return false; // Explicitly disabled: remote native clipboard.
        if (event.repeat || !remoteTarget(event.target) || !available()) {
            event.preventDefault(); return true;
        }
        newCapture(shortcut, event.target);
        return true; // Leave the browser default action enabled to obtain clipboardData.
    }

    function paste(event) {
        if (!remoteTarget(event.target) || !isActive() || event.defaultPrevented || caps?.enabled === false) return;
        event.preventDefault();
        if (!available()) return;
        const item = capture || newCapture('ctrl-v', event.target);
        clearTimeout(item.timer); restore(item); capture = null;
        const data = event.clipboardData;
        if ((!data || !Array.from(data.types || []).includes('text/plain')) && !item.copy) {
            queue = queue.filter(entry => entry !== item);
            fail('text_required'); pump(); return;
        }
        const bytes = new TextEncoder().encode(data?.getData('text/plain') || '');
        if (bytes.length > Math.min(MAX_BYTES, caps.max_bytes) || queuedBytes + bytes.length > MAX_BYTES) {
            queue = queue.filter(entry => entry !== item);
            fail('limit'); pump(); return;
        }
        item.bytes = bytes; queuedBytes += bytes.length;
        pump();
    }

    function receive(content, metadata) {
        if (metadata?.revision) {
            if (metadata.revision <= remoteRevision) return;
            remoteRevision = metadata.revision;
        }
        // Browser writes are acknowledgements/state updates, never a remote copy.
        if (metadata?.origin === 'browser' || caps?.enabled === false || disposed || !isActive() || !document.hasFocus()) return;
        const ownCopy = [...copies].find(copy => copy.sent);
        if (ownCopy) {
            clearTimeout(ownCopy.timer); copies.delete(ownCopy); ownCopy.resolve(content);
        }
        if (!ownCopy && (current || capture || queue.length)) return;
        const generation = ++writeGeneration, connection = epoch;
        writer = writer.catch(() => {}).then(async () => {
            if (generation !== writeGeneration || connection !== epoch || disposed || !isActive() || !document.hasFocus()) return;
            // Do not steal focus with execCommand/hidden textarea on permission failure.
            try {
                await navigator.clipboard.writeText(content);
                if (remoteCopy === ownCopy) remoteCopy = null;
            }
            catch (_) { if (generation === writeGeneration) fail('copy_unavailable'); }
        });
    }

    function startCopy(copy) {
        copy.sent = true;
        copy.timer = setTimeout(() => {
            copies.delete(copy);
            if (remoteCopy === copy) remoteCopy = null;
            copy.reject(Error('copy_unavailable'));
        }, 30000);
    }

    function noteCopy() {
        if (!open() || caps?.enabled === false || !isActive() || copies.size >= 32) return;
        const copy = {};
        copy.promise = new Promise((resolve, reject) => { copy.resolve = resolve; copy.reject = reject; });
        copy.promise.catch(() => {});
        copies.add(copy); remoteCopy = copy;
        if (pumping || queue.length) queue.push({ kind: 'copy', copy });
        else startCopy(copy);
    }

    // Native local UI copy supersedes any remote-copy shortcut we were tracking.
    const localCopy = event => { if (!remoteTarget(event.target)) remoteCopy = null; };
    const focusChanged = event => { if (!remoteTarget(event.target)) cancel(); };
    const visibilityChanged = () => { if (document.visibilityState === 'hidden') cancel(); };

    function message(type, data) {
        if (type === 'clipboard_caps') {
            if (data?.version === 1 && Number.isInteger(data.chunk_bytes) && data.chunk_bytes > 0
                && Number.isInteger(data.max_bytes) && data.max_bytes >= 0) caps = data;
        } else if (type === 'paste_ack' && current?.id === data?.id) {
            if (data.status === 'applied') {
                remoteRevision = Math.max(remoteRevision, data.revision || 0);
                current.resolve();
            } else current.reject(Error(data.status));
        }
    }

    function sendInput(message) {
        if (!isActive() || disposed || !open()) return;
        if (capture || pumping || queue.length) {
            if (queue.length >= 512) { fail('limit'); cancel(); return; }
            queue.push({ kind: 'input', message });
        } else send(message);
    }

    document.addEventListener('paste', paste);
    document.addEventListener('copy', localCopy);
    document.addEventListener('cut', localCopy);
    document.addEventListener('focusin', focusChanged);
    document.addEventListener('visibilitychange', visibilityChanged);
    return {
        beginPaste, sendInput, noteCopy, receive, message, cancel,
        connected() { cancel(); caps = null; remoteRevision = 0; command({ type: 'hello' }); },
        disconnected() { cancel(false); caps = null; },
        destroy() {
            cancel(); disposed = true;
            document.removeEventListener('paste', paste);
            document.removeEventListener('copy', localCopy);
            document.removeEventListener('cut', localCopy);
            document.removeEventListener('focusin', focusChanged);
            document.removeEventListener('visibilitychange', visibilityChanged);
        },
    };
}
