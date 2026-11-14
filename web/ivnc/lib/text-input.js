// One editable proxy for desktop IMEs and mobile software keyboards. Keep
// browser editing native; only committed text and editing commands go remote.
const SENTINEL = '\u200b';
const EDIT_KEYS = { Backspace: 0xff08, Delete: 0xffff, Enter: 0xff0d };
const INPUT_KEYS = { deleteContentBackward: 'Backspace', deleteContentForward: 'Delete', insertLineBreak: 'Enter', insertParagraph: 'Enter' };

export function usesTouchKeyboard() {
    return /Android|iPhone|iPad|iPod/.test(navigator.userAgent)
        || (navigator.maxTouchPoints > 0 && (matchMedia('(pointer: coarse)').matches
        || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)));
}

export class TextInput {
    constructor({ sendText, sendKey, canInput, onChange = () => {}, onComposition = () => {} }) {
        Object.assign(this, { sendText, sendKey, canInput, onChange, onComposition });
        this.enabled = false;
        this.composing = false;
        this.generation = 0;
        this.remote = null;
        this.lastTap = null;
        this.dismissed = false;
        this.listeners = new AbortController();
        const el = this.element = document.createElement('textarea');
        el.className = 'ivnc-text-input';
        el.tabIndex = -1;
        el.setAttribute('aria-label', 'Remote application keyboard input');
        el.setAttribute('autocomplete', 'off');
        el.setAttribute('autocorrect', 'off');
        el.setAttribute('autocapitalize', 'off');
        el.spellcheck = false;
        el.inputMode = 'text';
        el.style.cssText = 'position:fixed;left:12px;top:12px;width:1px;height:18px;color:transparent;background:transparent;caret-color:transparent;outline:none;border:0;padding:0;margin:0;pointer-events:none;z-index:99999;font-size:16px;line-height:18px;overflow:hidden;resize:none;';
        document.body.appendChild(el);
        this.resetValue();
        const listen = (type, fn) => el.addEventListener(type, fn.bind(this), { signal: this.listeners.signal });
        listen('beforeinput', this.beforeInput);
        listen('input', this.input);
        listen('compositionstart', this.compositionStart);
        listen('compositionupdate', e => { if (this.composing && this.available()) this.onComposition(true, e.data || ''); });
        listen('compositionend', this.compositionEnd);
        listen('focus', () => { this.dismissed = false; this.onChange(); });
        listen('blur', () => { this.cancel(); this.onChange(); });
        window.visualViewport?.addEventListener('resize', () => this.viewportChanged(), { signal: this.listeners.signal });
        window.visualViewport?.addEventListener('scroll', () => this.position(), { signal: this.listeners.signal });
        document.addEventListener('visibilitychange', () => { if (document.hidden) this.hide(false); }, { signal: this.listeners.signal });
        window.addEventListener('blur', () => { this.cancel(); this.onChange(); }, { signal: this.listeners.signal });
    }

    owns(target) { return target === this.element; }
    focused() { return document.activeElement === this.element && document.hasFocus(); }
    available() { return this.enabled && this.canInput() && this.focused(); }
    resetValue() {
        this.element.value = SENTINEL;
        this.element.setSelectionRange(1, 1);
    }
    cancel() {
        this.generation++;
        this.cancelledComposition ||= this.composing;
        this.composing = false;
        this.onComposition(false, '');
        this.clearTail();
        this.resetValue();
    }
    clearTail() {
        clearTimeout(this.tailTimer);
        this.tail = null;
    }
    // WebKit may emit a final input after compositionend. It belongs to the
    // same edit, not another insertion. A later user gesture starts a new edit.
    rememberTail(type, text = null) {
        this.clearTail();
        this.tail = { type, text };
        this.tailTimer = setTimeout(() => { this.tail = null; }, 0);
    }
    isTail(event) {
        return this.tail && (event.inputType === this.tail.type
            || (this.tail.text !== null && event.data === this.tail.text
                && ['insertText', 'insertFromComposition', 'insertCompositionText'].includes(event.inputType)));
    }
    position(point) {
        const v = window.visualViewport;
        const left = v?.offsetLeft || 0, top = v?.offsetTop || 0;
        this.element.style.left = `${Math.max(left + 4, Math.min(point?.clientX ?? left + 12, left + (v?.width || innerWidth) - 8))}px`;
        this.element.style.top = `${Math.max(top + 4, Math.min(point?.clientY ?? top + 12, top + (v?.height || innerHeight) - 24))}px`;
    }
    show(point) {
        if (!this.canInput()) return false;
        this.enabled = true;
        this.dismissed = false;
        if (!this.focused()) {
            this.focusHeight = window.visualViewport?.height || innerHeight;
            this.focusWidth = innerWidth;
            this.occluded = false;
        }
        this.position(point);
        // Called synchronously by the tap/click handler, including same-origin
        // embedded views. Never defer this focus through postMessage or a timer.
        this.element.focus({ preventScroll: true });
        this.onChange();
        return this.focused();
    }
    hide(manual = true) {
        this.dismissed = manual;
        this.enabled = false;
        this.lastTap = null;
        this.cancel(); // Invalidate late composition events before blur.
        this.element.blur();
        this.onChange();
    }
    viewportChanged() {
        this.position();
        if (!usesTouchKeyboard() || !this.focused()) return;
        if (window.visualViewport && Math.abs(window.visualViewport.scale - 1) > 0.01) return;
        if (Math.abs(innerWidth - this.focusWidth) > 80) {
            this.focusWidth = innerWidth;
            this.focusHeight = window.visualViewport?.height || innerHeight;
            this.occluded = false;
            return;
        }
        const difference = this.focusHeight - (window.visualViewport?.height || innerHeight);
        if (difference > 80) this.occluded = true;
        else if (this.occluded && difference < 20) this.hide();
    }
    touch(point) {
        this.clearTail();
        this.lastTap = { clientX: point.clientX, clientY: point.clientY, time: performance.now() };
        if (this.enabled || (usesTouchKeyboard() && this.remote?.enabled && !this.dismissed)) this.show(point);
    }
    remoteState(state) {
        const previous = this.remote;
        this.remote = state?.protocol === 1 ? state : null;
        const changedFocus = previous?.focus && previous.focus !== this.remote?.focus;
        const closedEditor = previous?.enabled && !this.remote?.enabled;
        if (usesTouchKeyboard() && (changedFocus || closedEditor)) this.hide(false);
        else if (changedFocus || closedEditor) this.cancel();
        const purpose = this.remote?.purpose;
        this.element.inputMode = ({ 2: 'decimal', 3: 'numeric', 4: 'tel', 5: 'url', 6: 'email', 9: 'numeric' })[purpose] || 'text';
        // No surrounding text or password is copied into this proxy.
        if (!this.remote?.enabled) return;
        if (usesTouchKeyboard() && !this.dismissed && this.lastTap
            && performance.now() - this.lastTap.time < 500 && navigator.userActivation?.isActive) {
            this.show(this.lastTap);
        }
    }
    keydown(event) {
        if (!this.owns(event.target)) return false;
        if (!this.available()) return true;
        if (event.isComposing || this.composing || event.keyCode === 229 || event.key === 'Process') return true;
        this.clearTail();
        if (event.ctrlKey || event.metaKey || (event.altKey && !event.getModifierState?.('AltGraph'))) return false;
        const key = EDIT_KEYS[event.key];
        if (key) {
            event.preventDefault();
            event.stopPropagation();
            this.sendKey(key);
            this.resetValue();
            const type = Object.keys(INPUT_KEYS).find(type => INPUT_KEYS[type] === event.key);
            this.rememberTail(type);
            return true;
        }
        // Printable keys need their browser default action, which generates
        // beforeinput/input. This also covers a physical keyboard on a tablet.
        return [...(event.key || '')].length === 1 || event.key === 'Unidentified';
    }
    beforeInput(event) {
        if (!this.available()) { if (event.cancelable) event.preventDefault(); return; }
        if (this.composing || event.isComposing) return;
        if (this.isTail(event)) { if (event.cancelable) event.preventDefault(); return; }
        this.clearTail();
        const key = INPUT_KEYS[event.inputType];
        if (key && event.cancelable) {
            event.preventDefault();
            this.sendKey(EDIT_KEYS[key]);
            this.resetValue();
            this.rememberTail(event.inputType);
        }
    }
    input(event) {
        if (!this.available()) { this.resetValue(); return; }
        if (this.composing || event.isComposing) return;
        if (this.isTail(event)) { this.clearTail(); this.resetValue(); return; }
        const key = INPUT_KEYS[event.inputType];
        if (key) this.sendKey(EDIT_KEYS[key]);
        else {
            const text = event.data ?? this.element.value.replace(/^\u200b/, '');
            if (text) this.sendText(text);
        }
        this.resetValue();
    }
    compositionStart() {
        if (!this.available()) return;
        this.cancelledComposition = false;
        this.clearTail();
        this.composing = true;
        this.compositionGeneration = this.generation;
        this.onComposition(true, '');
    }
    compositionEnd(event) {
        if (this.cancelledComposition) {
            this.cancelledComposition = false;
            this.resetValue();
            this.rememberTail('insertFromComposition', event.data);
            return;
        }
        if (!this.composing || !this.available() || this.compositionGeneration !== this.generation) return;
        this.composing = false;
        this.onComposition(false, '');
        const text = event.data ?? this.element.value.replace(/^\u200b/, '');
        if (text) this.sendText(text);
        this.resetValue();
        this.rememberTail('insertFromComposition', text);
    }
    destroy() { this.hide(false); this.listeners.abort(); this.element.remove(); }
}
