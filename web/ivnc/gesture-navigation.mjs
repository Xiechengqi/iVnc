// One gesture belongs to the whole desktop, even when its wheel events cross
// from the main video into an application's same-origin iframe.
const IDLE_MS = 260;
const AXIS_START = 8;
const SWITCH_DISTANCE = 100;
// Guard the rising peak of one swipe, then accept a sustained new burst rather
// than extending a lock every time the old swipe emits another momentum tick.
const REARM_MS = 100;
const PAUSE_MS = 90;
const DECISION_MS = 60;

export function createGestureNavigation({ navigate, canNavigate = () => true, canHandle, now = () => performance.now(),
    schedule = (fn, ms) => setTimeout(fn, ms), cancel = id => clearTimeout(id) }) {
    let mode = 'idle';
    let x = 0, y = 0, last = null, source = null, kind = null;
    let committedAt = 0, direction = 0, peak = 0, valley = Infinity, previous = 0;
    let candidate = null;
    let pendingX = 0, pendingY = 0, replay = null, decisionTimer = null;

    function discardPending() {
        if (decisionTimer !== null) cancel(decisionTimer);
        decisionTimer = null; pendingX = 0; pendingY = 0; replay = null;
    }

    function yieldPending() {
        const dx = pendingX, dy = pendingY, callback = replay;
        discardPending();
        if (canHandle(source)) callback?.(dx, dy);
    }

    function reserve(dx, dy, onScroll) {
        pendingX += dx; pendingY += dy; replay = onScroll;
        if (mode !== 'idle' || decisionTimer !== null) return;
        // An ambiguous final sample must not remain swallowed indefinitely.
        decisionTimer = schedule(() => {
            decisionTimer = null;
            if (mode !== 'idle') return;
            mode = 'vertical'; yieldPending();
        }, DECISION_MS);
    }

    function reset() {
        discardPending();
        mode = 'idle'; x = 0; y = 0; last = null; source = null; kind = null;
        committedAt = 0; direction = 0; peak = 0; valley = Infinity; previous = 0; candidate = null;
    }

    function commit(time, dx) {
        discardPending();
        mode = 'committed'; committedAt = time; direction = Math.sign(dx);
        valley = Infinity; candidate = null;
        navigate(direction);
    }

    function move(dx, dy, origin, blocked, inputKind, onScroll) {
        if (!Number.isFinite(dx) || !Number.isFinite(dy)) return false;
        if (!dx && !dy) return false;
        const time = now();
        const gap = last === null ? Infinity : time - last;
        if (kind !== inputKind || (inputKind === 'wheel' && gap > IDLE_MS)) reset();
        const allowed = canHandle(origin);
        const ax = Math.abs(dx), ay = Math.abs(dy);
        if (mode === 'consumed') { last = time; return true; }
        if (mode === 'committed') {
            last = time;
            // A view switch moves the remaining momentum into another iframe.
            // Hidden views can drain it, but cannot initiate another switch.
            if (inputKind === 'touch' || !allowed || blocked) {
                valley = Math.min(valley, ax); previous = ax;
                candidate = null; return true;
            }
            if (time - committedAt >= REARM_MS && ay >= Math.max(24, ax * 3)) {
                mode = 'vertical'; source = origin; candidate = null;
                return false;
            }
            const sign = Math.sign(dx), horizontal = ax >= AXIS_START && ax >= ay * 1.8;
            if (time - committedAt >= REARM_MS && horizontal) {
                const opposite = sign !== direction;
                const resumed = gap >= PAUSE_MS && ax >= previous * .9;
                const accelerated = valley <= Math.max(4, peak * .35) &&
                    ax >= Math.max(AXIS_START, valley * 2.2) && ax - valley >= 6;
                if (candidate && (candidate.sign !== sign || candidate.source !== origin || ax < candidate.peak * .55)) candidate = null;
                if (!candidate && (opposite || resumed || accelerated)) candidate = { sign, source: origin, x: 0, y: 0, samples: 0, peak: ax };
                if (candidate) {
                    candidate.x += dx; candidate.y += dy; candidate.samples++;
                    candidate.peak = Math.max(candidate.peak, ax);
                    if (candidate.samples >= 2 && Math.abs(candidate.x) >= SWITCH_DISTANCE && Math.abs(candidate.x) >= Math.abs(candidate.y) * 1.8) {
                        // This is a newly recognized gesture, not the old tail.
                        // At the boundary it belongs to the remote application.
                        if (!canNavigate(sign)) {
                            onScroll?.(candidate.x - dx, candidate.y - dy);
                            mode = 'passthrough'; source = origin; candidate = null;
                            return false;
                        }
                        peak = candidate.peak;
                        commit(time, candidate.x);
                    }
                }
            } else candidate = null;
            valley = Math.min(valley, ax); previous = ax;
            return true;
        }
        if (!allowed) { reset(); return false; }
        if (mode !== 'idle' && source !== origin) reset();
        last = time; source = origin; kind = inputKind;
        if (mode === 'vertical' || mode === 'passthrough') return false;
        if (mode === 'horizontal' && blocked) {
            // A modifier/pinch/drag beginning after reservation cancels this
            // switch, but must not deliver a partial gesture to the application.
            mode = 'consumed';
            discardPending();
            return true;
        }
        // Decide before reserving any horizontal motion. Keep a yielded gesture
        // yielded even if modifiers, direction or available targets change.
        if (blocked || (mode === 'idle' && (dx ? !canNavigate(Math.sign(dx)) : !canNavigate(-1) && !canNavigate(1)))) {
            mode = 'passthrough';
            yieldPending();
            return false;
        }
        x += dx; y += dy;
        peak = Math.max(peak, ax); previous = ax;
        const totalX = Math.abs(x), totalY = Math.abs(y);
        if (mode === 'horizontal' && totalX < SWITCH_DISTANCE && totalY > totalX * 1.2) {
            mode = 'vertical'; yieldPending(); return false;
        }
        if (mode === 'idle') {
            if (totalY > 0 && totalY >= totalX * 1.4) {
                mode = 'vertical'; yieldPending(); return false;
            }
            if (totalX >= AXIS_START && totalX >= totalY * 1.8) {
                mode = 'horizontal';
                if (decisionTimer !== null) cancel(decisionTimer);
                decisionTimer = null;
            }
            else if (totalX + totalY >= 40) { mode = 'vertical'; yieldPending(); return false; }
        }
        if (mode === 'horizontal' && totalX >= SWITCH_DISTANCE) commit(time, x);
        else reserve(dx, dy, onScroll);
        // Reserve horizontal motion from its first sample. This avoids moving
        // remote content before deciding to switch tabs.
        return true;
    }

    return {
        enabled: () => true,
        reset,
        cancelPending(origin) { if (source === origin && (mode === 'idle' || mode === 'horizontal')) reset(); },
        wheel(event, origin, blocked = false, onScroll = null) {
            const unit = event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? 800 : 1;
            const modified = event.ctrlKey || event.metaKey || event.altKey || event.shiftKey;
            return move(event.deltaX * unit, event.deltaY * unit, origin, blocked || modified, 'wheel', onScroll);
        },
        // Touch coordinates move opposite to scroll deltas. The caller tracks
        // both contacts and blocks navigation once their separation changes.
        touch(dx, dy, origin, blocked = false, onScroll = null) {
            return move(-dx, -dy, origin, blocked, 'touch', onScroll);
        },
        endTouch(cancelled = false) {
            if (kind !== 'touch') return;
            if (!cancelled && mode === 'idle') yieldPending();
            reset();
        },
    };
}
