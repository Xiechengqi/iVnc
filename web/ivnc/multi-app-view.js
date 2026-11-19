import styles from './multi-app-view.css?inline';
import { getLang, onLangChange } from './lib/i18n.js?v=1';
import { minimizeOtherOverlays, releaseOverlayInput } from './overlay-manager.js';

const STORAGE_KEY = 'ivnc.multiAppView';
const MAX_APPS = 4;
const TEXT = {
    zh: {
        title: '多应用视图', edit: '编辑组合', exit: '退出合看', save: '打开合看', single: '打开应用',
        cancel: '取消', add: '添加位置', choose: '选择应用', slot: n => `应用 ${n}`,
        up: '上移', down: '下移', remove: '移除',
        hint: '选择 2–4 个应用。普通桌面应用需在应用设置中开启独立运行并重启。',
        unavailable: '应用未运行或不可用', unsupported: '来源实例仅支持单窗口查看',
        open: name => `打开 ${name}`,
    },
    en: {
        title: 'Multi-app view', edit: 'Edit group', exit: 'Exit multi-app view', save: 'Open group', single: 'Open application',
        cancel: 'Cancel', add: 'Add slot', choose: 'Select an application', slot: n => `Application ${n}`,
        up: 'Move up', down: 'Move down', remove: 'Remove',
        hint: 'Select 2–4 applications. Enable Run independently in a desktop app’s settings and restart it to add its view.',
        unavailable: 'Application stopped or unavailable', unsupported: 'The source supports single-app viewing',
        open: name => `Open ${name}`,
    },
};

export const multiAppText = (key, ...args) => {
    const value = TEXT[getLang()][key];
    return typeof value === 'function' ? value(...args) : value;
};

function loadMembers() {
    try {
        const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) || 'null');
        if (!Array.isArray(stored) || stored.length < 2 || stored.length > MAX_APPS) return [];
        const ids = new Set();
        for (const member of stored) {
            if (typeof member?.id !== 'string' || !/^[a-zA-Z0-9_-]{1,128}$/.test(member.id)
                || typeof member.name !== 'string' || member.name.length > 256 || ids.has(member.id)) return [];
            ids.add(member.id);
        }
        return stored;
    } catch (_) {
        return [];
    }
}

function element(tag, className, parent) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    parent?.append(node);
    return node;
}

export function createMultiAppView({ layer, onShow, onExit, onSingle, beforeEdit }) {
    if (!document.getElementById('ivnc-multi-app-styles')) {
        const style = element('style', '', document.head);
        style.id = 'ivnc-multi-app-styles';
        style.textContent = styles;
    }
    const button = element('button', 'toolbar-multi-app');
    button.type = 'button';
    element('span', 'multi-app-icon', button).setAttribute('aria-hidden', 'true');
    const heading = element('div', 'multi-app-heading', layer);
    heading.hidden = true;
    const headingText = element('span', '', heading);
    const edit = element('button', '', heading);
    const exit = element('button', '', heading);
    edit.type = exit.type = 'button';

    const dialog = element('dialog', 'multi-app-dialog allow-native-input', document.body);
    dialog.setAttribute('aria-labelledby', 'ivnc-multi-app-title');
    const form = element('form', '', dialog);
    const title = element('h2', '', form);
    title.id = 'ivnc-multi-app-title';
    const hint = element('p', '', form);
    const rows = element('div', 'multi-app-rows', form);
    const add = element('button', '', form);
    add.type = 'button';
    const actions = element('div', 'multi-app-actions', form);
    const cancel = element('button', '', actions);
    cancel.type = 'button';
    const save = element('button', 'primary', actions);
    save.type = 'submit';

    let members = loadMembers();
    let catalog = new Map();
    let catalogKey = '';
    let draft = [];
    let showing = false;
    const remember = () => {
        try { localStorage.setItem(STORAGE_KEY, JSON.stringify(members)); } catch (_) {}
    };
    const selected = () => draft.filter(Boolean);
    const minimum = () => members.length ? 1 : 2;
    const valid = () => selected().length >= minimum();
    const label = id => catalog.get(id)?.name || members.find(member => member.id === id)?.name || id;

    function focusSlot(index) {
        rows.querySelectorAll('select')[Math.min(index, draft.length - 1)]?.focus();
    }

    function refreshOptions() {
        const selects = rows.querySelectorAll('select');
        selects.forEach((select, index) => {
            const id = draft[index] || '';
            select.replaceChildren(new Option(multiAppText('choose'), ''));
            if (id && !catalog.has(id)) select.add(new Option(label(id), id));
            for (const candidate of catalog.values()) {
                if (candidate.id !== id && (!candidate.available || draft.includes(candidate.id))) continue;
                select.add(new Option(candidate.name, candidate.id));
            }
            select.value = id;
        });
        add.disabled = draft.length >= MAX_APPS;
        save.disabled = !valid();
        save.textContent = multiAppText(selected().length === 1 ? 'single' : 'save');
    }

    function renderRows() {
        rows.replaceChildren();
        draft.forEach((_, index) => {
            const row = element('div', 'multi-app-row', rows);
            const select = element('select', '', row);
            select.setAttribute('aria-label', multiAppText('slot', index + 1));
            select.onchange = () => {
                draft[index] = select.value;
                refreshOptions();
            };
            const move = direction => {
                const target = index + direction;
                [draft[index], draft[target]] = [draft[target], draft[index]];
                renderRows();
                focusSlot(target);
            };
            const rowButton = (symbol, key, disabled, action) => {
                const control = element('button', '', row);
                control.type = 'button'; control.textContent = symbol;
                control.title = multiAppText(key);
                control.setAttribute('aria-label', control.title);
                control.disabled = disabled;
                control.onclick = action;
            };
            rowButton('↑', 'up', index === 0, () => move(-1));
            rowButton('↓', 'down', index === draft.length - 1, () => move(1));
            rowButton('×', 'remove', draft.length === 1, () => {
                draft.splice(index, 1);
                renderRows();
                focusSlot(index);
            });
        });
        refreshOptions();
    }

    function openEditor() {
        if (dialog.open || document.documentElement.hasAttribute('data-ivnc-modal-blocked')) return;
        releaseOverlayInput();
        beforeEdit();
        minimizeOtherOverlays();
        draft = members.length ? members.map(member => member.id) : ['', ''];
        renderRows();
        dialog.showModal();
        focusSlot(0);
    }

    button.onclick = () => {
        if (showing) onExit();
        else if (members.length) onShow(members);
        else openEditor();
    };
    edit.onclick = openEditor;
    exit.onclick = onExit;
    cancel.onclick = () => dialog.close();
    add.onclick = () => {
        if (draft.length >= MAX_APPS) return;
        draft.push('');
        renderRows();
        focusSlot(draft.length - 1);
    };
    form.onsubmit = event => {
        event.preventDefault();
        if (!valid()) return;
        const next = selected().map(id => ({ id, name: label(id) }));
        members = next.length >= 2 ? next : [];
        remember();
        dialog.close();
        if (members.length) onShow(members);
        else onSingle(next[0].id);
    };

    const translate = () => {
        title.textContent = headingText.textContent = button.title = multiAppText('title');
        button.setAttribute('aria-label', button.title);
        hint.textContent = multiAppText('hint');
        edit.textContent = multiAppText('edit');
        exit.textContent = multiAppText('exit');
        add.textContent = multiAppText('add');
        cancel.textContent = multiAppText('cancel');
        if (dialog.open) renderRows();
    };
    const untranslate = onLangChange(translate);
    translate();
    button.setAttribute('aria-pressed', 'false');

    return {
        button,
        isEditing: () => dialog.open,
        setShowing(value) {
            showing = value;
            heading.hidden = !value;
            button.setAttribute('aria-pressed', String(value));
        },
        update(apps) {
            const key = JSON.stringify(apps);
            if (key === catalogKey) return;
            catalogKey = key;
            catalog = new Map(apps.map(app => [app.id, app]));
            let renamed = false;
            for (const member of members) {
                const name = catalog.get(member.id)?.name;
                if (name && name !== member.name) { member.name = name; renamed = true; }
            }
            if (renamed) remember();
            if (dialog.open) refreshOptions();
        },
        destroy() {
            untranslate();
            if (dialog.open) dialog.close();
            dialog.remove();
            button.remove();
        },
    };
}
