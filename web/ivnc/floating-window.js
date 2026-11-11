const clamp = (value, low, high) => Math.max(low, Math.min(high, value));

// Geometry uses viewport CSS coordinates for position and unscaled dimensions
// for size. Browser DPR is already accounted for by clientX/clientY.
export function createFloatingWindow({ modal, getScale, getBounds, getLimits, onResize, onInteraction }) {
    const key = `ivnc.terminal.window.v1:${location.pathname}`;
    const defaults = { width: 920, height: 560, x: .5, y: .5, pinned: false };
    let state = { ...defaults }, gesture = null, frame = 0, rendered = null;
    const events = new AbortController();
    const listen = (target, type, handler, options = {}) => target.addEventListener(type, handler, { ...options, signal: events.signal });
    try {
        const stored = JSON.parse(sessionStorage.getItem(key));
        if (stored && ['width', 'height', 'x', 'y'].every(name => Number.isFinite(stored[name]))
            && stored.width >= 100 && stored.width <= 10000 && stored.height >= 80 && stored.height <= 10000
            && stored.x >= 0 && stored.x <= 1 && stored.y >= 0 && stored.y <= 1 && typeof stored.pinned === 'boolean') {
            state = { width: stored.width, height: stored.height, x: stored.x, y: stored.y, pinned: stored.pinned };
        }
    } catch (_) {}
    modal.dataset.terminalPinned = String(state.pinned);
    modal.classList.add('web-terminal-floating');
    modal.style.transformOrigin = 'top left';
    modal.style.minWidth = '0'; modal.style.minHeight = '0';
    for (const edge of ['n', 'ne', 'e', 'se', 's', 'sw', 'w', 'nw']) {
        const handle = document.createElement('div');
        handle.className = `web-terminal-resize resize-${edge}`; handle.dataset.edge = edge;
        handle.setAttribute('aria-hidden', 'true'); modal.appendChild(handle);
    }
    function save() { try { sessionStorage.setItem(key, JSON.stringify(state)); } catch (_) {} }
    function measure() {
        const scale = getScale(), bounds = getBounds(), limits = getLimits();
        const availableWidth = Math.max(1, bounds.right - bounds.left), availableHeight = Math.max(1, bounds.bottom - bounds.top);
        const maxWidth = Math.min(availableWidth, limits.maxWidth * scale);
        const maxHeight = Math.min(availableHeight, limits.maxHeight * scale);
        return { scale, bounds, maxWidth, maxHeight, minWidth: Math.min(320 * scale, maxWidth), minHeight: Math.min(240 * scale, maxHeight) };
    }
    function paint() {
        frame = 0;
        const { scale, bounds, minWidth, minHeight, maxWidth, maxHeight } = measure();
        const width = clamp(state.width * scale, minWidth, maxWidth), height = clamp(state.height * scale, minHeight, maxHeight);
        const left = bounds.left + Math.max(0, bounds.right - bounds.left - width) * state.x;
        const top = bounds.top + Math.max(0, bounds.bottom - bounds.top - height) * state.y;
        rendered = { left, top, width, height };
        modal.style.setProperty('--terminal-resize-hit', `${6 / scale}px`);
        modal.style.setProperty('--terminal-resize-corner', `${10 / scale}px`);
        Object.assign(modal.style, { left: `${left}px`, top: `${top}px`, width: `${width / scale}px`, height: `${height / scale}px`, transform: `scale(${scale})` });
    }
    function layout() { if (!frame) frame = requestAnimationFrame(paint); }
    function finish(rollback = false) {
        if (!gesture) return;
        const previous = gesture; gesture = null;
        if (rollback) state = previous.state;
        try { previous.target.releasePointerCapture(previous.id); } catch (_) {}
        modal.classList.remove('window-interacting');
        if (frame) { cancelAnimationFrame(frame); frame = 0; }
        paint(); save(); onResize();
    }
    listen(modal, 'pointerdown', event => {
        const edge = event.target.closest('.web-terminal-resize')?.dataset.edge;
        const header = event.target.closest('.web-terminal-header');
        if (gesture || event.button !== 0 || (!edge && (!header || event.target.closest('.web-terminal-actions')))) return;
        paint();
        gesture = { id: event.pointerId, target: event.target, x: event.clientX, y: event.clientY,
            edge, rect: { ...rendered }, state: { ...state }, scale: getScale() };
        event.target.setPointerCapture(event.pointerId);
        event.preventDefault(); event.stopPropagation();
        modal.classList.add('window-interacting'); onInteraction();
    });
    listen(modal, 'pointermove', event => {
        if (!gesture || gesture.id !== event.pointerId) return;
        event.preventDefault(); event.stopPropagation();
        const { scale, bounds, minWidth, minHeight, maxWidth, maxHeight } = measure();
        if (scale !== gesture.scale) { finish(); return; }
        const dx = event.clientX - gesture.x, dy = event.clientY - gesture.y;
        const start = gesture.rect, edge = gesture.edge;
        let { left, top, width, height } = start;
        if (!edge) { left += dx; top += dy; }
        else {
            if (edge.includes('e')) width = clamp(start.width + dx, minWidth, Math.min(maxWidth, bounds.right - left));
            if (edge.includes('w')) {
                width = clamp(start.width - dx, minWidth, Math.min(maxWidth, left + start.width - bounds.left));
                left = start.left + start.width - width;
            }
            if (edge.includes('s')) height = clamp(start.height + dy, minHeight, Math.min(maxHeight, bounds.bottom - top));
            if (edge.includes('n')) {
                height = clamp(start.height - dy, minHeight, Math.min(maxHeight, top + start.height - bounds.top));
                top = start.top + start.height - height;
            }
            state.width = width / scale; state.height = height / scale;
        }
        const roomX = Math.max(0, bounds.right - bounds.left - width), roomY = Math.max(0, bounds.bottom - bounds.top - height);
        state.x = roomX ? clamp((left - bounds.left) / roomX, 0, 1) : .5;
        state.y = roomY ? clamp((top - bounds.top) / roomY, 0, 1) : .5;
        layout();
    });
    listen(modal, 'pointerup', event => { if (gesture?.id === event.pointerId) { event.preventDefault(); finish(); } });
    listen(modal, 'pointercancel', () => finish());
    listen(modal, 'lostpointercapture', event => { if (gesture?.id === event.pointerId) finish(); });
    listen(document, 'keydown', event => {
        if (gesture && event.key === 'Escape' && !event.isComposing) {
            event.preventDefault(); event.stopImmediatePropagation(); finish(true);
        }
    }, { capture: true });
    listen(window, 'blur', () => finish());
    listen(document, 'visibilitychange', () => { if (document.hidden) finish(); });
    listen(window, 'ivnc-overlay-layout', () => { if (gesture) finish(); layout(); });
    paint();
    return {
        layout: () => { if (frame) cancelAnimationFrame(frame); paint(); },
        isPinned: () => state.pinned,
        setPinned(value) { state.pinned = !!value; modal.dataset.terminalPinned = String(state.pinned); save(); },
        cancel: () => finish(),
        destroy() { finish(); events.abort(); cancelAnimationFrame(frame); },
    };
}
