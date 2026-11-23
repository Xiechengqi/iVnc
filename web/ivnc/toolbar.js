// Owns toolbar DOM, display order and gestures. Stream views remain owned by
// app-streams.js; moving tabs never moves or recreates an application's iframe.
import toolbarStyles from './toolbar.css?inline';
import { getLang, onLangChange, serverErrorText } from './lib/i18n.js?v=1';
import { createOrderNotifications } from './app-order.js';

const STRINGS = {
    zh: { apps: '应用', previous: '向左滚动应用', next: '向右滚动应用', more: '更多工具',
        morePage: '更多', backPage: '返回', assist: '辅助功能',
        move: '按住标签左右拖动排序，或使用左右方向键移动', saved: '应用顺序已保存', failed: '无法保存应用顺序', network: '网络状态',
        rename: '修改应用名', save: '保存', cancel: '取消', pending: '保存中…', nameFailed: '无法保存应用名',
        latest: '当前名称：', changed: '应用名已发生变化。请核对当前名称，再次保存以使用你的输入。',
        invalid: '名称不能为空、不能包含换行或控制字符，且不能超过 256 个 UTF-8 字节。',
        menu: '应用操作', stop: '停止', restart: '重启', logs: '日志', edit: '编辑', renameAction: '重命名',
        stopped: '应用已停止', restarted: '应用已重启', actionFailed: '操作失败：', manageUnavailable: '当前页面不能打开控制台' },
    en: { apps: 'Applications', previous: 'Scroll apps left', next: 'Scroll apps right', more: 'More tools',
        morePage: 'More', backPage: 'Back', assist: 'Accessibility shortcut',
        move: 'Hold and drag the tab left or right to reorder, or use left/right arrow keys', saved: 'Application order saved', failed: 'Could not save application order', network: 'Network status',
        rename: 'Rename application', save: 'Save', cancel: 'Cancel', pending: 'Saving…', nameFailed: 'Could not save application name',
        latest: 'Current name: ', changed: 'The application name changed. Review the current name, then save again to use your draft.',
        invalid: 'Enter a name without line breaks or control characters, up to 256 UTF-8 bytes.',
        menu: 'Application actions', stop: 'Stop', restart: 'Restart', logs: 'Logs', edit: 'Edit', renameAction: 'Rename',
        stopped: 'Application stopped', restarted: 'Application restarted', actionFailed: 'Action failed: ', manageUnavailable: 'The console is not available on this page' }
};

export function createToolbar({ element, canReorder = false, canRename = false, activateWindow, onInteraction = () => {}, onAppAction = null }) {
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
    const assist = make('button', 'toolbar-assist', document.body);
    assist.type = 'button'; assist.hidden = true;
    assist.setAttribute('aria-expanded', 'false');
    assist.innerHTML = '<span class="toolbar-assist-ring" aria-hidden="true"></span>';
    const appStrip = make('div', 'toolbar-app-strip', element);
    const apps = make('div', 'toolbar-apps', appStrip);
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
    const assistPager = make('button', 'toolbar-assist-pager', menu);
    assistPager.type = 'button'; assistPager.hidden = true;
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
    const appMenuPanel = make('div', 'toolbar-app-menu', element);
    appMenuPanel.hidden = true;
    appMenuPanel.setAttribute('role', 'menu');
    const appMenuActions = ['stop', 'restart', 'logs', 'edit', 'rename'];
    const appMenuButtons = new Map(appMenuActions.map(action => {
        const button = make('button', 'toolbar-app-menu-item', appMenuPanel);
        button.type = 'button'; button.dataset.action = action; button.setAttribute('role', 'menuitem');
        return [action, button];
    }));
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
    let lastActive = null, editor = null, renameBusy = false, appMenu = null, actionBusy = false;
    let press = null, pressTimer, paintPending = false, suppressedPointer = null;
    let navigation = null, navigationTimer;
    const ASSIST_KEY = 'ivnc.toolbar-assist.v1';
    const ASSIST_SIZE = 48, ASSIST_RADIUS = 104, ASSIST_PAGE = 8;
    const assistQuery = window.matchMedia('(hover: none) and (pointer: coarse)');
    let assistPoint = loadAssistPoint();
    let assistDrag = null, assistIgnoreClick = false, assistPage = 0, assistHideTimer, assistOpen = 0;

    function loadAssistPoint() {
        try {
            const stored = JSON.parse(localStorage.getItem(ASSIST_KEY) || 'null');
            if (stored && ['left', 'right', 'top', 'bottom'].includes(stored.edge) && typeof stored.along === 'number') {
                return { edge: stored.edge, along: Math.min(1, Math.max(0, stored.along)) };
            }
        } catch (_) {}
        return { edge: 'left', along: 0.72 };
    }
    function assistActive() {
        if (element.clientWidth <= 0) return false;
        const points = navigator.maxTouchPoints || 0;
        if (points < 1) return false;
        // iPadOS desktop Safari keeps a fine pointer. A fine-pointer laptop does not.
        if (navigator.platform === 'MacIntel' && points > 1) return true;
        return assistQuery.matches;
    }
    function assistFrame() {
        const visual = window.visualViewport;
        const left = visual?.offsetLeft || 0, top = visual?.offsetTop || 0;
        return { left, top, right: left + (visual?.width || window.innerWidth), bottom: top + (visual?.height || window.innerHeight) };
    }
    function placeAssist() {
        const size = ASSIST_SIZE, margin = 8, frame = assistFrame();
        const spanX = Math.max(0, frame.right - frame.left - size - margin * 2);
        const spanY = Math.max(0, frame.bottom - frame.top - size - margin * 2);
        const alongX = frame.left + margin + assistPoint.along * spanX;
        const alongY = frame.top + margin + assistPoint.along * spanY;
        const x = assistPoint.edge === 'right' ? frame.left + margin + spanX : assistPoint.edge === 'left' ? frame.left + margin : alongX;
        const y = assistPoint.edge === 'bottom' ? frame.top + margin + spanY : assistPoint.edge === 'top' ? frame.top + margin : alongY;
        assist.style.left = `${x}px`;
        assist.style.top = `${y}px`;
    }
    function assistReducedMotion() {
        return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    }

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

    function placePanel(panel, key) {
        const anchor = nodes.get(key)?.getBoundingClientRect();
        if (!anchor) return;
        const rect = element.getBoundingClientRect();
        const left = Math.max(8, Math.min(window.innerWidth - panel.offsetWidth - 8, anchor.left));
        panel.style.left = `${left - rect.left}px`;
    }

    function positionEditor() {
        if (editor) placePanel(renamePanel, editor.key);
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
        closeEditor(); closeAppMenu(false); cancelDrag(); closeMenu(); closeInfo();
        editor = { id: entry.appId, key: entry.key, revision: app.name_revision, saving: false };
        renameInput.value = app.name; renameError.textContent = '';
        renamePanel.hidden = false; paintEditor(); onInteraction(true);
        renameInput.focus({ preventScroll: true }); renameInput.select();
    }

    function closeAppMenu(restoreFocus = false) {
        if (!appMenu) return;
        const state = appMenu; appMenu = null;
        appMenuPanel.hidden = true;
        if (restoreFocus) nodes.get(state.key)?.focus({ preventScroll: true });
        if (!editor) onInteraction(false);
    }

    function menuActions(entry) {
        const actions = [];
        if (entry.kind !== 'cli') {
            actions.push(entry.running ? 'stop' : 'restart');
            if (entry.kind !== 'remote') actions.push('logs');
        }
        actions.push('edit', 'rename');
        return actions;
    }

    function paintAppMenu() {
        if (!appMenu) return;
        const entry = entries.find(e => e.key === appMenu.key && e.appId === appMenu.id);
        if (!entry?.canManage) { closeAppMenu(); return; }
        appMenu.entry = entry;
        const available = new Set(menuActions(entry));
        for (const [action, button] of appMenuButtons) {
            button.hidden = !available.has(action);
            button.disabled = actionBusy || (action === 'rename' && (!canRename || !entry.canRename));
            button.textContent = tr(action === 'rename' ? 'renameAction' : action);
        }
        appMenuPanel.setAttribute('aria-busy', String(actionBusy));
        placePanel(appMenuPanel, entry.key);
    }

    function openAppMenu(node) {
        const entry = node?._entry;
        if (!canRename || !entry?.canManage || !entry.appId || destroyed) return false;
        closeEditor(); closeAppMenu(false); cancelDrag(); closeMenu(); closeInfo();
        appMenu = { id: entry.appId, key: entry.key, entry };
        appMenuPanel.hidden = false;
        paintAppMenu(); onInteraction(true);
        appMenuButtons.get(menuActions(entry)[0])?.focus({ preventScroll: true });
        return true;
    }

    async function runAppAction(action) {
        if (!appMenu || actionBusy || destroyed) return;
        const state = appMenu, entry = state.entry;
        if (action === 'rename') {
            const node = nodes.get(state.key);
            closeAppMenu(false);
            if (node) openEditor(node);
            return;
        }
        if (action === 'edit' || action === 'logs') {
            closeAppMenu(true);
            if (typeof onAppAction === 'function') onAppAction(action, entry);
            else notify(tr('manageUnavailable'), true);
            return;
        }
        actionBusy = true; paintAppMenu();
        const controller = new AbortController();
        try {
            await request(`/api/apps/${encodeURIComponent(state.id)}/${action}`, { method: 'POST' }, controller);
            if (destroyed || appMenu !== state) return;
            notify(tr(action === 'stop' ? 'stopped' : 'restarted'));
            notice.hidden = false;
            closeAppMenu(true);
        } catch (error) {
            if (destroyed || appMenu !== state) return;
            notify(`${tr('actionFailed')}${error.message}`, true);
        } finally {
            actionBusy = false;
            if (!destroyed && appMenu === state) paintAppMenu();
        }
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
    listen(appMenuPanel, 'click', event => {
        const action = event.target.closest('[data-action]')?.dataset.action;
        if (!action) return;
        event.preventDefault(); event.stopPropagation();
        runAppAction(action);
    });
    listen(appMenuPanel, 'keydown', event => {
        if (event.isComposing || event.keyCode === 229) return;
        const items = [...appMenuButtons.values()].filter(button => !button.hidden && !button.disabled);
        const index = items.indexOf(document.activeElement);
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault(); event.stopPropagation();
            const next = items[(index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length];
            next?.focus({ preventScroll: true });
        } else if (event.key === 'Home' || event.key === 'End') {
            event.preventDefault(); event.stopPropagation();
            items[event.key === 'Home' ? 0 : items.length - 1]?.focus({ preventScroll: true });
        } else if ((event.key === 'Enter' || event.key === ' ') && index >= 0) {
            event.preventDefault(); event.stopPropagation();
            runAppAction(items[index].dataset.action);
        }
    });
    listen(list, 'contextmenu', event => {
        const node = event.target.closest('.taskbar-item');
        if (!canRename || !node?._entry.canManage) return;
        event.preventDefault(); event.stopPropagation(); openAppMenu(node);
    });
    listen(list, 'keydown', event => {
        const node = event.target.closest('.taskbar-item');
        if (!node) return;
        if (event.key === 'F2' && canRename && node._entry.canRename) {
            event.preventDefault(); event.stopPropagation(); openEditor(node);
            return;
        }
        if (event.shiftKey && event.key === 'F10' && canRename && node._entry.canManage) {
            event.preventDefault(); event.stopPropagation(); openAppMenu(node);
        }
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
            if (canRename && entry.canRename) shortcuts.push('F2');
            if (entry.canManage) shortcuts.push('Shift+F10');
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
        paintAppMenu();
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
        const floating = assistActive();
        element.classList.toggle('toolbar-assistive', floating);
        menu.classList.toggle('toolbar-assist-menu', floating);
        const compact = !floating && element.clientWidth <= 700;
        let toolCursor = more, menuCursor = assistPager;
        for (const tool of [...toolEntries].reverse()) {
            const parent = (floating || (compact && !tool.primary)) ? menu : tools;
            const cursor = parent === tools ? toolCursor : menuCursor;
            if (tool.node.parentNode !== parent || tool.node.nextElementSibling !== cursor) parent.insertBefore(tool.node, cursor);
            if (parent === tools) toolCursor = tool.node;
            else menuCursor = tool.node;
            tool.label.textContent = tool.node.dataset.toolLabel || tool.node.title;
            tool.label.hidden = parent === tools;
            if (!floating) { tool.node.hidden = false; tool.node.style.left = tool.node.style.top = ''; }
        }
        const menuParent = floating ? document.body : tools;
        if (menu.parentNode !== menuParent) menuParent.appendChild(menu);
        more.hidden = floating || toolEntries.every(tool => tool.node.parentNode !== menu);
        assist.hidden = !floating;
        if (!compact && !floating) closeMenu();
        if (!floating) {
            assist.setAttribute('aria-expanded', 'false');
            menu.style.left = menu.style.top = menu.style.width = menu.style.height = '';
            assistPage = 0;
        }
        placeAssist();
        placeAssistMenu();
        updateOverflow();
        positionEditor();
        if (appMenu) placePanel(appMenuPanel, appMenu.key);
    }

    function assistArc(count, button, frame) {
        const inward = { left: 0, right: Math.PI, top: Math.PI / 2, bottom: -Math.PI / 2 }[assistPoint.edge];
        const center = { x: button.left + button.width / 2, y: button.top + button.height / 2 };
        const item = 56, margin = 8;
        const bar = element.getBoundingClientRect();
        const fits = (start) => {
            const step = count < 2 ? 0 : Math.PI / (count - 1);
            return Array.from({ length: count }, (_, index) => {
                const angle = start + (index - (count - 1) / 2) * step;
                return { x: center.x + Math.cos(angle) * ASSIST_RADIUS, y: center.y + Math.sin(angle) * ASSIST_RADIUS };
            }).every(point => point.x - item / 2 >= frame.left + margin && point.x + item / 2 <= frame.right - margin
                && point.y - item / 2 >= frame.top + margin && point.y + item / 2 <= frame.bottom - margin
                && (point.y + item / 2 <= bar.top || point.y - item / 2 >= bar.bottom));
        };
        const turns = [0, -Math.PI / 4, Math.PI / 4, -Math.PI / 2, Math.PI / 2, Math.PI];
        const start = turns.find(turn => fits(inward + turn)) ?? 0;
        const step = count < 2 ? 0 : Math.PI / (count - 1);
        return { center, start, step };
    }

    function placeAssistMenu() {
        if (!assistActive()) return;
        const toolsOnMenu = toolEntries.filter(tool => tool.node.parentNode === menu);
        const pages = Math.max(1, Math.ceil(toolsOnMenu.length / (ASSIST_PAGE - 1)));
        assistPage = Math.min(assistPage, pages - 1);
        const paged = toolsOnMenu.length > ASSIST_PAGE;
        const visible = paged ? toolsOnMenu.slice(assistPage * (ASSIST_PAGE - 1), (assistPage + 1) * (ASSIST_PAGE - 1)) : toolsOnMenu;
        for (const tool of toolsOnMenu) tool.node.hidden = paged && !visible.includes(tool);
        assistPager.hidden = !paged;
        if (paged) {
            const last = assistPage >= pages - 1;
            assistPager.dataset.page = last ? 'back' : 'more';
            assistPager.textContent = tr(last ? 'backPage' : 'morePage');
            assistPager.setAttribute('aria-label', assistPager.textContent);
        }
        if (menu.hidden || menu.classList.contains('closing')) return;
        const shown = [...visible.map(tool => tool.node), ...(paged ? [assistPager] : [])];
        const button = assist.getBoundingClientRect();
        const arc = assistArc(Math.max(1, shown.length), button, assistFrame());
        menu.style.left = '0px';
        menu.style.top = '0px';
        menu.style.width = '0px';
        menu.style.height = '0px';
        shown.forEach((node, index) => {
            const angle = arc.start + (index - (shown.length - 1) / 2) * arc.step;
            node.style.left = `${arc.center.x + Math.cos(angle) * ASSIST_RADIUS}px`;
            node.style.top = `${arc.center.y + Math.sin(angle) * ASSIST_RADIUS}px`;
        });
    }

    function snapAssist(x, y) {
        const size = ASSIST_SIZE, margin = 8;
        const frame = assistFrame();
        const cx = x + size / 2, cy = y + size / 2;
        const edges = [['left', cx - frame.left], ['right', frame.right - cx], ['top', cy - frame.top], ['bottom', frame.bottom - cy]];
        const edge = edges.reduce((best, item) => item[1] < best[1] ? item : best)[0];
        const vertical = edge === 'left' || edge === 'right';
        const span = Math.max(1, (vertical ? frame.bottom - frame.top : frame.right - frame.left) - size - margin * 2);
        const origin = vertical ? frame.top : frame.left;
        assistPoint = { edge, along: Math.min(1, Math.max(0, ((vertical ? y : x) - origin - margin) / span)) };
        try { localStorage.setItem(ASSIST_KEY, JSON.stringify(assistPoint)); } catch (_) {}
        placeAssist();
        placeAssistMenu();
    }

    function finishCloseMenu() {
        assistOpen++;
        menu.hidden = true;
        menu.classList.remove('closing');
        more.setAttribute('aria-expanded', 'false');
        assist.setAttribute('aria-expanded', 'false');
        assistPage = 0;
        onInteraction(false);
    }

    function closeMenu() {
        if (menu.hidden || menu.classList.contains('closing')) return;
        clearTimeout(assistHideTimer);
        assistOpen++;
        if (!assistActive() || assistReducedMotion()) { finishCloseMenu(); return; }
        const generation = assistOpen;
        const box = assist.getBoundingClientRect();
        const cx = box.left + box.width / 2, cy = box.top + box.height / 2;
        for (const node of menu.querySelectorAll('.taskbar-pin:not([hidden]), .toolbar-assist-pager:not([hidden])')) {
            node.style.transition = 'none';
            node.style.left = `${cx}px`;
            node.style.top = `${cy}px`;
        }
        requestAnimationFrame(() => {
            if (generation !== assistOpen || menu.hidden) return;
            for (const node of menu.querySelectorAll('.taskbar-pin, .toolbar-assist-pager')) node.style.transition = '';
            menu.classList.add('closing');
            assistHideTimer = setTimeout(() => { if (generation === assistOpen) finishCloseMenu(); }, 220);
        });
        assist.setAttribute('aria-expanded', 'false');
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
    listen(window, 'blur', () => { cancelAssistDrag({ pointerId: assistDrag?.pointerId }); cancelInteraction(); });
    listen(assistQuery, 'change', () => { closeMenu(); layout(); });
    if (window.visualViewport) {
        listen(window.visualViewport, 'resize', () => { placeAssist(); placeAssistMenu(); });
        listen(window.visualViewport, 'scroll', () => { placeAssist(); placeAssistMenu(); });
    }
    listen(document, 'visibilitychange', () => { if (document.hidden) { cancelAssistDrag({ pointerId: assistDrag?.pointerId }); cancelInteraction(); closeMenu(); resetNavigation(); } });
    listen(document, 'keydown', (event) => {
        if (event.key !== 'Escape') return;
        if (event.isComposing || event.keyCode === 229) return;
        if (editor) { event.preventDefault(); event.stopPropagation(); closeEditor(true); }
        else if (appMenu) { event.preventDefault(); event.stopPropagation(); closeAppMenu(true); }
        if (drag?.started) { event.preventDefault(); event.stopPropagation(); }
        cancelInteraction(); closeMenu(); closeInfo();
    }, { capture: true });
    listen(list, 'dragstart', event => event.preventDefault());
    listen(viewport, 'wheel', (event) => {
        if (event.ctrlKey || Math.abs(event.deltaX) >= Math.abs(event.deltaY) || viewport.scrollWidth <= viewport.clientWidth) return;
        event.preventDefault(); event.stopPropagation();
        viewport.scrollLeft += event.deltaY * (event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? viewport.clientWidth : 1);
    }, { passive: false });
    listen(viewport, 'scroll', () => { updateOverflow(); positionEditor(); if (appMenu) placePanel(appMenuPanel, appMenu.key); }, { passive: true });
    listen(previous, 'click', () => viewport.scrollBy({ left: -Math.max(100, viewport.clientWidth * .7), behavior: 'smooth' }));
    listen(next, 'click', () => viewport.scrollBy({ left: Math.max(100, viewport.clientWidth * .7), behavior: 'smooth' }));
    function revealAssist(generation) {
        if (generation !== assistOpen || menu.hidden || menu.classList.contains('closing')) return;
        menu.classList.remove('opening');
        placeAssistMenu();
    }

    function toggleAssist(open = menu.hidden) {
        clearTimeout(assistHideTimer);
        menu.classList.remove('closing');
        if (open) {
            const generation = ++assistOpen;
            menu.hidden = false;
            assist.setAttribute('aria-expanded', 'true');
            more.setAttribute('aria-expanded', 'false');
            placeAssistMenu();
            if (!assistReducedMotion()) {
                const box = assist.getBoundingClientRect();
                const cx = box.left + box.width / 2, cy = box.top + box.height / 2;
                menu.classList.add('opening');
                for (const node of menu.querySelectorAll('.taskbar-pin:not([hidden]), .toolbar-assist-pager:not([hidden])')) {
                    node.style.transition = 'none';
                    node.style.left = `${cx}px`;
                    node.style.top = `${cy}px`;
                }
                requestAnimationFrame(() => {
                    if (generation !== assistOpen) return;
                    for (const node of menu.querySelectorAll('.taskbar-pin, .toolbar-assist-pager')) node.style.transition = '';
                    requestAnimationFrame(() => revealAssist(generation));
                });
            }
            onInteraction(true);
            return;
        }
        closeMenu();
    }
    listen(more, 'click', () => {
        menu.hidden = !menu.hidden; more.setAttribute('aria-expanded', String(!menu.hidden)); onInteraction(!menu.hidden);
    });
    listen(assist, 'pointerdown', (event) => {
        if (event.button !== 0 || !assistActive()) return;
        assistIgnoreClick = false;
        assistDrag = { pointerId: event.pointerId, x: event.clientX, y: event.clientY,
            left: assist.getBoundingClientRect().left, top: assist.getBoundingClientRect().top, moved: false };
        assist.setPointerCapture(event.pointerId);
    });
    listen(document, 'pointermove', (event) => {
        if (!assistDrag || assistDrag.pointerId !== event.pointerId) return;
        const dx = event.clientX - assistDrag.x, dy = event.clientY - assistDrag.y;
        if (!assistDrag.moved && Math.hypot(dx, dy) < 8) return;
        if (!assistDrag.moved) closeMenu();
        assistDrag.moved = true;
        event.preventDefault();
        const size = ASSIST_SIZE, margin = 8, frame = assistFrame();
        assist.style.left = `${Math.min(frame.right - size - margin, Math.max(frame.left + margin, assistDrag.left + dx))}px`;
        assist.style.top = `${Math.min(frame.bottom - size - margin, Math.max(frame.top + margin, assistDrag.top + dy))}px`;
    }, { passive: false });
    listen(document, 'pointerup', (event) => {
        if (!assistDrag || assistDrag.pointerId !== event.pointerId) return;
        const moved = assistDrag.moved;
        const box = assist.getBoundingClientRect();
        assistDrag = null;
        if (moved) { assistIgnoreClick = true; snapAssist(box.left, box.top); }
    });
    const cancelAssistDrag = (event) => {
        if (!assistDrag || (event.pointerId !== undefined && event.pointerId !== assistDrag.pointerId)) return;
        const moved = assistDrag.moved;
        assistDrag = null;
        if (moved) snapAssist(assist.getBoundingClientRect().left, assist.getBoundingClientRect().top);
        else placeAssist();
    };
    listen(assist, 'pointercancel', cancelAssistDrag);
    listen(assist, 'lostpointercapture', cancelAssistDrag);
    listen(assist, 'click', (event) => {
        if (!assistActive()) return;
        if (assistIgnoreClick) { assistIgnoreClick = false; event.preventDefault(); event.stopPropagation(); return; }
        event.stopPropagation();
        toggleAssist();
    });
    listen(assistPager, 'click', (event) => {
        event.preventDefault();
        event.stopPropagation();
        const toolsOnMenu = toolEntries.filter(tool => tool.node.parentNode === menu);
        const pages = Math.max(1, Math.ceil(toolsOnMenu.length / (ASSIST_PAGE - 1)));
        assistPage = assistPage >= pages - 1 ? 0 : assistPage + 1;
        placeAssistMenu();
    });

    // Tool handlers stop propagation; close before they open their own panels.
    const closeMenuFromTool = (event) => { if (event.target.closest('.taskbar-pin')) closeMenu(); };
    listen(tools, 'click', closeMenuFromTool, { capture: true });
    listen(menu, 'click', closeMenuFromTool, { capture: true });
    listen(document, 'pointerdown', (event) => {
        if (!renamePanel.contains(event.target)) closeEditor();
        if (!appMenuPanel.contains(event.target)) closeAppMenu();
        if (!tools.contains(event.target) && !assist.contains(event.target) && !menu.contains(event.target)) closeMenu();
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
    let resizeFrame = 0;
    const resize = new ResizeObserver(() => {
        if (resizeFrame) return;
        // Updating overflow arrows resizes the observed viewport.
        resizeFrame = requestAnimationFrame(() => { resizeFrame = 0; layout(); });
    });
    resize.observe(element); resize.observe(viewport); resize.observe(list); resize.observe(status);
    const translate = () => {
        previous.title = tr('previous'); next.title = tr('next'); more.title = tr('more'); assist.title = tr('assist');
        [previous, next, more, assist].forEach(n => n.setAttribute('aria-label', n.title));
        apps.setAttribute('aria-label', tr('apps'));
        renameCaption.textContent = tr('rename'); renamePanel.setAttribute('aria-label', tr('rename'));
        renameInput.setAttribute('aria-label', tr('rename')); renameCancel.textContent = tr('cancel');
        appMenuPanel.setAttribute('aria-label', tr('menu'));
        paintAppMenu();
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
    // Query and activation share ordering, pending intent and eligibility.
    // A gesture can yield at an edge without sending a focus request.
    function navigationTarget(direction) {
        if (destroyed || saving || press || drag?.started || editor || appMenu || !menu.hidden || !info.hidden || ![-1, 1].includes(direction)) return null;
        const sorted = orderedEntries();
        const focused = sorted.findIndex(entry => navigation ? entry.key === navigation.key : entry.active);
        const current = focused < 0 ? (direction > 0 ? -1 : sorted.length) : focused;
        for (let index = current + direction; index >= 0 && index < sorted.length; index += direction) {
            const entry = sorted[index];
            if (entry.available !== false && entry.stable !== false) return entry;
        }
        return null;
    }
    return {
        status,
        addTool,
        addAppControl(node) { appStrip.prepend(node); layout(); },
        canNavigate: direction => !!navigationTarget(direction),
        navigate(direction) {
            const entry = navigationTarget(direction);
            if (!entry || !activateEntry(entry)) return null;
            clearTimeout(noticeTimer);
            notice.textContent = entry.label;
            notice.hidden = false;
            noticeTimer = setTimeout(() => { notice.hidden = true; }, 1300);
            return entry.key;
        },
        resetNavigation,
        update(data, streams = [], streamActive = false) {
            acceptOrder(data);
            for (const w of data.windows || []) acceptName({ id: w.managed_app_id, name: w.managed_app_name, name_revision: w.name_revision });
            for (const s of streams) acceptName({ id: s.appId, name: s.appName, name_revision: s.nameRevision });
            entries = (data.windows || []).map(w => ({
                key: `window:${w.window_key || w.id}`, appId: w.managed_app_id,
                canRename: w.can_rename === true,
                canManage: !!w.managed_app_id && w.can_rename === true,
                kind: 'desktop', running: true,
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
            if (appMenu && !entries.some(e => e.key === appMenu.key && e.appId === appMenu.id && e.canManage)) closeAppMenu();
            for (const id of names.keys()) if (!entries.some(e => e.appId === id)) names.delete(id);
            if (press && !entries.some(e => e.key === press.key && e.appId === press.appId)) press.cancelled = true;
            if (drag && (drag.revision !== confirmed.order_revision || drag.ids.length !== visibleIdsFromConfirmed().length || !drag.ids.every(id => entries.some(e => e.appId === id)))) cancelDrag();
            paint();
        },
        interacting: () => !!editor || !!appMenu || !!press || !!drag?.started || !!assistDrag?.moved || !menu.hidden || !info.hidden,
        destroy() {
            destroyed = true;
            clearTimeout(navigationTimer);
            cancelAnimationFrame(resizeFrame);
            closeEditor(); closeAppMenu(); cancelInteraction(); requests.forEach(controller => controller.abort()); events.abort(); resize.disconnect();
            clearTimeout(noticeTimer); untranslate(); notifications.destroy();
            clearTimeout(assistHideTimer); assist.remove(); menu.remove(); element.remove(); nodes.clear();
        }
    };
}
