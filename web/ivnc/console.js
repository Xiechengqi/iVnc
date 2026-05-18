import { getLang, setLang, onLangChange } from './lib/i18n.js?v=1';

const API = '/api/apps';
let editId = null;
let lastDataHash = '';
let currentLang = getLang();

const I18N = {
    en: {
        pageTitle: 'VNC App Console',
        langToggle: '中文',
        addApp: '+ Add App',
        thName: 'Name',
        thType: 'Type',
        thConfig: 'Config',
        thStatus: 'Status',
        thData: 'Data',
        thActions: 'Actions',
        loading: 'Loading...',
        empty: 'No apps yet. Click "Add App" to create one.',
        desktop: 'Desktop',
        background: 'Background',
        running: 'running',
        stopped: 'stopped',
        crashed: 'crashed',
        start: 'Start',
        stop: 'Stop',
        restart: 'Restart',
        visit: 'Visit',
        edit: 'Edit',
        log: 'Logs',
        clear: 'Clear',
        delete: 'Delete',
        editDisabled: 'Stop the app before editing',
        addModal: 'Add App',
        editModal: 'Edit App',
        addSave: 'Add',
        editSave: 'Save',
        name: 'Name',
        namePlaceholder: 'App display name',
        appType: 'App Type',
        autostart: 'Autostart',
        launchCommand: 'Launch Command',
        launchCwd: 'Working Directory',
        launchCwdDefault: '(Optional, default: current directory)',
        launchTimeout: 'Wait Timeout (seconds)',
        launchTimeoutDefault: '(Optional, default: 30)',
        accessUrl: 'Access Address',
        accessUrlDefault: '(Optional, default: none)',
        launchEnv: 'Environment Variables',
        exec: 'Launch Command',
        env: 'Environment Variables',
        envDefault: '(Optional, default: none)',
        launchCommandHelp: 'Command to run when the app starts',
        execHelp: 'Command to run when the app starts',
        nameInputPlaceholder: 'App display name',
        backgroundCommandPlaceholder: 'Enter launch command, e.g. python3 app.py --host 0.0.0.0 --port 7860',
        desktopCommandPlaceholder: 'Enter launch command, e.g. /usr/bin/code --no-sandbox',
        cwdPlaceholder: 'Enter working directory, e.g. /workspace/app',
        timeoutPlaceholder: 'Enter timeout, e.g. 30',
        accessUrlPlaceholder: 'Enter access address, e.g. http://127.0.0.1:7860',
        launchEnvPlaceholder: 'Enter environment variables, one per line, format: KEY=value\nKEY1=value1\nKEY2=value2',
        envPlaceholder: 'Enter environment variables, one per line, format: KEY=value',
        cancel: 'Cancel',
        close: 'Close',
        logsTitle: 'App Logs',
        logLoading: 'Loading...',
        savePending: 'Saving...',
        fetchFailed: 'Failed to load: ',
        missingName: 'Please enter an app name',
        missingExec: 'Please enter a launch command',
        missingLaunchCommand: 'Please enter a launch command',
        invalidUrl: 'Please enter a valid access address',
        updated: 'Updated',
        added: 'Added',
        actionFailed: 'Operation failed: ',
        actionFailedShort: 'Operation failed',
        started: 'Started',
        stoppedToast: 'Stopped',
        restarted: 'Restarted',
        confirmDelete: (name) => `Delete app "${name}"?`,
        deleted: 'Deleted',
        deleteFailed: 'Delete failed',
        confirmClear: (name) => `Clear cached data for "${name}"?\nThis removes cookies, local storage, and cache.`,
        cleared: 'Cleared',
        clearFailed: 'Clear failed',
        logsFor: (name) => `Logs: ${name}`,
        logLoadFailed: 'Load failed: ',
        backgroundOption: 'Background App',
        desktopOption: 'Desktop App',
        accessUrlCopied: 'Access link copied',
        accessUrlCopyFailed: 'Failed to copy access link',
    },
    zh: {
        pageTitle: 'VNC 应用控制台',
        langToggle: 'English',
        addApp: '+ 添加应用',
        thName: '名称',
        thType: '类型',
        thConfig: '配置',
        thStatus: '状态',
        thData: '数据',
        thActions: '操作',
        loading: '加载中...',
        empty: '暂无应用，点击“添加应用”创建。',
        desktop: '桌面',
        background: '后台',
        running: '运行中',
        stopped: '已停止',
        crashed: '已崩溃',
        start: '启动',
        stop: '停止',
        restart: '重启',
        visit: '访问',
        edit: '编辑',
        log: '日志',
        clear: '清理',
        delete: '删除',
        editDisabled: '请先停止应用再编辑',
        addModal: '添加应用',
        editModal: '编辑应用',
        addSave: '添加',
        editSave: '保存',
        name: '名称',
        namePlaceholder: '应用显示名称',
        appType: '应用类型',
        autostart: '开机启动',
        launchCommand: '启动命令',
        launchCwd: '工作目录',
        launchCwdDefault: '（可选，默认值：当前目录）',
        launchTimeout: '等待超时（秒）',
        launchTimeoutDefault: '（可选，默认值：30）',
        accessUrl: '访问地址',
        accessUrlDefault: '（可选，默认值：无）',
        launchEnv: '环境变量',
        exec: '启动命令',
        env: '环境变量',
        envDefault: '（可选，默认值：无）',
        launchCommandHelp: '应用启动时要执行的命令',
        execHelp: '应用启动时要执行的命令',
        nameInputPlaceholder: '应用显示名称',
        backgroundCommandPlaceholder: '请输入启动命令，例如： python3 app.py --host 0.0.0.0 --port 7860',
        desktopCommandPlaceholder: '请输入启动命令，例如： /usr/bin/code --no-sandbox',
        cwdPlaceholder: '请输入工作目录，例如： /workspace/app',
        timeoutPlaceholder: '请输入超时时间，例如： 30',
        accessUrlPlaceholder: '请输入访问地址，例如： http://127.0.0.1:7860',
        launchEnvPlaceholder: '请输入环境变量，每行一个，格式：KEY=value\nKEY1=value1\nKEY2=value2',
        envPlaceholder: '请输入环境变量，每行一个，格式：KEY=value',
        cancel: '取消',
        close: '关闭',
        logsTitle: '应用日志',
        logLoading: '加载中...',
        savePending: '保存中...',
        fetchFailed: '获取失败: ',
        missingName: '请输入应用名称',
        missingExec: '请输入启动命令',
        missingLaunchCommand: '请输入启动命令',
        invalidUrl: '请输入有效的访问地址',
        updated: '已更新',
        added: '已添加',
        actionFailed: '操作失败: ',
        actionFailedShort: '操作失败',
        started: '已启动',
        stoppedToast: '已停止',
        restarted: '已重启',
        confirmDelete: (name) => `确认删除应用 "${name}"？`,
        deleted: '已删除',
        deleteFailed: '删除失败',
        confirmClear: (name) => `确认清理 "${name}" 的缓存数据？\n将清除所有Cookie、LocalStorage和缓存。`,
        cleared: '已清理',
        clearFailed: '清理失败',
        logsFor: (name) => `日志: ${name}`,
        logLoadFailed: '加载失败: ',
        backgroundOption: '后台应用',
        desktopOption: '桌面应用',
        accessUrlCopied: '访问链接已复制',
        accessUrlCopyFailed: '复制访问链接失败',
    }
};

function t(key) {
    return I18N[currentLang][key];
}

function esc(s) {
    return String(s)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');
}

function getHash(data) {
    return JSON.stringify(data.apps.map(a => ({
        id: a.id,
        status: a.status,
        size: a.data_size_human,
        url: a.url,
        exec: a.exec_command,
        launch: a.launch_command
    })));
}

function applyTranslations() {
    document.documentElement.lang = currentLang === 'en' ? 'en' : 'zh-CN';
    document.title = t('pageTitle');
    document.getElementById('page-title').textContent = t('pageTitle');
    document.getElementById('lang-toggle').textContent = t('langToggle');
    document.getElementById('add-app-btn').textContent = t('addApp');
    document.getElementById('th-name').textContent = t('thName');
    document.getElementById('th-type').textContent = t('thType');
    document.getElementById('th-config').textContent = t('thConfig');
    document.getElementById('th-status').textContent = t('thStatus');
    document.getElementById('th-data').textContent = t('thData');
    document.getElementById('th-actions').textContent = t('thActions');
    document.getElementById('label-name').textContent = t('name');
    document.getElementById('f-name').placeholder = t('namePlaceholder');
    document.getElementById('label-app-type').textContent = t('appType');
    document.querySelector('#f-app-type option[value="background"]').textContent = t('backgroundOption');
    document.querySelector('#f-app-type option[value="desktop"]').textContent = t('desktopOption');
    document.getElementById('label-autostart').textContent = t('autostart');
    document.getElementById('label-launch-command').textContent = t('launchCommand');
    document.getElementById('label-launch-command-help').textContent = t('launchCommandHelp');
    document.getElementById('label-launch-cwd').textContent = t('launchCwd');
    document.getElementById('label-launch-cwd-default').textContent = t('launchCwdDefault');
    document.getElementById('label-launch-timeout').textContent = t('launchTimeout');
    document.getElementById('label-launch-timeout-default').textContent = t('launchTimeoutDefault');
    document.getElementById('label-access-url').textContent = t('accessUrl');
    document.getElementById('label-access-url-default').textContent = t('accessUrlDefault');
    document.getElementById('label-launch-env').textContent = t('launchEnv');
    document.getElementById('label-launch-env-default').textContent = t('envDefault');
    document.getElementById('label-exec').textContent = t('exec');
    document.getElementById('label-exec-help').textContent = t('execHelp');
    document.getElementById('label-env').textContent = t('env');
    document.getElementById('label-env-default').textContent = t('envDefault');
    document.getElementById('modal-cancel').textContent = t('cancel');
    document.getElementById('log-close-btn').textContent = t('close');
    document.getElementById('modal-close').setAttribute('aria-label', t('close'));
    document.getElementById('f-name').placeholder = t('nameInputPlaceholder');
    document.getElementById('f-launch-command').placeholder = t('backgroundCommandPlaceholder');
    document.getElementById('f-exec').placeholder = t('desktopCommandPlaceholder');
    document.getElementById('f-launch-cwd').placeholder = t('cwdPlaceholder');
    document.getElementById('f-launch-timeout').placeholder = t('timeoutPlaceholder');
    document.getElementById('f-url').placeholder = t('accessUrlPlaceholder');
    document.getElementById('f-launch-env').placeholder = t('launchEnvPlaceholder');
    document.getElementById('f-env').placeholder = t('envPlaceholder');

    if (!editId) {
        document.getElementById('modal-title').textContent = t('addModal');
        document.getElementById('modal-save').textContent = t('addSave');
    }

    const logTitle = document.getElementById('log-title');
    if (!logTitle.dataset.appName) {
        logTitle.textContent = t('logsTitle');
    }
}

async function load() {
    try {
        const r = await fetch(API);
        const d = await r.json();

        const currentHash = getHash(d);
        if (currentHash === lastDataHash) return;
        lastDataHash = currentHash;

        const tb = document.getElementById('app-list');
        if (!d.apps || !d.apps.length) {
            tb.innerHTML = `<tr><td colspan="6" class="empty">${esc(t('empty'))}</td></tr>`;
            return;
        }

        const fragment = document.createDocumentFragment();
        d.apps.forEach(a => {
            const tr = document.createElement('tr');
            const type = a.app_type === 'desktop' ? t('desktop') : t('background');
            const configStr = a.app_type === 'desktop'
                ? (a.exec_command || '')
                : [a.url ? `${t('visit')}: ${a.url}` : '', a.launch_command ? `cmd: ${a.launch_command}` : ''].filter(Boolean).join('\n');
            const configShort = configStr.length > 30 ? configStr.slice(0, 30) + '...' : configStr;
            const statusText = t(a.status) || a.status;

            tr.innerHTML = `
                <td><strong>${esc(a.name)}</strong></td>
                <td><span class="badge">${type}</span></td>
                <td title="${esc(configStr)}">${esc(configShort)}</td>
                <td>
                    <div class="status-wrapper">
                        <span class="status status-${a.status}"></span>
                        <span>${statusText}</span>
                    </div>
                </td>
                <td class="data-size">${a.data_size_human}</td>
                <td class="actions"></td>
            `;

            const actionsCell = tr.querySelector('.actions');

            if (a.status === 'running') {
                const stopBtn = createBtn(t('stop'), 'btn-stop btn-sm', () => act(a.id, 'stop'));
                const restartBtn = createBtn(t('restart'), 'btn-restart btn-sm', () => act(a.id, 'restart'));
                actionsCell.append(stopBtn, restartBtn);
            } else {
                const startBtn = createBtn(t('start'), 'btn-start btn-sm', () => act(a.id, 'start'));
                actionsCell.append(startBtn);
            }

            const editBtn = createBtn(t('edit'), 'btn-edit btn-sm', () => showEdit(a.id));
            if (a.status === 'running') {
                editBtn.disabled = true;
                editBtn.title = t('editDisabled');
            }

            const logBtn = createBtn(t('log'), 'btn-log btn-sm', () => showLogs(a.id, a.name));
            const clearBtn = createBtn(t('clear'), 'btn-clear btn-sm', () => clearData(a.id, a.name));
            const delBtn = createBtn(t('delete'), 'btn-delete btn-sm', () => del(a.id, a.name));

            actionsCell.append(editBtn);
            if (a.app_type !== 'desktop' && a.url) {
                actionsCell.append(createBtn(t('visit'), 'btn-visit btn-sm', () => copyAccessUrl(a.url)));
            }
            actionsCell.append(logBtn, clearBtn, delBtn);
            fragment.appendChild(tr);
        });

        tb.innerHTML = '';
        tb.appendChild(fragment);
    } catch (e) {
        console.error('Load failed:', e);
    }
}

function createBtn(text, cls, onClick) {
    const b = document.createElement('button');
    b.className = 'btn ' + cls;
    b.textContent = text;
    b.addEventListener('click', onClick);
    return b;
}

async function copyAccessUrl(url) {
    try {
        await copyText(url);
        toast(t('accessUrlCopied'), 'ok');
    } catch (e) {
        console.warn('Failed to copy access URL:', e);
        toast(t('accessUrlCopyFailed'), 'err');
    }
}

async function copyText(text) {
    if (navigator.clipboard?.writeText && window.isSecureContext) {
        await navigator.clipboard.writeText(text);
        return;
    }

    const input = document.createElement('textarea');
    input.value = text;
    input.setAttribute('readonly', '');
    input.style.position = 'fixed';
    input.style.top = '-9999px';
    input.style.left = '-9999px';
    document.body.appendChild(input);
    input.focus();
    input.select();

    try {
        if (!document.execCommand('copy')) {
            throw new Error('execCommand copy returned false');
        }
    } finally {
        input.remove();
    }
}

function showAdd() {
    editId = null;
    const modal = document.getElementById('modal');
    modal.querySelector('#modal-title').textContent = t('addModal');
    modal.querySelector('#modal-save').textContent = t('addSave');

    document.getElementById('f-name').value = '';
    document.getElementById('f-name').disabled = false;
    document.getElementById('f-app-type').value = 'background';
    document.getElementById('f-app-type').disabled = false;
    document.getElementById('f-url').value = '';
    document.getElementById('f-autostart').checked = false;
    document.getElementById('f-launch-command').value = '';
    document.getElementById('f-launch-cwd').value = '';
    document.getElementById('f-launch-timeout').value = '';
    document.getElementById('f-launch-env').value = '';
    document.getElementById('f-exec').value = '';
    document.getElementById('f-env').value = '';

    updateAppTypeVisibility();
    modal.classList.add('show');
}

async function showEdit(id) {
    try {
        const r = await fetch(API + '/' + id);
        const a = await r.json();
        editId = id;
        const modal = document.getElementById('modal');
        modal.querySelector('#modal-title').textContent = t('editModal');
        modal.querySelector('#modal-save').textContent = t('editSave');

        document.getElementById('f-name').value = a.name;
        document.getElementById('f-name').disabled = true;
        document.getElementById('f-app-type').value = a.app_type || 'background';
        document.getElementById('f-app-type').disabled = true;
        document.getElementById('f-autostart').checked = !!a.autostart;

        if (a.app_type === 'desktop') {
            document.getElementById('f-exec').value = a.exec_command || '';
            document.getElementById('f-env').value = envToText(a.env_vars);
        } else {
            document.getElementById('f-url').value = a.url || '';
            document.getElementById('f-launch-command').value = a.launch_command || '';
            document.getElementById('f-launch-cwd').value = a.launch_cwd || '';
            document.getElementById('f-launch-timeout').value = a.launch_wait_timeout_secs || '';
            document.getElementById('f-launch-env').value = envToText(a.launch_env_vars);
        }
        updateAppTypeVisibility();
        modal.classList.add('show');
    } catch (e) {
        toast(t('fetchFailed') + e, 'err');
    }
}

function hideModal() {
    editId = null;
    document.getElementById('modal').classList.remove('show');
}

function validateUrl(url) {
    if (!url) return false;
    try {
        new URL(url);
        return true;
    } catch {
        return false;
    }
}

function envToText(envVars) {
    return envVars ? Object.entries(envVars).map(([k, v]) => `${k}=${v}`).join('\n') : '';
}

function parseEnvText(text) {
    const trimmed = text.trim();
    if (!trimmed) return null;
    const envVars = {};
    trimmed.split('\n').forEach(line => {
        const idx = line.indexOf('=');
        if (idx > 0) {
            const key = line.substring(0, idx).trim();
            const value = line.substring(idx + 1).trim();
            if (key && value) envVars[key] = value;
        }
    });
    return Object.keys(envVars).length ? envVars : null;
}

async function saveApp() {
    const btn = document.getElementById('modal-save');
    const appType = document.getElementById('f-app-type').value;
    const body = {
        app_type: appType,
        autostart: document.getElementById('f-autostart').checked
    };

    if (!editId) {
        body.name = document.getElementById('f-name').value.trim();
        if (!body.name) return toast(t('missingName'), 'err');
    }

    if (appType === 'desktop') {
        body.exec_command = document.getElementById('f-exec').value.trim();
        if (!body.exec_command) return toast(t('missingExec'), 'err');

        body.env_vars = parseEnvText(document.getElementById('f-env').value);
    } else {
        body.launch_command = document.getElementById('f-launch-command').value.trim() || null;
        if (!body.launch_command) return toast(t('missingLaunchCommand'), 'err');
        body.launch_cwd = document.getElementById('f-launch-cwd').value.trim() || null;
        const launchTimeout = document.getElementById('f-launch-timeout').value;
        body.launch_wait_timeout_secs = launchTimeout ? parseInt(launchTimeout, 10) : null;
        body.url = document.getElementById('f-url').value.trim() || null;
        if (body.url && !validateUrl(body.url)) return toast(t('invalidUrl'), 'err');
        body.launch_env_vars = parseEnvText(document.getElementById('f-launch-env').value);
    }

    btn.disabled = true;
    const originalText = btn.textContent;
    btn.textContent = t('savePending');

    try {
        const r = await fetch(editId ? API + '/' + editId : API, {
            method: editId ? 'PUT' : 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body)
        });
        const d = await r.json();
        if (d.error) {
            toast(d.error, 'err');
            return;
        }
        hideModal();
        toast(editId ? t('updated') : t('added'), 'ok');
        load();
    } catch (e) {
        toast(t('actionFailed') + e, 'err');
    } finally {
        btn.disabled = false;
        btn.textContent = originalText;
    }
}

async function act(id, action) {
    try {
        const r = await fetch(`${API}/${id}/${action}`, { method: 'POST' });
        const d = await r.json();
        if (d.error) {
            toast(d.error, 'err');
            return;
        }
        toast(action === 'start' ? t('started') : action === 'stop' ? t('stoppedToast') : t('restarted'), 'ok');
        load();
    } catch (e) {
        toast(t('actionFailedShort'), 'err');
    }
}

async function del(id, name) {
    if (!confirm(t('confirmDelete')(name))) return;
    try {
        const r = await fetch(API + '/' + id, { method: 'DELETE' });
        const d = await r.json();
        if (d.error) {
            toast(d.error, 'err');
            return;
        }
        toast(t('deleted'), 'ok');
        load();
    } catch (e) {
        toast(t('deleteFailed'), 'err');
    }
}

async function clearData(id, name) {
    if (!confirm(t('confirmClear')(name))) return;
    try {
        const r = await fetch(`${API}/${id}/clear-data`, { method: 'POST' });
        const d = await r.json();
        if (d.error) {
            toast(d.error, 'err');
            return;
        }
        toast(t('cleared'), 'ok');
        load();
    } catch (e) {
        toast(t('clearFailed'), 'err');
    }
}

function toast(msg, type) {
    const tEl = document.getElementById('toast');
    tEl.textContent = msg;
    tEl.className = 'toast show toast-' + type;
    setTimeout(() => tEl.classList.remove('show'), 2500);
}

async function showLogs(id, name) {
    const modal = document.getElementById('log-modal');
    const title = document.getElementById('log-title');
    title.dataset.appName = name;
    title.textContent = t('logsFor')(name);
    const content = document.getElementById('log-content');
    content.textContent = t('logLoading');
    modal.classList.add('show');
    try {
        const r = await fetch(`${API}/${id}/logs`);
        const d = await r.json();
        content.textContent = d.logs || '(empty)';
        content.scrollTop = content.scrollHeight;
    } catch (e) {
        content.textContent = t('logLoadFailed') + e;
    }
}

function hideLogModal() {
    const title = document.getElementById('log-title');
    delete title.dataset.appName;
    title.textContent = t('logsTitle');
    document.getElementById('log-modal').classList.remove('show');
}

function updateAppTypeVisibility() {
    const type = document.getElementById('f-app-type').value;
    const autostartControl = document.getElementById('autostart-control');
    const generalSlot = document.getElementById('general-autostart-slot');
    const backgroundSlot = document.getElementById('background-autostart-slot');
    const desktopSlot = document.getElementById('desktop-autostart-slot');

    document.getElementById('background-config').style.display = type === 'desktop' ? 'none' : 'block';
    document.getElementById('desktop-config').style.display = type === 'desktop' ? 'block' : 'none';

    autostartControl.classList.remove('inline-slot');
    if (type === 'desktop') {
        desktopSlot.appendChild(autostartControl);
        autostartControl.classList.add('inline-slot');
    } else {
        backgroundSlot.appendChild(autostartControl);
        autostartControl.classList.add('inline-slot');
    }

    if (!autostartControl.parentElement) {
        generalSlot.appendChild(autostartControl);
    }
}

document.addEventListener('DOMContentLoaded', () => {
    document.getElementById('add-app-btn').addEventListener('click', showAdd);
    document.getElementById('lang-toggle').addEventListener('click', () => {
        setLang(currentLang === 'en' ? 'zh' : 'en');
    });
    onLangChange((lang) => {
        currentLang = lang;
        lastDataHash = '';
        applyTranslations();
        load();
    });
    document.getElementById('f-app-type').addEventListener('change', updateAppTypeVisibility);
    document.getElementById('modal-close').addEventListener('click', hideModal);

    document.querySelectorAll('.btn-cancel').forEach(btn => {
        btn.addEventListener('click', (e) => {
            if (e.target.closest('#log-modal')) hideLogModal();
            else hideModal();
        });
    });

    document.getElementById('modal-save').addEventListener('click', saveApp);

    applyTranslations();
    load();
    setInterval(load, 5000);
});
