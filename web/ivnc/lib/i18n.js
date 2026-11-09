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
        shareUrlHelp: 'On another iVnc, add a Remote App with this URL and password.',
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
        shareUrlHelp: '在另一台 iVnc 中添加“远程应用”，填入此地址和密码。',
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
