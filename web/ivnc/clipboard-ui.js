import { getLang, onLangChange } from './lib/i18n.js?v=1';
import { releaseOverlayInput } from './overlay-manager.js';

const messages = {
    zh: {
        title: '复制到本机', copy: '复制到本机', manual: '手动复制', close: '关闭',
        writing: '正在复制…', copied: '已复制到本机剪贴板。',
        help: '请按 Ctrl+C / ⌘C，或使用右键菜单复制所选文本。',
        empty: '远端文本为空，可使用复制快捷键清空本机剪贴板。',
        text: '远端复制的文本',
        copy_insecure: '当前连接不支持自动复制到本机。可手动复制，或使用 HTTPS 访问。',
        copy_gesture: '自动复制需要一次点击。请点击“复制到本机”，或手动复制。',
        copy_format: '当前连接或浏览器不支持复制此类内容，请使用支持剪贴板功能的浏览器并通过 HTTPS 访问。',
        copy_unsupported: '当前浏览器不支持自动复制到本机，请手动复制。',
        copy_permission: '网站的剪贴板写入权限已被禁用。可检查网站权限，或手动复制。',
        copy_policy: '页面的嵌入策略限制了剪贴板写入，请手动复制。',
        copy_blocked: '浏览器阻止了自动复制。可点击“复制到本机”重试，或手动复制。',
        copy_paused: '自动复制已暂停。可点击“复制到本机”，或手动复制。',
        copy_failed: '未能完成自动复制。可点击“复制到本机”重试，或手动复制。',
    },
    en: {
        title: 'Copy to this device', copy: 'Copy to device', manual: 'Copy manually', close: 'Close',
        writing: 'Copying…', copied: 'Copied to this device’s clipboard.',
        help: 'Press Ctrl+C / ⌘C, or use the context menu to copy the selected text.',
        empty: 'The remote text is empty. Use the copy shortcut to clear the local clipboard.',
        text: 'Text copied from the remote app',
        copy_insecure: 'This connection cannot copy automatically. Copy manually, or access iVnc over HTTPS.',
        copy_gesture: 'Click “Copy to device” to copy this content, or copy manually.',
        copy_format: 'This connection or browser cannot copy this content type. Use a browser with clipboard support over HTTPS.',
        copy_unsupported: 'This browser does not support automatic clipboard writes. Copy manually.',
        copy_permission: 'Clipboard writes are disabled for this site. Check site permissions, or copy manually.',
        copy_policy: 'The embedding page’s policy restricts clipboard writes. Copy manually.',
        copy_blocked: 'The browser blocked automatic copying. Click “Copy to device” to retry, or copy manually.',
        copy_paused: 'Automatic copying was paused. Click “Copy to device”, or copy manually.',
        copy_failed: 'Automatic copying did not complete. Click “Copy to device” to retry, or copy manually.',
    },
};

const styles = `
.clipboard-assist { position:fixed; top:16px; right:16px; z-index:1850; box-sizing:border-box;
    width:380px; max-width:calc(100vw - 32px); max-height:calc(100dvh - 32px); overflow:auto;
    padding:16px; border:1px solid rgba(255,255,255,.2); border-radius:8px;
    color:#f3f4f6; background:rgba(30,30,30,.98); box-shadow:0 4px 16px rgba(0,0,0,.4);
    font:13px/1.5 system-ui,sans-serif; text-align:left; user-select:text; }
.clipboard-assist[hidden], .clipboard-assist [hidden] { display:none !important; }
.clipboard-assist-header { display:flex; align-items:center; justify-content:space-between; gap:12px; }
.clipboard-assist-header strong { font-size:14px; }
.clipboard-assist button { font:inherit; cursor:pointer; color:inherit; background:#374151;
    border:1px solid #596170; border-radius:5px; padding:6px 10px; }
.clipboard-assist button:hover { background:#4b5563; }
.clipboard-assist button:focus-visible, .clipboard-assist textarea:focus-visible { outline:2px solid #8ab4ff; outline-offset:2px; }
.clipboard-assist button:disabled { opacity:.55; cursor:wait; }
.clipboard-assist .clipboard-close { padding:0 5px; font-size:20px; line-height:24px; background:transparent; border:0; }
.clipboard-assist-message, .clipboard-assist-help { margin:10px 0; }
.clipboard-assist-actions { display:flex; flex-wrap:wrap; gap:8px; }
.clipboard-assist textarea { display:block; box-sizing:border-box; width:100%; height:160px;
    max-height:40dvh; margin:10px 0; padding:10px; resize:vertical; border:1px solid #596170;
    border-radius:5px; background:#171a20; color:#f3f4f6; font:13px/1.5 monospace; user-select:text; }
`;

// Each view owns its copy UI and plaintext; no clipboard payload is posted to
// a parent frame or persisted. Only an explicit interaction opens the text.
export function createClipboardUI({ getController, restoreFocus }) {
    const style = document.createElement('style'); style.textContent = styles;
    const root = document.createElement('section');
    root.className = 'clipboard-assist allow-native-input'; root.hidden = true;
    root.setAttribute('aria-labelledby', 'clipboard-assist-title');
    root.innerHTML = `<div class="clipboard-assist-header"><strong id="clipboard-assist-title"></strong>
        <button type="button" class="clipboard-close">×</button></div>
        <p class="clipboard-assist-message" role="status" aria-live="polite"></p>
        <div class="clipboard-assist-actions"><button type="button" class="clipboard-copy"></button>
        <button type="button" class="clipboard-manual"></button></div>
        <div class="clipboard-assist-panel" hidden><p class="clipboard-assist-help"></p>
        <textarea readonly spellcheck="false" autocomplete="off" autocapitalize="off"></textarea></div>`;
    document.head.appendChild(style); document.body.appendChild(root);
    const title = root.querySelector('strong'), closeButton = root.querySelector('.clipboard-close');
    const message = root.querySelector('.clipboard-assist-message'), copyButton = root.querySelector('.clipboard-copy');
    const manualButton = root.querySelector('.clipboard-manual'), panel = root.querySelector('.clipboard-assist-panel');
    const help = root.querySelector('.clipboard-assist-help'), textarea = root.querySelector('textarea');
    let state = null, manual = false, timer = null, outsideGesture = false, retrying = false;
    const text = key => (messages[getLang()] || messages.en)[key] || messages.en.copy_blocked;
    const consume = event => { event.preventDefault(); event.stopImmediatePropagation(); };

    function syncFullscreen() {
        // Embedded views fullscreen their video container; body siblings would
        // otherwise be outside the browser's fullscreen layer.
        const fullscreen = document.fullscreenElement;
        const host = fullscreen?.matches('.video-container') ? fullscreen : document.body;
        if (root.parentElement !== host) {
            const focused = root.contains(document.activeElement) ? document.activeElement : null;
            host.appendChild(root);
            focused?.focus({ preventScroll: true });
        }
        if (fullscreen && !root.hidden) releaseOverlayInput();
    }
    syncFullscreen();
    document.addEventListener('fullscreenchange', syncFullscreen);

    function render() {
        root.hidden = !state || state.status === 'pending';
        title.textContent = text('title'); closeButton.setAttribute('aria-label', text('close'));
        let reason = state?.status === 'copied' ? 'copied' : state?.reason;
        if (state?.canManual === false && ['copy_insecure', 'copy_unsupported'].includes(reason)) reason = 'copy_format';
        const notice = state ? text(reason) : '';
        if (message.textContent !== notice) message.textContent = notice;
        copyButton.textContent = text(state?.busy ? 'writing' : 'copy');
        copyButton.disabled = !!state?.busy;
        copyButton.hidden = !state?.canRetry || state?.status === 'copied' || manual;
        manualButton.textContent = text('manual');
        manualButton.disabled = !!state?.busy;
        manualButton.hidden = manual || state?.canManual === false || state?.status === 'copied';
        panel.hidden = !manual || state?.status === 'copied';
        help.textContent = text(textarea.value ? 'help' : 'empty');
        textarea.setAttribute('aria-label', text('text'));
        root.setAttribute('role', manual ? 'dialog' : 'region');
    }

    function close(restore = true) {
        const ownedFocus = root.contains(document.activeElement);
        getController()?.dismissCopy();
        if (restore && ownedFocus) restoreFocus();
    }
    copyButton.addEventListener('click', () => {
        // Disabling a focused button moves focus to body in Chromium. Remember
        // the explicit interaction so completion can restore remote input.
        retrying = true;
        if (!getController()?.retryCopy()) retrying = false;
    });
    manualButton.addEventListener('click', () => {
        const snapshot = getController()?.copySnapshot();
        if (!snapshot || snapshot.id !== state?.id) return;
        manual = true; textarea.value = snapshot.text; render();
        textarea.focus({ preventScroll: true }); textarea.select();
    });
    closeButton.addEventListener('click', () => close());
    root.addEventListener('pointerdown', () => releaseOverlayInput());
    root.addEventListener('keydown', event => {
        if (event.key === 'Escape' && !event.isComposing) { consume(event); close(); }
    });
    // Empty selections have no default copy payload. This is a trusted native
    // copy gesture, which also works without the asynchronous Clipboard API.
    textarea.addEventListener('copy', event => {
        if (!textarea.value && event.clipboardData) {
            event.clipboardData.setData('text/plain', ''); event.preventDefault();
        }
    });
    const pointerDown = event => {
        outsideGesture = false;
        if (!manual || root.hidden || root.contains(event.target) || event.button !== 0) return;
        outsideGesture = true; consume(event); close();
    };
    const pointerUp = event => { if (outsideGesture) consume(event); };
    const click = event => { if (outsideGesture) { outsideGesture = false; consume(event); } };
    const cancelPointer = () => { outsideGesture = false; };
    // Outside dismissal consumes the whole gesture before other overlays or
    // the remote application react to it.
    window.addEventListener('pointerdown', pointerDown, true);
    window.addEventListener('pointerup', pointerUp, true);
    window.addEventListener('click', click, true);
    window.addEventListener('pointercancel', cancelPointer, true);
    const unsubscribe = onLangChange(render);
    return {
        update(next) {
            clearTimeout(timer);
            const previousStatus = state?.status;
            if (!next || next.id !== state?.id) retrying = false;
            if (!next || next.id !== state?.id || next.status === 'copied') { manual = false; textarea.value = ''; }
            const wasFocused = root.contains(document.activeElement);
            const wasHidden = root.hidden;
            state = next; render();
            // A locked pointer cannot reach the new controls. Unlock without
            // moving keyboard focus away from the remote application.
            if (wasHidden && !root.hidden) syncFullscreen();
            if (state?.status === 'copied') {
                if (wasFocused || retrying) restoreFocus();
                retrying = false;
                timer = setTimeout(() => close(false), 2000);
            } else if (retrying && previousStatus === 'writing' && state?.status === 'ready') {
                retrying = false;
                if (document.activeElement === document.body) copyButton.focus({ preventScroll: true });
            }
        },
        destroy() {
            clearTimeout(timer); unsubscribe(); textarea.value = ''; root.remove(); style.remove();
            document.removeEventListener('fullscreenchange', syncFullscreen);
            window.removeEventListener('pointerdown', pointerDown, true);
            window.removeEventListener('pointerup', pointerUp, true);
            window.removeEventListener('click', click, true);
            window.removeEventListener('pointercancel', cancelPointer, true);
        },
    };
}
