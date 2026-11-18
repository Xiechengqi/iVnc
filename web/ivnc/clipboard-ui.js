import { getLang, onLangChange } from './lib/i18n.js?v=1';
import { releaseOverlayInput } from './overlay-manager.js';

const messages = {
    zh: {
        title: '复制到本机', copy: '复制到本机', manual: '手动复制', close: '关闭',
        copyImage: '复制图片', copyText: '复制文本', download: '下载图片', image: '远端复制的图片',
        imageHelp: '右键点击图片并选择浏览器的“复制图片”；触屏设备可长按图片。也可下载后在本机打开。',
        copy_receiving: '正在接收图片，请完成复制后再切换到其他程序粘贴。',
        copy_text_only: '仅复制了文本，图片尚未复制。请使用下方图片预览手动复制。',
        copy_disabled: '图片同步已关闭，请在来源 iVnc 的设置中开启“同步图片和富文本”后重新复制。',
        copy_format: '此图片格式暂不支持，请先转换为 PNG、JPEG 或 WebP 后重新复制。',
        copy_timeout: '接收图片超时，请重新复制。',
        copy_limit: '剪贴板内容超过 16 MiB，请使用较小的图片。',
        copy_image_limit: '图片尺寸过大，请缩小图片后重新复制。',
        copy_invalid_image: '图片数据损坏或无法解码，请重新复制。',
        copy_invalid_content: '剪贴板数据无效，请重新复制。',
        copy_read_failed: '未能从应用读取剪贴板，请重新复制。',
        copy_busy: '图片处理繁忙，请稍后重新复制。',
        writing: '正在复制…', copied: '已复制到本机剪贴板。',
        help: '请按 Ctrl+C / ⌘C，或使用右键菜单复制所选文本。',
        empty: '远端文本为空，可使用复制快捷键清空本机剪贴板。',
        text: '远端复制的文本',
        copy_insecure: '当前连接不支持自动复制到本机。可手动复制，或使用 HTTPS 访问。',
        copy_gesture: '自动复制需要一次点击。请点击“复制到本机”，或手动复制。',
        copy_rich_manual: '当前连接或浏览器不支持复制此类内容，请通过 HTTPS 访问。',
        copy_image_manual: '当前连接或浏览器不能直接写入图片剪贴板。请在下方预览上使用浏览器的“复制图片”，或通过 HTTPS 访问。',
        copy_unsupported: '当前浏览器不支持自动复制到本机，请手动复制。',
        copy_permission: '网站的剪贴板写入权限已被禁用。可检查网站权限，或手动复制。',
        copy_policy: '页面的嵌入策略限制了剪贴板写入，请手动复制。',
        copy_blocked: '浏览器阻止了自动复制。可点击“复制到本机”重试，或手动复制。',
        copy_paused: '自动复制已暂停。可点击“复制到本机”，或手动复制。',
        copy_failed: '未能完成自动复制。可点击“复制到本机”重试，或手动复制。',
    },
    en: {
        title: 'Copy to this device', copy: 'Copy to device', manual: 'Copy manually', close: 'Close',
        copyImage: 'Copy image', copyText: 'Copy text', download: 'Download image', image: 'Image copied from the remote app',
        imageHelp: 'Right-click the image and choose the browser’s “Copy image”, or touch and hold on a touchscreen. You can also download it.',
        copy_receiving: 'Receiving image. Finish copying before switching to another app to paste.',
        copy_text_only: 'Only text was copied. Copy the image manually from the preview below.',
        copy_disabled: 'Image sync is off. Enable “Sync images and rich text” in the source iVnc settings, then copy again.',
        copy_format: 'This image format is not supported. Convert it to PNG, JPEG or WebP and copy again.',
        copy_timeout: 'Image transfer timed out. Copy again.',
        copy_limit: 'Clipboard content exceeds 16 MiB. Use a smaller image.',
        copy_image_limit: 'Image dimensions are too large. Use a smaller image.',
        copy_invalid_image: 'The image is damaged or could not be decoded. Copy again.',
        copy_invalid_content: 'Invalid clipboard data. Copy again.',
        copy_read_failed: 'Could not read the application clipboard. Copy again.',
        copy_busy: 'Image processing is busy. Copy again shortly.',
        writing: 'Copying…', copied: 'Copied to this device’s clipboard.',
        help: 'Press Ctrl+C / ⌘C, or use the context menu to copy the selected text.',
        empty: 'The remote text is empty. Use the copy shortcut to clear the local clipboard.',
        text: 'Text copied from the remote app',
        copy_insecure: 'This connection cannot copy automatically. Copy manually, or access iVnc over HTTPS.',
        copy_gesture: 'Click “Copy to device” to copy this content, or copy manually.',
        copy_rich_manual: 'This connection or browser cannot copy this content type. Access iVnc over HTTPS.',
        copy_image_manual: 'This connection or browser cannot write images directly. Use the browser’s “Copy image” on the preview below, or access iVnc over HTTPS.',
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
.clipboard-assist button, .clipboard-assist .clipboard-download { font:inherit; cursor:pointer; color:inherit; background:#374151;
    border:1px solid #596170; border-radius:5px; padding:6px 10px; }
.clipboard-assist .clipboard-download { display:inline-block; text-decoration:none; }
.clipboard-assist button:hover, .clipboard-assist .clipboard-download:hover { background:#4b5563; }
.clipboard-assist .clipboard-download:focus-visible, .clipboard-assist img:focus-visible, .clipboard-assist button:focus-visible, .clipboard-assist textarea:focus-visible { outline:2px solid #8ab4ff; outline-offset:2px; }
.clipboard-assist button:disabled { opacity:.55; cursor:wait; }
.clipboard-assist .clipboard-close { padding:0 5px; font-size:20px; line-height:24px; background:transparent; border:0; }
.clipboard-assist-message, .clipboard-assist-help { margin:10px 0; }
.clipboard-assist-actions { display:flex; flex-wrap:wrap; gap:8px; }
.clipboard-assist textarea { display:block; box-sizing:border-box; width:100%; height:160px;
    max-height:40dvh; margin:10px 0; padding:10px; resize:vertical; border:1px solid #596170;
    border-radius:5px; background:#171a20; color:#f3f4f6; font:13px/1.5 monospace; user-select:text; }
.clipboard-assist img { display:block; width:100%; height:240px; max-height:40dvh; object-fit:contain;
    margin:12px auto; background:repeating-conic-gradient(#ddd 0% 25%,white 0% 50%) 0 / 16px 16px;
    -webkit-touch-callout:default; user-select:auto; }
`;

// Each view owns its copy UI and snapshot; no clipboard payload is posted to
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
        <button type="button" class="clipboard-manual"></button>
        <a class="clipboard-download" download="ivnc-clipboard.png" hidden></a></div>
        <div class="clipboard-image-panel" hidden><img class="clipboard-image" tabindex="0" draggable="false">
        <p class="clipboard-image-help"></p></div>
        <div class="clipboard-assist-panel" hidden><p class="clipboard-assist-help"></p>
        <textarea readonly spellcheck="false" autocomplete="off" autocapitalize="off"></textarea></div>`;
    document.head.appendChild(style); document.body.appendChild(root);
    const title = root.querySelector('strong'), closeButton = root.querySelector('.clipboard-close');
    const message = root.querySelector('.clipboard-assist-message'), copyButton = root.querySelector('.clipboard-copy');
    const manualButton = root.querySelector('.clipboard-manual'), panel = root.querySelector('.clipboard-assist-panel');
    const help = root.querySelector('.clipboard-assist-help'), textarea = root.querySelector('textarea');
    const imagePanel = root.querySelector('.clipboard-image-panel'), image = root.querySelector('.clipboard-image');
    const imageHelp = root.querySelector('.clipboard-image-help'), download = root.querySelector('.clipboard-download');
    let imageUrl = null, imageId = null;
    let state = null, manual = false, timer = null, outsideGesture = false, retrying = false;
    // Focus changes publish state during mousedown. Replacing an unchanged
    // control's text node then cancels the ensuing native click in WebKit.
    const setText = (element, value) => { if (element.textContent !== value) element.textContent = value; };
    function clearImage() {
        if (imageUrl) URL.revokeObjectURL(imageUrl);
        imageUrl = imageId = null;
        image.removeAttribute('src'); download.removeAttribute('href');
    }
    function syncImage() {
        if (!state?.hasImage || state.status === 'pending' || state.status === 'copied') clearImage();
        else if (imageId !== state.id) {
            clearImage();
            const snapshot = getController()?.imageSnapshot();
            if (snapshot?.id === state.id) {
                imageUrl = URL.createObjectURL(new Blob([snapshot.data], {type:'image/png'}));
                imageId = state.id; image.src = imageUrl; download.href = imageUrl;
            }
        }
        imagePanel.hidden = !imageUrl; download.hidden = !imageUrl;
        image.alt = text('image'); setText(imageHelp, text('imageHelp')); setText(download, text('download'));
    }
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
        setText(title, text('title')); closeButton.setAttribute('aria-label', text('close'));
        let reason = state?.status === 'copied' ? 'copied' : state?.reason;
        if (state?.canManual === false && ['copy_insecure', 'copy_unsupported'].includes(reason)) reason = state?.hasImage ? 'copy_image_manual' : 'copy_rich_manual';
        const notice = state ? text(reason) : '';
        setText(message, notice);
        setText(copyButton, text(state?.busy ? 'writing' : state?.textOnly ? 'copyText' : state?.hasImage ? 'copyImage' : 'copy'));
        copyButton.disabled = !!state?.busy;
        copyButton.hidden = !state?.canRetry || state?.status === 'copied' || manual;
        setText(manualButton, text('manual'));
        manualButton.disabled = !!state?.busy;
        manualButton.hidden = manual || state?.canManual === false || state?.status === 'copied';
        panel.hidden = !manual || state?.status === 'copied';
        setText(help, text(textarea.value ? 'help' : 'empty'));
        textarea.setAttribute('aria-label', text('text'));
        syncImage();
        root.setAttribute('role', manual || imageUrl ? 'dialog' : 'region');
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
    // Keep native browser image menus; no remote context-menu action.
    root.addEventListener('contextmenu', event => event.stopPropagation());
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
        if ((!manual && !imageUrl) || root.hidden || root.contains(event.target) || event.button !== 0) return;
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
            clearTimeout(timer); unsubscribe(); clearImage(); textarea.value = ''; root.remove(); style.remove();
            document.removeEventListener('fullscreenchange', syncFullscreen);
            window.removeEventListener('pointerdown', pointerDown, true);
            window.removeEventListener('pointerup', pointerUp, true);
            window.removeEventListener('click', click, true);
            window.removeEventListener('pointercancel', cancelPointer, true);
        },
    };
}
