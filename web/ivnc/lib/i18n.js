const STORAGE_KEY = 'ivnc.lang';

function detect() {
    try {
        const saved = localStorage.getItem(STORAGE_KEY);
        if (saved === 'zh' || saved === 'en') return saved;
    } catch (_) {}
    const browser = (navigator.language || 'en').toLowerCase();
    return browser.startsWith('zh') ? 'zh' : 'en';
}

let _lang = detect();
const _listeners = new Set();

function _notify() {
    _listeners.forEach(fn => {
        try { fn(_lang); } catch (e) { console.error('[i18n] listener failed:', e); }
    });
}

export function getLang() {
    return _lang;
}

export function setLang(lang) {
    if (lang !== 'zh' && lang !== 'en') return;
    if (lang === _lang) return;
    _lang = lang;
    try { localStorage.setItem(STORAGE_KEY, lang); } catch (_) {}
    _notify();
}

export function onLangChange(fn) {
    _listeners.add(fn);
    return () => _listeners.delete(fn);
}

export function t(table, key, ...args) {
    const set = table[_lang] || table.en || {};
    const entry = set[key];
    if (typeof entry === 'function') return entry(...args);
    return entry !== undefined ? entry : key;
}

if (typeof window !== 'undefined') {
    window.addEventListener('storage', (e) => {
        if (e.key !== STORAGE_KEY) return;
        if (e.newValue !== 'zh' && e.newValue !== 'en') return;
        if (e.newValue === _lang) return;
        _lang = e.newValue;
        _notify();
    });
}

// Picks the string for the current language; for one-off text outside a table.
export function localized(zh, en) {
    return _lang === 'zh' ? zh : en;
}

// Application, sharing and remote-app strings shared by the console and the
// desktop toolbar. Status keys match the `status` / `remote_state` values of
// /api/apps and the taskbar `apps` list.
export const APP_STRINGS = {
    en: {
        running: 'running',
        starting: 'starting',
        stopped: 'stopped',
        crashed: 'crashed',
        missing: 'missing',
        installed: 'installed',
        not_file: 'not a file',
        not_executable: 'not executable',
        invalid_path: 'invalid path',
        remote: 'Remote',
        remoteOption: 'Remote App',
        remoteState_disconnected: 'Not started',
        remoteState_connecting: 'Connecting',
        remoteState_starting: 'Starting on remote',
        remoteState_connected: 'Connected',
        remoteState_stopped: 'Not running on remote',
        remoteState_not_ready: 'Restart needed on remote',
        remoteState_auth_required: 'Re-enter password',
        remoteState_incompatible: 'Incompatible version',
        remoteState_unreachable: 'Unreachable',
        badgeShared: 'Shared',
        badgeSharedRestart: 'Shared · restart needed',
        badgeRestart: 'Restart needed',
        badgeRestartStopTitle: 'Restart the app to stop sharing',
        badgePlaintext: 'Unencrypted',
        sectionBasic: 'General',
        sectionSharing: 'Remote access',
        shareEnabled: 'Allow remote access',
        shareEnabledHelp: 'Other iVnc instances can connect with the password. They only see and control this app.',
        sharePassword: 'Access password',
        sharePasswordHelp: 'At least 8 characters. Changing it disconnects all remote sessions.',
        sharePasswordLegacy: 'Set but not viewable; leave blank to keep',
        shareUrl: 'Connection URL',
        shareUrlHelp: 'On another iVnc, add a Remote App with this URL and password. The URL follows how this browser reaches this instance; if the other iVnc reaches it differently (e.g. Docker port mapping, reverse proxy, LAN IP), use an address it can reach instead.',
        show: 'Show',
        hide: 'Hide',
        generate: 'Generate',
        copy: 'Copy',
        copied: 'Copied',
        copyFailed: 'Copy failed',
        confirmPasswordChange: 'Changing the password disconnects all remote sessions. Continue?',
        sharingRevoked: 'Saved; remote sessions were disconnected',
        sharingLoadFailed: 'Failed to load remote access settings: ',
        sharePasswordRequired: 'Set an access password',
        sharePasswordLength: 'Password must be 8–256 characters',
        restartSaved: 'Saved. Restart the app to apply the changes.',
        restartPending: 'Restart the app to apply the remote access setting.',
        restartNow: 'Restart now',
        restarting: 'Restarting…',
        remoteUrl: 'Connection URL',
        remoteUrlPlaceholder: 'https://host/remote/v1/apps/…',
        remotePassword: 'Access password',
        remotePasswordPlaceholder: 'Password set on the sharing instance',
        remotePasswordKeep: 'Leave blank to keep',
        remoteHelp: 'On the sharing instance, edit the app, turn on remote access, then copy its URL and password.',
        remotePlaintextHint: 'Unencrypted connection: the access password and signalling travel in clear text. Prefer HTTPS.',
        remoteNamePlaceholder: 'Leave blank to use the remote app name',
        remoteTest: 'Test connection',
        remoteTesting: 'Connecting…',
        remoteTestRunning: (name) => `${name} · running`,
        remoteTestStopped: (name) => `${name} · not running`,
        remoteUrlRequired: 'Enter the connection URL',
        remoteUrlInvalid: 'Enter a valid http(s) URL',
        remotePasswordRequired: 'Enter the access password',
        remotePasswordForUrl: 'Enter the password when changing the URL',
        streamReconnecting: 'Disconnected; reconnecting…',
        streamRevoked: 'Access was withdrawn. Click the tab to retry.',
    },
    zh: {
        running: '运行中',
        starting: '启动中',
        stopped: '已停止',
        crashed: '已崩溃',
        missing: '缺失',
        installed: '已安装',
        not_file: '不是文件',
        not_executable: '不可执行',
        invalid_path: '路径无效',
        remote: '远程',
        remoteOption: '远程应用',
        remoteState_disconnected: '未启动',
        remoteState_connecting: '连接中',
        remoteState_starting: '对方应用启动中',
        remoteState_connected: '已连接',
        remoteState_stopped: '对方应用未运行',
        remoteState_not_ready: '对方应用需重启',
        remoteState_auth_required: '需重新输入密码',
        remoteState_incompatible: '版本不兼容',
        remoteState_unreachable: '无法连接',
        badgeShared: '共享中',
        badgeSharedRestart: '共享 · 需重启',
        badgeRestart: '需重启',
        badgeRestartStopTitle: '重启应用后停止共享',
        badgePlaintext: '未加密',
        sectionBasic: '基本信息',
        sectionSharing: '远程访问',
        shareEnabled: '允许远程访问',
        shareEnabledHelp: '其他 iVnc 可凭密码连接，只能看到和操作此应用。',
        sharePassword: '访问密码',
        sharePasswordHelp: '至少 8 位。修改密码会断开所有远程连接。',
        sharePasswordLegacy: '已设置但无法查看，留空保持不变',
        shareUrl: '连接地址',
        shareUrlHelp: '在另一台 iVnc 中添加“远程应用”，填入此地址和密码。地址按当前浏览器访问本机的方式生成；如果另一台 iVnc 访问本机的方式不同（如 Docker 端口映射、反向代理、内网 IP），请改用它能访问到的地址。',
        show: '显示',
        hide: '隐藏',
        generate: '生成',
        copy: '复制',
        copied: '已复制',
        copyFailed: '复制失败',
        confirmPasswordChange: '修改密码会断开所有远程连接，确定继续？',
        sharingRevoked: '已保存，远程连接已断开',
        sharingLoadFailed: '远程访问设置加载失败：',
        sharePasswordRequired: '请设置访问密码',
        sharePasswordLength: '密码需为 8–256 个字符',
        restartSaved: '已保存，重启应用后生效。',
        restartPending: '远程访问设置需重启应用后生效。',
        restartNow: '立即重启',
        restarting: '重启中…',
        remoteUrl: '连接地址',
        remoteUrlPlaceholder: 'https://主机/remote/v1/apps/…',
        remotePassword: '访问密码',
        remotePasswordPlaceholder: '共享端设置的访问密码',
        remotePasswordKeep: '留空保持不变',
        remoteHelp: '在共享端编辑该应用并开启远程访问，然后复制连接地址和密码。',
        remotePlaintextHint: '未加密连接：访问密码和信令以明文传输，建议使用 HTTPS。',
        remoteNamePlaceholder: '留空则使用远程应用名称',
        remoteTest: '测试连接',
        remoteTesting: '正在连接…',
        remoteTestRunning: (name) => `${name} · 运行中`,
        remoteTestStopped: (name) => `${name} · 未运行`,
        remoteUrlRequired: '请输入连接地址',
        remoteUrlInvalid: '请输入有效的 http(s) 地址',
        remotePasswordRequired: '请输入访问密码',
        remotePasswordForUrl: '修改连接地址时需输入访问密码',
        streamReconnecting: '连接已断开，正在重连…',
        streamRevoked: '访问已被撤销，点击标签重试。',
    }
};

export function appText(key, ...args) {
    return t(APP_STRINGS, key, ...args);
}

// Label for a remote app's `remote_state` (taskbar `status` of kind "remote").
export function remoteStateText(state) {
    const key = 'remoteState_' + (state || 'disconnected');
    const label = appText(key);
    return label === key ? String(state) : label;
}

// Text for the `code` of JSON errors from the server and for the
// `remote_detail_code` of /api/apps.
const SERVER_ERRORS = {
    en: {
        internal: 'Internal server error',
        not_found: 'Not found',
        app_not_found: 'Application not found',
        app_name_required: 'Enter an application name',
        app_name_too_long: 'Application name is too long',
        app_name_exists: (p) => `An application named "${p.name}" already exists`,
        app_type_invalid: 'Invalid application type',
        app_id_invalid: 'Invalid application ID',
        launch_command_required: 'Enter a launch command',
        exec_command_required: 'Enter a launch command',
        cli_binary_path_required: 'Enter a binary path',
        miao_read_only: 'Miao settings are read-only; configure startup through the iVnc CLI',
        miao_not_deletable: 'Built-in Miao cannot be deleted; disable autostart instead',
        miao_data_not_clearable: 'Miao data is shared proxy configuration and cannot be cleared here',
        proxy_disabled: 'Proxy disabled by administrator',
        remote_data_not_clearable: 'Remote application data is managed on the sharing instance',
        app_already_running: 'Application is already running',
        app_not_running: 'Application is not running',
        app_order_invalid: 'Invalid application order',
        app_order_conflict: 'Application order changed; refreshed to the latest order',
        app_starting: 'Application is starting',
        app_start_failed: 'Failed to start',
        app_stop_failed: 'Failed to stop',
        cli_not_startable: 'CLI apps are one-shot tools and cannot be started',
        app_runtime_failed: 'Application runtime failed to start; see the application log',
        app_runtime_timeout: 'Application runtime did not become ready; see the application log',
        app_runtime_unavailable: 'Application runtime is unavailable; see the application log',
        app_runtime_no_response: 'Application runtime did not respond',
        app_restart_required: 'Restart the app on the sharing instance to apply its remote access setting',
        sharing_desktop_only: 'Only local desktop apps support remote access',
        sharing_enabled_invalid: 'Invalid remote access setting',
        sharing_disabled: 'Remote access is not enabled for this app',
        share_password_required: 'Set an access password before enabling remote access',
        share_password_length: 'Access password must be 8–256 bytes',
        session_invalid: 'Session expired or remote access is disabled',
        auth_invalid: 'Wrong access password',
        auth_rate_limited: 'Too many failed attempts; try again later',
        auth_busy: 'Authentication is busy; try again shortly',
        auth_access_changed: 'Access settings changed; try again',
        remote_not_configured: 'Remote connection is not configured',
        remote_not_started: 'Start this remote app first',
        remote_url_required: 'Enter the connection URL',
        remote_url_invalid: 'Invalid connection URL',
        remote_url_unsupported: 'Use an http(s) URL without credentials, query or fragment',
        remote_url_not_share: 'Use the connection URL copied from the sharing instance',
        remote_password_required: 'Enter the access password',
        remote_password_for_url: 'Enter the password when changing the URL',
        remote_self: 'This address is the current instance; open the local app directly',
        remote_auth_failed: 'Wrong access password, or remote access is disabled on the sharing instance',
        remote_rate_limited: 'The sharing instance is busy or saw too many failed attempts; try again later',
        remote_rejected: (p) => `The sharing instance refused the connection: HTTP ${p.status}`,
        remote_unreachable: 'Cannot reach the sharing instance',
        remote_dns: "Cannot resolve the sharing instance's host name; enter an address this instance can reach",
        remote_refused: 'The sharing instance refused the connection; check the address and port',
        remote_timeout: 'Connection to the sharing instance timed out',
        remote_certificate: 'Untrusted certificate on the sharing instance; trust a private CA with IVNC_REMOTE_CA_FILE',
        remote_tls_config: 'Invalid certificates in IVNC_REMOTE_CA_FILE',
        remote_incompatible: 'The sharing instance does not support this protocol; run the same iVnc version on both',
        remote_identity_changed: 'The sharing instance identity or protocol changed; edit the connection and enter the password again',
        remote_legacy_password: 'Saved by an earlier version; edit the connection and enter the access password again',
        remote_access_revoked: 'Sharing was disabled or the password changed; edit the connection and enter the password again',
        remote_http_error: (p) => `The sharing instance returned HTTP ${p.status}`,
        remote_response_too_large: 'Response from the sharing instance is too large',
        remote_invalid_response: 'Invalid response from the sharing instance',
        remote_app_starting: 'Starting on the sharing instance',
        remote_app_stopped: 'Not running on the sharing instance',
        remote_app_not_ready: 'The app on the sharing instance is not ready',
        view_desktop_only: 'Only desktop apps have views',
        view_expired: 'App view expired; open it again',
        view_restarted: 'App restarted; open it again',
        too_many_views: 'Too many app views',
        too_many_sessions: 'Too many app sessions',
        access_ended: 'App access ended',
        origin_invalid: 'Invalid request origin',
    },
    zh: {
        internal: '服务器内部错误',
        not_found: '未找到',
        app_not_found: '应用不存在',
        app_name_required: '请输入应用名称',
        app_name_too_long: '应用名称过长',
        app_name_exists: (p) => `应用名称“${p.name}”已存在`,
        app_type_invalid: '应用类型无效',
        app_id_invalid: '应用 ID 无效',
        launch_command_required: '请输入启动命令',
        exec_command_required: '请输入启动命令',
        cli_binary_path_required: '请输入 binary 路径',
        miao_read_only: 'Miao 应用设置为只读，请通过 iVnc 命令行配置启动方式',
        miao_not_deletable: '内置 Miao 不能删除，可关闭自启动',
        miao_data_not_clearable: 'Miao 数据是共享的代理配置，不能在此清理',
        proxy_disabled: '代理已被管理员禁用',
        remote_data_not_clearable: '远程应用的数据由共享端管理',
        app_already_running: '应用已在运行',
        app_not_running: '应用未运行',
        app_order_invalid: '应用排序数据无效',
        app_order_conflict: '应用顺序已发生变化，已同步最新顺序',
        app_starting: '应用正在启动',
        app_start_failed: '启动失败',
        app_stop_failed: '停止失败',
        cli_not_startable: 'CLI 应用是一次性工具，不能启动',
        app_runtime_failed: '应用运行环境启动失败，请查看应用日志',
        app_runtime_timeout: '应用运行环境未能就绪，请查看应用日志',
        app_runtime_unavailable: '应用运行环境不可用，请查看应用日志',
        app_runtime_no_response: '应用运行环境无响应',
        app_restart_required: '需在共享端重启应用以应用远程访问设置',
        sharing_desktop_only: '只有本机桌面应用支持远程访问',
        sharing_enabled_invalid: '远程访问开关的值无效',
        sharing_disabled: '此应用未开启远程访问',
        share_password_required: '请先设置访问密码再开启远程访问',
        share_password_length: '访问密码需为 8–256 个字节',
        session_invalid: '会话已过期或远程访问已关闭',
        auth_invalid: '访问密码错误',
        auth_rate_limited: '失败次数过多，请稍后再试',
        auth_busy: '认证繁忙，请稍后再试',
        auth_access_changed: '访问设置已变更，请重试',
        remote_not_configured: '远程连接未配置',
        remote_not_started: '请先启动此远程应用',
        remote_url_required: '请输入连接地址',
        remote_url_invalid: '连接地址无效',
        remote_url_unsupported: '请使用不含账号、查询参数和片段的 http(s) 地址',
        remote_url_not_share: '请使用从共享端复制的连接地址',
        remote_password_required: '请输入访问密码',
        remote_password_for_url: '修改连接地址时需输入访问密码',
        remote_self: '该地址是当前实例，请直接打开本机应用',
        remote_auth_failed: '访问密码错误，或共享端已关闭远程访问',
        remote_rate_limited: '共享端认证繁忙或失败次数过多，请稍后再试',
        remote_rejected: (p) => `共享端拒绝连接：HTTP ${p.status}`,
        remote_unreachable: '无法连接共享端',
        remote_dns: '无法解析共享端的主机名，请填写本机能访问到的地址',
        remote_refused: '共享端拒绝连接，请检查地址和端口',
        remote_timeout: '连接共享端超时',
        remote_certificate: '共享端证书不受信任；私有 CA 需通过 IVNC_REMOTE_CA_FILE 配置',
        remote_tls_config: 'IVNC_REMOTE_CA_FILE 证书配置无效',
        remote_incompatible: '共享端不支持此共享协议，请两端使用相同版本的 iVnc',
        remote_identity_changed: '共享端实例身份或协议已变化，请编辑连接并重新输入密码',
        remote_legacy_password: '此连接由旧版本保存，请编辑并重新输入访问密码',
        remote_access_revoked: '共享端已关闭共享或修改了密码，请编辑连接并重新输入密码',
        remote_http_error: (p) => `共享端返回 HTTP ${p.status}`,
        remote_response_too_large: '共享端响应过大',
        remote_invalid_response: '共享端响应无效',
        remote_app_starting: '共享端应用启动中',
        remote_app_stopped: '共享端应用未运行',
        remote_app_not_ready: '共享端应用尚未就绪',
        view_desktop_only: '只有桌面应用可以打开视图',
        view_expired: '应用视图已过期，请重新打开',
        view_restarted: '应用已重启，请重新打开',
        too_many_views: '打开的应用视图过多',
        too_many_sessions: '应用会话过多',
        access_ended: '应用访问已结束',
        origin_invalid: '请求来源无效',
    },
};

// A server error `{error, code?, params?}` in the user's language, '' when
// there is none. `params.detail`, e.g. a network error, cannot be translated
// and follows the text, as does the English `error` of internal failures.
// Errors without a known code keep their English text, after a generic
// text in Chinese.
export function serverErrorText(body) {
    const { error = '', code, params } = body || {};
    const entry = (SERVER_ERRORS[_lang] || SERVER_ERRORS.en)[code];
    const note = (text, detail) => (_lang === 'zh' ? `${text}（${detail}）` : `${text} (${detail})`);
    if (!entry) return _lang === 'zh' && error ? note('请求失败', error) : error;
    if (code === 'internal') return note(entry, error);
    const text = typeof entry === 'function' ? entry(params || {}) : entry;
    return params?.detail ? note(text, params.detail) : text;
}
