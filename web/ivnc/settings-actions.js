import { getLang, setLang, onLangChange } from './lib/i18n.js?v=1';
import { bindLogoutButton, onAuthChange, credentialsChanged } from './auth-client.js';
import styles from './settings-actions.css?inline';

const STRINGS = {
    zh: {
        settings: '设置', password: '修改密码', update: '重启或升级', shutdown: '关机', language: '切换为英文',
        newPassword: '新密码', confirmPassword: '确认新密码', passwordHint: '至少 4 个字符',
        passwordShort: '密码至少需要 4 个字符', passwordLong: '密码不能超过 4096 个 UTF-8 字节',
        mismatch: '两次输入的密码不一致', passwordFailed: '修改密码失败',
        passwordChanged: '密码已修改，所有连接需要重新登录。',
        passwordNotice: '修改后所有连接需要重新登录。此次修改在服务重启后恢复为配置文件中的密码。',
        cancel: '取消', confirm: '确定', close: '关闭', pending: '处理中…',
        updateNotice: '将重启 iVnc 服务，当前连接会断开，恢复后需要重新登录。',
        upgrade: '升级到最新版本', upgradeHint: '勾选后会先更新 iVnc 程序，再重启服务。',
        useGithubProxy: '使用 GitHub 中国代理', githubProxy: 'GitHub 代理', customProxy: '自定义代理',
        proxyAddress: '代理地址', proxyHint: '优先使用所选代理下载，失败时自动尝试其他来源。',
        proxyAddressHint: '填写代理地址前缀，如 https://example.com/gh/；省略协议时默认使用 HTTPS。',
        proxyInvalid: '请输入有效的 HTTP(S) 代理地址前缀，不含账号、密码、查询参数或片段。',
        restart: '重启服务', upgradeRestart: '升级并重启', preparing: '正在处理…',
        restartSent: '重启请求已发送，等待服务恢复…', upgradeDone: '升级完成，等待服务恢复…',
        recovered: '服务已恢复，即将刷新页面…', notRecovered: '服务尚未恢复，请稍后刷新页面。',
        networkError: '请求失败，请检查网络后重试。', upgradeFailed: '升级连接中断，无法确认升级结果，请检查服务状态。',
        failed: '操作失败', minimalHint: '账户与服务操作',
        shutdownNotice: '确认关闭 iVnc？系统将保存应用运行状态、关闭当前连接并退出 iVnc。再次使用时需要手动启动服务。',
        shutdownConfirm: '确认关机', shutdownPending: '正在发送关机请求…',
        shutdownAccepted: '关机请求已接受，iVnc 正在退出。',
        shutdownUncertain: '连接中断，无法确认关机结果，请检查服务状态。',
    },
    en: {
        settings: 'Settings', password: 'Change password', update: 'Restart or upgrade', shutdown: 'Shut down', language: 'Switch to Chinese',
        newPassword: 'New password', confirmPassword: 'Confirm new password', passwordHint: 'At least 4 characters',
        passwordShort: 'Use at least 4 characters', passwordLong: 'Password cannot exceed 4096 UTF-8 bytes',
        mismatch: 'Passwords do not match', passwordFailed: 'Could not change password',
        passwordChanged: 'Password changed. All connections must sign in again.',
        passwordNotice: 'All connections must sign in again. Restarting the service restores the password from its configuration file.',
        cancel: 'Cancel', confirm: 'Confirm', close: 'Close', pending: 'Working…',
        updateNotice: 'The iVnc service will restart. Connections will close and require a new sign-in after recovery.',
        upgrade: 'Upgrade to latest', upgradeHint: 'Update the iVnc program before restarting the service.',
        useGithubProxy: 'Use a GitHub proxy for China', githubProxy: 'GitHub proxy', customProxy: 'Custom proxy',
        proxyAddress: 'Proxy address', proxyHint: 'Download through the selected proxy first, then try other sources if it fails.',
        proxyAddressHint: 'Enter a proxy URL prefix, such as https://example.com/gh/. HTTPS is used when the scheme is omitted.',
        proxyInvalid: 'Enter a valid HTTP(S) proxy URL prefix without credentials, query parameters or fragments.',
        restart: 'Restart service', upgradeRestart: 'Upgrade and restart', preparing: 'Working…',
        restartSent: 'Restart requested. Waiting for the service…', upgradeDone: 'Upgrade complete. Waiting for the service…',
        recovered: 'Service recovered. Reloading…', notRecovered: 'The service has not recovered. Refresh the page later.',
        networkError: 'Request failed. Check your network and retry.', upgradeFailed: 'Upgrade connection closed unexpectedly. Check the service status before retrying.',
        failed: 'Operation failed', minimalHint: 'Account and service actions',
        shutdownNotice: 'Shut down iVnc? Running application state will be saved, connections will close, and iVnc will exit. Start the service manually to use it again.',
        shutdownConfirm: 'Confirm shutdown', shutdownPending: 'Sending shutdown request…',
        shutdownAccepted: 'Shutdown accepted. iVnc is exiting.',
        shutdownUncertain: 'Connection interrupted. The shutdown result could not be confirmed. Check the service status.',
    },
};
export const settingsText = key => STRINGS[getLang()][key];
const icon = content => `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${content}</svg>`;

// Shared by the full console and the account/service-only settings page. No
// privileged request, including the upgrade WebSocket, starts when mounting.
export function createSettingsActions(element, dialogs) {
    if (!document.getElementById('ivnc-settings-action-styles')) {
        const style = document.createElement('style');
        style.id = 'ivnc-settings-action-styles'; style.textContent = styles;
        document.head.appendChild(style);
    }
    element.classList.add('settings-actions');
    element.innerHTML = `
        <button id="settings-password" class="settings-action" type="button" hidden>${icon('<rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>')}<span></span></button>
        <button id="settings-update" class="settings-action" type="button">${icon('<path d="M21 3v6h-6M3 21v-6h6M3 10a9 9 0 0 1 15-5l3 4M21 14A9 9 0 0 1 6 19l-3-4"/>')}<span></span></button>
        <button id="settings-shutdown" class="settings-action settings-shutdown" type="button">${icon('<path d="M12 2v10M6.3 5.6a9 9 0 1 0 11.4 0"/>')}<span></span></button>
        <button id="lang-toggle" class="settings-action settings-language" type="button"></button>
        <button id="signout-btn" class="settings-action settings-signout" type="button" hidden>${icon('<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/>')}</button>`;
    const password = element.querySelector('#settings-password');
    const update = element.querySelector('#settings-update');
    const shutdown = element.querySelector('#settings-shutdown');
    const language = element.querySelector('#lang-toggle');
    bindLogoutButton(element.querySelector('#signout-btn'));
    const translate = () => {
        for (const [button, key] of [[password, 'password'], [update, 'update'], [shutdown, 'shutdown']]) {
            button.title = settingsText(key); button.setAttribute('aria-label', settingsText(key));
            button.querySelector('span').textContent = settingsText(key);
        }
        language.textContent = getLang() === 'zh' ? 'English' : '中文';
        language.title = settingsText('language'); language.setAttribute('aria-label', settingsText('language'));
    };
    translate();
    const untranslate = onLangChange(translate);
    const unauth = onAuthChange(value => { password.hidden = !value.enabled; });
    language.addEventListener('click', () => setLang(getLang() === 'zh' ? 'en' : 'zh'));
    password.addEventListener('click', () => openPassword(dialogs));
    update.addEventListener('click', () => openUpdate(dialogs));
    shutdown.addEventListener('click', () => openShutdown(dialogs));
    window.addEventListener('pagehide', event => { if (!event.persisted) { untranslate(); unauth(); } });
}

function openDialog(dialogs, type, contents, onClose = () => {}) {
    if (dialogs.current?.locked) return null;
    const overlay = document.createElement('div');
    overlay.className = `modal-bg settings-dialog-bg ${type}-overlay allow-native-input`;
    overlay.innerHTML = `<form class="modal settings-dialog ${type}-dialog">
        <h2 id="settings-dialog-title">${settingsText({ pwd: 'password', update: 'update', shutdown: 'shutdown' }[type])}</h2>${contents}</form>`;
    document.body.appendChild(overlay);
    const viewport = window.visualViewport;
    const layout = () => {
        if (!viewport || Math.abs(viewport.scale - 1) > .01) return;
        overlay.style.top = `${viewport.offsetTop}px`;
        overlay.style.height = `${viewport.height}px`;
        overlay.style.setProperty('--settings-dialog-height', `${viewport.height}px`);
    };
    layout();
    viewport?.addEventListener('resize', layout);
    viewport?.addEventListener('scroll', layout);
    const cleanup = () => {
        viewport?.removeEventListener('resize', layout); viewport?.removeEventListener('scroll', layout);
        onClose(); overlay.remove();
    };
    const session = dialogs.open(overlay, cleanup);
    if (!session) { cleanup(); return null; }
    return { overlay, session, form: overlay.querySelector('form'), find: selector => overlay.querySelector(selector) };
}

async function request(url, options, session, timeout = 15000, readResponse = response => response) {
    const controller = new AbortController();
    const abort = () => controller.abort();
    session.abort.signal.addEventListener('abort', abort, { once: true });
    const timer = setTimeout(abort, timeout);
    try { return await readResponse(await fetch(url, { ...options, cache: 'no-store', signal: controller.signal })); }
    finally { clearTimeout(timer); session.abort.signal.removeEventListener('abort', abort); }
}

function openShutdown(dialogs) {
    const view = openDialog(dialogs, 'shutdown', `
        <p class="settings-hint">${settingsText('shutdownNotice')}</p>
        <p id="shutdown-msg" class="settings-message" role="status" aria-live="polite"></p>
        <div class="settings-dialog-buttons"><button id="shutdown-cancel" type="button" autofocus>${settingsText('cancel')}</button><button id="shutdown-ok" class="settings-danger" type="submit">${settingsText('shutdownConfirm')}</button></div>`);
    if (!view) return;
    const { session, form, find } = view;
    const message = find('#shutdown-msg'), ok = find('#shutdown-ok'), cancel = find('#shutdown-cancel');
    let accepted = false;
    cancel.addEventListener('click', () => dialogs.close());
    form.addEventListener('submit', async event => {
        event.preventDefault();
        if (accepted || session.locked || !session.isCurrent()) return;
        session.setLocked(true); ok.disabled = cancel.disabled = true;
        ok.textContent = settingsText('pending'); message.textContent = settingsText('shutdownPending');
        try {
            const { response, data } = await request('/api/shutdown', { method: 'POST' }, session, 15000,
                async response => ({ response, data: await response.json().catch(() => null) }));
            if (!session.isCurrent()) return;
            if (!response.ok) throw Error(data?.error || `HTTP ${response.status}`);
            if (response.status !== 202 || data?.ok !== true || data?.mode !== 'shutdown') throw Error(settingsText('shutdownUncertain'));
            accepted = true;
            message.classList.add('success'); message.textContent = settingsText('shutdownAccepted');
            ok.hidden = true; cancel.textContent = settingsText('close');
        } catch (error) {
            if (!session.isCurrent()) return;
            message.textContent = error.name === 'AbortError' || error instanceof TypeError
                ? settingsText('shutdownUncertain') : error.message;
            ok.disabled = false; ok.textContent = settingsText('shutdownConfirm');
        }
        session.setLocked(false); cancel.disabled = false;
    });
}

function openPassword(dialogs) {
    const view = openDialog(dialogs, 'pwd', `
        <p class="settings-hint">${settingsText('passwordNotice')}</p>
        <label for="pwd-new">${settingsText('newPassword')}</label>
        <input id="pwd-new" type="password" autocomplete="new-password" placeholder="${settingsText('passwordHint')}" autofocus required>
        <label for="pwd-confirm">${settingsText('confirmPassword')}</label>
        <input id="pwd-confirm" type="password" autocomplete="new-password" required>
        <p id="pwd-msg" class="settings-message" role="status" aria-live="polite"></p>
        <div class="settings-dialog-buttons"><button id="pwd-cancel" type="button">${settingsText('cancel')}</button><button id="pwd-ok" class="settings-primary" type="submit">${settingsText('confirm')}</button></div>`);
    if (!view) return;
    const { session, form, find } = view;
    const message = find('#pwd-msg'), ok = find('#pwd-ok'), cancel = find('#pwd-cancel');
    cancel.addEventListener('click', () => dialogs.close());
    form.addEventListener('submit', async event => {
        event.preventDefault();
        if (session.locked || !session.isCurrent()) return;
        const password = find('#pwd-new').value;
        const error = password.length < 4 ? 'passwordShort' : new TextEncoder().encode(password).length > 4096 ? 'passwordLong'
            : password !== find('#pwd-confirm').value ? 'mismatch' : null;
        message.classList.remove('success');
        if (error) { message.textContent = settingsText(error); return; }
        session.setLocked(true); ok.disabled = cancel.disabled = true;
        find('#pwd-new').readOnly = find('#pwd-confirm').readOnly = true;
        ok.textContent = settingsText('pending'); message.textContent = '';
        try {
            const response = await request('/api/change-password', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ new_password: password }) }, session);
            if (!session.isCurrent()) return;
            if (!response.ok) {
                const data = await response.json().catch(() => ({}));
                throw Error(data.error || settingsText('passwordFailed'));
            }
            message.classList.add('success'); message.textContent = settingsText('passwordChanged');
            // Notify other tabs before revoked media can navigate the parent
            // away and dispose this iframe. A delayed broadcast can be lost.
            credentialsChanged();
            return;
        } catch (error) {
            if (!session.isCurrent()) return;
            message.textContent = error.name === 'AbortError' ? settingsText('networkError') : error.message;
        }
        session.setLocked(false); ok.disabled = cancel.disabled = false;
        find('#pwd-new').readOnly = find('#pwd-confirm').readOnly = false;
        ok.textContent = settingsText('confirm');
    });
}

function openUpdate(dialogs) {
    let socket, pollTimer, reloadTimer, activityTimer;
    let running = false, waiting = false, sawRestart = false, failed = false, lastLevel, lastMessage;
    const view = openDialog(dialogs, 'update', `
        <div id="update-info"><p class="settings-hint">${settingsText('updateNotice')}</p>
            <label class="settings-upgrade-option"><input id="upgrade-latest" type="checkbox">${settingsText('upgrade')}</label>
            <p class="settings-hint">${settingsText('upgradeHint')}</p>
            <div id="upgrade-proxy-options" class="settings-proxy-options" hidden>
                <label class="settings-upgrade-option"><input id="upgrade-use-proxy" type="checkbox" disabled>${settingsText('useGithubProxy')}</label>
                <div id="upgrade-proxy-settings" class="settings-proxy-fields" hidden>
                    <label for="upgrade-proxy">${settingsText('githubProxy')}</label>
                    <select id="upgrade-proxy" disabled>
                        <option value="https://sina.dev/">sina.dev</option>
                        <option value="https://httptunnel.top/gh/">httptunnel.top/gh</option>
                        <option value="custom">${settingsText('customProxy')}</option>
                    </select>
                    <div id="upgrade-custom-proxy-field" class="settings-proxy-fields" hidden>
                        <label for="upgrade-custom-proxy">${settingsText('proxyAddress')}</label>
                        <input id="upgrade-custom-proxy" type="text" inputmode="url" autocomplete="off" autocapitalize="none" spellcheck="false" maxlength="2048" placeholder="https://example.com/gh/" aria-describedby="upgrade-proxy-address-hint" disabled>
                        <p id="upgrade-proxy-address-hint" class="settings-hint">${settingsText('proxyAddressHint')}</p>
                    </div>
                    <p class="settings-hint">${settingsText('proxyHint')}</p>
                    <p id="upgrade-proxy-message" class="settings-message" role="alert" hidden></p>
                </div>
            </div></div>
        <div id="update-progress" hidden><progress id="progress-fill" max="100" value="0"></progress><p id="progress-text" role="status" aria-live="polite"></p></div>
        <div id="update-logs" class="settings-logs" role="log" aria-label="${settingsText('update')}" hidden></div>
        <div class="settings-dialog-buttons"><button id="update-cancel" type="button">${settingsText('cancel')}</button><button id="update-ok" class="settings-primary" type="submit">${settingsText('restart')}</button></div>`,
    () => {
        clearTimeout(pollTimer); clearTimeout(reloadTimer); clearTimeout(activityTimer);
        if (socket) { socket.onclose = socket.onerror = socket.onmessage = null; socket.close(); }
    });
    if (!view) return;
    const { session, form, find } = view;
    const ok = find('#update-ok'), cancel = find('#update-cancel'), upgrade = find('#upgrade-latest');
    const useProxy = find('#upgrade-use-proxy'), proxySelect = find('#upgrade-proxy'), customProxy = find('#upgrade-custom-proxy');
    const proxyMessage = find('#upgrade-proxy-message');
    const progress = find('#progress-fill'), status = find('#progress-text'), logs = find('#update-logs');
    function log(text, level = 'info') {
        const entry = document.createElement('div');
        entry.className = `settings-log-${['info', 'success', 'error', 'warning', 'progress'].includes(level) ? level : 'info'}`;
        entry.textContent = text; logs.appendChild(entry);
        // Bound memory usage during downloads that emit many progress messages.
        while (logs.children.length > 300) logs.firstElementChild.remove();
        logs.scrollTop = logs.scrollHeight;
    }
    function fail(message) {
        if (!session.isCurrent() || failed) return;
        failed = true; running = false;
        clearTimeout(pollTimer); clearTimeout(activityTimer);
        log(message, 'error'); status.textContent = settingsText('failed');
        cancel.disabled = false; cancel.textContent = settingsText('close');
        session.setLocked(false);
    }
    function watchActivity() {
        clearTimeout(activityTimer);
        activityTimer = setTimeout(() => { fail(settingsText('upgradeFailed')); socket?.close(); }, 120000);
    }
    async function waitForRestart() {
        if (waiting || !session.isCurrent() || failed) return;
        waiting = true;
        clearTimeout(activityTimer);
        log(settingsText(upgrade.checked ? 'upgradeDone' : 'restartSent'), 'success');
        status.textContent = settingsText('restartSent');
        // Delay the first probe so the old process can leave before recovery.
        const deadline = Date.now() + 30000;
        async function poll() {
            if (!session.isCurrent() || failed) return;
            try {
                const response = await request('/health', {}, session, 2000);
                if (!session.isCurrent() || failed) return;
                if (response.ok) {
                    progress.value = 100; status.textContent = settingsText('recovered');
                    log(settingsText('recovered'), 'success');
                    reloadTimer = setTimeout(() => {
                        // The settings page is usually embedded; reload the main
                        // desktop so expired sessions go through normal login.
                        let target = window;
                        try { if (window.top.location.origin === location.origin) target = window.top; } catch (_) {}
                        target.location.reload();
                    }, 1000);
                    return;
                }
            } catch { /* a restarting process is temporarily unreachable */ }
            if (!session.isCurrent() || failed) return;
            if (Date.now() >= deadline) { fail(settingsText('notRecovered')); return; }
            pollTimer = setTimeout(poll, 1000);
        }
        pollTimer = setTimeout(poll, 1500);
    }
    cancel.addEventListener('click', () => dialogs.close());
    function syncProxyOptions() {
        ok.textContent = settingsText(upgrade.checked ? 'upgradeRestart' : 'restart');
        find('#upgrade-proxy-options').hidden = !upgrade.checked;
        useProxy.disabled = !upgrade.checked;
        const enabled = upgrade.checked && useProxy.checked;
        find('#upgrade-proxy-settings').hidden = !enabled;
        proxySelect.disabled = !enabled;
        const custom = enabled && proxySelect.value === 'custom';
        find('#upgrade-custom-proxy-field').hidden = !custom;
        customProxy.disabled = !custom; customProxy.required = custom;
        proxyMessage.hidden = true;
    }
    for (const control of [upgrade, useProxy, proxySelect]) control.addEventListener('change', syncProxyOptions);
    customProxy.addEventListener('input', () => { proxyMessage.hidden = true; });
    form.addEventListener('submit', async event => {
        event.preventDefault();
        if (running || failed || !session.isCurrent()) return;
        let proxy;
        if (upgrade.checked && useProxy.checked) {
            try {
                const input = (proxySelect.value === 'custom' ? customProxy.value : proxySelect.value).trim();
                if (!input || new TextEncoder().encode(input).length > 2048 || /[\s\\\u0000-\u001f\u007f-\u009f]/u.test(input)) throw Error();
                const url = new URL(input.includes('://') ? input : `https://${input}`);
                if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password || url.href.includes('?') || url.href.includes('#')) throw Error();
                proxy = `${url.href.replace(/\/+$/, '')}/`;
                if (proxy.length > 2048) throw Error();
            } catch {
                proxyMessage.textContent = settingsText('proxyInvalid'); proxyMessage.hidden = false;
                if (proxySelect.value === 'custom') customProxy.focus();
                return;
            }
        }
        running = true; session.setLocked(true);
        ok.disabled = cancel.disabled = true; upgrade.disabled = true;
        useProxy.disabled = proxySelect.disabled = customProxy.disabled = true;
        find('#update-info').hidden = true; find('#update-progress').hidden = logs.hidden = false;
        progress.value = 5; status.textContent = settingsText('preparing');
        if (!upgrade.checked) {
            try {
                const response = await request('/api/restart', { method: 'POST' }, session);
                if (!session.isCurrent()) return;
                if (!response.ok) {
                    const data = await response.json().catch(() => ({}));
                    throw Error(data.error || `HTTP ${response.status}`);
                }
                progress.value = 50; waitForRestart();
            } catch (error) { fail(error.name === 'AbortError' ? settingsText('networkError') : error.message); }
            return;
        }
        try {
            const url = new URL('/api/upgrade/ws', location.href);
            url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
            if (proxy) url.searchParams.set('github_proxy', proxy);
            socket = new WebSocket(url.href);
            watchActivity();
            socket.onmessage = event => {
                if (!session.isCurrent() || failed) return;
                try {
                    const entry = JSON.parse(event.data);
                    if (typeof entry.message !== 'string') return;
                    watchActivity();
                    lastLevel = entry.level; lastMessage = entry.message;
                    log(entry.message, entry.level);
                    if (Number.isFinite(entry.step) && Number.isFinite(entry.total_steps) && entry.total_steps > 0) {
                        progress.value = Math.max(0, Math.min(100, Math.round(entry.step / entry.total_steps * 100)));
                        sawRestart ||= entry.step >= entry.total_steps;
                    }
                    if (entry.level === 'error') {
                        // A pre-upgrade state-save warning is not terminal; the
                        // server can continue. Keep the operation locked until
                        // the server closes the socket or reaches restart.
                        status.textContent = entry.message;
                    }
                } catch { /* ignore malformed log messages */ }
            };
            socket.onclose = event => {
                if (sawRestart && lastLevel !== 'error' && event.code !== 1008) waitForRestart();
                else fail(lastLevel === 'error' ? lastMessage : settingsText('upgradeFailed'));
            };
            socket.onerror = () => { if (!sawRestart) fail(settingsText('upgradeFailed')); };
        } catch { fail(settingsText('upgradeFailed')); }
    });
}
