// Keep the work area clear before revealing the toolbar and until its hiding
// transition finishes. Layout changes alone never change toolbar visibility.
export function createToolbarVisibility({ element, pinned, interacting = () => false, onChange = () => {} }) {
    let disposed = false, hideTimer = 0, hidingFrame = 0;
    let reserved = !!pinned || element.classList.contains('visible');
    element.classList.toggle('pinned', !!pinned);

    function reserve(value) {
        if (reserved === value) return;
        reserved = value;
        onChange();
    }
    function cancelHide() {
        clearTimeout(hideTimer); hideTimer = 0;
        cancelAnimationFrame(hidingFrame); hidingFrame = 0;
    }
    function finishHide() {
        hidingFrame = 0;
        if (disposed || pinned || element.classList.contains('visible')) return;
        const style = getComputedStyle(element);
        // Inspect the actual transform, rather than relying on transitionend:
        // zero-duration/reduced-motion and cancelled transitions may omit it.
        const translation = style.transform === 'none' ? 0 : new DOMMatrixReadOnly(style.transform).m42;
        const hidden = !element.isConnected || style.display === 'none'
            || translation >= element.getBoundingClientRect().height - 0.5;
        if (hidden) reserve(false);
        else hidingFrame = requestAnimationFrame(finishHide);
    }
    function conceal() {
        cancelHide();
        element.classList.remove('visible');
        if (!pinned) finishHide();
    }
    return {
        isReserved: () => reserved,
        show() {
            if (disposed) return;
            cancelHide();
            reserve(true);
            element.classList.add('visible');
        },
        hide({ immediate = false } = {}) {
            if (disposed || pinned || interacting()) return;
            clearTimeout(hideTimer);
            if (immediate) conceal();
            else hideTimer = setTimeout(() => {
                hideTimer = 0;
                if (!disposed && !pinned && !interacting()) conceal();
            }, 400);
        },
        setPinned(value) {
            if (disposed || pinned === !!value) return;
            pinned = !!value;
            cancelHide();
            if (pinned) reserve(true);
            element.classList.toggle('pinned', pinned);
            if (!pinned && !element.classList.contains('visible')) finishHide();
        },
        destroy() { disposed = true; cancelHide(); },
    };
}
