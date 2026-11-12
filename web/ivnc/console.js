import { getLang, setLang, onLangChange, APP_STRINGS, remoteStateText, serverErrorText } from './lib/i18n.js?v=1';
import { notifyAppStarted } from './app-streams.js';
import { createOrderNotifications } from './app-order.js';
import { createConsoleDialogs } from './console-dialogs.js';
import { enhanceSelect } from './select-control.js';

const API = '/api/apps';
let launchContext = null;
const CONSOLE_API = '/api/console';
const CAP_API = '/api/capabilities';
let editId = null;
let editBuiltin = false;
let editApp = null;        // record of the app being edited
let sharingState = null;   // GET /api/apps/{id}/sharing of the desktop app being edited
let appSignature = '';     // launch settings as last saved; a change on a running app needs a restart
let modalSaved = false;    // the open dialog has saved at least once
const dialogs = createConsoleDialogs();
const pendingAppWrites = new Map();
let appDialog = null, appTypeControl = null;
let lastDataHash = '';
let appsLoadGeneration = 0;
let latestOrderRevision = -1;
const latestNames = new Map();
const orderNotifications = createOrderNotifications((revision) => {
    if (revision <= latestOrderRevision) return;
    lastDataHash = '';
    if (activeSection === 'apps') load();
}, (id, revision) => {
    if (revision <= (latestNames.get(id)?.name_revision ?? -1)) return;
    lastDataHash = '';
    if (activeSection === 'apps') load();
});
window.addEventListener('pagehide', () => orderNotifications.destroy(), { once: true });
let currentLang = getLang();
let activeSection = 'overview';
let overviewCache = null;
let lastOverviewAt = 0;

const I18N = {
    en: {
        pageTitle: 'Management Console',
        subtitle: 'Manage apps and runtime settings',
        navOverview: 'Overview',
        navApps: 'Apps',
        navSettings: 'Connection',
        sectionOverviewDesc: 'Current iVnc status and quick checks.',
        sectionAppsDesc: 'Manage desktop, background, CLI and remote applications.',
        sectionSettingsDesc: 'Common runtime controls for stream quality.',
        saveApply: 'Save & Apply',
        requestKeyframe: 'Request Keyframe',
        paramTargetFps: 'Target FPS',
        paramTargetFpsHelp: 'Target video frame rate. Higher values are smoother but increase encoding and network load.',
        paramVideoBitrate: 'Video bitrate',
        paramVideoBitrateHelp: 'Video encoding bitrate. Higher values improve clarity but use more bandwidth.',
        paramAudioBitrate: 'Audio bitrate',
        paramAudioBitrateHelp: 'Audio encoding bitrate. 128000 is a common default.',
        paramBinaryClipboard: 'Binary clipboard',
        paramBinaryClipboardHelp: 'Sync HTML rich text and PNG images together with plain text.',
        paramKeyframe: 'Keyframe',
        paramKeyframeHelp: 'Request the next video keyframe immediately, useful after visual artifacts or reconnects.',
        paramAgentProvider: 'Default provider',
        paramAgentProviderHelp: 'VLM provider used by new Agent runs by default.',
        paramAgentMaxSteps: 'Max steps',
        paramAgentMaxStepsHelp: 'Maximum decision steps for one task. The run stops when this limit is reached.',
        paramAgentMaxWall: 'Max wall time',
        paramAgentMaxWallHelp: 'Maximum wall-clock seconds for one task. The run stops after this duration.',
        paramAgentScreenshotBytes: 'Screenshot max size',
        paramAgentScreenshotBytesHelp: 'Maximum encoded size for each screenshot sent to the model.',
        paramAgentRecordTrajectory: 'Record trajectory',
        paramAgentRecordTrajectoryHelp: 'Save Agent observations, actions, and results for debugging and review.',
        paramAgentDryRun: 'Dry run',
        paramAgentDryRunHelp: 'Generate plans and actions without controlling the desktop. Useful for Provider debugging.',
        paramProviderEndpoint: 'Endpoint',
        paramProviderEndpointHelp: 'Provider API base URL. Leave empty to use the built-in default.',
        paramProviderName: 'Name',
        paramProviderType: 'Provider type',
        paramProviderModel: 'Model',
        paramProviderModelHelp: 'Model name to call. It must be supported by the selected Provider.',
        paramProviderApiFormat: 'API format',
        paramProviderApiFormatHelp: 'OpenAI wire API used by this Provider. Responses calls /responses; Chat Completions calls /chat/completions.',
        paramProviderApiKey: 'API Key',
        paramProviderApiKeyHelp: 'Provider credential. When set, the value is masked; click "Replace" to enter a new key, or "Clear" to remove the saved key.',
        paramProviderCoordSpace: 'Coordinate space',
        paramProviderCoordSpaceHelp: 'Coordinate space returned by the Provider. Default uses the Provider built-in convention.',
        toggleEnabled: 'Enabled',
        defaultOption: 'Default',
        settingsSaved: 'Settings applied',
        langToggle: '中文',
        addApp: '+ Add App',
        thName: 'Name',
        thType: 'Type',
        thConfig: 'Config',
        thStatus: 'Status',
        thData: 'Data',
        thActions: 'Actions',
        capTools: 'tools',
        capSkills: 'skills',
        capDescribeOk: 'describe ok',
        capDescribeFailed: 'describe failed',
        empty: 'No apps yet. Click "Add App" to create one.',
        desktop: 'Desktop',
        background: 'Background',
        cli: 'CLI',
        start: 'Start',
        stop: 'Stop',
        restart: 'Restart',
        visit: 'Visit',
        edit: 'Edit',
        log: 'Logs',
        clear: 'Clear',
        delete: 'Delete',
        addModal: 'Add App',
        editModal: 'Edit App',
        addSave: 'Add',
        editSave: 'Save',
        name: 'Name',
        namePlaceholder: 'App display name',
        appType: 'App Type',
        autostart: 'Autostart',
        miaoService: 'Built-in proxy service',
        configureProxy: 'Configure proxy',
        disabledByAdmin: 'Disabled by administrator',
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
        desktopCommandPlaceholder: 'Enter launch command, e.g. /usr/bin/code --ozone-platform=wayland',
        electronTemplate: 'Electron Wayland template',
        electronTemplateHint: 'Fill an empty command, then replace the program path. Sandbox is preserved unless you explicitly disable it for this template. Existing commands are never changed.',
        electronSandbox: 'Disable this app’s sandbox in the template',
        launchIdentity: (i) => `Runs as ${i.user} (UID ${i.uid}). Apps share iVnc’s user and display session; su/sudo login wrappers can break this connection.`,
        launchRootHint: ' Electron requires --no-sandbox as root. To keep its sandbox, run iVnc and the app as the same non-root user.',
        launchContextUnavailable: 'Runtime identity is unavailable. Check the server user before choosing a sandbox policy.',
        desktopWindowReady: 'Window available',
        desktopWaitingWindow: 'Process running · waiting for window',
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
        cliOption: 'CLI App',
        skillPaths: 'Skill Paths',
        skillPathsDefault: '(Optional, one .md/.txt file or directory with SKILL.md per line)',
        skillPathsPlaceholder: '/root/.config/ivnc/skills/app/SKILL.md',
        cliBinary: 'Binary Path',
        cliEnv: 'Execution Environment Variables',
        cliEnvDefault: '(Optional, added when this CLI is invoked; one KEY=value per line)',
        cliBinaryPlaceholder: '/usr/local/bin/agent-browser',
        cliEnvPlaceholder: 'NO_COLOR=1\nAGENT_BROWSER_SESSION=ivnc-default',
        missingCliBinary: 'Please enter a binary path',
        accessUrlCopied: 'Access link copied',
        accessUrlCopyFailed: 'Failed to copy access link',

        // overview groups
        settingsStreamTitle: 'Stream Parameters',
        settingsActionsTitle: 'Instant Actions',
        settingsActionsHint: 'Applied immediately, no save required',
        settingsHint: 'Changes apply instantly and persist as next-startup defaults.',
        suffixFps: 'fps',
        suffixKbps: 'kbps',
        suffixBps: 'bps',
        suffixSteps: 'steps',
        suffixSeconds: 's',
        suffixBytes: 'bytes',
        dirtyIndicator: '● Unsaved changes',
        metaLastRefresh: 'Last refresh',
        metaJustNow: 'just now',
        metaAutoRefresh: 'Auto-refresh 5s',
        // overview tiles
        tileVersion: 'Version',
        tileDisplay: 'Display',
        tileSessions: 'Sessions',
        tileFpsTarget: (fps) => `target ${fps} FPS`,
        sessionsHelpNone: 'no viewer connected',
        sessionsHelpOne: '1 viewer',
        sessionsHelpMany: (n) => `${n} viewers`,
        // quick actions
        quickApps: 'Apps',
        quickAppsDesc: 'Add or manage desktop & background apps',
        quickSettings: 'Connection',
        quickSettingsDesc: 'Tune FPS, bitrate, clipboard',
        // provider list
        // runs
    },
    zh: {
        pageTitle: '管理控制台',
        subtitle: '管理应用与运行参数',
        navOverview: '概览',
        navApps: '应用',
        navSettings: '连接质量',
        sectionOverviewDesc: '当前 iVnc 运行状态和快捷入口。',
        sectionAppsDesc: '管理桌面、后台、CLI 和远程应用。',
        sectionSettingsDesc: '调整常用的实时串流质量参数。',
        saveApply: '保存并应用',
        requestKeyframe: '请求关键帧',
        paramTargetFps: 'Target FPS',
        paramTargetFpsHelp: '目标画面帧率，越高越流畅但会增加编码和网络压力。',
        paramVideoBitrate: '视频码率',
        paramVideoBitrateHelp: '视频码率，越高越清晰但占用更多带宽。',
        paramAudioBitrate: '音频码率',
        paramAudioBitrateHelp: '音频编码码率，常用值为 128000。',
        paramBinaryClipboard: '二进制剪贴板',
        paramBinaryClipboardHelp: '允许剪贴板在纯文本之外同步 HTML 富文本和 PNG 图片。',
        paramKeyframe: '关键帧',
        paramKeyframeHelp: '立即请求下一帧关键帧，画面花屏或恢复连接后可手动触发。',
        paramAgentProvider: '默认 Provider',
        paramAgentProviderHelp: '新建 Agent run 默认使用的 VLM Provider。',
        paramAgentMaxSteps: '最大步数',
        paramAgentMaxStepsHelp: '单次任务允许的最大决策步数，达到后自动停止。',
        paramAgentMaxWall: '最大时长',
        paramAgentMaxWallHelp: '单次任务的最长运行秒数，超过后自动结束。',
        paramAgentScreenshotBytes: '单张截图大小',
        paramAgentScreenshotBytesHelp: '发送给模型的单张截图最大字节数，用于控制请求体大小。',
        paramAgentRecordTrajectory: '记录轨迹',
        paramAgentRecordTrajectoryHelp: '保存 Agent 的观察、动作和结果，便于复盘问题。',
        paramAgentDryRun: '模拟模式',
        paramAgentDryRunHelp: '只生成计划和动作，不真正控制桌面，适合调试 Provider。',
        paramProviderEndpoint: 'Endpoint',
        paramProviderEndpointHelp: 'Provider 的 API 基础地址，留空使用内置默认值。',
        paramProviderName: '名称',
        paramProviderType: 'Provider 类型',
        paramProviderModel: 'Model',
        paramProviderModelHelp: '调用的模型名称，需与所选 Provider 支持的模型一致。',
        paramProviderApiFormat: 'API 格式',
        paramProviderApiFormatHelp: 'Provider 使用的 OpenAI 接口协议。Responses 请求 /responses；Chat Completions 请求 /chat/completions。',
        paramProviderApiKey: 'API Key',
        paramProviderApiKeyHelp: 'Provider 凭据。已配置时显示遮罩；点击"替换"输入新值，或"清除"删除已保存的密钥。',
        paramProviderCoordSpace: '坐标空间',
        paramProviderCoordSpaceHelp: 'Provider 返回坐标的空间类型，默认使用该 Provider 的内置约定。',
        toggleEnabled: '启用',
        defaultOption: '默认',
        settingsSaved: '设置已应用',
        langToggle: 'English',
        addApp: '+ 添加应用',
        thName: '名称',
        thType: '类型',
        thConfig: '配置',
        thStatus: '状态',
        thData: '数据',
        thActions: '操作',
        capTools: '工具',
        capSkills: '技能',
        capDescribeOk: 'describe 正常',
        capDescribeFailed: 'describe 失败',
        empty: '暂无应用，点击"添加应用"创建。',
        desktop: '桌面',
        background: '后台',
        cli: 'CLI',
        start: '启动',
        stop: '停止',
        restart: '重启',
        visit: '访问',
        edit: '编辑',
        log: '日志',
        clear: '清理',
        delete: '删除',
        addModal: '添加应用',
        editModal: '编辑应用',
        addSave: '添加',
        editSave: '保存',
        name: '名称',
        namePlaceholder: '应用显示名称',
        appType: '应用类型',
        autostart: '开机启动',
        miaoService: '内置代理服务',
        configureProxy: '配置代理',
        disabledByAdmin: '管理员已禁用',
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
        desktopCommandPlaceholder: '请输入启动命令，例如： /usr/bin/code --ozone-platform=wayland',
        electronTemplate: 'Electron Wayland 模板',
        electronTemplateHint: '启动命令为空时可填入模板，再替换程序路径。默认保留沙箱，只有勾选后才在模板中禁用。已有命令不会被改写。',
        electronSandbox: '模板中禁用此应用的沙箱',
        launchIdentity: (i) => `运行用户：${i.user}（UID ${i.uid}）。应用与 iVnc 共用运行用户和显示会话，su/sudo 登录切换可能导致连接失败。`,
        launchRootHint: ' root 运行 Electron 需要 --no-sandbox；如需保留沙箱，应让 iVnc 和应用以同一普通用户运行。',
        launchContextUnavailable: '暂时无法获取运行用户，请先核对服务端身份，再选择沙箱策略。',
        desktopWindowReady: '窗口已出现',
        desktopWaitingWindow: '进程运行中 · 等待窗口',
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
        cliOption: 'CLI 应用',
        skillPaths: 'Skill 路径',
        skillPathsDefault: '（可选，每行一个 .md/.txt 文件或包含 SKILL.md 的目录）',
        skillPathsPlaceholder: '/root/.config/ivnc/skills/app/SKILL.md',
        cliBinary: 'Binary Path',
        cliEnv: '执行环境变量',
        cliEnvDefault: '（可选，调用该 CLI 时附加，每行 KEY=value）',
        cliBinaryPlaceholder: '/usr/local/bin/agent-browser',
        cliEnvPlaceholder: 'NO_COLOR=1\nAGENT_BROWSER_SESSION=ivnc-default',
        missingCliBinary: '请输入 binary 路径',
        accessUrlCopied: '访问链接已复制',
        accessUrlCopyFailed: '复制访问链接失败',

        // overview groups
        settingsStreamTitle: '串流参数',
        settingsActionsTitle: '即时操作',
        settingsActionsHint: '立即生效，无需保存',
        settingsHint: '改动会立即应用并保存为下次启动的默认值。',
        suffixFps: 'fps',
        suffixKbps: 'kbps',
        suffixBps: 'bps',
        suffixSteps: '步',
        suffixSeconds: '秒',
        suffixBytes: '字节',
        dirtyIndicator: '● 未保存改动',
        metaLastRefresh: '最后刷新',
        metaJustNow: '刚刚',
        metaAutoRefresh: '自动刷新 5s',
        // overview tiles
        tileVersion: '版本',
        tileDisplay: '画面',
        tileSessions: '连接',
        tileFpsTarget: (fps) => `目标 ${fps} FPS`,
        sessionsHelpNone: '当前无观看者',
        sessionsHelpOne: '1 位观看者',
        sessionsHelpMany: (n) => `${n} 位观看者`,
        // quick actions
        quickApps: '应用',
        quickAppsDesc: '添加或管理桌面应用与后台应用',
        quickSettings: '连接质量',
        quickSettingsDesc: '调整 FPS、码率与剪贴板',
        // provider list
        // runs
    }
};

// Status, sharing and remote-app strings live in lib/i18n.js so the desktop
// toolbar can share them.
for (const lang of Object.keys(I18N)) Object.assign(I18N[lang], APP_STRINGS[lang]);

function t(key) {
    return I18N[currentLang][key];
}

function esc(s) {
    return String(s ?? '')
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
        launch: a.launch_command,
        autostart: a.autostart,
        ready: a.ready,
        enabled: a.enabled,
        error: a.error,
        cli: a.cli_binary_path,
        install: a.install_status,
        skills: a.skill_paths,
        name: a.name,
        name_revision: a.name_revision,
        sharing: a.sharing,
        started: a.started,
        remote: [a.remote_state, a.remote_detail, a.remote_detail_code, a.remote_detail_params, a.remote_plaintext]
    })).concat((data.capabilities?.apps || []).map(a => ({
        capId: a.id,
        tools: a.tool_count,
        skills: a.skill_count,
        diagnostics: a.diagnostics?.length || 0
    }))));
}

function setTextById(id, value) {
    const el = document.getElementById(id);
    if (el) el.textContent = value;
}

function setText(id, key) {
    setTextById(id, t(key));
}

function setPlaceholderById(id, value) {
    const el = document.getElementById(id);
    if (el) el.placeholder = value;
}

function applyTranslations() {
    document.documentElement.lang = currentLang === 'en' ? 'en' : 'zh-CN';
    document.title = t('pageTitle');
    setText('page-title', 'pageTitle');
    setText('console-subtitle', 'subtitle');
    setText('nav-overview', 'navOverview');
    setText('nav-apps', 'navApps');
    setText('nav-settings', 'navSettings');
    setText('overview-meta-auto', 'metaAutoRefresh');

    setText('settings-stream-title', 'settingsStreamTitle');
    setText('settings-actions-title', 'settingsActionsTitle');
    setText('settings-actions-hint', 'settingsActionsHint');
    setText('settings-hint', 'settingsHint');
    setText('settings-save', 'saveApply');
    setText('settings-dirty', 'dirtyIndicator');
    setText('request-keyframe', 'requestKeyframe');




    [
        ['label-target-fps', 'paramTargetFps'],
        ['help-target-fps', 'paramTargetFpsHelp'],
        ['label-video-bitrate', 'paramVideoBitrate'],
        ['help-video-bitrate', 'paramVideoBitrateHelp'],
        ['label-audio-bitrate', 'paramAudioBitrate'],
        ['help-audio-bitrate', 'paramAudioBitrateHelp'],
        ['label-binary-clipboard', 'paramBinaryClipboard'],
        ['help-binary-clipboard', 'paramBinaryClipboardHelp'],
        ['label-keyframe', 'paramKeyframe'],
        ['help-keyframe', 'paramKeyframeHelp'],
        ['label-agent-provider', 'paramAgentProvider'],
        ['help-agent-provider', 'paramAgentProviderHelp'],
        ['label-agent-max-steps', 'paramAgentMaxSteps'],
        ['help-agent-max-steps', 'paramAgentMaxStepsHelp'],
        ['label-agent-max-wall', 'paramAgentMaxWall'],
        ['help-agent-max-wall', 'paramAgentMaxWallHelp'],
        ['label-agent-screenshot-bytes', 'paramAgentScreenshotBytes'],
        ['help-agent-screenshot-bytes', 'paramAgentScreenshotBytesHelp'],
        ['label-agent-record-trajectory', 'paramAgentRecordTrajectory'],
        ['help-agent-record-trajectory', 'paramAgentRecordTrajectoryHelp'],
        ['label-agent-dry-run', 'paramAgentDryRun'],
        ['help-agent-dry-run', 'paramAgentDryRunHelp'],
        ['label-provider-endpoint', 'paramProviderEndpoint'],
        ['help-provider-endpoint', 'paramProviderEndpointHelp'],
        ['label-provider-display-name', 'paramProviderName'],
        ['label-provider-type', 'paramProviderType'],
        ['label-provider-model', 'paramProviderModel'],
        ['help-provider-model', 'paramProviderModelHelp'],
        ['label-provider-api-format', 'paramProviderApiFormat'],
        ['help-provider-api-format', 'paramProviderApiFormatHelp'],
        ['label-provider-api-key', 'paramProviderApiKey'],
        ['help-provider-api-key', 'paramProviderApiKeyHelp'],
        ['label-provider-coord-space', 'paramProviderCoordSpace'],
        ['help-provider-coord-space', 'paramProviderCoordSpaceHelp'],
        ['toggle-binary-clipboard', 'toggleEnabled'],
        ['toggle-agent-record-trajectory', 'toggleEnabled'],
        ['toggle-agent-dry-run', 'toggleEnabled'],
        ['suffix-target-fps', 'suffixFps'],
        ['suffix-video-bitrate', 'suffixKbps'],
        ['suffix-audio-bitrate', 'suffixBps'],
        ['suffix-agent-max-steps', 'suffixSteps'],
        ['suffix-agent-max-wall', 'suffixSeconds'],
        ['suffix-agent-screenshot-bytes', 'suffixBytes'],
    ].forEach(([id, key]) => setText(id, key));

    const coordDefault = document.querySelector('#provider-coord-space option[value=""]');
    if (coordDefault) coordDefault.textContent = t('defaultOption');

    setText('lang-toggle', 'langToggle');
    setText('add-app-btn', 'addApp');
    setText('th-name', 'thName');
    setText('th-type', 'thType');
    setText('th-config', 'thConfig');
    setText('th-status', 'thStatus');
    setText('th-data', 'thData');
    setText('th-actions', 'thActions');
    setText('label-name', 'name');
    setPlaceholderById('f-name', t('namePlaceholder'));
    setText('label-app-type', 'appType');
    const bgOpt = document.querySelector('#f-app-type option[value="background"]');
    const dtOpt = document.querySelector('#f-app-type option[value="desktop"]');
    const cliOpt = document.querySelector('#f-app-type option[value="cli"]');
    const remoteOpt = document.querySelector('#f-app-type option[value="remote"]');
    if (bgOpt) bgOpt.textContent = t('backgroundOption');
    if (dtOpt) dtOpt.textContent = t('desktopOption');
    if (cliOpt) cliOpt.textContent = t('cliOption');
    if (remoteOpt) remoteOpt.textContent = t('remoteOption');
    setText('label-autostart', 'autostart');
    setText('label-skill-paths', 'skillPaths');
    setText('label-skill-paths-default', 'skillPathsDefault');
    setText('label-launch-command', 'launchCommand');
    setText('label-launch-command-help', 'launchCommandHelp');
    setText('label-launch-cwd', 'launchCwd');
    setText('label-launch-cwd-default', 'launchCwdDefault');
    setText('label-launch-timeout', 'launchTimeout');
    setText('label-launch-timeout-default', 'launchTimeoutDefault');
    setText('label-access-url', 'accessUrl');
    setText('label-access-url-default', 'accessUrlDefault');
    setText('label-launch-env', 'launchEnv');
    setText('label-launch-env-default', 'envDefault');
    setText('label-exec', 'exec');
    setText('label-exec-help', 'execHelp');
    setText('label-env', 'env');
    setText('label-env-default', 'envDefault');
    setText('label-cli-binary', 'cliBinary');
    setText('label-cli-env', 'cliEnv');
    setText('label-cli-env-default', 'cliEnvDefault');
    [
        ['basic-title', 'sectionBasic'],
        ['sharing-title', 'sectionSharing'],
        ['label-share-enabled', 'shareEnabled'],
        ['help-share-enabled', 'shareEnabledHelp'],
        ['label-share-password', 'sharePassword'],
        ['help-share-password', 'sharePasswordHelp'],
        ['label-share-url', 'shareUrl'],
        ['help-share-url', 'shareUrlHelp'],
        ['share-password-generate', 'generate'],
        ['share-url-copy', 'copy'],
        ['restart-now', 'restartNow'],
        ['label-remote-url', 'remoteUrl'],
        ['label-remote-password', 'remotePassword'],
        ['remote-plaintext-hint', 'remotePlaintextHint'],
        ['remote-test', 'remoteTest'],
        ['remote-help', 'remoteHelp'],
    ].forEach(([id, key]) => setText(id, key));
    setPlaceholderById('f-remote-url', t('remoteUrlPlaceholder'));
    ['f-remote-password', 'f-share-password'].forEach(syncPasswordToggle);
    const notice = document.getElementById('restart-notice');
    if (notice?.dataset.key) setText('restart-notice-text', notice.dataset.key);
    setText('modal-cancel', editBuiltin || modalSaved ? 'close' : 'cancel');
    setText('log-close-btn', 'close');
    setText('electron-template', 'electronTemplate');
    setText('electron-template-hint', 'electronTemplateHint');
    setText('electron-sandbox-label', 'electronSandbox');
    renderLaunchContext();
    document.getElementById('modal-close')?.setAttribute('aria-label', t('close'));
    setPlaceholderById('f-name', t('nameInputPlaceholder'));
    setPlaceholderById('f-launch-command', t('backgroundCommandPlaceholder'));
    setPlaceholderById('f-exec', t('desktopCommandPlaceholder'));
    setPlaceholderById('f-launch-cwd', t('cwdPlaceholder'));
    setPlaceholderById('f-launch-timeout', t('timeoutPlaceholder'));
    setPlaceholderById('f-url', t('accessUrlPlaceholder'));
    setPlaceholderById('f-launch-env', t('launchEnvPlaceholder'));
    setPlaceholderById('f-env', t('envPlaceholder'));
    setPlaceholderById('f-skill-paths', t('skillPathsPlaceholder'));
    setPlaceholderById('f-cli-binary', t('cliBinaryPlaceholder'));
    setPlaceholderById('f-cli-env', t('cliEnvPlaceholder'));
    updateTypePlaceholders();

    if (!editId && appDialog?.kind !== 'edit') {
        setText('modal-title', 'addModal');
        setText('modal-save', 'addSave');
    } else {
        setText('modal-title', 'editModal');
        setText('modal-save', 'editSave');
    }
    if (appDialog?.saving) setText('modal-save', 'savePending');
    if (appDialog?.restarting) setText('restart-now', 'restarting');

    const logTitle = document.getElementById('log-title');
    if (logTitle && !logTitle.dataset.appName) {
        logTitle.textContent = t('logsTitle');
    }
    updateSectionHeader();
    if (activeSection === 'overview') {
        updateOverviewMeta();
        renderQuickActions();
    }
}

function updateSectionHeader() {
    const titleMap = {
        overview: 'navOverview',
        apps: 'navApps',
        settings: 'navSettings'
    };
    const descMap = {
        overview: 'sectionOverviewDesc',
        apps: 'sectionAppsDesc',
        settings: 'sectionSettingsDesc'
    };
    setTextById('section-title', t(titleMap[activeSection]));
    setTextById('section-description', t(descMap[activeSection]));
}

function switchSection(section) {
    activeSection = section;
    document.body.dataset.section = section;
    document.querySelectorAll('.nav-item').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.section === section);
        if (btn.dataset.section === section) btn.setAttribute('aria-current', 'page');
        else btn.removeAttribute('aria-current');
    });
    document.querySelectorAll('.section').forEach(el => {
        el.classList.toggle('active', el.id === `section-${section}`);
    });
    updateSectionHeader();
    loadSection(section);
}

async function loadSection(section = activeSection) {
    if (section === 'overview') await loadOverview();
    if (section === 'apps') await load();
    if (section === 'settings') await loadSettings();
}

// Dot colour for each remote_state of a remote app.
const REMOTE_STATUS_CLASS = {
    disconnected: 'disconnected',
    connecting: 'connecting',
    starting: 'connecting',
    connected: 'connected',
    stopped: 'stopped',
    not_ready: 'warning',
    auth_required: 'error',
    incompatible: 'error',
    unreachable: 'error'
};

function isActive(a) {
    return a.app_type === 'remote' ? !!a.started : a.status === 'running' || a.status === 'starting';
}

function appStatus(a) {
    if (a.app_type === 'remote') {
        const state = a.remote_state || (a.started ? 'connecting' : 'disconnected');
        const detail = serverErrorText({ error: a.remote_detail, code: a.remote_detail_code, params: a.remote_detail_params });
        return { text: remoteStateText(state), cls: REMOTE_STATUS_CLASS[state] || 'error', detail };
    }
    if (a.app_type === 'cli') {
        const key = a.install_status || 'missing';
        return { text: t(key) || key, cls: a.installed ? 'running' : 'stopped', detail: a.install_error || '' };
    }
    let text = a.enabled === false ? t('disabledByAdmin') : (t(a.status) || a.status);
    if (a.app_type === 'desktop' && a.status === 'running' && typeof a.window_ready === 'boolean') {
        text = t(a.window_ready ? 'desktopWindowReady' : 'desktopWaitingWindow');
    }
    return { text, cls: a.status, detail: serverErrorText(a.launch_error) || a.error || '' };
}

// restart_required: the running app is (or is not) in its sharing worker
// contrary to the saved setting.
function sharingBadge(a) {
    const sharing = a.app_type === 'desktop' ? a.sharing : null;
    if (!sharing) return '';
    if (sharing.enabled && !sharing.restart_required) return `<span class="badge-ok">${esc(t('badgeShared'))}</span>`;
    if (sharing.enabled) return `<span class="badge-warn">${esc(t('badgeSharedRestart'))}</span>`;
    if (sharing.restart_required) return `<span class="badge-warn" title="${esc(t('badgeRestartStopTitle'))}">${esc(t('badgeRestart'))}</span>`;
    return '';
}

async function load() {
    const generation = ++appsLoadGeneration;
    try {
        const [appsResp, capResp] = await Promise.all([
            fetch(API),
            fetch(CAP_API).catch(() => null)
        ]);
        const d = await appsResp.json();
        const caps = capResp && capResp.ok ? await capResp.json() : null;
        if (generation !== appsLoadGeneration || (d.order_revision ?? -1) < latestOrderRevision) return;
        latestOrderRevision = d.order_revision ?? latestOrderRevision;
        for (const app of d.apps || []) {
            const previous = latestNames.get(app.id);
            if (previous && previous.name_revision > app.name_revision) {
                app.name = previous.name; app.name_revision = previous.name_revision;
            } else latestNames.set(app.id, { name: app.name, name_revision: app.name_revision });
            if (app.id === editId && editApp && app.name_revision > editApp.name_revision) {
                const input = document.getElementById('f-name');
                // Keep a dirty draft and its baseline so a concurrent rename
                // produces a conflict instead of an unnoticed overwrite.
                if (input.value.trim() === editApp.name) {
                    input.value = app.name;
                    editApp = { ...editApp, name: app.name, name_revision: app.name_revision };
                }
            }
        }
        d.capabilities = caps;
        const capByApp = new Map((caps?.apps || []).map(app => [app.id, app]));

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
            tr.dataset.appId = a.id;
            const cap = capByApp.get(a.id);
            const isRemote = a.app_type === 'remote';
            const type = isRemote ? t('remote') : a.app_type === 'desktop' ? t('desktop') : (a.app_type === 'cli' ? t('cli') : t('background'));
            const capLines = cap ? capabilitySummaryLines(cap) : [];
            const configStr = isRemote ? (a.url || '') : a.builtin ? t('miaoService') : a.app_type === 'desktop'
                ? (a.exec_command || '')
                : a.app_type === 'cli'
                    ? [a.cli_binary_path || '', ...capLines, ...(a.skill_paths || []).map(p => `skill: ${p}`)].filter(Boolean).join('\n')
                    : [a.url ? `${t('visit')}: ${a.url}` : '', a.launch_command ? `cmd: ${a.launch_command}` : ''].filter(Boolean).join('\n');
            const configShort = configStr.length > 30 ? configStr.slice(0, 30) + '...' : configStr;
            const plaintext = isRemote && a.remote_plaintext ? `<span class="badge-warn">${esc(t('badgePlaintext'))}</span>` : '';
            const status = appStatus(a);
            const dataText = a.app_type === 'cli' ? '-' : a.data_size_human;

            tr.innerHTML = `
                <td><strong>${esc(a.name)}</strong></td>
                <td><span class="badge">${esc(type)}</span></td>
                <td title="${esc(configStr)}">${plaintext ? `<div class="config-cell"><span>${esc(configShort)}</span>${plaintext}</div>` : esc(configShort)}</td>
                <td>
                    <div class="status-wrapper">
                        <span class="status status-${esc(status.cls)}"></span>
                        <span title="${esc(status.detail)}">${esc(status.text)}</span>
                        ${sharingBadge(a)}
                    </div>
                    ${a.launch_error ? `<div class="launch-error">${esc(serverErrorText(a.launch_error))}</div>` : ''}
                </td>
                <td class="data-size">${esc(isRemote ? '-' : dataText)}</td>
                <td class="actions"></td>
            `;

            const actionsCell = tr.querySelector('.actions');

            if (a.app_type !== 'cli') {
                if (isActive(a)) {
                    actionsCell.append(createBtn(t('stop'), 'btn-stop btn-sm', () => act(a.id, 'stop')));
                    if (!isRemote) {
                        const restartBtn = createBtn(t('restart'), 'btn-restart btn-sm', () => act(a.id, 'restart'));
                        restartBtn.disabled = a.enabled === false;
                        actionsCell.append(restartBtn);
                    }
                } else {
                    const startBtn = createBtn(t('start'), 'btn-start btn-sm', () => act(a.id, 'start'));
                    startBtn.disabled = a.enabled === false;
                    actionsCell.append(startBtn);
                }
            }

            const editBtn = createBtn(t('edit'), 'btn-edit btn-sm', () => showEdit(a.id));
            const logBtn = createBtn(t('log'), 'btn-log btn-sm', () => showLogs(a.id, a.name));
            const clearBtn = createBtn(t('clear'), 'btn-clear btn-sm', () => clearData(a.id, a.name));
            const delBtn = createBtn(t('delete'), 'btn-delete btn-sm', () => del(a.id, a.name));

            actionsCell.append(editBtn);
            if (a.panel_url) {
                const panelBtn = createBtn(t('configureProxy'), 'btn-visit btn-sm', () => window.open(a.panel_url, '_blank', 'noopener'));
                panelBtn.disabled = a.enabled === false;
                actionsCell.append(panelBtn);
            } else if (a.app_type !== 'desktop' && !isRemote && a.url) {
                actionsCell.append(createBtn(t('visit'), 'btn-visit btn-sm', () => copyAccessUrl(a.url)));
            }
            if (a.app_type !== 'cli' && !isRemote) {
                actionsCell.append(logBtn);
                if (a.can_clear_data !== false) actionsCell.append(clearBtn);
            }
            if (a.can_delete !== false) actionsCell.append(delBtn);
            fragment.appendChild(tr);
        });

        tb.innerHTML = '';
        tb.appendChild(fragment);
    } catch (e) {
        console.error('Load failed:', e);
    }
}

function capabilitySummaryLines(cap) {
    const lines = [];
    if (cap.tool_count) lines.push(`${t('capTools')}: ${cap.tool_count}`);
    if (cap.skill_count) lines.push(`${t('capSkills')}: ${cap.skill_count}`);
    if ((cap.capabilities || []).includes('cli_describe')) {
        lines.push(t('capDescribeOk'));
    } else if ((cap.diagnostics || []).some(d => String(d.code || '').startsWith('describe_'))) {
        lines.push(t('capDescribeFailed'));
    }
    return lines;
}

// ---------- Overview ----------

async function loadOverview() {
    try {
        const d = await fetchJson(`${CONSOLE_API}/overview`);
        overviewCache = d;
        lastOverviewAt = Date.now();
        renderOverviewTiles(d);
        renderQuickActions();
        updateOverviewMeta();
    } catch (e) {
        toast(t('fetchFailed') + e, 'err');
    }
}

function renderOverviewTiles(d) {
    const grid = document.getElementById('overview-grid');
    if (!grid) return;
    grid.innerHTML = '';

    const commit = d.build?.commit;
    const versionHelp = commit && commit !== 'unknown' ? commit.slice(0, 12) : '';

    const sessions = d.connections?.webrtc_sessions ?? 0;
    let sessionsHelp;
    if (sessions === 0) sessionsHelp = t('sessionsHelpNone');
    else if (sessions === 1) sessionsHelp = t('sessionsHelpOne');
    else sessionsHelp = t('sessionsHelpMany')(sessions);

    const tiles = [
        {
            label: t('tileVersion'),
            value: d.version || '—',
            help: versionHelp,
            tone: 'tone-info',
        },
        {
            label: t('tileDisplay'),
            value: `${d.display?.width || 0}×${d.display?.height || 0}`,
            help: t('tileFpsTarget')(d.runtime?.target_fps ?? 0),
            tone: 'tone-muted',
        },
        {
            label: t('tileSessions'),
            value: String(sessions),
            help: sessionsHelp,
            tone: sessions > 0 ? 'tone-ok' : 'tone-muted',
        },
    ];

    tiles.forEach(({ label, value, help, tone }) => {
        const tile = document.createElement('div');
        tile.className = `summary-tile ${tone}`;
        tile.innerHTML = `
            <div class="summary-label">${esc(label)}</div>
            <div class="summary-value">${esc(value)}</div>
            <div class="summary-help">${help ? `<span class="summary-dot"></span>${esc(help)}` : ''}</div>`;
        grid.appendChild(tile);
    });
}

function renderQuickActions() {
    const box = document.getElementById('overview-quick-actions');
    if (!box) return;
    const cards = [
        { jump: 'apps', icon: '⊞', label: t('quickApps'), desc: t('quickAppsDesc') },
        { jump: 'settings', icon: '⚙', label: t('quickSettings'), desc: t('quickSettingsDesc') },
    ];
    box.innerHTML = cards.map(c => `
        <button type="button" class="quick-card" data-jump="${esc(c.jump)}">
            <span class="quick-icon" aria-hidden="true">${esc(c.icon)}</span>
            <div>
                <strong>${esc(c.label)}</strong>
                <small>${esc(c.desc)}</small>
            </div>
        </button>`).join('');
}

function updateOverviewMeta() {
    const el = document.getElementById('overview-meta-text');
    if (!el) return;
    const rel = lastOverviewAt ? formatRelative(lastOverviewAt) : t('metaJustNow');
    el.textContent = `${t('metaLastRefresh')} ${rel}`;
}

function formatRelative(ms) {
    const sec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
    if (sec < 5) return t('metaJustNow');
    if (sec < 60) return `${sec}s`;
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min}m`;
    const hr = Math.floor(min / 60);
    return `${hr}h`;
}

function formatDuration(ms) {
    if (!ms || ms < 0) return '0s';
    if (ms < 1000) return `${Math.round(ms)}ms`;
    const s = Math.round(ms / 1000);
    if (s < 60) return `${s}s`;
    const m = Math.floor(s / 60), r = s % 60;
    return r ? `${m}m ${r}s` : `${m}m`;
}

function formatNumber(n) {
    if (!Number.isFinite(n)) return '0';
    if (n >= 1000) return n.toLocaleString();
    return String(n);
}

function pad2(n) { return String(n).padStart(2, '0'); }

// Absolute local datetime, e.g. "2026-05-23 14:08:31"
function formatDateTime(ms) {
    if (!ms) return '—';
    const d = new Date(ms);
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ` +
        `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

// Compact local date+time for cards, e.g. "05-23 14:08"
function formatDateTimeShort(ms) {
    if (!ms) return '—';
    const d = new Date(ms);
    return `${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

// Offset from a base, e.g. "+12.4s" / "+1m 03s"
function formatOffset(ms) {
    if (ms == null || ms < 0) return '+0s';
    if (ms < 1000) return `+${Math.round(ms)}ms`;
    const s = ms / 1000;
    if (s < 60) return `+${s.toFixed(1)}s`;
    const m = Math.floor(s / 60), r = Math.round(s % 60);
    return `+${m}m ${pad2(r)}s`;
}

// ---------- Settings ----------

async function loadSettings() {
    try {
        const d = await fetchJson(`${CONSOLE_API}/settings`);
        const current = d.saved || d.current || {};
        setValue('set-target-fps', current.target_fps ?? d.current?.target_fps ?? 60);
        setValue('set-video-bitrate', current.video_bitrate_kbps ?? d.current?.video_bitrate_kbps ?? 8000);
        setValue('set-audio-bitrate', current.audio_bitrate ?? d.current?.audio_bitrate ?? 128000);
        const binChk = document.getElementById('set-binary-clipboard');
        if (binChk) binChk.checked = !!(current.binary_clipboard_enabled ?? d.current?.binary_clipboard_enabled);
        snapshotDirtyGroup('settings');
        markDirty('settings', false);
    } catch (e) {
        toast(t('fetchFailed') + e, 'err');
    }
}

async function saveSettings() {
    const body = {
        target_fps: numberValue('set-target-fps'),
        video_bitrate_kbps: numberValue('set-video-bitrate'),
        audio_bitrate: numberValue('set-audio-bitrate'),
        binary_clipboard_enabled: document.getElementById('set-binary-clipboard').checked
    };
    await putJson(`${CONSOLE_API}/settings`, body);
    toast(t('settingsSaved'), 'ok');
    snapshotDirtyGroup('settings');
    markDirty('settings', false);
    loadOverview();
}

async function requestKeyframe() {
    await fetchJson(`${CONSOLE_API}/keyframe`, { method: 'POST' });
    toast(t('requestKeyframe'), 'ok');
}

// ---------- Agent config ----------

// ---------- Providers ----------

// ---------- Runs ----------

// ---------- Run detail ----------


function truncate(s, n) {
    s = String(s ?? '');
    return s.length > n ? s.slice(0, n) + '…' : s;
}

// Quiet refresh of the open detail overlay: re-fetch steps + report and
// re-render in place, preserving the user's filter, expanded rows and scroll.
// ---------- Misc ----------

async function fetchJson(url, options = {}) {
    const r = await fetch(url, { cache: 'no-store', ...options });
    const d = await parseJsonResponse(r);
    if (!r.ok || d.error) throw new Error(serverErrorText(d) || r.statusText);
    return d;
}

async function putJson(url, body) {
    const r = await fetch(url, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body)
    });
    const d = await parseJsonResponse(r);
    if (!r.ok || d.error) throw new Error(serverErrorText(d) || r.statusText);
    return d;
}

async function postJson(url, body, options = {}) {
    const r = await fetch(url, {
        ...options,
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body)
    });
    const d = await parseJsonResponse(r);
    if (!r.ok || d.error) throw new Error(serverErrorText(d) || r.statusText);
    return d;
}

async function parseJsonResponse(response) {
    const text = await response.text();
    if (!text) return {};
    try {
        return JSON.parse(text);
    } catch (e) {
        const message = response.ok
            ? `Invalid JSON response: ${text.slice(0, 160)}`
            : `${response.status} ${response.statusText}: ${text.slice(0, 160)}`;
        return { error: message };
    }
}

function setValue(id, value) {
    const el = document.getElementById(id);
    if (el) el.value = value ?? '';
}

function numberValue(id) {
    const value = Number(document.getElementById(id).value);
    return Number.isFinite(value) ? value : 0;
}

function createBtn(text, cls, onClick) {
    const b = document.createElement('button');
    b.className = 'btn ' + cls;
    b.textContent = text;
    b.addEventListener('click', onClick);
    return b;
}

// ---------- Dirty tracking ----------

const dirtySnapshots = new Map();

function snapshotDirtyGroup(group) {
    const snap = {};
    document.querySelectorAll(`[data-dirty-group="${group}"]`).forEach(el => {
        snap[el.id] = inputSignature(el);
    });
    dirtySnapshots.set(group, snap);
}

function inputSignature(el) {
    if (el.type === 'checkbox') return el.checked ? '1' : '0';
    return el.value ?? '';
}

function markDirty(group, isDirty) {
    const el = document.getElementById(group + '-dirty');
    if (el) el.hidden = !isDirty;
}

function reconcileDirty(group) {
    const snap = dirtySnapshots.get(group);
    if (!snap) return;
    let dirty = false;
    document.querySelectorAll(`[data-dirty-group="${group}"]`).forEach(el => {
        if (inputSignature(el) !== (snap[el.id] ?? '')) dirty = true;
    });
    markDirty(group, dirty);
}

function bindDirtyTracking() {
    ['settings'].forEach(group => {
        document.querySelectorAll(`[data-dirty-group="${group}"]`).forEach(el => {
            const evt = el.type === 'checkbox' || el.tagName === 'SELECT' ? 'change' : 'input';
            el.addEventListener(evt, () => reconcileDirty(group));
            if (el.type !== 'checkbox' && el.tagName !== 'SELECT') {
                el.addEventListener('change', () => reconcileDirty(group));
            }
        });
    });
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

// ---------- App dialog ----------

const APP_FIELDS = ['f-name', 'f-url', 'f-launch-command', 'f-launch-cwd', 'f-launch-timeout', 'f-launch-env',
    'f-exec', 'f-env', 'f-cli-binary', 'f-cli-env', 'f-skill-paths', 'f-remote-url', 'f-remote-password'];

// Clears per-open state so nothing leaks between apps.
function resetModal() {
    launchContext = null;
    document.getElementById('electron-no-sandbox').checked = false;
    renderLaunchContext();
    editId = null;
    editBuiltin = false;
    editApp = null;
    sharingState = null;
    appSignature = '';
    modalSaved = false;
    APP_FIELDS.forEach(id => setValue(id, ''));
    document.getElementById('f-autostart').checked = false;
    ['f-remote-password', 'f-share-password'].forEach(id => setPasswordVisible(id, false));
    document.getElementById('remote-plaintext-hint').hidden = true;
    setTestResult('', '');
    document.getElementById('sharing-section').hidden = true;
    document.getElementById('basic-title').hidden = true;
    document.getElementById('f-share-enabled').checked = false;
    setValue('f-share-password', '');
    setValue('f-share-url', '');
    showRestartNotice(null);
    clearFormErrors();
    document.getElementById('remote-test').disabled = false;
    document.getElementById('restart-now').disabled = false;
    setText('restart-now', 'restartNow');
}

function openAppDialog() {
    dialogs.close(false);
    resetModal();
    const modal = document.getElementById('modal');
    appDialog = dialogs.open(modal, () => {
        appTypeControl?.close();
        appDialog = null;
        modal.classList.remove('loading');
        modal.removeAttribute('aria-busy');
        resetModal();
    });
    loadLaunchContext(appDialog);
    return appDialog;
}

async function loadLaunchContext(session) {
    try {
        const context = await fetchJson(API + '/launch-context', { signal: session.abort.signal });
        if (!session.isCurrent()) return;
        launchContext = context;
    } catch (_) {
        if (!session.isCurrent()) return;
        launchContext = null;
    }
    renderLaunchContext();
}

function renderLaunchContext() {
    const identity = launchContext?.identity;
    setTextById('desktop-launch-context', identity
        ? t('launchIdentity')(identity) + (identity.uid === 0 ? t('launchRootHint') : '')
        : t('launchContextUnavailable'));
}

function showAdd() {
    openAppDialog();
    const modal = document.getElementById('modal');
    modal.querySelector('#modal-title').textContent = t('addModal');
    modal.querySelector('#modal-save').textContent = t('addSave');
    document.getElementById('f-app-type').value = 'background';
    updateAppTypeVisibility();
    appTypeControl?.sync();
    document.getElementById('f-name').focus();
}

async function showEdit(id) {
    const session = openAppDialog();
    session.kind = 'edit';
    const modal = document.getElementById('modal');
    modal.classList.add('loading');
    modal.setAttribute('aria-busy', 'true');
    modal.querySelector('#modal-title').textContent = t('editModal');
    document.getElementById('modal-save').disabled = true;
    try {
        // Reopening the same app waits for the previous submitted operation,
        // so it cannot read and overwrite an intermediate app/sharing state.
        if (pendingAppWrites.has(id)) await pendingAppWrites.get(id);
        if (!session.isCurrent()) return;
        const a = await fetchJson(API + '/' + id, { signal: session.abort.signal });
        if (!session.isCurrent()) return;
        editId = id;
        editApp = a;
        editBuiltin = a.builtin === true && a.managed_by === 'ivnc';
        modal.querySelector('#modal-title').textContent = t('editModal');
        modal.querySelector('#modal-save').textContent = t('editSave');

        const appType = a.app_type || 'background';
        document.getElementById('f-name').value = a.name;
        document.getElementById('f-app-type').value = appType;
        document.getElementById('f-autostart').checked = !!a.autostart;
        document.getElementById('f-skill-paths').value = listToText(a.skill_paths);

        if (appType === 'remote') {
            document.getElementById('f-remote-url').value = a.url || '';
            syncPlaintextHint();
        } else if (appType === 'desktop') {
            document.getElementById('f-exec').value = a.exec_command || '';
            document.getElementById('f-env').value = envToText(a.env_vars);
        } else if (appType === 'cli') {
            document.getElementById('f-cli-binary').value = a.cli_binary_path || '';
            document.getElementById('f-cli-env').value = envToText(a.cli_env_vars);
        } else {
            document.getElementById('f-url').value = a.url || '';
            document.getElementById('f-launch-command').value = a.launch_command || '';
            document.getElementById('f-launch-cwd').value = a.launch_cwd || '';
            document.getElementById('f-launch-timeout').value = a.launch_wait_timeout_secs || '';
            document.getElementById('f-launch-env').value = envToText(a.launch_env_vars);
        }
        updateAppTypeVisibility();
        appSignature = JSON.stringify(readAppBody(appType));
        appTypeControl?.sync();
        if (appType === 'desktop' && !editBuiltin) loadSharing(id, session);
    } catch (e) {
        if (session.isCurrent()) { hideModal(); toast(t('fetchFailed') + e.message, 'err'); }
    } finally {
        if (session.isCurrent()) {
            modal.classList.remove('loading'); modal.removeAttribute('aria-busy');
            dialogs.current?.element.querySelector('.modal').focus({ preventScroll: true });
        }
    }
}

function hideModal() {
    if (appDialog?.isCurrent()) dialogs.close();
}

function clearFormErrors() {
    document.querySelectorAll('#modal .field-error').forEach(el => { el.textContent = ''; });
    document.querySelectorAll('#modal .invalid').forEach(el => el.classList.remove('invalid'));
}

// Shows `message` under the field and focuses it. Returns false for validators.
function fieldError(id, message) {
    const input = document.getElementById(id);
    setTextById('err-' + id, message);
    if (input) {
        input.classList.add('invalid');
        input.focus();
    }
    return false;
}

function showModalError(message) {
    setTextById('modal-error', message);
}

function showRestartNotice(key) {
    const notice = document.getElementById('restart-notice');
    notice.hidden = !key;
    if (key) {
        notice.dataset.key = key;
        setText('restart-notice-text', key);
    } else {
        delete notice.dataset.key;
    }
}

function setPasswordVisible(id, visible) {
    const input = document.getElementById(id);
    if (input) input.type = visible ? 'text' : 'password';
    syncPasswordToggle(id);
}

function syncPasswordToggle(id) {
    const input = document.getElementById(id);
    const btn = document.querySelector(`[data-password-toggle="${id}"]`);
    if (input && btn) btn.textContent = t(input.type === 'password' ? 'show' : 'hide');
}

// Uniformly random from a 64-symbol alphabet: 24 symbols carry 144 bits.
function randomPassword() {
    const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_+=@#.';
    const bytes = crypto.getRandomValues(new Uint8Array(24));
    return Array.from(bytes, b => alphabet[b & 63]).join('');
}

function isHttpUrl(url) {
    try {
        const parsed = new URL(url);
        return parsed.protocol === 'http:' || parsed.protocol === 'https:';
    } catch {
        return false;
    }
}

function syncPlaintextHint() {
    const url = document.getElementById('f-remote-url').value.trim();
    document.getElementById('remote-plaintext-hint').hidden = !/^http:\/\//i.test(url);
}

function setTestResult(text, tone) {
    const el = document.getElementById('remote-test-result');
    el.textContent = text;
    el.classList.toggle('test-ok', tone === 'ok');
    el.classList.toggle('test-err', tone === 'err');
}

// ---------- Remote access (desktop apps) ----------

async function loadSharing(id, session = appDialog) {
    document.getElementById('sharing-section').hidden = false;
    document.getElementById('basic-title').hidden = false;
    document.getElementById('f-share-enabled').disabled = true;
    document.getElementById('sharing-fields').hidden = true;
    try {
        const data = await fetchJson(`${API}/${id}/sharing`, { signal: session.abort.signal });
        if (session.isCurrent()) renderSharing(data);
    } catch (e) {
        if (session.isCurrent()) setTextById('err-f-share-enabled', t('sharingLoadFailed') + e.message);
    }
}

function renderSharing(data) {
    sharingState = data;
    const enabled = document.getElementById('f-share-enabled');
    enabled.checked = !!data.enabled;
    enabled.disabled = false;
    const password = document.getElementById('f-share-password');
    // Legacy rows keep only a hash: the password exists but cannot be shown.
    password.value = data.password_viewable && data.password ? data.password : '';
    password.placeholder = data.password_set && !data.password_viewable ? t('sharePasswordLegacy') : '';
    setPasswordVisible('f-share-password', false);
    document.getElementById('f-share-url').value = new URL(data.share_path, location.origin).href;
    syncSharingFields();
    if (data.restart_required) showRestartNotice('restartPending');
}

function syncSharingFields() {
    document.getElementById('sharing-fields').hidden = !document.getElementById('f-share-enabled').checked;
}

function onSharingToggle() {
    const password = document.getElementById('f-share-password');
    setTextById('err-f-share-enabled', '');
    // First enable: offer a strong password the owner can copy or replace.
    if (this.checked && sharingState && !sharingState.password_set && !password.value) {
        password.value = randomPassword();
        setPasswordVisible('f-share-password', true);
    }
    syncSharingFields();
}

function byteLength(text) {
    return new TextEncoder().encode(text).length;
}

// The PUT /sharing body, null when nothing changed, false when invalid.
function readSharingChange() {
    if (!sharingState) return null;
    const enabled = document.getElementById('f-share-enabled').checked;
    const value = document.getElementById('f-share-password').value;
    const original = sharingState.password_viewable ? (sharingState.password || '') : '';
    const body = { enabled };
    if (enabled) {
        if (!value) {
            if (!sharingState.password_set || sharingState.password_viewable) {
                return fieldError('f-share-password', t('sharePasswordRequired'));
            }
        } else if (value !== original) {
            const length = byteLength(value);
            if (length < 8 || length > 256) return fieldError('f-share-password', t('sharePasswordLength'));
            body.password = value;
        }
    }
    if (enabled === !!sharingState.enabled && !body.password) return null;
    return body;
}

async function saveSharing(id, body) {
    return putJson(`${API}/${id}/sharing`, body);
}

async function copyShareUrl() {
    const session = appDialog;
    const btn = document.getElementById('share-url-copy');
    try {
        await copyText(document.getElementById('f-share-url').value);
        if (!session?.isCurrent()) return;
        btn.textContent = t('copied');
        setTimeout(() => { if (session.isCurrent()) setText('share-url-copy', 'copy'); }, 1500);
    } catch (e) {
        console.warn('Failed to copy share URL:', e);
        toast(t('copyFailed'), 'err');
    }
}

async function restartFromModal() {
    const session = appDialog;
    const id = editId;
    if (!id || !session?.isCurrent() || session.saving || session.restarting) return;
    session.restarting = true;
    const btn = document.getElementById('restart-now');
    btn.disabled = true;
    btn.textContent = t('restarting');
    try {
        await fetchJson(`${API}/${id}/restart`, { method: 'POST' });
        notifyAppStarted(id);
        if (session.isCurrent()) hideModal();
        toast(t('restarted'), 'ok');
        load();
    } catch (e) {
        if (session.isCurrent()) showModalError(t('actionFailed') + e.message);
        else toast(t('actionFailed') + e.message, 'err');
    } finally {
        session.restarting = false;
        if (session.isCurrent()) { btn.disabled = false; btn.textContent = t('restartNow'); }
    }
}

async function testRemote() {
    const session = appDialog;
    if (!session?.isCurrent() || session.testing) return;
    const btn = document.getElementById('remote-test');
    ['f-remote-url', 'f-remote-password'].forEach(id => {
        setTextById('err-' + id, '');
        document.getElementById(id).classList.remove('invalid');
    });
    const url = document.getElementById('f-remote-url').value.trim();
    const password = document.getElementById('f-remote-password').value;
    if (!url) return fieldError('f-remote-url', t('remoteUrlRequired'));
    if (!isHttpUrl(url)) return fieldError('f-remote-url', t('remoteUrlInvalid'));
    if (!password) return fieldError('f-remote-password', t('remotePasswordRequired'));
    session.testing = true;
    const stillRelevant = () => session.isCurrent() && document.getElementById('f-app-type').value === 'remote'
        && document.getElementById('f-remote-url').value.trim() === url && document.getElementById('f-remote-password').value === password;
    btn.disabled = true;
    setTestResult(t('remoteTesting'), '');
    try {
        const data = await postJson('/api/remote/test', { url, password }, { signal: session.abort.signal });
        if (!stillRelevant()) return;
        setTestResult(testResultText(data), 'ok');
        const name = document.getElementById('f-name');
        if (!name.value.trim() && data.name) name.value = data.name;
    } catch (e) {
        if (stillRelevant()) setTestResult(e.message, 'err');
    } finally {
        session.testing = false;
        if (session.isCurrent()) btn.disabled = false;
    }
}

// B's /info answer: status is the app status there, ready is false while
// its runtime does not match its sharing setting.
function testResultText(data) {
    if (data.status === 'starting') return `${data.name} · ${remoteStateText('starting')}`;
    if (data.status !== 'running') return t('remoteTestStopped')(data.name);
    if (data.ready === false) return `${data.name} · ${remoteStateText('not_ready')}`;
    return t('remoteTestRunning')(data.name);
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

function listToText(values) {
    return Array.isArray(values) ? values.join('\n') : '';
}

function parseListText(text) {
    const items = text.split('\n').map(line => line.trim()).filter(Boolean);
    return items.length ? items : null;
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

function readAppBody(appType) {
    const body = {
        app_type: appType,
        autostart: appType !== 'cli' && document.getElementById('f-autostart').checked,
        skill_paths: parseListText(document.getElementById('f-skill-paths').value)
    };

    const name = document.getElementById('f-name').value.trim();
    if (!editId || (appType === 'remote' && name !== editApp?.name)) {
        body.name = name;
        if (editId) body.name_revision = editApp.name_revision;
    }

    if (appType === 'remote') {
        body.url = document.getElementById('f-remote-url').value.trim();
        body.password = document.getElementById('f-remote-password').value;
    } else if (appType === 'desktop') {
        body.exec_command = document.getElementById('f-exec').value.trim();
        body.env_vars = parseEnvText(document.getElementById('f-env').value);
    } else if (appType === 'cli') {
        body.cli_binary_path = document.getElementById('f-cli-binary').value.trim();
        body.cli_env_vars = parseEnvText(document.getElementById('f-cli-env').value);
    } else {
        body.launch_command = document.getElementById('f-launch-command').value.trim() || null;
        body.launch_cwd = document.getElementById('f-launch-cwd').value.trim() || null;
        const launchTimeout = document.getElementById('f-launch-timeout').value;
        body.launch_wait_timeout_secs = launchTimeout ? parseInt(launchTimeout, 10) : null;
        body.url = document.getElementById('f-url').value.trim() || null;
        body.launch_env_vars = parseEnvText(document.getElementById('f-launch-env').value);
    }
    return body;
}

function validateAppBody(body) {
    // A remote app without a name takes the name reported by the remote side.
    if (!editId && body.app_type !== 'remote' && !body.name) return fieldError('f-name', t('missingName'));
    if (body.app_type === 'remote') {
        if (!body.url) return fieldError('f-remote-url', t('remoteUrlRequired'));
        if (!isHttpUrl(body.url)) return fieldError('f-remote-url', t('remoteUrlInvalid'));
        if (!editId && !body.password) return fieldError('f-remote-password', t('remotePasswordRequired'));
        if (editId && !body.password && body.url !== (editApp?.url || '')) {
            return fieldError('f-remote-password', t('remotePasswordForUrl'));
        }
    } else if (body.app_type === 'desktop') {
        if (!body.exec_command) return fieldError('f-exec', t('missingExec'));
    } else if (body.app_type === 'cli') {
        if (!body.cli_binary_path) return fieldError('f-cli-binary', t('missingCliBinary'));
    } else {
        if (!body.launch_command) return fieldError('f-launch-command', t('missingLaunchCommand'));
        if (body.url && !validateUrl(body.url)) return fieldError('f-url', t('invalidUrl'));
    }
    return true;
}

async function saveApp() {
    const session = appDialog;
    if (!session?.isCurrent() || session.saving || session.restarting || editBuiltin
        || session.element.classList.contains('loading')) return;
    clearFormErrors();
    const appType = document.getElementById('f-app-type').value;
    const body = readAppBody(appType);
    if (!validateAppBody(body)) return;
    const sharing = editId && appType === 'desktop' ? readSharingChange() : null;
    if (sharing === false) return;
    if (sharing?.password && sharingState.enabled && sharingState.password_set && !confirm(t('confirmPasswordChange'))) return;
    const snapshot = { id: editId, app: editApp && { ...editApp }, signature: appSignature,
        sharing: sharingState && { ...sharingState }, wasEditing: !!editId };
    const btn = document.getElementById('modal-save');
    session.saving = true;
    btn.disabled = true; btn.textContent = t('savePending');
    const report = message => {
        if (session.isCurrent()) showModalError(message);
        else toast(`${body.name || snapshot.app?.name || ''}: ${message}`, 'err');
    };
    const operation = Promise.resolve().then(async () => {
        try {
            const r = await fetch(snapshot.id ? API + '/' + snapshot.id : API, {
                method: snapshot.id ? 'PUT' : 'POST',
                headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body)
            });
            const d = await parseJsonResponse(r);
            if (!r.ok || d.error) {
                if (d.code === 'app_name_conflict' && snapshot.app && Number.isSafeInteger(d.params?.name_revision)) {
                    const app = { ...snapshot.app, name: d.params.name, name_revision: d.params.name_revision };
                    latestNames.set(snapshot.id, { name: app.name, name_revision: app.name_revision });
                    if (session.isCurrent()) editApp = app;
                    load();
                }
                report(serverErrorText(d) || r.statusText);
                return;
            }
            if (d.id && Number.isSafeInteger(d.name_revision)) orderNotifications.publishName(d.id, d.name_revision);
            const signature = JSON.stringify(body);
            let restart = snapshot.wasEditing && !['cli', 'remote'].includes(appType)
                && signature !== snapshot.signature && isActive(snapshot.app);
            let revoked = false;
            if (session.isCurrent()) {
                appSignature = signature;
                if (d.id) editApp = { ...editApp, ...d };
            }
            if (sharing) {
                try {
                    const result = await saveSharing(snapshot.id, sharing);
                    revoked = !!result.revoked;
                    restart = restart || !!result.restart_required;
                    if (session.isCurrent()) {
                        sharingState = { ...snapshot.sharing, enabled: sharing.enabled,
                            password_set: snapshot.sharing.password_set || !!sharing.password,
                            password: sharing.password ?? snapshot.sharing.password,
                            password_viewable: sharing.password ? true : snapshot.sharing.password_viewable,
                            restart_required: !!result.restart_required };
                    }
                } catch (e) {
                    if (session.isCurrent()) {
                        modalSaved = true; setText('modal-cancel', 'close');
                        setTextById('err-f-share-enabled', e.message);
                    }
                    // The application write has succeeded, even if sharing failed.
                    toast(`${body.name || snapshot.app?.name || ''}: ${t('updated')} · ${e.message}`, 'err');
                    load(); return;
                }
            }
            load();
            if (session.isCurrent()) {
                if (restart) { modalSaved = true; setText('modal-cancel', 'close'); showRestartNotice('restartSaved'); }
                else hideModal();
            }
            toast(revoked ? t('sharingRevoked') : snapshot.wasEditing ? t('updated') : t('added'), 'ok');
        } catch (e) {
            report(t('actionFailed') + e.message);
        } finally {
            session.saving = false;
            if (snapshot.id && pendingAppWrites.get(snapshot.id) === operation) pendingAppWrites.delete(snapshot.id);
            if (session.isCurrent()) {
                btn.disabled = editBuiltin;
                btn.textContent = t(snapshot.wasEditing ? 'editSave' : 'addSave');
            }
        }
    });
    if (snapshot.id) pendingAppWrites.set(snapshot.id, operation);
    await operation;
}

async function act(id, action) {
    try {
        const r = await fetch(`${API}/${id}/${action}`, { method: 'POST' });
        const d = await r.json();
        if (d.error) {
            toast(serverErrorText(d), 'err');
            return;
        }
        toast(action === 'start' ? t('started') : action === 'stop' ? t('stoppedToast') : t('restarted'), 'ok');
        if (action === 'start' || action === 'restart') notifyAppStarted(id);
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
            toast(serverErrorText(d), 'err');
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
            toast(serverErrorText(d), 'err');
            return;
        }
        toast(t('cleared'), 'ok');
        load();
    } catch (e) {
        toast(t('clearFailed'), 'err');
    }
}

function toast(msg, type = 'info') {
    const tEl = document.getElementById('toast');
    if (!tEl) return;
    tEl.textContent = msg;
    tEl.className = 'toast show toast-' + type;
    setTimeout(() => tEl.classList.remove('show'), 2500);
}

async function showLogs(id, name) {
    const modal = document.getElementById('log-modal');
    const title = document.getElementById('log-title');
    const session = dialogs.open(modal, () => {
        delete title.dataset.appName;
        title.textContent = t('logsTitle');
    });
    title.dataset.appName = name;
    title.textContent = t('logsFor')(name);
    const content = document.getElementById('log-content');
    content.textContent = t('logLoading');
    try {
        const d = await fetchJson(`${API}/${id}/logs`, { signal: session.abort.signal });
        if (!session.isCurrent()) return;
        content.textContent = d.logs || '(empty)';
        content.scrollTop = content.scrollHeight;
    } catch (e) {
        if (session.isCurrent()) content.textContent = t('logLoadFailed') + e.message;
    }
}

function hideLogModal() {
    if (dialogs.current?.element.id === 'log-modal') dialogs.close();
}

function updateAppTypeVisibility() {
    const type = document.getElementById('f-app-type').value;
    const autostartControl = document.getElementById('autostart-control');
    const generalSlot = document.getElementById('general-autostart-slot');
    const backgroundSlot = document.getElementById('background-autostart-slot');
    const desktopSlot = document.getElementById('desktop-autostart-slot');

    document.getElementById('background-config').style.display = type === 'background' ? 'block' : 'none';
    document.getElementById('desktop-config').style.display = type === 'desktop' ? 'block' : 'none';
    document.getElementById('cli-config').style.display = type === 'cli' ? 'block' : 'none';

    autostartControl.classList.remove('inline-slot');
    if (type === 'desktop') {
        desktopSlot.appendChild(autostartControl);
        autostartControl.classList.add('inline-slot');
    } else if (type === 'background') {
        backgroundSlot.appendChild(autostartControl);
        autostartControl.classList.add('inline-slot');
    } else {
        generalSlot.appendChild(autostartControl);
    }

    // A remote app with autostart is started (connected) when iVnc starts.
    autostartControl.style.display = type === 'cli' ? 'none' : '';
    document.getElementById('remote-config').hidden = type !== 'remote';
    document.getElementById('f-skill-paths').closest('.form-line').hidden = type === 'remote';
    document.getElementById('name-required-mark').hidden = type === 'remote';
    document.getElementById('remote-password-required-mark').hidden = !!editId;
    if (type !== 'desktop' || !editId || editBuiltin) {
        document.getElementById('sharing-section').hidden = true;
        document.getElementById('basic-title').hidden = true;
    }
    updateTypePlaceholders();

    if (!autostartControl.parentElement) {
        generalSlot.appendChild(autostartControl);
    }

    const modal = document.getElementById('modal');
    modal.querySelectorAll('input:not([type="checkbox"]):not([data-fixed-readonly]), textarea').forEach(field => {
        field.readOnly = editBuiltin;
    });
    document.getElementById('f-name').disabled = !!editId && !editBuiltin && type !== 'remote';
    document.getElementById('f-app-type').disabled = !!editId;
    document.getElementById('f-autostart').disabled = editBuiltin;
    document.getElementById('modal-save').hidden = editBuiltin;
    document.getElementById('modal-save').disabled = editBuiltin || !!appDialog?.saving;
    document.getElementById('modal-cancel').textContent = t(editBuiltin || modalSaved ? 'close' : 'cancel');
    appTypeControl?.sync();
    updateElectronTemplate();
}

function updateElectronTemplate() {
    const disabled = editBuiltin || !!appDialog?.saving || !!document.getElementById('f-exec').value.trim();
    document.getElementById('electron-template').disabled = disabled;
    document.getElementById('electron-no-sandbox').disabled = disabled;
}

// Placeholders that depend on the selected type and on add vs. edit.
function updateTypePlaceholders() {
    const type = document.getElementById('f-app-type').value;
    setPlaceholderById('f-name', t(type === 'remote' ? 'remoteNamePlaceholder' : 'nameInputPlaceholder'));
    setPlaceholderById('f-remote-password', t(editId ? 'remotePasswordKeep' : 'remotePasswordPlaceholder'));
}


document.addEventListener('DOMContentLoaded', () => {
    document.body.dataset.section = activeSection;
    appTypeControl = enhanceSelect(document.getElementById('f-app-type'), document.getElementById('label-app-type'));
    document.getElementById('nav-overview').setAttribute('aria-current', 'page');
    document.querySelectorAll('.nav-item').forEach(btn => {
        btn.addEventListener('click', () => switchSection(btn.dataset.section));
    });
    document.getElementById('add-app-btn').addEventListener('click', showAdd);
    document.getElementById('f-exec').addEventListener('input', updateElectronTemplate);
    document.getElementById('electron-template').addEventListener('click', () => {
        const command = document.getElementById('f-exec');
        if (command.value.trim() || editBuiltin || appDialog?.saving) return;
        command.value = '/path/to/electron-app --ozone-platform=wayland --enable-wayland-ime --wayland-text-input-version=3';
        if (document.getElementById('electron-no-sandbox').checked) command.value += ' --no-sandbox';
        command.dispatchEvent(new Event('input', { bubbles: true }));
        command.focus();
        command.setSelectionRange(0, '/path/to/electron-app'.length);
    });
    document.getElementById('lang-toggle').addEventListener('click', () => {
        setLang(currentLang === 'en' ? 'zh' : 'en');
    });
    onLangChange((lang) => {
        currentLang = lang;
        lastDataHash = '';
        try {
            applyTranslations();
        } catch (e) {
            console.error('[console] applyTranslations failed:', e);
        }
        loadSection();
    });
    document.getElementById('f-app-type').addEventListener('change', () => {
        clearFormErrors();
        updateAppTypeVisibility();
    });
    document.getElementById('modal-close').addEventListener('click', hideModal);
    document.getElementById('modal-cancel').addEventListener('click', hideModal);
    document.getElementById('log-close-btn').addEventListener('click', hideLogModal);
    // Editing a field clears its inline error.
    document.getElementById('modal').addEventListener('input', (e) => {
        if (!e.target.classList?.contains('invalid')) return;
        e.target.classList.remove('invalid');
        setTextById('err-' + e.target.id, '');
    });
    document.querySelectorAll('[data-password-toggle]').forEach(btn => {
        btn.addEventListener('click', () => {
            const id = btn.dataset.passwordToggle;
            setPasswordVisible(id, document.getElementById(id).type === 'password');
        });
    });
    document.getElementById('f-share-enabled').addEventListener('change', onSharingToggle);
    document.getElementById('share-password-generate').addEventListener('click', () => {
        setValue('f-share-password', randomPassword());
        setPasswordVisible('f-share-password', true);
        document.getElementById('f-share-password').classList.remove('invalid');
        setTextById('err-f-share-password', '');
    });
    document.getElementById('share-url-copy').addEventListener('click', copyShareUrl);
    document.getElementById('restart-now').addEventListener('click', restartFromModal);
    document.getElementById('f-remote-url').addEventListener('input', () => {
        syncPlaintextHint();
        setTestResult('', '');
    });
    document.getElementById('f-remote-password').addEventListener('input', () => setTestResult('', ''));
    document.getElementById('remote-test').addEventListener('click', testRemote);

    document.getElementById('modal-save').addEventListener('click', saveApp);
    document.getElementById('settings-save').addEventListener('click', () => saveSettings().catch(e => toast(t('actionFailed') + e, 'err')));
    document.getElementById('request-keyframe').addEventListener('click', () => requestKeyframe().catch(e => toast(t('actionFailed') + e, 'err')));
    // Quick action / overview jump buttons
    document.addEventListener('click', (e) => {
        const btn = e.target.closest('[data-jump]');
        if (btn) switchSection(btn.dataset.jump);
    });

    bindDirtyTracking();
    try {
        applyTranslations();
    } catch (e) {
        console.error('[console] applyTranslations failed:', e);
    }
    loadSection();
    setInterval(() => {
        if (activeSection === 'apps') load();
        if (activeSection === 'overview') loadOverview();
    }, 5000);
    setInterval(() => {
        if (activeSection === 'overview') updateOverviewMeta();
    }, 1000);
});
