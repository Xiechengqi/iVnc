// Video readiness is independent of ICE/DataChannel readiness. Keep playback
// failures visible and recover only on stream replacement or view activation.
export function createVideoPlayback({ video, onChange = () => {}, getStats = async () => null,
    requestKeyframe = () => {}, diagnostic = () => {}, documentRef = globalThis.document,
    timeoutMs = 8000, intervalMs = 1000, now = () => performance.now() }) {
    let connected = false, active = true, firstFrame = false, state = 'connecting';
    let timer = null, frameCallback = null, generation = 0, started = null, requested = false, disposed = false;
    let sample = null, polling = null, playAttempt = 0;
    const visible = () => active && !documentRef?.hidden;
    const notify = (next, detail = null) => {
        if (disposed || (state === next && !detail)) return;
        state = next;
        if (detail || ['blocked', 'no-data', 'decode-error', 'playback-error', 'connection-error', 'unsupported'].includes(next)) {
            diagnostic({ state: next, detail, readyState: video.readyState, paused: video.paused,
                width: video.videoWidth, height: video.videoHeight, video: sample?.video || null });
        }
        onChange({ state, firstFrame, detail });
    };
    const cancelFrame = () => {
        if (frameCallback !== null) video.cancelVideoFrameCallback?.(frameCallback);
        frameCallback = null;
    };
    const rendered = () => {
        if (disposed || !connected || ['unsupported', 'connection-error'].includes(state) || !visible() || !video.srcObject || video.paused || video.readyState < 2 || !video.videoWidth) return;
        firstFrame = true; started = null;
        notify('playing');
    };
    const watchFrame = () => {
        cancelFrame();
        if (!video.requestVideoFrameCallback || firstFrame || !visible()) return;
        const current = generation;
        frameCallback = video.requestVideoFrameCallback(() => {
            frameCallback = null;
            if (current === generation) rendered();
        });
    };
    const play = () => {
        if (disposed || ['unsupported', 'connection-error'].includes(state) || !video.srcObject || !visible()) return Promise.resolve(false);
        video.muted = true;
        started = null; requested = false;
        const current = generation;
        const attempt = ++playAttempt;
        if (connected && state !== 'playing') notify('waiting');
        watchFrame();
        let promise;
        try { promise = video.play(); } catch (error) { promise = Promise.reject(error); }
        return Promise.resolve(promise).then(() => {
            if (disposed || current !== generation || attempt !== playAttempt) return false;
            rendered();
            return true;
        }).catch(error => {
            if (disposed || current !== generation || attempt !== playAttempt || !visible()) return false;
            if (error.name === 'AbortError') return false;
            notify(error.name === 'NotAllowedError' ? 'blocked' : error.name === 'NotSupportedError' ? 'decode-error' : 'playback-error', error.name);
            return false;
        });
    };
    const tick = async () => {
        if (disposed || !connected || !visible()) { started = null; return; }
        rendered();
        if (state === 'blocked' || state === 'unsupported' || state === 'connection-error' || firstFrame) return;
        if (started === null) started = now();
        const elapsed = now() - started;
        if (!requested && elapsed >= timeoutMs / 2) { requested = true; requestKeyframe(); }
        if (polling === generation) return;
        const current = generation;
        polling = current;
        try {
            const stats = await getStats();
            if (disposed || current !== generation || !connected || !visible() || firstFrame) return;
            sample = stats;
            if (elapsed >= timeoutMs && !['blocked', 'decode-error', 'playback-error'].includes(state)) {
                const v = sample?.video;
                // Some Safari versions omit frame counters. Missing telemetry
                // cannot establish a decoder failure.
                notify(v?.bytesReceived === 0 ? 'no-data' : v?.framesDecoded === 0 && v?.bytesReceived > 0 ? 'decode-error' : 'playback-error');
            }
        } catch (_) {
            if (!disposed && current === generation && connected && visible() && elapsed >= timeoutMs && !firstFrame) notify('playback-error', 'stats unavailable');
        } finally { if (polling === current) polling = null; }
    };
    const resume = () => {
        started = null; requested = false;
        if (!visible()) { cancelFrame(); return; }
        if (connected && video.srcObject) {
            if (video.paused) { firstFrame = false; play(); }
            else { rendered(); watchFrame(); }
            requestKeyframe();
        }
    };
    const mediaError = () => {
        if (video.srcObject && !['unsupported', 'connection-error'].includes(state)) {
            firstFrame = false; notify('playback-error', video.error?.code);
        }
    };
    const paused = () => {
        if (connected && visible() && video.srcObject && !['unsupported', 'connection-error'].includes(state)) {
            firstFrame = false; started = null; notify('blocked');
        }
    };
    for (const event of ['loadeddata', 'playing', 'timeupdate']) video.addEventListener(event, rendered);
    video.addEventListener('error', mediaError);
    video.addEventListener('pause', paused);
    documentRef?.addEventListener('visibilitychange', resume);
    return {
        play,
        resume,
        state: () => state,
        firstFrame: () => firstFrame,
        attach(stream) {
            if (disposed) return;
            generation++; firstFrame = false; sample = null; requested = false; started = null;
            video.srcObject = stream;
            notify('waiting');
            play();
        },
        connection(next) {
            if (disposed) return;
            connected = next === 'connected'; started = null;
            clearInterval(timer); timer = null;
            if (connected) {
                timer = setInterval(tick, intervalMs);
                if (state === 'connection-error') notify('waiting');
                rendered();
                if (!firstFrame && !['blocked', 'unsupported', 'decode-error', 'playback-error'].includes(state)) notify('waiting');
                watchFrame();
            } else {
                cancelFrame();
                if (state !== 'unsupported') notify(['failed', 'disconnected', 'closed'].includes(next) ? 'connection-error' : 'connecting');
            }
        },
        setActive(value) { const changed = active !== !!value; active = !!value; if (changed) resume(); },
        fail(next, detail) { generation++; cancelFrame(); firstFrame = false; notify(next, detail); },
        reset() {
            generation++; connected = false; firstFrame = false; sample = null; started = null; requested = false;
            clearInterval(timer); timer = null; cancelFrame();
            video.srcObject = null; notify('connecting');
        },
        destroy() {
            disposed = true; generation++; clearInterval(timer); cancelFrame();
            video.srcObject = null;
            for (const event of ['loadeddata', 'playing', 'timeupdate']) video.removeEventListener(event, rendered);
            video.removeEventListener('error', mediaError); video.removeEventListener('pause', paused);
            documentRef?.removeEventListener('visibilitychange', resume);
        },
    };
}
