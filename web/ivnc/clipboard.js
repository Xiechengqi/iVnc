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

// A clipboard operation may contain several MIME representations of the same
// selection. Sizes refer to the concatenated, bounded transfer body.
function packFormats(records, limit = MAX_BYTES) {
    const seen = new Set(); let size = 0;
    for (const r of records) {
        if (!['text/plain', 'text/html', 'image/png'].includes(r.mime) || seen.has(r.mime) || !(r.data instanceof Uint8Array)) throw Error('invalid_content');
        seen.add(r.mime); size += r.data.length;
    }
    if (!records.length || size > limit) throw Error('limit');
    const bytes = new Uint8Array(size); let offset = 0;
    for (const r of records) { bytes.set(r.data, offset); offset += r.data.length; }
    const plain = records.find(r => r.mime === 'text/plain');
    return { bytes, records, text: plain ? new TextDecoder('utf-8', { fatal: true }).decode(plain.data) : null,
        formats: records.length === 1 && plain ? undefined : records.map(r => ({ mime: r.mime, size: r.data.length })) };
}

export function createClipboard({ webrtc, overlay, imeTarget, isActive, getTarget, report,
    isViewActive = isActive, onCopyState = () => {}, diagnostic = () => {} }) {
    let disposed = false, epoch = 0, sequence = 0, caps = null;
    let queue = [], capture = null, current = null, pumping = false, queuedBytes = 0;
    let remoteRevision = 0, writeGeneration = 0, copySequence = 0;
    let offer = null, writing = null, scheduledWrite = null, permission = null;
    let remoteCopy = null, lateCopy = null, receiving = null, selectionStatus = null;
    const copies = new Set();
    const open = () => webrtc._send_channel?.readyState === 'open';
    const send = message => {
        if (!open()) throw Error('disconnected');
        return webrtc.sendDataChannelMessage(message);
    };
    const command = message => send('paste,' + JSON.stringify(message));
    const remoteTarget = target => !!target && !target.closest?.(NATIVE_INPUT)
        && (target === overlay || target === imeTarget());
    const assistTarget = target => !!target?.closest?.('.clipboard-assist');
    const focusedRemote = () => remoteTarget(document.activeElement)
        && !document.documentElement?.hasAttribute('data-ivnc-modal-blocked');
    const live = () => !disposed && open() && isActive() && document.visibilityState !== 'hidden' && document.hasFocus();
    const restore = item => {
        if (!item?.element) return;
        item.element.readOnly = item.wasReadOnly;
        item.element.value = '';
    };
    const fail = reason => { if (!disposed) report(reason); };

    // Permission names and policy introspection are not supported by every browser.
    // Unknown means try the write; it is not evidence of a denied permission.
    const supportsImage = () => globalThis.isSecureContext !== false && !!navigator.clipboard?.write && !!globalThis.ClipboardItem
        && (!ClipboardItem.supports || ClipboardItem.supports('image/png'));

    function capability(ignorePermission = false) {
        // Apply a recognized policy to both modern and legacy writes, including
        // HTTP pages and browsers without the modern API.
        const policy = document.permissionsPolicy || document.featurePolicy;
        try {
            if (policy?.features?.().includes('clipboard-write') && !policy.allowsFeature('clipboard-write')) return 'copy_policy';
        } catch (_) {}
        if (globalThis.isSecureContext === false) return 'copy_insecure';
        if (typeof navigator.clipboard?.writeText !== 'function' && !(offer?.formats && navigator.clipboard?.write && globalThis.ClipboardItem)) return 'copy_unsupported';
        if (offer?.text === null && !supportsImage()) return 'copy_unsupported';
        if (!ignorePermission && permission?.state === 'denied') return 'copy_permission';
        return null;
    }

    // The legacy command can write plaintext on ordinary HTTP, but the browser
    // still decides whether the current user gesture permits it. Do not use it
    // to retry a modern API's permission/policy rejection.
    function canLegacyCopy(reason) {
        return ['copy_insecure', 'copy_unsupported'].includes(reason)
            && typeof offer?.text === 'string' && typeof document.execCommand === 'function';
    }

    function publishCopy() {
        if (disposed) return;
        const reason = capability(true);
        onCopyState(offer ? { id: offer.id, status: offer.status, reason: offer.reason,
            busy: !!writing, hasImage: offer.records.some(r => r.mime === 'image/png'),
            canManual: typeof offer.text === 'string', canRetry: offer.records.length > 0 && (!reason || canLegacyCopy(reason)),
            textOnly: offer.records.some(r => r.mime === 'image/png') && !supportsImage() }
            : receiving ? { id: `receiving-${receiving.revision}`, status: 'receiving', reason: 'copy_receiving',
                busy: false, canManual: false, canRetry: false, hasImage: false } : null);
    }

    function diagnose(reason, error, manual = false) {
        diagnostic({ phase: manual ? 'manual-write' : 'automatic-write', reason,
            error: ['NotAllowedError', 'SecurityError', 'TypeError', 'AbortError'].includes(error?.name) ? error.name : 'Error',
            secure: globalThis.isSecureContext, focused: document.hasFocus(),
            activation: navigator.userActivation?.isActive ?? null, permission: permission?.state || 'unknown' });
    }

    function permissionChanged() {
        if (offer?.status === 'ready' && offer.records.length) {
            const reason = capability();
            offer.reason = (canLegacyCopy(reason) ? null : reason)
                || (offer.reason === 'copy_permission' ? 'copy_blocked' : offer.reason);
            publishCopy();
        }
    }
    try {
        Promise.resolve(navigator.permissions?.query({ name: 'clipboard-write' })).then(status => {
            if (disposed || !status) return;
            permission = status;
            permission.addEventListener?.('change', permissionChanged);
            permissionChanged();
        }).catch(() => {});
    } catch (_) {}

    function cancel(notify = true, { preserveOffer = false } = {}) {
        ++epoch; ++writeGeneration;
        scheduledWrite = null;
        if (current && open()) { try { command({ type: 'cancel', id: current.id }); } catch (_) {} }
        if (capture) { clearTimeout(capture.timer); restore(capture); }
        current?.reject?.(Error('cancelled'));
        for (const copy of copies) { clearTimeout(copy.timer); copy.reject(Error('cancelled')); }
        for (const item of queue) item.sent?.(false);
        copies.clear(); remoteCopy = lateCopy = receiving = selectionStatus = null;
        capture = current = null;
        queue = []; queuedBytes = 0; pumping = false;
        if (!preserveOffer) offer = null;
        else if (offer && offer.status !== 'copied') {
            offer.reason ||= 'copy_paused'; offer.status = 'ready';
        }
        publishCopy();
        if (notify && open()) { try { send('kr'); } catch (_) {} }
    }

    function suspend() {
        // Retain only the result of a copy that started in this view. Never
        // resume automatic writes after blur or admit unrelated background offers.
        const intent = lateCopy || (([...copies].some(copy => copy.sent) || receiving) ? {
            until: Date.now() + 30000, minimum: receiving?.revision ?? remoteRevision + 1, revision: receiving?.revision ?? null,
        } : null);
        cancel(true, { preserveOffer: true });
        lateCopy = intent;
    }

    function dismissCopy() {
        ++writeGeneration; scheduledWrite = null; offer = receiving = lateCopy = null;
        publishCopy();
    }

    function finishWrite(job, error) {
        if (writing === job) writing = null;
        if (job.generation === writeGeneration && offer === job.offer && live()) {
            if (error) {
                const reason = capability();
                offer.reason = (canLegacyCopy(reason) ? null : reason)
                    || (['NotAllowedError', 'SecurityError'].includes(error?.name) ? 'copy_blocked' : 'copy_failed');
                offer.status = 'ready';
                diagnose(offer.reason, error, job.manual);
            } else {
                if (remoteCopy === job.offer.copy) remoteCopy = null;
                if (job.partial || offer.imageMissing) {
                    offer.status = 'ready';
                    offer.reason = offer.imageMissing || 'copy_text_only';
                } else if (job.manual) { offer.text = ''; offer.status = 'copied'; }
                else offer = null;
            }
        }
        publishCopy();
        drainWrite();
    }

    function legacyCopy(text) {
        let handled = false, failure = null;
        const copy = event => {
            event.preventDefault();
            // This event belongs to our synchronous command. Keep local UI copy
            // handlers from replacing its data or cancelling remote paste intent.
            event.stopImmediatePropagation();
            if (!event.clipboardData) return;
            try { event.clipboardData.setData('text/plain', text); handled = true; }
            catch (error) { failure = error; }
        };
        document.addEventListener('copy', copy, true);
        try {
            // No selection/focus changes, cached content or asynchronous event
            // handler: even an empty string replaces the previous clipboard.
            const copied = document.execCommand('copy');
            if (failure) throw failure;
            if (!copied || !handled) throw new DOMException('Copy requires a user gesture', 'NotAllowedError');
        } finally { document.removeEventListener('copy', copy, true); }
    }

    function write(snapshot, manual, legacy = false) {
        const job = { offer: snapshot, generation: writeGeneration, manual };
        writing = job;
        if (manual) snapshot.status = 'writing';
        publishCopy();
        // Called synchronously by retryCopy: never await a permission query or
        // enqueue the API call after a user clicks the copy button.
        let result;
        try {
            if (legacy) {
                legacyCopy(snapshot.text);
                job.partial = snapshot.records.some(r => r.mime === 'image/png');
            }
            else if (snapshot.formats && navigator.clipboard?.write && globalThis.ClipboardItem) {
                const data = Object.fromEntries(snapshot.records.filter(r => !ClipboardItem.supports || ClipboardItem.supports(r.mime))
                    .map(r => [r.mime, new Blob([r.data], { type: r.mime })]));
                if (!Object.keys(data).length) throw Error('unsupported');
                job.partial = snapshot.records.some(r => r.mime === 'image/png') && !data['image/png'];
                result = navigator.clipboard.write([new ClipboardItem(data)]);
            } else if (typeof snapshot.text === 'string') {
                job.partial = snapshot.records.some(r => r.mime === 'image/png');
                result = navigator.clipboard.writeText(snapshot.text);
            }
            else throw Error('unsupported');
        }
        catch (error) { finishWrite(job, error); return; }
        Promise.resolve(result).then(() => finishWrite(job, null), error => finishWrite(job, error));
    }

    function drainWrite() {
        if (writing || !scheduledWrite) return;
        const pending = scheduledWrite; scheduledWrite = null;
        if (pending.generation !== writeGeneration || pending.offer !== offer || !live() || !focusedRemote()) return;
        const reason = capability();
        const legacy = canLegacyCopy(reason);
        if (reason && (!legacy || navigator.userActivation?.isActive !== true)) {
            offer.reason = legacy ? 'copy_gesture' : reason;
            offer.status = 'ready'; diagnose(offer.reason); publishCopy();
        } else write(offer, false, legacy);
    }

    function retryCopy() {
        if (!offer || offer.status !== 'ready' || writing || !live()
            || (!focusedRemote() && !assistTarget(document.activeElement))) return false;
        const reason = capability(true);
        const legacy = canLegacyCopy(reason);
        if (reason && !legacy) { offer.reason = reason; diagnose(reason, null, true); publishCopy(); return false; }
        ++writeGeneration; scheduledWrite = null;
        write(offer, true, legacy);
        return true;
    }

    function imageSnapshot() {
        const image = offer?.records.find(r => r.mime === 'image/png');
        return image && offer.status !== 'copied' && live() ? { id: offer.id, data: image.data } : null;
    }

    function copySnapshot() {
        return offer && typeof offer.text === 'string' && offer.status !== 'copied' && live() ? { id: offer.id, text: offer.text } : null;
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
                if (item.kind === 'input') {
                    if (item.isCurrent && !item.isCurrent()) { item.sent?.(false); continue; }
                    try { const sent = send(item.message); item.sent?.(sent !== false); }
                    catch (error) { item.sent?.(false); throw error; }
                    continue;
                }
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
                        const snapshot = await item.copy.promise;
                        if (generation !== epoch) throw Error('cancelled');
                        const payload = typeof snapshot === 'string' ? packFormats([{ mime: 'text/plain', data: new TextEncoder().encode(snapshot) }]) : snapshot;
                        const { bytes, formats } = payload;
                        item.formats = formats;
                        queuedBytes += bytes.length - item.bytes.length;
                        if (queuedBytes > MAX_BYTES) throw Error('limit');
                        item.bytes = bytes;
                    }
                    command({ type: 'start', id: item.id, size: item.bytes.length, shortcut: item.shortcut, target: item.target, ...(item.formats ? { formats: item.formats } : {}) });
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
            if (generation === epoch) {
                fail(error.message);
                const failedCopy = error.message === 'remote_copy_failed' ? remoteCopy : null;
                cancel(true, { preserveOffer: !!failedCopy });
                // A failed new selection must not expose the previous OS
                // clipboard on the next paste in the same remote context.
                if (failedCopy) remoteCopy = failedCopy;
            }
        } finally {
            if (generation === epoch) pumping = false;
        }
    }

    function newCapture(shortcut, element) {
        lateCopy = null;
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
        scheduledWrite = null;
        if (!item.copy) { offer = null; publishCopy(); }
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

    async function paste(event) {
        if (!remoteTarget(event.target) || !isActive() || event.defaultPrevented || caps?.enabled === false) return;
        event.preventDefault();
        if (!available()) return;
        const generation = epoch, item = capture || newCapture('ctrl-v', event.target);
        clearTimeout(item.timer); restore(item); capture = null;
        const data = event.clipboardData, records = [];
        const types = Array.from(data?.types || []), allowed = caps.mime_types || ['text/plain'];
        // Capture synchronous string data before awaiting a PNG File read.
        if (types.includes('text/plain')) records.push({ mime: 'text/plain', data: new TextEncoder().encode(data.getData('text/plain')) });
        if (!item.copy && types.includes('text/html') && allowed.includes('text/html')) records.push({ mime: 'text/html', data: new TextEncoder().encode(data.getData('text/html')) });
        const png = !item.copy && allowed.includes('image/png') ? Array.from(data?.items || []).find(i => i.kind === 'file' && i.type === 'image/png')?.getAsFile() : null;
        try {
            if (png) {
                if (png.size > Math.min(MAX_BYTES, caps.max_bytes)) throw Error('limit');
                records.push({ mime: 'image/png', data: new Uint8Array(await png.arrayBuffer()) });
            }
            if (generation !== epoch || !queue.includes(item)) return;
            if (!records.length && item.copy) records.push({ mime: 'text/plain', data: new Uint8Array() });
            if (!records.length) throw Error('text_required');
            const payload = packFormats(records, Math.min(MAX_BYTES, caps.max_bytes));
            if (queuedBytes + payload.bytes.length > MAX_BYTES) throw Error('limit');
            item.bytes = payload.bytes; item.formats = payload.formats; queuedBytes += item.bytes.length;
            pump();
        } catch (error) {
            if (generation !== epoch) return;
            queue = queue.filter(entry => entry !== item);
            fail(error.message); pump();
        }
    }

    function receive(content, metadata) {
        if (typeof content !== 'string') return;
        receiveFormats([{ mime: 'text/plain', data: new TextEncoder().encode(content) }], metadata);
    }

    function receiveFormats(records, metadata) {
        let snapshot;
        try { snapshot = packFormats(records); } catch (_) { fail('limit'); cancel(); return; }
        if (metadata?.revision) {
            if (metadata.revision <= remoteRevision) return;
            remoteRevision = metadata.revision;
        }
        // Browser writes are acknowledgements/state updates, never a remote copy.
        if (metadata?.origin === 'browser' || caps?.enabled === false) return;
        const foreground = live() && focusedRemote();
        const late = !foreground && lateCopy && isViewActive() && open()
            && Date.now() <= lateCopy.until && metadata?.revision >= lateCopy.minimum
            && (lateCopy.revision === null || metadata.revision === lateCopy.revision);
        if (!foreground && !late) return;
        const ownCopy = [...copies].find(copy => copy.sent);
        if (ownCopy) {
            clearTimeout(ownCopy.timer); copies.delete(ownCopy); ownCopy.resolve(snapshot);
        }
        if (!ownCopy && (current || capture || queue.length)) return;
        const copy = ownCopy || { promise: Promise.resolve(snapshot) };
        if (!ownCopy && foreground && !lateCopy) remoteCopy = copy;
        const paused = !!late || !!(lateCopy && Date.now() <= lateCopy.until
            && (lateCopy.revision === null || metadata?.revision === lateCopy.revision));
        const imageMissing = selectionStatus?.revision === metadata?.revision
            && ['disabled', 'format'].includes(selectionStatus.status) ? 'copy_' + selectionStatus.status : null;
        receiving = lateCopy = null;
        offer = { ...snapshot, id: ++copySequence, copy, status: paused ? 'ready' : 'pending',
            reason: paused ? 'copy_paused' : null, imageMissing };
        scheduledWrite = paused ? null : { offer, generation: ++writeGeneration };
        publishCopy();
        if (!paused) Promise.resolve().then(drainWrite);
    }

    function startCopy(copy) {
        copy.sent = true;
        copy.timer = setTimeout(() => {
            copies.delete(copy);
            if (remoteCopy === copy) remoteCopy = null;
            copy.reject(Error('remote_copy_timeout'));
            // Ctrl+C can be an application's interrupt command or have no
            // selection. Only an awaiting paste needs an error notification.
        }, 30000);
    }

    function createCopy() {
        const copy = {};
        copy.promise = new Promise((resolve, reject) => { copy.resolve = resolve; copy.reject = reject; });
        copy.promise.catch(() => {});
        return copy;
    }

    function noteCopy() {
        if (!open() || caps?.enabled === false || !isActive() || copies.size >= 32) return;
        lateCopy = receiving = selectionStatus = null;
        const copy = createCopy();
        copies.add(copy); remoteCopy = copy;
        if (pumping || queue.length) queue.push({ kind: 'copy', copy });
        else startCopy(copy);
    }

    // Native local UI copy supersedes any remote-copy shortcut we were tracking.
    const localCopy = event => {
        if (assistTarget(event.target)) remoteCopy = lateCopy = null;
        else if (!remoteTarget(event.target)) cancel();
    };
    const focusChanged = event => {
        if (!remoteTarget(event.target)) cancel(true, { preserveOffer: assistTarget(event.target) });
    };
    const visibilityChanged = () => { if (document.visibilityState === 'hidden') suspend(); };

    function message(type, data) {
        if (type === 'clipboard_status') {
            if (!Number.isSafeInteger(data?.revision) || data.revision <= remoteRevision
                || data.revision < (selectionStatus?.revision || 0) || caps?.enabled === false
                || !['receiving','disabled','format','timeout','limit','image_limit','invalid_image','invalid_content','read_failed','busy'].includes(data.status)) return;
            const foreground = live() && focusedRemote();
            const late = lateCopy && isViewActive() && open() && Date.now() <= lateCopy.until && data.revision >= lateCopy.minimum
                && (lateCopy.revision === null || data.revision === lateCopy.revision);
            if (!foreground && !late) return;
            selectionStatus = data;
            if (late && lateCopy.revision === null) lateCopy.revision = data.revision;
            if (data.status === 'receiving') {
                receiving = data;
                offer = null; ++writeGeneration; scheduledWrite = null;
                // Menu copies have no Ctrl+C intent. Bind an immediate paste
                // to this in-progress selection, never to an earlier snapshot.
                if (foreground && ![...copies].some(copy => copy.sent) && !current && !capture && !queue.length) {
                    const copy = createCopy();
                    copies.add(copy); remoteCopy = copy; startCopy(copy);
                }
            } else {
                receiving = null; ++writeGeneration; scheduledWrite = null;
                offer = { records: [], text: null, id: ++copySequence, status: 'ready', reason: 'copy_' + data.status };
                for (const copy of copies) {
                    if (!copy.sent) continue;
                    clearTimeout(copy.timer); copies.delete(copy); copy.reject(Error('remote_copy_failed'));
                }
                if (foreground) {
                    remoteCopy = createCopy();
                    remoteCopy.reject(Error('remote_copy_failed'));
                }
            }
            publishCopy();
        } else if (type === 'clipboard_caps') {
            if (data?.version === 1 && Number.isInteger(data.chunk_bytes) && data.chunk_bytes > 0
                && Number.isInteger(data.max_bytes) && data.max_bytes >= 0) {
                caps = data;
                if (caps.enabled === false) cancel();
                else if (caps.mime_types && !caps.mime_types.includes('image/png')) {
                    cancel();
                }
            }
        } else if (type === 'paste_ack' && current?.id === data?.id) {
            if (data.status === 'applied') {
                remoteRevision = Math.max(remoteRevision, data.revision || 0);
                current.resolve();
            } else current.reject(Error(data.status));
        }
    }

    function sendInput(message, isCurrent = null) {
        if (!isActive() || disposed || !open() || (isCurrent && !isCurrent())) return false;
        if (capture || pumping || queue.length) {
            if (queue.length >= 512) { fail('limit'); cancel(); return false; }
            // A text receipt timeout starts when it actually leaves this
            // ordered queue, not while a preceding paste is still uploading.
            return new Promise(sent => queue.push({ kind: 'input', message, sent, isCurrent }));
        } else return send(message);
    }

    const pageFocus = () => publishCopy();
    globalThis.window?.addEventListener('focus', pageFocus);
    document.addEventListener('paste', paste);
    document.addEventListener('copy', localCopy);
    document.addEventListener('cut', localCopy);
    document.addEventListener('focusin', focusChanged);
    document.addEventListener('visibilitychange', visibilityChanged);
    return {
        beginPaste, sendInput, noteCopy, receive, receiveFormats, message, cancel, suspend, retryCopy, copySnapshot, imageSnapshot, dismissCopy,
        connected() { cancel(); caps = null; remoteRevision = 0; command({ type: 'hello' }); },
        disconnected() { cancel(false); caps = null; },
        destroy() {
            cancel(); disposed = true;
            globalThis.window?.removeEventListener('focus', pageFocus);
            permission?.removeEventListener?.('change', permissionChanged);
            document.removeEventListener('paste', paste);
            document.removeEventListener('copy', localCopy);
            document.removeEventListener('cut', localCopy);
            document.removeEventListener('focusin', focusChanged);
            document.removeEventListener('visibilitychange', visibilityChanged);
        },
    };
}
