// Owns one console child dialog at a time, including its async lifetime.
export function createConsoleDialogs() {
    let current = null, sequence = 0, down = null;
    const documentId = globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const events = new AbortController();
    const listen = (target, type, handler, options = {}) => target.addEventListener(type, handler, { ...options, signal: events.signal });
    const state = {
        get id() { return current?.id ?? null; },
        get element() { return current?.element ?? null; },
        dismiss(id) { if (current?.id === id) close(); },
        focus() { focus(); },
    };
    window.__ivncConsoleDialogState = state;

    function notify() {
        if (parent === window) return;
        // Same-origin synchronization installs the shield before another input
        // can enter an application iframe. Messages also handle frame loading.
        try { parent.__ivncSyncConsoleDialogs?.(window); } catch (_) {}
        parent.postMessage({ type: 'ivnc-console-dialog-state' }, location.origin);
    }
    function focus() {
        const element = current?.element;
        if (!element) return;
        const target = [...element.querySelectorAll('[autofocus], input:not([disabled]):not([type="hidden"]), button:not([disabled]), [tabindex="0"]')]
            .find(node => node.getClientRects().length && getComputedStyle(node).visibility !== 'hidden');
        (target || element).focus({ preventScroll: true });
    }
    function close(restoreFocus = true) {
        if (!current) return;
        const previous = current;
        current = null;
        previous.abort.abort();
        previous.element.classList.remove('show');
        previous.onClose?.();
        notify();
        if (restoreFocus && previous.opener?.isConnected) previous.opener.focus({ preventScroll: true });
    }
    function open(element, onClose) {
        close(false);
        const session = {
            id: `${documentId}:${++sequence}`, element, onClose, opener: document.activeElement,
            abort: new AbortController(), saving: false,
            isCurrent: () => current === session,
        };
        current = session;
        element.classList.add('show');
        const dialog = element.querySelector('.modal');
        dialog.setAttribute('role', 'dialog');
        dialog.setAttribute('aria-modal', 'true');
        dialog.tabIndex = -1;
        const title = dialog.querySelector('h2');
        if (title?.id) dialog.setAttribute('aria-labelledby', title.id);
        notify();
        focus();
        return session;
    }
    listen(document, 'pointerdown', event => {
        down = current && event.target === current.element && event.button === 0
            ? { id: current.id, x: event.clientX, y: event.clientY } : null;
    }, { capture: true });
    listen(document, 'click', event => {
        if (current && event.target === current.element && down?.id === current.id
            && Math.hypot(event.clientX - down.x, event.clientY - down.y) < 5) {
            event.preventDefault(); event.stopImmediatePropagation(); close();
        }
        down = null;
    }, { capture: true });
    listen(document, 'pointercancel', () => { down = null; });
    listen(document, 'keydown', event => {
        if (!current || event.isComposing || event.keyCode === 229 || event.defaultPrevented) return;
        if (event.key === 'Escape') {
            // A select popup handles its own first Escape before this bubble listener.
            event.preventDefault(); event.stopPropagation(); close();
        } else if (event.key === 'Tab') {
            const items = [...current.element.querySelectorAll('button, input, textarea, select, a[href], [tabindex="0"]')]
                .filter(node => !node.disabled && node.tabIndex >= 0 && node.getClientRects().length && getComputedStyle(node).visibility !== 'hidden');
            if (!items.length) { event.preventDefault(); focus(); return; }
            const first = items[0], last = items.at(-1), active = document.activeElement;
            if (event.shiftKey && (active === first || !items.includes(active))) {
                event.preventDefault(); last.focus();
            } else if (!event.shiftKey && (active === last || !items.includes(active))) {
                event.preventDefault(); first.focus();
            }
        }
    });
    listen(document, 'focusin', event => {
        if (current && !current.element.contains(event.target) && !event.target.closest?.('[data-console-select-popup]')) focus();
    });
    listen(window, 'message', event => {
        if (event.origin !== location.origin || event.source !== parent) return;
        if (event.data?.type === 'ivnc-console-dialog-dismiss') state.dismiss(event.data.id);
        if (event.data?.type === 'ivnc-console-dialog-query') notify();
    });
    listen(window, 'pageshow', notify);
    listen(window, 'pagehide', event => {
        if (event.persisted) return;
        close(false); events.abort();
        if (window.__ivncConsoleDialogState === state) delete window.__ivncConsoleDialogState;
    });
    notify();
    return { open, close, get current() { return current; } };
}
