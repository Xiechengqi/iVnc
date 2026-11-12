// One gesture belongs to the whole desktop, even when its wheel events cross
// from the main video into an application's same-origin iframe.
const IDLE_MS = 260;
const AXIS_START = 8;
const SWITCH_DISTANCE = 100;

export function createGestureNavigation({ navigate, canHandle, now = () => performance.now() }) {
    let enabled = false;
    let reversed = false;
    let mode = 'idle';
    let x = 0, y = 0, last = 0, source = null;

    function reset() {
        mode = 'idle'; x = 0; y = 0; last = 0; source = null;
    }

    function move(dx, dy, origin, blocked) {
        if (!enabled || !Number.isFinite(dx) || !Number.isFinite(dy)) return false;
        const time = now();
        if (last && time - last > IDLE_MS) reset();
        // The newly selected iframe can receive the old gesture's momentum.
        // Consume it before checking which view is active.
        if (mode === 'committed') { last = time; return true; }
        if (blocked || !canHandle(origin)) { reset(); return false; }
        if (mode !== 'idle' && source !== origin) reset();
        last = time; source = origin;
        if (mode === 'vertical') return false;
        x += dx; y += dy;
        const ax = Math.abs(x), ay = Math.abs(y);
        if (mode === 'horizontal' && ax < SWITCH_DISTANCE && ay > ax * 1.2) {
            mode = 'vertical'; return false;
        }
        if (mode === 'idle') {
            if (ay >= AXIS_START && ay >= ax * 1.4) {
                mode = 'vertical'; return false;
            }
            if (ax >= AXIS_START && ax >= ay * 1.8) mode = 'horizontal';
            else if (ax + ay >= 40) { mode = 'vertical'; return false; }
        }
        if (mode === 'horizontal' && ax >= SWITCH_DISTANCE) {
            mode = 'committed';
            navigate((x > 0 ? 1 : -1) * (reversed ? -1 : 1));
        }
        // Reserve horizontal motion from its first sample. This avoids moving
        // remote content before deciding to switch tabs.
        return true;
    }

    return {
        enabled: () => enabled,
        reversed: () => reversed,
        setEnabled(value) { enabled = !!value; reset(); },
        setReversed(value) { reversed = !!value; reset(); },
        reset,
        wheel(event, origin, blocked = false) {
            if (!enabled) return false;
            if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) {
                if (mode !== 'committed') reset();
                return false;
            }
            const unit = event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? 800 : 1;
            return move(event.deltaX * unit, event.deltaY * unit, origin, blocked);
        },
        // Touch coordinates move opposite to scroll deltas. The caller tracks
        // both contacts and blocks navigation once their separation changes.
        touch(dx, dy, origin, blocked = false) {
            return move(-dx, -dy, origin, blocked);
        },
        endTouch() { reset(); },
    };
}
