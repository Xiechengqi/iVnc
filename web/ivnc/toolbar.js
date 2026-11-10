// Owns toolbar DOM, display order and gestures. Stream views remain owned by
// app-streams.js; moving tabs never moves or recreates an application's iframe.
import toolbarStyles from './toolbar.css?inline';
import { getLang, onLangChange, serverErrorText } from './lib/i18n.js?v=1';
import { createOrderNotifications } from './app-order.js';

const STRINGS = {
    zh: { apps: '应用', previous: '向左滚动应用', next: '向右滚动应用', more: '更多工具',
        move: '拖动排序，或使用左右方向键移动', saved: '应用顺序已保存', failed: '无法保存应用顺序', network: '网络状态' },
    en: { apps: 'Applications', previous: 'Scroll apps left', next: 'Scroll apps right', more: 'More tools',
        move: 'Drag to reorder, or use left/right arrow keys', saved: 'Application order saved', failed: 'Could not save application order', network: 'Network status' }
};

export function createToolbar({ element, canReorder = false, activateWindow, onInteraction = () => {} }) {
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
    const notifications = createOrderNotifications();
    const toolEntries = [], nodes = new Map();
    let entries = [], confirmed = { ordered_ids: [], order_revision: -1 }, pendingIds = null;
    let drag = null, saving = false, destroyed = false, ghost = null, frame = 0, noticeTimer;
    let lastActive = null, suppressClickUntil = 0, requestController;

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
        const sorted = orderedEntries();
        const live = new Set(sorted.map(e => e.key));
        for (const [key, node] of nodes) {
            if (!live.has(key)) { node.remove(); nodes.delete(key); }
        }
        const left = viewport.scrollLeft;
        let cursor = list.firstElementChild;
        let firstDragged = true;
        for (const entry of sorted) {
            let node = nodes.get(entry.key);
            if (!node) {
                node = make('div', 'taskbar-item');
                node.tabIndex = 0; node.setAttribute('role', 'button');
                const dot = make('span', 'taskbar-app-dot', node);
                const label = make('span', 'toolbar-label', node);
                const grip = make('button', 'toolbar-grip', node);
                grip.type = 'button'; grip.textContent = '⠿';
                node._parts = { dot, label, grip };
                node.onclick = (event) => {
                    event.stopPropagation();
                    if (event.target.closest('.toolbar-grip') || performance.now() < suppressClickUntil) return;
                    node._entry.activate();
                };
                node.onkeydown = (event) => {
                    if (event.target !== node || !['Enter', ' '].includes(event.key)) return;
                    event.preventDefault(); event.stopPropagation(); node._entry.activate();
                };
                grip.onkeydown = (event) => {
                    if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
                    event.preventDefault(); event.stopPropagation();
                    if (saving || drag) return;
                    const ids = visibleIds(), from = ids.indexOf(node._entry.appId);
                    const to = from + (event.key === 'ArrowLeft' ? -1 : 1);
                    if (from < 0 || to < 0 || to >= ids.length) return;
                    [ids[from], ids[to]] = [ids[to], ids[from]];
                    persist(ids, confirmed.order_revision);
                };
                nodes.set(entry.key, node);
            }
            node._entry = entry;
            node.dataset.key = entry.key;
            node.dataset.appId = entry.appId || '';
            node.classList.toggle('focused', !!entry.active);
            node.classList.toggle('taskbar-app', !!entry.dot);
            const dragged = !!drag?.started && drag.id === entry.appId;
            node.classList.toggle('dragging', dragged);
            node.classList.toggle('drop-target', dragged && firstDragged);
            if (dragged) firstDragged = false;
            node.title = entry.title;
            node.setAttribute('aria-label', entry.title);
            node.setAttribute('aria-pressed', String(!!entry.active));
            node._parts.label.textContent = entry.label;
            node._parts.dot.hidden = !entry.dot;
            node._parts.dot.className = `taskbar-app-dot ${entry.dot || ''}`;
            node._parts.grip.hidden = !canReorder || !confirmed.ordered_ids.includes(entry.appId);
            node._parts.grip.disabled = saving;
            node._parts.grip.title = tr('move');
            node._parts.grip.setAttribute('aria-label', `${tr('move')}: ${entry.label}`);
            if (node !== cursor) list.insertBefore(node, cursor);
            cursor = node.nextElementSibling;
        }
        viewport.scrollLeft = left;
        updateOverflow();
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
        more.hidden = !compact || !toolEntries.some(t => !t.primary);
        for (const tool of toolEntries) {
            const parent = compact && !tool.primary ? menu : tools;
            if (tool.node.parentNode !== parent) parent.insertBefore(tool.node, parent === tools ? more : null);
            tool.label.textContent = tool.node.title;
        }
        if (!compact) closeMenu();
        updateOverflow();
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

    async function request(url, options = {}) {
        requestController = new AbortController();
        const controller = requestController;
        const timer = setTimeout(() => controller.abort(), 10000);
        try {
            const response = await fetch(url, { ...options, signal: controller.signal });
            const body = await response.json();
            if (!response.ok) throw new Error(serverErrorText(body) || response.statusText);
            return body;
        } finally {
            clearTimeout(timer);
            if (requestController === controller) requestController = null;
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
        if (started) { suppressClickUntil = performance.now() + 400; onInteraction(false); }
        if (!destroyed) paint();
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

    listen(list, 'pointerdown', (event) => {
        const node = event.target.closest('.taskbar-item');
        if (!canReorder || saving || drag || event.button !== 0 || !node || !confirmed.ordered_ids.includes(node._entry.appId)) return;
        if (event.pointerType !== 'mouse' && !event.target.closest('.toolbar-grip')) return;
        drag = { id: node._entry.appId, pointerId: event.pointerId, x: event.clientX, y: event.clientY,
            startX: event.clientX, startY: event.clientY, started: false, ids: visibleIds(), revision: confirmed.order_revision };
    });
    listen(document, 'pointermove', (event) => {
        if (!drag || drag.pointerId !== event.pointerId) return;
        drag.x = event.clientX; drag.y = event.clientY;
        if (!drag.started && Math.hypot(drag.x - drag.startX, drag.y - drag.startY) < 5) return;
        event.preventDefault();
        if (!drag.started) {
            drag.started = true;
            viewport.focus({ preventScroll: true });
            viewport.setPointerCapture(event.pointerId);
            ghost = make('div', 'toolbar-drag-ghost', document.body);
            ghost.textContent = entries.find(e => e.appId === drag.id)?.label || drag.id;
            onInteraction(true); paint(); autoScroll();
        }
        previewAt(drag.x);
    }, { passive: false });
    listen(document, 'pointerup', (event) => {
        if (!drag || event.pointerId !== drag.pointerId) return;
        const state = drag, rect = apps.getBoundingClientRect();
        const inside = event.clientX >= rect.left && event.clientX <= rect.right && event.clientY >= rect.top - 20 && event.clientY <= rect.bottom + 20;
        const changed = state.ids.join('\0') !== visibleIdsFromConfirmed().join('\0');
        cancelDrag();
        if (state.started && inside && changed) persist(state.ids, state.revision);
    });
    const visibleIdsFromConfirmed = () => [...new Set(orderedEntries(confirmed.ordered_ids).map(e => e.appId).filter(id => confirmed.ordered_ids.includes(id)))];
    listen(document, 'pointercancel', cancelDrag);
    // Touch starts with implicit capture on the handle. Transferring capture
    // to the viewport also emits a bubbling lostpointercapture on that handle.
    listen(viewport, 'lostpointercapture', (event) => {
        if (event.target === viewport && drag?.started) cancelDrag();
    });
    listen(window, 'blur', cancelDrag);
    listen(document, 'visibilitychange', () => { if (document.hidden) cancelDrag(); });
    listen(document, 'keydown', (event) => {
        if (event.key !== 'Escape') return;
        if (drag?.started) { event.preventDefault(); event.stopPropagation(); }
        cancelDrag(); closeMenu(); closeInfo();
    }, { capture: true });
    listen(list, 'dragstart', event => event.preventDefault());
    listen(viewport, 'wheel', (event) => {
        if (event.ctrlKey || Math.abs(event.deltaX) >= Math.abs(event.deltaY) || viewport.scrollWidth <= viewport.clientWidth) return;
        event.preventDefault(); event.stopPropagation();
        viewport.scrollLeft += event.deltaY * (event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? viewport.clientWidth : 1);
    }, { passive: false });
    listen(viewport, 'scroll', updateOverflow, { passive: true });
    listen(previous, 'click', () => viewport.scrollBy({ left: -Math.max(100, viewport.clientWidth * .7), behavior: 'smooth' }));
    listen(next, 'click', () => viewport.scrollBy({ left: Math.max(100, viewport.clientWidth * .7), behavior: 'smooth' }));
    listen(more, 'click', () => {
        menu.hidden = !menu.hidden; more.setAttribute('aria-expanded', String(!menu.hidden)); onInteraction(!menu.hidden);
    });
    // Tool handlers stop propagation; close before they open their own panels.
    listen(menu, 'click', (event) => { if (event.target.closest('.taskbar-pin')) closeMenu(); }, { capture: true });
    listen(document, 'pointerdown', (event) => {
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
        previous.title = tr('previous'); next.title = tr('next'); more.title = tr('more');
        [previous, next, more].forEach(n => n.setAttribute('aria-label', n.title));
        apps.setAttribute('aria-label', tr('apps'));
        layout(); paint();
    };
    const untranslate = onLangChange(translate);
    translate();
    return {
        status,
        addTool(node, primary = false) {
            const label = make('span', 'toolbar-tool-label', node);
            toolEntries.push({ node, label, primary });
            node.setAttribute('role', 'button'); node.tabIndex = 0;
            listen(node, 'keydown', event => {
                if (['Enter', ' '].includes(event.key) && !node.classList.contains('disabled')) {
                    event.preventDefault(); node.click();
                }
            });
            layout();
        },
        update(data, streams = [], streamActive = false) {
            acceptOrder(data);
            entries = (data.windows || []).map(w => ({
                key: `window:${w.id}`, appId: w.managed_app_id,
                label: w.title || w.display_name || w.app_id || `Window ${w.id}`,
                title: `${w.title || w.display_name || w.app_id || ''} (${w.app_id || ''})`,
                active: w.focused && !streamActive,
                activate: () => activateWindow(w.id)
            })).concat(streams);
            if (drag && (drag.revision !== confirmed.order_revision || drag.ids.length !== visibleIdsFromConfirmed().length || !drag.ids.every(id => entries.some(e => e.appId === id)))) cancelDrag();
            paint();
        },
        interacting: () => !!drag?.started || !menu.hidden || !info.hidden,
        destroy() {
            destroyed = true;
            cancelDrag(); requestController?.abort(); events.abort(); resize.disconnect();
            clearTimeout(noticeTimer); untranslate(); notifications.destroy();
            element.remove(); nodes.clear();
        }
    };
}
