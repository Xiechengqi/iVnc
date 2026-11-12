const controllers = () => window.__ivncOverlayControllers || [];
const consume = event => { event.preventDefault(); event.stopImmediatePropagation(); };
const criticalTarget = target => target?.closest?.('.pwd-overlay, .update-overlay, .force-update-modal');

export function releaseOverlayInput() {
    if (document.pointerLockElement) document.exitPointerLock();
    navigator.keyboard?.unlock?.();
    window.dispatchEvent(new Event('ivnc-overlay-interaction'));
}

export function minimizeOtherOverlays(active = null) {
    for (const controller of controllers()) {
        if (controller !== active && controller.isOpen?.() && !controller.isPinned?.()) controller.minimize?.('auto');
    }
}

export function installOverlayGuard(updateLayout) {
    if (window.__ivncOverlayModalInputGuardInstalled) return;
    window.__ivncOverlayModalInputGuardInstalled = true;
    window.addEventListener('resize', updateLayout);
    const sync = source => controllers().forEach(controller => controller.syncChild?.(source));
    window.__ivncSyncConsoleDialogs = sync;
    window.addEventListener('message', event => {
        if (event.origin !== location.origin) return;
        if (event.data?.type === 'ivnc-console-dialog-state') sync(event.source);
        if (event.data?.type === 'ivnc-overlay-app-interaction'
            && [...document.querySelectorAll('.app-streams iframe')].some(frame => frame.contentWindow === event.source)) {
            const owner = blocking();
            if (owner) owner.childState()?.focus();
            else minimizeOtherOverlays();
        }
    });
    const blocking = () => controllers().find(controller => controller.isOpen?.() && controller.childState?.()?.id);
    const outsideChild = (owner, target) => owner && !criticalTarget(target)
        && !target?.closest?.('#web-console-minimize') && !target?.closest?.('.web-console-frame');
    let gesture = null;
    document.addEventListener('pointerdown', event => {
        if (window.__IVNC_EMBED__ && !event.target?.closest?.('.clipboard-assist')) {
            parent.postMessage({ type: 'ivnc-overlay-app-interaction' }, location.origin);
        }
        const owner = blocking();
        if (outsideChild(owner, event.target)) {
            const state = owner.childState();
            gesture = { owner, id: state.id, x: event.clientX, y: event.clientY, valid: event.button === 0 };
            try { event.target.setPointerCapture(event.pointerId); } catch (_) {}
            consume(event); return;
        }
        gesture = null;
        if (event.target?.closest?.('.web-terminal-modal, .clipboard-assist, .taskbar-pin, .force-update-modal, .modal, .pwd-overlay, .update-overlay')) return;
        for (const controller of controllers()) {
            if (controller.isOpen?.() && !controller.isPinned?.() && !controller.contains?.(event.target)) controller.minimize?.('auto');
        }
    }, true);
    document.addEventListener('pointerup', event => {
        if (!gesture) return;
        gesture.valid &&= Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) < 5;
        consume(event);
    }, true);
    document.addEventListener('click', event => {
        const owner = blocking();
        if (!outsideChild(owner, event.target) && !gesture) return;
        consume(event);
        if (gesture) {
            if (gesture.valid) gesture.owner.childState()?.dismiss(gesture.id);
        } else {
            const state = owner.childState(); state?.dismiss(state.id);
        }
        gesture = null;
    }, true);
    document.addEventListener('pointercancel', () => { gesture = null; }, true);
    document.addEventListener('contextmenu', event => { if (outsideChild(blocking(), event.target)) consume(event); }, true);
    document.addEventListener('wheel', event => { if (outsideChild(blocking(), event.target)) consume(event); }, { capture: true, passive: false });
    document.addEventListener('keydown', event => {
        const owner = blocking();
        if (!owner || criticalTarget(event.target) || event.isComposing) return;
        if (event.key === 'Escape') { const state = owner.childState(); consume(event); state?.dismiss(state.id); }
        else if (event.key === 'Tab') { consume(event); owner.childState()?.focus(); }
    }, true);
    document.addEventListener('focusin', event => {
        const owner = blocking();
        if (owner && !owner.contains(event.target) && !criticalTarget(event.target)) owner.childState()?.focus();
    }, true);
    window.addEventListener('blur', () => {
        gesture = null;
        setTimeout(() => {
            if (document.activeElement?.closest?.('.app-streams')) {
                const owner = blocking();
                if (owner) owner.childState()?.focus();
                else minimizeOtherOverlays();
            }
        }, 0);
    });
}

export function createConsoleShield(modal, frame) {
    const shield = document.createElement('div');
    shield.className = 'console-dialog-shield allow-native-input'; shield.hidden = true;
    document.body.appendChild(shield);
    let wasActive = false;
    function childState() {
        try { return frame.contentWindow?.__ivncConsoleDialogState; } catch (_) { return null; }
    }
    function syncChild(source = frame.contentWindow) {
        if (source !== frame.contentWindow) return;
        const active = !!childState()?.id && !modal.classList.contains('minimized');
        shield.hidden = !active;
        modal.classList.toggle('has-child-dialog', active);
        document.documentElement.toggleAttribute('data-ivnc-modal-blocked', active);
        if (active && !wasActive) releaseOverlayInput();
        wasActive = active;
    }
    const loaded = () => {
        syncChild();
        frame.contentWindow?.postMessage({ type: 'ivnc-console-dialog-query' }, location.origin);
    };
    frame.addEventListener('load', loaded);
    return {
        childState, syncChild,
        destroy() { frame.removeEventListener('load', loaded); shield.remove(); modal.classList.remove('has-child-dialog'); document.documentElement.removeAttribute('data-ivnc-modal-blocked'); },
    };
}
