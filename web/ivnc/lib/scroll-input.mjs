// Browser deltas are logical pixels, independent of video scaling and DPR.
export const SCROLL_LINE_PIXELS = 24;
export const MAX_SCROLL_PIXELS = 32768;
// Finger travel is shorter than a mouse notch. 48 CSS pixels at speed 1
// become one 120-pixel notch. Mouse wheels and trackpads do not use this.
export const TOUCH_SCROLL_GAIN = 2.5;
// Wayland's conventional 10 axis units per wheel notch map to 120 Chromium
// wheel pixels. Keep the same calibration when talking to an older server.
const LEGACY_PIXELS_PER_UNIT = 12;
// wl_pointer uses 24.8 fixed point. Retain its remainder here rather than
// losing the same fraction on every Wayland event (or losing tiny motion).
const PIXEL_QUANTUM = LEGACY_PIXELS_PER_UNIT / 256;
const clamp = value => Math.max(-MAX_SCROLL_PIXELS, Math.min(MAX_SCROLL_PIXELS, value));

export function normalizeWheel(event, width = 800, height = 800) {
    const { deltaX, deltaY, deltaMode = 0 } = event;
    if (!Number.isFinite(deltaX) || !Number.isFinite(deltaY) || ![0, 1, 2].includes(deltaMode)) return null;
    const page = size => Number.isFinite(size) && size > 0 ? size : 800;
    const xUnit = deltaMode === 1 ? SCROLL_LINE_PIXELS : deltaMode === 2 ? page(width) : 1;
    const yUnit = deltaMode === 1 ? SCROLL_LINE_PIXELS : deltaMode === 2 ? page(height) : 1;
    return { x: clamp(deltaX * xUnit), y: clamp(deltaY * yUnit) };
}

// An 8 ms batch retains all normal motion, including fractional legacy units.
// No delayed retry is allowed to carry input into another view/connection.
export function createScrollInput({ send, canSend, getSpeed = () => 1,
    schedule = (fn, ms) => setTimeout(fn, ms), cancel = id => clearTimeout(id) }) {
    let x = 0, y = 0, timer = null, precise = false;
    function reset() {
        if (timer !== null) cancel(timer);
        timer = null; x = 0; y = 0;
    }
    function flush() {
        if (timer !== null) cancel(timer);
        timer = null;
        if (!canSend()) { reset(); return; }
        const unit = precise ? PIXEL_QUANTUM : LEGACY_PIXELS_PER_UNIT;
        const quantize = value => Math.trunc(value / unit) * (precise ? unit : 1);
        const dx = quantize(x), dy = quantize(y);
        if (!dx && !dy) return;
        if (send(`${precise ? 'w2' : 'w'},${dx},${dy}`) === false) { reset(); return; }
        x -= dx * (precise ? 1 : unit); y -= dy * (precise ? 1 : unit);
    }
    return {
        add(dx, dy) {
            if (!canSend() || !Number.isFinite(dx) || !Number.isFinite(dy)) return;
            const speed = getSpeed();
            x = clamp(x + clamp(dx) * speed); y = clamp(y + clamp(dy) * speed);
            if ((x || y) && timer === null) timer = schedule(flush, 8);
        },
        setProtocol(version) {
            if (precise === (version === 1)) return;
            // Include a legacy fractional remainder when upgrading the channel.
            flush(); precise = version === 1;
            if (x || y) timer = schedule(flush, 8);
        },
        reset, flush,
    };
}
