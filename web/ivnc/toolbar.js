// Owns toolbar DOM, display order and gestures. Stream views remain owned by
// app-streams.js; moving tabs never moves or recreates an application's iframe.
import toolbarStyles from './toolbar.css?inline';
import { getLang, onLangChange, serverErrorText } from './lib/i18n.js?v=1';
import { createOrderNotifications } from './app-order.js';

const STRINGS = {
    zh: { apps: '应用', previous: '向左滚动应用', next: '向右滚动应用', more: '更多工具',
        swipe: '滑动切换应用', swipeOn: '滑动切换应用：已开启，点击关闭', swipeOff: '滑动切换应用：已关闭，点击开启',
        swipeHint: '双指左右滑动切换应用，纵向滑动滚动内容',
        move: '按住标签左右拖动排序，或使用左右方向键移动', saved: '应用顺序已保存', failed: '无法保存应用顺序', network: '网络状态',
        rename: '修改应用名', save: '保存', cancel: '取消', pending: '保存中…', nameFailed: '无法保存应用名',
        latest: '当前名称：', changed: '应用名已发生变化。请核对当前名称，再次保存以使用你的输入。',
        invalid: '名称不能为空、不能包含换行或控制字符，且不能超过 256 个 UTF-8 字节。' },
    en: { apps: 'Applications', previous: 'Scroll apps left', next: 'Scroll apps right', more: 'More tools',
        swipe: 'Swipe to switch apps', swipeOn: 'Swipe to switch apps: on, click to turn off', swipeOff: 'Swipe to switch apps: off, click to turn on',
        swipeHint: 'Swipe horizontally with two fingers to switch apps; swipe vertically to scroll',
        move: 'Hold and drag the tab left or right to reorder, or use left/right arrow keys', saved: 'Application order saved', failed: 'Could not save application order', network: 'Network status',
        rename: 'Rename application', save: 'Save', cancel: 'Cancel', pending: 'Saving…', nameFailed: 'Could not save application name',
        latest: 'Current name: ', changed: 'The application name changed. Review the current name, then save again to use your draft.',
        invalid: 'Enter a name without line breaks or control characters, up to 256 UTF-8 bytes.' }
};

export function createToolbar({ element, canReorder = false, canRename = false, activateWindow, onInteraction = () => {} }) {
    // The desktop entry is loaded as a module at runtime by index.html; Vite
    // cannot attach its extracted styles to that dynamically created script.
    if (!document.getElementById('ivnc-toolbar-styles')) {
        const style = document.createElement('style');
        style.id = 'ivnc-toolbar-styles';
        style.textContent = toolbarStyles;
        document.head.appendChild(style);
    }
    const tr = (key) => STRINGS[getLang()][key];
    const events = new AbortController();
    const listen = (target, type, handler, options = {}) => target.addEventListener(type, handler, { ...options, signal: events.signal });
    const make = (tag, cls, parent) => {
        const node = document.createElement(tag);
        node.className = cls;
        parent?.appendChild(node);
        return node;
    };
    element.classList.add('ivnc-toolbar');
    const tools = make('div', 'toolbar-tools', element);
    const apps = make('div', 'toolbar-apps', element);
    const status = make('div', 'toolbar-status', element);
    const previous = make('button', 'toolbar-scroll', apps);
    previous.type = 'button'; previous.textContent = '‹';
    const viewport = make('div', 'toolbar-viewport', apps);
    viewport.tabIndex = -1;
    const list = make('div', 'toolbar-list', viewport);
    const next = make('button', 'toolbar-scroll', apps);
    next.type = 'button'; next.textContent = '›';
    const more = make('button', 'toolbar-more', tools);
    more.type = 'button'; more.textContent = '⋯'; more.hidden = true;
    more.setAttribute('aria-expanded', 'false');
    const menu = make('div', 'toolbar-more-menu', tools);
    menu.hidden = true;
    const info = make('div', 'toolbar-info', status);
    info.hidden = true;
    const notice = make('div', 'toolbar-notice', element);
    notice.hidden = true; notice.setAttribute('role', 'status');
    const renamePanel = make('form', 'toolbar-rename allow-native-input', element);
    renamePanel.hidden = true;
    renamePanel.setAttribute('role', 'dialog'); renamePanel.setAttribute('aria-modal', 'false');
    const renameLabel = make('label', 'toolbar-rename-label', renamePanel);
    const renameCaption = make('span', '', renameLabel);
    const renameInput = make('input', 'toolbar-rename-input', renameLabel);
    renameInput.type = 'text'; renameInput.autocomplete = 'off'; renameInput.spellcheck = false;
    const renameLatest = make('div', 'toolbar-rename-latest', renamePanel);
    const renameError = make('div', 'toolbar-rename-error', renamePanel);
    renameError.setAttribute('role', 'alert');
    const renameActions = make('div', 'toolbar-rename-actions', renamePanel);
    const renameCancel = make('button', 'toolbar-rename-cancel', renameActions); renameCancel.type = 'button';
    const renameSave = make('button', 'toolbar-rename-save', renameActions); renameSave.type = 'button';
    const names = new Map(), requests = new Set(), nameRefreshes = new Set();
    const notifications = createOrderNotifications(undefined, async (id, revision) => {
        if (destroyed || revision <= (names.get(id)?.name_revision ?? -1) || nameRefreshes.has(id) || !entries.some(e => e.appId === id)) return;
        nameRefreshes.add(id);
        try {
            const app = await request(`/api/apps/${encodeURIComponent(id)}`);
            if (!destroyed && entries.some(e => e.appId === id)) { acceptName(app); paint(); }
        } catch (_) {} finally { nameRefreshes.delete(id); }
    });
    const toolEntries = [], nodes = new Map();
    let entries = [], confirmed = { ordered_ids: [], order_revision: -1 }, pendingIds = null;
    let drag = null, saving = false, destroyed = false, ghost = null, frame = 0, noticeTimer;
    let lastActive = null, editor = null, renameBusy = false;
    let press = null, pressTimer, paintPending = false, suppressedPointer = null;
    let navigation = null, navigationTimer, syncGestureButton = () => {};

    function resetNavigation(hideNotice = true, repaint = true) {
        const changed = !!navigation;
        clearTimeout(navigationTimer);
        navigation = null;
        if (hideNotice) { clearTimeout(noticeTimer); notice.hidden = true; }
        if (changed && repaint && !destroyed) paint();
    }

    function activateEntry(entry) {
        if (entry.active && !navigation) return entry.activate() !== false;
        // Set intent before activation: stream selection can synchronously
        // re-enter update(), whereas desktop focus arrives over the network.
        clearTimeout(navigationTimer);
        const state = { key: entry.key, appId: entry.appId,
            base: entries.find(e => e.active)?.key, wasActive: !!entry.active, sawChange: false };
        navigation = state;
        navigationTimer = setTimeout(() => resetNavigation(), 2000);
        const previousEntries = entries;
        try {
            if (entry.activate() === false) { if (navigation === state) resetNavigation(); return false; }
        } catch (error) {
            if (navigation === state) resetNavigation();
            throw error;
        }
        if (entries === previousEntries) paint();
        reveal(nodes.get(entry.key));
        return true;
    }

    function finishPress(repaint = true) {
        clearTimeout(pressTimer);
        press = null;
        if (repaint && paintPending && !destroyed) paint();
    }

    function cancelInteraction() {
        if (press) suppressedPointer = press.pointerId;
        cancelDrag();
        finishPress();
    }

    function activateNode(node) {
        if (destroyed || (press?.cancelled && press.key === node._entry.key)) return;
        const entry = entries.find(e => e.key === node._entry.key && e.appId === node._entry.appId);
        if (entry) activateEntry(entry);
    }

    function acceptName(app) {
        if (!app || typeof app.id !== 'string' || typeof app.name !== 'string' || !Number.isSafeInteger(app.name_revision) || app.name_revision < 0) return;
        const previous = names.get(app.id);
        if (previous && app.name_revision <= previous.name_revision) return;
        names.set(app.id, { name: app.name, name_revision: app.name_revision });
        if (previous) notifications.publishName(app.id, app.name_revision);
    }

    function describe(entry) {
        const app = names.get(entry.appId);
        if (!app) return;
        const manyWindows = entry.windowTitle !== undefined && entries.filter(e => e.windowTitle !== undefined && e.appId === entry.appId).length > 1;
        entry.label = app.name;
        if (manyWindows && entry.windowTitle && entry.windowTitle !== app.name) entry.label += ` · ${entry.windowTitle}`;
        entry.title = [app.name, entry.windowTitle || entry.statusText].filter(Boolean).join(' · ');
        if (entry.windowClass) entry.title += ` (${entry.windowClass})`;
    }

    function positionEditor() {
        if (!editor) return;
        const anchor = nodes.get(editor.key)?.getBoundingClientRect();
        if (!anchor) return;
        const rect = element.getBoundingClientRect();
        const left = Math.max(8, Math.min(window.innerWidth - renamePanel.offsetWidth - 8, anchor.left));
        renamePanel.style.left = `${left - rect.left}px`;
    }

    function paintEditor() {
        if (!editor) return;
        const app = names.get(editor.id);
        renameLatest.textContent = app ? `${tr('latest')}${app.name}` : '';
        renameSave.disabled = renameBusy;
        // Disabling the focused input would move focus to the desktop and
        // leak the matching keyup (or a retry's Enter) into remote input.
        renameInput.readOnly = editor.saving;
        renamePanel.setAttribute('aria-busy', String(editor.saving));
        renameSave.textContent = tr(editor.saving ? 'pending' : 'save');
        positionEditor();
    }

    function closeEditor(restoreFocus = false) {
        if (!editor) return;
        const state = editor; editor = null;
        state.controller?.abort(); renamePanel.hidden = true;
        if (restoreFocus) nodes.get(state.key)?.focus({ preventScroll: true });
        onInteraction(false);
    }

    function openEditor(node) {
        const entry = node._entry, app = names.get(entry.appId);
        if (!canRename || !entry.canRename || !app || destroyed) return;
        closeEditor(); cancelDrag(); closeMenu(); closeInfo();
        editor = { id: entry.appId, key: entry.key, revision: app.name_revision, saving: false };
        renameInput.value = app.name; renameError.textContent = '';
        renamePanel.hidden = false; paintEditor(); onInteraction(true);
        renameInput.focus({ preventScroll: true }); renameInput.select();
    }

    async function saveName() {
        if (!editor || renameBusy || destroyed) return;
        const state = editor, raw = renameInput.value, name = raw.trim();
        if (!name || /[\p{Cc}\u2028\u2029]/u.test(raw) || new TextEncoder().encode(name).length > 256) {
            renameError.textContent = tr('invalid'); return;
        }
        renameBusy = state.saving = true; renameError.textContent = '';
        renameInput.focus({ preventScroll: true }); paintEditor();
        state.controller = new AbortController();
        const url = `/api/apps/${encodeURIComponent(state.id)}`;
        const success = app => {
            acceptName(app);
            closeEditor(true); paint();
        };
        try {
            const app = await request(`${url}/name`, { method: 'PUT', headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ name, base_revision: state.revision }) }, state.controller);
            if (!destroyed && editor === state) success(app);
        } catch (error) {
            if (destroyed || editor !== state) return;
            if (error.status === 404) { closeEditor(); return; }
            let current;
            if (error.body?.code === 'app_name_conflict') {
                current = { id: state.id, name: error.body.params.name, name_revision: error.body.params.name_revision };
            } else if (!error.status) {
                // A lost response can follow a committed transaction. Resolve
                // it from the API before allowing an explicit retry.
                state.controller = new AbortController();
                try { current = await request(url, {}, state.controller); }
                catch (readError) { if (readError.status === 404 && editor === state) closeEditor(); }
                if (destroyed || editor !== state) return;
                if (current?.name === name && current.name_revision >= state.revision) { success(current); return; }
            }
            if (current) { acceptName(current); state.revision = names.get(state.id).name_revision; paint(); }
            renameError.textContent = error.body?.code === 'app_name_conflict' ? tr('changed') : `${tr('nameFailed')}: ${error.message}`;
        } finally {
            renameBusy = false; state.saving = false;
            if (!destroyed) paintEditor();
        }
    }

    // Only explicit button clicks and non-composing Enter save. Native form
    // submission during IME confirmation must never submit the draft.
    listen(renamePanel, 'submit', event => event.preventDefault());
    listen(renameSave, 'click', saveName);
    listen(renameInput, 'keydown', event => {
        if (event.isComposing || event.keyCode === 229) return;
        if (event.key === 'Enter') { event.preventDefault(); event.stopPropagation(); saveName(); }
    });
    listen(renameCancel, 'click', () => closeEditor(true));
    listen(list, 'contextmenu', event => {
        const node = event.target.closest('.taskbar-item');
        if (!canRename || !node?._entry.canRename) return;
        event.preventDefault(); event.stopPropagation(); openEditor(node);
    });
    listen(list, 'keydown', event => {
        if (event.key !== 'F2' && !(event.shiftKey && event.key === 'F10')) return;
        const node = event.target.closest('.taskbar-item');
        if (!canRename || !node?._entry.canRename) return;
        event.preventDefault(); event.stopPropagation(); openEditor(node);
    });

    function notify(message, error = false) {
        if (destroyed) return;
        clearTimeout(noticeTimer);
        notice.textContent = message;
        notice.hidden = !error;
        noticeTimer = setTimeout(() => { notice.hidden = true; }, 5000);
    }

    function acceptOrder(data) {
        if (!Array.isArray(data.ordered_ids) || !Number.isSafeInteger(data.order_revision) || data.order_revision < confirmed.order_revision) return;
        const changed = data.order_revision !== confirmed.order_revision;
        confirmed = { ordered_ids: data.ordered_ids.slice(), order_revision: data.order_revision };
        if (changed) notifications.publish(confirmed.order_revision);
    }

    function orderedEntries(ids = pendingIds || confirmed.ordered_ids) {
        const ranks = new Map(ids.map((id, i) => [id, i]));
        return entries.slice().sort((a, b) => (ranks.get(a.appId) ?? Infinity) - (ranks.get(b.appId) ?? Infinity));
    }

    // Keep non-visible apps in exactly the same slots as the backend merge.
    function merge(ids) {
        const selected = new Set(ids);
        let index = 0;
        return confirmed.ordered_ids.map(id => selected.has(id) ? ids[index++] : id);
    }

    const visibleIds = () => [...new Set(orderedEntries().map(e => e.appId).filter(id => confirmed.ordered_ids.includes(id)))];

    function paint() {
        // WebKit cancels a native click if its hit-tested node is removed
        // between press and release. Keep both the row and its position until
        // that click is dispatched, including for non-sortable and touch tabs.
        if (press) { paintPending = true; return; }
        paintPending = false;
        list.setAttribute('aria-busy', String(saving));
        viewport.classList.toggle('sorting', !!drag?.started);
        const sorted = orderedEntries();
        const live = new Set(sorted.map(e => e.key));
        for (const [key, node] of nodes) {
            if (!live.has(key)) { node.remove(); nodes.delete(key); }
        }
        const left = viewport.scrollLeft;
        let cursor = list.firstElementChild;
        let firstDragged = true;
        for (const entry of sorted) {
            describe(entry);
            let node = nodes.get(entry.key);
            if (!node) {
                node = make('div', 'taskbar-item');
                node.tabIndex = 0; node.setAttribute('role', 'button');
                const dot = make('span', 'taskbar-app-dot', node);
                const label = make('span', 'toolbar-label', node);
                const text = document.createTextNode('');
                label.appendChild(text);
                node._parts = { dot, label, text };
                node.onclick = (event) => {
                    event.stopPropagation();
                    try {
                        // State may have changed while the DOM was held still.
                        activateNode(node);
                    } finally { finishPress(); }
                };
                node.onkeydown = (event) => {
                    if (event.target !== node) return;
                    if (['Enter', ' '].includes(event.key)) {
                        event.preventDefault(); event.stopPropagation(); activateNode(node);
                        return;
                    }
                    if (!canReorder || !confirmed.ordered_ids.includes(node._entry.appId) || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
                    event.preventDefault(); event.stopPropagation();
                    if (saving || press || drag) return;
                    const ids = visibleIds(), from = ids.indexOf(node._entry.appId);
                    const to = from + (event.key === 'ArrowLeft' ? -1 : 1);
                    if (from < 0 || to < 0 || to >= ids.length) return;
                    [ids[from], ids[to]] = [ids[to], ids[from]];
                    persist(ids, confirmed.order_revision);
                    // Moving the focused row can blur it in WebKit. Keep the
                    // keyboard on this application for consecutive moves.
                    node.focus({ preventScroll: true }); reveal(node);
                };
                nodes.set(entry.key, node);
            }
            node._entry = entry;
            node.dataset.key = entry.key;
            node.dataset.appId = entry.appId || '';
            node.classList.toggle('focused', !!entry.active);
            node.classList.toggle('switch-pending', navigation?.key === entry.key && !entry.active);
            node.setAttribute('aria-busy', String(navigation?.key === entry.key && !entry.active));
            node.classList.toggle('taskbar-app', !!entry.dot);
            const dragged = !!drag?.started && drag.id === entry.appId;
            node.classList.toggle('dragging', dragged);
            node.classList.toggle('drop-target', dragged && firstDragged);
            if (dragged) firstDragged = false;
            const sortable = canReorder && confirmed.ordered_ids.includes(entry.appId);
            node.classList.toggle('sortable', sortable && !saving);
            node.title = sortable ? `${entry.title}\n${tr('move')}` : entry.title;
            node.setAttribute('aria-label', entry.title);
            node.setAttribute('aria-pressed', String(!!entry.active));
            const shortcuts = ['Enter', 'Space'];
            if (sortable) shortcuts.push('ArrowLeft', 'ArrowRight');
            if (canRename && entry.canRename) shortcuts.push('F2', 'Shift+F10');
            node.setAttribute('aria-keyshortcuts', shortcuts.join(' '));
            if (sortable) node.setAttribute('aria-description', tr('move'));
            else node.removeAttribute('aria-description');
            if (node._parts.text.data !== entry.label) node._parts.text.data = entry.label;
            node._parts.dot.hidden = !entry.dot;
            node._parts.dot.className = `taskbar-app-dot ${entry.dot || ''}`;
            if (node !== cursor) list.insertBefore(node, cursor);
            cursor = node.nextElementSibling;
        }
        viewport.scrollLeft = left;
        updateOverflow();
        paintEditor();
        const active = sorted.find(e => e.active)?.key || null;
        if (active !== lastActive && !drag && !saving) {
            lastActive = active;
            reveal(nodes.get(active));
        }
    }

    function reveal(node) {
        if (!node) return;
        const a = viewport.getBoundingClientRect(), b = node.getBoundingClientRect();
        if (b.left < a.left) viewport.scrollLeft -= a.left - b.left;
        else if (b.right > a.right) viewport.scrollLeft += b.right - a.right;
    }

    function updateOverflow() {
        // Measure without arrows first, so arrow visibility has no feedback loop.
        const overflow = list.scrollWidth > apps.clientWidth + 1;
        previous.hidden = next.hidden = !overflow;
        previous.disabled = viewport.scrollLeft < 1;
        next.disabled = viewport.scrollLeft >= viewport.scrollWidth - viewport.clientWidth - 1;
    }

    function layout() {
        const compact = element.clientWidth <= 700;
        for (const tool of toolEntries) {
            const parent = compact && !tool.primary ? menu : tools;
            if (tool.node.parentNode !== parent) parent.insertBefore(tool.node, parent === tools ? more : null);
            tool.label.textContent = tool.node.title;
        }
        more.hidden = !menu.children.length;
        if (!compact) closeMenu();
        updateOverflow();
        positionEditor();
    }

    function closeMenu() {
        const open = !menu.hidden;
        menu.hidden = true; more.setAttribute('aria-expanded', 'false');
        if (open) onInteraction(false);
    }

    function closeInfo() {
        const open = !info.hidden;
        info.hidden = true;
        if (open) onInteraction(false);
    }

    async function request(url, options = {}, controller = new AbortController()) {
        requests.add(controller);
        const timer = setTimeout(() => controller.abort(), 10000);
        try {
            const response = await fetch(url, { ...options, signal: controller.signal });
            const body = await response.json();
            if (!response.ok) throw Object.assign(new Error(serverErrorText(body) || response.statusText), { status: response.status, body });
            return body;
        } finally {
            clearTimeout(timer);
            requests.delete(controller);
        }
    }

    async function persist(ids, revision) {
        if (saving || destroyed) return;
        saving = true;
        pendingIds = merge(ids);
        paint();
        try {
            const body = await request('/api/apps/order', {
                method: 'PUT', headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ ordered_ids: ids, base_revision: revision })
            });
            if (destroyed) return;
            acceptOrder(body);
            notify(tr('saved'));
        } catch (error) {
            if (destroyed) return;
            // A timeout may happen after the transaction committed. Re-read the
            // authoritative order instead of blindly restoring stale positions.
            try { acceptOrder(await request('/api/apps/order')); } catch (_) {}
            notify(`${tr('failed')}: ${error.message}`, true);
        } finally {
            saving = false; pendingIds = null;
            if (!destroyed) paint();
        }
    }

    function cancelDrag() {
        if (!drag) return;
        const started = drag.started;
        const pointerId = drag.pointerId;
        drag = null; pendingIds = null;
        if (viewport.hasPointerCapture(pointerId)) viewport.releasePointerCapture(pointerId);
        cancelAnimationFrame(frame); frame = 0;
        ghost?.remove(); ghost = null;
        if (started) {
            suppressedPointer = pointerId;
            onInteraction(false);
            if (!destroyed) paint();
        }
    }

    function previewAt(x) {
        if (!drag?.started) return;
        const ids = visibleIds().filter(id => id !== drag.id);
        let insertion = ids.length;
        for (let i = 0; i < ids.length; i++) {
            const group = [...list.children].filter(n => n.dataset.appId === ids[i]);
            const first = group[0]?.getBoundingClientRect(), last = group.at(-1)?.getBoundingClientRect();
            if (first && x < (first.left + last.right) / 2) { insertion = i; break; }
        }
        ids.splice(insertion, 0, drag.id);
        if (ids.join('\0') !== drag.ids.join('\0')) {
            drag.ids = ids; pendingIds = merge(ids); paint();
        }
        ghost.style.left = `${Math.min(window.innerWidth - 80, Math.max(0, x + 10))}px`;
        ghost.style.top = `${Math.max(0, drag.y - 40)}px`;
    }

    function autoScroll() {
        if (!drag?.started) return;
        const rect = viewport.getBoundingClientRect(), edge = Math.min(36, rect.width / 3);
        let speed = 0;
        if (drag.x < rect.left + edge) speed = -12 * Math.min(1, (rect.left + edge - drag.x) / edge);
        else if (drag.x > rect.right - edge) speed = 12 * Math.min(1, (drag.x - rect.right + edge) / edge);
        const before = viewport.scrollLeft;
        viewport.scrollLeft += speed;
        if (viewport.scrollLeft !== before) previewAt(drag.x);
        frame = requestAnimationFrame(autoScroll);
    }

    listen(document, 'pointerdown', (event) => {
        if (event.isPrimary === false || event.button !== 0) return;
        cancelInteraction();
        // A new press always starts a new click, even immediately after a drag.
        suppressedPointer = null;
    }, { capture: true });
    listen(document, 'click', (event) => {
        if (suppressedPointer === null || event.detail === 0) return;
        if (event.pointerId !== undefined && event.pointerId !== suppressedPointer) return;
        suppressedPointer = null;
        event.preventDefault(); event.stopImmediatePropagation();
        finishPress();
    }, { capture: true });
    listen(document, 'click', () => finishPress());
    listen(list, 'pointerdown', (event) => {
        const node = event.target.closest('.taskbar-item');
        if (event.isPrimary === false || event.button !== 0 || !node) return;
        press = { key: node._entry.key, appId: node._entry.appId, pointerId: event.pointerId, cancelled: false, released: false };
        if (!canReorder || saving || drag || !confirmed.ordered_ids.includes(node._entry.appId)) return;
        // Fingers keep native horizontal scrolling. Mouse/trackpad presses
        // and pen input can start sorting from anywhere on the tab.
        if (event.pointerType === 'touch') return;
        drag = { id: node._entry.appId, pointerId: event.pointerId, x: event.clientX, y: event.clientY,
            startX: event.clientX, startY: event.clientY, started: false, ids: visibleIds(), revision: confirmed.order_revision };
    });
    listen(document, 'pointermove', (event) => {
        if (!drag || drag.pointerId !== event.pointerId) return;
        drag.x = event.clientX; drag.y = event.clientY;
        if (!drag.started) {
            const dx = Math.abs(drag.x - drag.startX), dy = Math.abs(drag.y - drag.startY);
            if (dx < 5 || dx <= dy) return;
        }
        event.preventDefault();
        if (!drag.started) {
            drag.started = true;
            finishPress(false);
            viewport.focus({ preventScroll: true });
            viewport.setPointerCapture(event.pointerId);
            ghost = make('div', 'toolbar-drag-ghost', document.body);
            ghost.textContent = entries.find(e => e.appId === drag.id)?.label || drag.id;
            onInteraction(true); paint(); autoScroll();
        }
        previewAt(drag.x);
    }, { passive: false });
    listen(document, 'pointerup', (event) => {
        if (drag && event.pointerId === drag.pointerId) {
            const state = drag, rect = apps.getBoundingClientRect();
            const inside = event.clientX >= rect.left && event.clientX <= rect.right && event.clientY >= rect.top - 20 && event.clientY <= rect.bottom + 20;
            const changed = state.ids.join('\0') !== visibleIdsFromConfirmed().join('\0');
            cancelDrag();
            if (state.started && inside && changed) persist(state.ids, state.revision);
        }
        if (!press || event.pointerId !== press.pointerId) return;
        press.released = true;
        if (press.cancelled || event.target.closest('.taskbar-item')?.dataset.key !== press.key) {
            suppressedPointer = press.pointerId;
            finishPress();
        } else {
            // Native click normally finishes synchronously after pointerup.
            // Also allow delayed touch clicks, with a fallback if none arrives.
            pressTimer = setTimeout(finishPress, 500);
        }
    });
    const visibleIdsFromConfirmed = () => [...new Set(orderedEntries(confirmed.ordered_ids).map(e => e.appId).filter(id => confirmed.ordered_ids.includes(id)))];
    listen(document, 'pointercancel', (event) => {
        if (event.pointerId === press?.pointerId || event.pointerId === drag?.pointerId) cancelInteraction();
    });
    listen(element, 'pointerleave', (event) => {
        // A release inside an iframe never reaches this document. A pending
        // click that leaves the toolbar must not hold subsequent updates.
        // Touch also leaves after pointerup, before its compatibility click.
        if (event.pointerId === press?.pointerId && !press.released) cancelInteraction();
    });
    // Pen input can start with implicit capture on the tab; its transfer to
    // the viewport must not be mistaken for the drag losing capture.
    listen(viewport, 'lostpointercapture', (event) => {
        if (event.target === viewport && drag?.started) cancelDrag();
    });
    listen(window, 'blur', cancelInteraction);
    listen(document, 'visibilitychange', () => { if (document.hidden) { cancelInteraction(); resetNavigation(); } });
    listen(document, 'keydown', (event) => {
        if (event.key !== 'Escape') return;
        if (event.isComposing || event.keyCode === 229) return;
        if (editor) { event.preventDefault(); event.stopPropagation(); closeEditor(true); }
        if (drag?.started) { event.preventDefault(); event.stopPropagation(); }
        cancelInteraction(); closeMenu(); closeInfo();
    }, { capture: true });
    listen(list, 'dragstart', event => event.preventDefault());
    listen(viewport, 'wheel', (event) => {
        if (event.ctrlKey || Math.abs(event.deltaX) >= Math.abs(event.deltaY) || viewport.scrollWidth <= viewport.clientWidth) return;
        event.preventDefault(); event.stopPropagation();
        viewport.scrollLeft += event.deltaY * (event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? viewport.clientWidth : 1);
    }, { passive: false });
    listen(viewport, 'scroll', () => { updateOverflow(); positionEditor(); }, { passive: true });
    listen(previous, 'click', () => viewport.scrollBy({ left: -Math.max(100, viewport.clientWidth * .7), behavior: 'smooth' }));
    listen(next, 'click', () => viewport.scrollBy({ left: Math.max(100, viewport.clientWidth * .7), behavior: 'smooth' }));
    listen(more, 'click', () => {
        menu.hidden = !menu.hidden; more.setAttribute('aria-expanded', String(!menu.hidden)); onInteraction(!menu.hidden);
    });
    // Tool handlers stop propagation; close before they open their own panels.
    listen(menu, 'click', (event) => { if (event.target.closest('.taskbar-pin, .toolbar-gesture-action')) closeMenu(); }, { capture: true });
    listen(document, 'pointerdown', (event) => {
        if (!renamePanel.contains(event.target)) closeEditor();
        if (!tools.contains(event.target)) closeMenu();
        if (!status.contains(event.target)) closeInfo();
    });
    function showNetworkInfo(event) {
        const badge = event.target.closest('.taskbar-network-quality');
        if (!badge) return;
        info.textContent = `${badge.textContent}\n${badge.title}`;
        info.hidden = !info.hidden; onInteraction(!info.hidden);
    }
    listen(status, 'click', showNetworkInfo);
    listen(status, 'keydown', (event) => {
        if (['Enter', ' '].includes(event.key) && event.target.closest('.taskbar-network-quality')) {
            event.preventDefault(); showNetworkInfo(event);
        }
    });
    const resize = new ResizeObserver(layout);
    resize.observe(element); resize.observe(viewport); resize.observe(list); resize.observe(status);
    const translate = () => {
        syncGestureButton();
        previous.title = tr('previous'); next.title = tr('next'); more.title = tr('more');
        [previous, next, more].forEach(n => n.setAttribute('aria-label', n.title));
        apps.setAttribute('aria-label', tr('apps'));
        renameCaption.textContent = tr('rename'); renamePanel.setAttribute('aria-label', tr('rename'));
        renameInput.setAttribute('aria-label', tr('rename')); renameCancel.textContent = tr('cancel');
        layout(); paint();
    };
    const untranslate = onLangChange(translate);
    translate();
    function addTool(node, primary = false) {
        const label = make('span', 'toolbar-tool-label', node);
        toolEntries.push({ node, label, primary });
        node.setAttribute('role', 'button'); node.tabIndex = 0;
        listen(node, 'keydown', event => {
            if (['Enter', ' '].includes(event.key) && !node.classList.contains('disabled')) {
                event.preventDefault(); event.stopPropagation();
                if (!event.repeat) node.click();
            }
        });
        layout();
    }
    return {
        status,
        addTool,
        addGestureToggle(controller, onToggle = () => {}) {
            const button = make('button', 'taskbar-pin toolbar-gesture-action');
            button.type = 'button';
            button.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 12h16m-4-4 4 4-4 4M8 8l-4 4 4 4"/></svg>';
            syncGestureButton = () => {
                button.title = tr(controller.enabled() ? 'swipeOn' : 'swipeOff');
                button.setAttribute('aria-label', tr('swipe'));
                button.setAttribute('aria-description', tr('swipeHint'));
                button.setAttribute('aria-pressed', String(controller.enabled()));
                button.classList.toggle('active', controller.enabled());
            };
            listen(button, 'click', () => {
                closeMenu(); closeInfo();
                controller.setEnabled(!controller.enabled());
                resetNavigation();
                syncGestureButton();
                onToggle(controller.enabled());
            });
            listen(document, 'visibilitychange', () => { if (document.hidden) controller.reset(); });
            syncGestureButton();
            addTool(button, true);
            return button;
        },
        navigate(direction) {
            if (destroyed || saving || press || drag?.started || editor || !menu.hidden || !info.hidden || ![-1, 1].includes(direction)) return null;
            const sorted = orderedEntries();
            const focused = sorted.findIndex(entry => navigation ? entry.key === navigation.key : entry.active);
            const current = focused < 0 ? (direction > 0 ? -1 : sorted.length) : focused;
            for (let index = current + direction; index >= 0 && index < sorted.length; index += direction) {
                const entry = sorted[index];
                if (entry.available === false || entry.stable === false) continue;
                if (!activateEntry(entry)) return null;
                const label = entry.label;
                clearTimeout(noticeTimer);
                notice.textContent = label;
                notice.hidden = false;
                noticeTimer = setTimeout(() => { notice.hidden = true; }, 1300);
                return entry.key;
            }
            return null;
        },
        resetNavigation,
        update(data, streams = [], streamActive = false) {
            acceptOrder(data);
            for (const w of data.windows || []) acceptName({ id: w.managed_app_id, name: w.managed_app_name, name_revision: w.name_revision });
            for (const s of streams) acceptName({ id: s.appId, name: s.appName, name_revision: s.nameRevision });
            entries = (data.windows || []).map(w => ({
                key: `window:${w.window_key || w.id}`, appId: w.managed_app_id,
                canRename: w.can_rename === true,
                windowTitle: w.title || '', windowClass: w.app_id,
                label: w.title || w.display_name || w.app_id || `Window ${w.id}`,
                title: `${w.title || w.display_name || w.app_id || ''} (${w.app_id || ''})`,
                active: w.focused && !streamActive,
                stable: typeof w.window_key === 'string' && w.window_key.length > 0,
                activate: () => activateWindow(w.id, w.window_key)
            })).concat(streams);
            if (navigation) {
                const target = entries.find(e => e.key === navigation.key && e.appId === navigation.appId);
                if (entries.find(e => e.active)?.key !== navigation.base) navigation.sawChange = true;
                if (!target || target.available === false || target.stable === false) resetNavigation(true, false);
                else if (target.active && (!navigation.wasActive || navigation.sawChange)) resetNavigation(false, false);
            }
            if (editor && !entries.some(e => e.key === editor.key && e.appId === editor.id && e.canRename)) closeEditor();
            for (const id of names.keys()) if (!entries.some(e => e.appId === id)) names.delete(id);
            if (press && !entries.some(e => e.key === press.key && e.appId === press.appId)) press.cancelled = true;
            if (drag && (drag.revision !== confirmed.order_revision || drag.ids.length !== visibleIdsFromConfirmed().length || !drag.ids.every(id => entries.some(e => e.appId === id)))) cancelDrag();
            paint();
        },
        interacting: () => !!editor || !!press || !!drag?.started || !menu.hidden || !info.hidden,
        destroy() {
            destroyed = true;
            clearTimeout(navigationTimer);
            closeEditor(); cancelInteraction(); requests.forEach(controller => controller.abort()); events.abort(); resize.disconnect();
            clearTimeout(noticeTimer); untranslate(); notifications.destroy();
            element.remove(); nodes.clear();
        }
    };
}
