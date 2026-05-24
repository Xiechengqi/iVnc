import { getLang, setLang, onLangChange } from './lib/i18n.js?v=1';

const API = '/api/apps';
const CONSOLE_API = '/api/console';
let editId = null;
let lastDataHash = '';
let currentLang = getLang();
let activeSection = 'overview';
let overviewCache = null;
let providersCache = [];
let agentDefaultProvider = null;
let runsCache = [];
let runsFilter = 'all';
let runsSearchTerm = '';
let lastOverviewAt = 0;
let providerOriginalSettings = null;
let apiKeyMode = 'stored';

const I18N = {
    en: {
        pageTitle: 'Management Console',
        subtitle: 'Manage apps, runtime settings, and Agent',
        navOverview: 'Overview',
        navApps: 'Apps',
        navSettings: 'Connection',
        navAgent: 'MCP / Agent',
        navProviders: 'Provider',
        navRuns: 'Logs / Trajectories',
        navSchedules: 'Schedules',
        sectionOverviewDesc: 'Current iVnc status and quick checks.',
        sectionAppsDesc: 'Manage desktop and background applications.',
        sectionSettingsDesc: 'Common runtime controls for stream quality.',
        sectionAgentDesc: 'Default parameters for MCP Agent runs.',
        sectionProvidersDesc: 'Provider endpoint, model, and credential settings.',
        sectionRunsDesc: 'Recent Agent runs and trajectory paths.',
        save: 'Save',
        saveApply: 'Save & Apply',
        refresh: 'Refresh',
        reset: 'Reset',
        stopAgent: 'Stop Agent',
        providerStatus: 'Provider Status',
        agentStatus: 'Agent Status',
        requestKeyframe: 'Request Keyframe',
        paramTargetFps: 'Target FPS',
        paramTargetFpsHelp: 'Target video frame rate. Higher values are smoother but increase encoding and network load.',
        paramVideoBitrate: 'Video bitrate',
        paramVideoBitrateHelp: 'Video encoding bitrate. Higher values improve clarity but use more bandwidth.',
        paramAudioBitrate: 'Audio bitrate',
        paramAudioBitrateHelp: 'Audio encoding bitrate. 128000 is a common default.',
        paramBinaryClipboard: 'Binary clipboard',
        paramBinaryClipboardHelp: 'Allow clipboard sync for binary content such as images or file fragments.',
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
        keepApiKeyPlaceholder: 'Leave empty to keep unchanged',
        enterNewApiKey: 'Enter new API Key',
        agentDefaults: 'Agent Defaults',
        providerConfig: 'Provider Config',
        saved: 'Saved',
        providerSaved: 'Provider saved',
        testButton: 'Test',
        testRunning: 'Testing…',
        testOk: 'OK',
        testFailed: 'Failed:',
        testSkipped: 'No network test for this provider',
        settingsSaved: 'Settings applied',
        agentSaved: 'Agent defaults saved',
        noRuns: 'No Agent runs yet.',
        noRunsHint: 'Runs started from MCP or this console will appear here.',
        noRunsMatch: 'No runs match the current filter.',
        configured: 'Configured',
        missingConfig: 'Missing config',
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

        // overview groups
        settingsStreamTitle: 'Stream Parameters',
        settingsActionsTitle: 'Instant Actions',
        settingsActionsHint: 'Applied immediately, no save required',
        settingsHint: 'Changes apply instantly and persist as next-startup defaults.',
        agentBudgetTitle: 'Run Budget',
        agentBudgetHint: 'Caps the resources spent on a single Agent run',
        agentBehaviorTitle: 'Provider & Behavior',
        agentSaveHint: 'Defaults are used by subsequent runs started via MCP or this console.',
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
        overviewAgentRaw: 'Raw data',
        overviewJumpProviders: 'Configure →',
        // overview tiles
        tileVersion: 'Version',
        tileDisplay: 'Display',
        tileSessions: 'Sessions',
        tileAgent: 'Agent',
        tileFpsTarget: (fps) => `target ${fps} FPS`,
        tileWebRTC: 'WebRTC',
        tileMcpAgentAll: 'mcp · agent-all',
        tileMcpAgent: 'mcp · agent',
        tileNoFeatures: 'core only',
        tileExclusive: 'exclusive',
        tileIdle: 'idle',
        tileStopRequested: 'stop requested',
        tileRunning: 'running',
        sessionsHelpNone: 'no viewer connected',
        sessionsHelpOne: '1 viewer',
        sessionsHelpMany: (n) => `${n} viewers`,
        // agent card
        agentTaskLabel: 'Current task',
        agentNoTaskRunning: 'Agent is running without a task summary.',
        agentNoTaskIdle: 'No active task. Start one via MCP or the Agent tab.',
        agentMetricRun: 'Run ID',
        agentMetricExclusive: 'Exclusive',
        agentMetricStop: 'Stop',
        agentStateRunning: 'Running',
        agentStateIdle: 'Idle',
        agentStateExclusive: 'Exclusive',
        agentStateStopping: 'Stopping',
        agentStateDisabled: 'Disabled',
        yes: 'Yes',
        no: 'No',
        // quick actions
        quickActionsTitle: 'Quick Actions',
        quickApps: 'Apps',
        quickAppsDesc: 'Add or manage desktop & background apps',
        quickSettings: 'Connection',
        quickSettingsDesc: 'Tune FPS, bitrate, clipboard',
        quickAgent: 'MCP / Agent',
        quickAgentDesc: 'Run budgets and Agent defaults',
        quickRuns: 'Logs / Trajectories',
        quickRunsDesc: 'Inspect recent Agent runs',
        // provider list
        providerEmptyText: 'Pick a Provider on the left to view or edit its configuration.',
        providerDefaultBadge: 'Default',
        providerEndpointHintDefault: (v) => `Default: ${v}`,
        providerModelHintDefault: (v) => `Default: ${v}`,
        providerNoneConfigured: 'No Provider configured yet. Set one up to enable Agent.',
        apiKeyStatusSaved: 'Saved',
        apiKeyStatusEmpty: 'Not set',
        apiKeyReplace: 'Replace',
        apiKeyClear: 'Clear',
        apiKeyCancel: 'Cancel',
        apiKeyUndoClear: 'Undo',
        apiKeyPendingText: 'Saved API Key will be cleared on save',
        // runs
        runsSearchPlaceholder: 'Search by run ID or task',
        runsFilterAll: 'All',
        runState_running: 'Running',
        runState_done: 'Done',
        runState_failed: 'Failed',
        runState_interrupted: 'Interrupted',
        runState_ask: 'Awaiting Answer',
        runState_safety: 'Safety',
        runState_budget: 'Budget Exceeded',
        runState_max_steps: 'Max Steps',
        runState_provider_error: 'Provider Error',
        runsMetricSteps: 'steps',
        runsMetricTokens: 'tokens',
        runsCopyId: 'Copy ID',
        runsCopyPath: 'Copy trajectory path',
        runsNoPath: 'No trajectory saved',
        runsIdCopied: 'Run ID copied',
        runsPathCopied: 'Trajectory path copied',
        copyFailed: 'Copy failed',
        emptyRunsTitle: 'No runs yet',
        emptyRunsFilterTitle: 'No matching runs',
        runsStartedAt: 'started',
        runDetailBack: 'Back',
        runDetailLoading: 'Loading trajectory…',
        runDetailEmpty: 'No step records on disk',
        runDetailEmptyHint: 'Trajectory file not found or empty. Enable record_trajectory to capture steps.',
        runDetailStarted: 'Started',
        runDetailFinished: 'Finished',
        runDetailDuration: 'Duration',
        runDetailSteps: 'Steps',
        runDetailTokens: 'Tokens (in / out)',
        runDetailCost: 'Cost',
        runDetailReason: 'Result',
        resultTitle: 'Result',
        resultCopy: 'Copy',
        resultCopied: 'Result copied',
        resultEmpty: 'No text result (task completed)',
        resultFailed: 'Not completed',
        resultRunning: 'Task in progress…',
        resultWarnings: 'Warnings',
        badgeScheduled: 'Scheduled',
        newTaskTitle: 'New task',
        newTaskHint: 'Launch one agent run',
        newTaskProvider: 'Provider',
        newTaskMaxSteps: 'Max steps',
        newTaskMaxWall: 'Max wall (s)',
        newTaskStart: 'Start',
        newTaskStarting: 'Starting…',
        newTaskPlaceholder: 'Describe the task for the agent',
        newTaskUseDefault: 'Use default',
        newTaskTaskRequired: 'Task is required',
        newTaskProviderRequired: 'Provider is required',
        newTaskStarted: 'Task started',
        newTaskAlreadyActive: 'A run is already active',
        newTaskFailed: 'Failed to start: ',
        newTaskRunning: 'A run is already in progress — stop it first',
        schedulesTitle: 'Schedules',
        schedulesHint: 'Five-field cron in server local timezone. Overlapping fires are skipped.',
        schedulesEmpty: 'No schedules yet.',
        schedulesNew: 'New',
        scheduleEditTitle: 'Edit schedule',
        scheduleEditNewTitle: 'New schedule',
        scheduleName: 'Name',
        scheduleCron: 'cron (5 fields)',
        scheduleTask: 'Task',
        scheduleProvider: 'Provider',
        scheduleMaxSteps: 'Max steps',
        scheduleMaxWall: 'Max wall (s)',
        scheduleEnabled: 'Enabled',
        scheduleSave: 'Save',
        scheduleCancel: 'Cancel',
        scheduleEdit: 'Edit',
        scheduleDelete: 'Delete',
        scheduleRunNow: 'Run now',
        scheduleNextRun: 'Next',
        scheduleLastRun: 'Last',
        scheduleNever: 'never',
        scheduleSaved: 'Schedule saved',
        scheduleDeleted: 'Schedule deleted',
        scheduleRunStarted: 'Schedule fired',
        scheduleRunFailed: 'Failed to fire: ',
        scheduleSaveFailed: 'Save failed: ',
        scheduleDeleteFailed: 'Delete failed: ',
        scheduleDeleteConfirm: 'Delete this schedule?',
        scheduleOutcomeStarted: 'started',
        scheduleOutcomeSkipped: 'skipped',
        scheduleOutcomeFailed: 'failed',
        runDetailFilterAll: 'All actions',
        stepLatency: 'latency',
        stepGap: 'gap',
        stepTokens: 'tokens',
        stepCost: 'cost',
        stepResultOk: 'ok',
        stepCopyJson: 'Copy JSON',
        stepJsonCopied: 'Step JSON copied',
        stepFrame: 'screenshot',
        stepNoFrame: 'no screenshot',
        stepCumulative: 'cumulative',
    },
    zh: {
        pageTitle: '管理控制台',
        subtitle: '管理应用、运行参数和 Agent',
        navOverview: '概览',
        navApps: '应用',
        navSettings: '连接质量',
        navAgent: 'MCP / Agent',
        navProviders: 'Provider',
        navRuns: '日志 / 轨迹',
        navSchedules: '定时任务',
        sectionOverviewDesc: '当前 iVnc 运行状态和快捷入口。',
        sectionAppsDesc: '管理桌面应用和后台应用。',
        sectionSettingsDesc: '调整常用的实时串流质量参数。',
        sectionAgentDesc: '配置 MCP Agent run 的默认参数。',
        sectionProvidersDesc: '配置 Provider 的 endpoint、模型和凭据。',
        sectionRunsDesc: '查看最近 Agent run 和轨迹路径。',
        save: '保存',
        saveApply: '保存并应用',
        refresh: '刷新',
        reset: '重置',
        stopAgent: '停止 Agent',
        providerStatus: 'Provider 状态',
        agentStatus: 'Agent 状态',
        requestKeyframe: '请求关键帧',
        paramTargetFps: 'Target FPS',
        paramTargetFpsHelp: '目标画面帧率，越高越流畅但会增加编码和网络压力。',
        paramVideoBitrate: '视频码率',
        paramVideoBitrateHelp: '视频码率，越高越清晰但占用更多带宽。',
        paramAudioBitrate: '音频码率',
        paramAudioBitrateHelp: '音频编码码率，常用值为 128000。',
        paramBinaryClipboard: '二进制剪贴板',
        paramBinaryClipboardHelp: '允许剪贴板同步二进制内容，例如图片或文件片段。',
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
        keepApiKeyPlaceholder: '保持为空表示不修改',
        enterNewApiKey: '输入新的 API Key',
        agentDefaults: 'Agent 默认参数',
        providerConfig: 'Provider 配置',
        saved: '已保存',
        providerSaved: 'Provider 已保存',
        testButton: '测试',
        testRunning: '测试中…',
        testOk: '通过',
        testFailed: '失败：',
        testSkipped: '该 Provider 无需网络测试',
        settingsSaved: '设置已应用',
        agentSaved: 'Agent 默认参数已保存',
        noRuns: '暂无 Agent run。',
        noRunsHint: '通过 MCP 或控制台启动的 Agent run 会出现在这里。',
        noRunsMatch: '当前筛选下没有匹配的 run。',
        configured: '已配置',
        missingConfig: '未配置',
        langToggle: 'English',
        addApp: '+ 添加应用',
        thName: '名称',
        thType: '类型',
        thConfig: '配置',
        thStatus: '状态',
        thData: '数据',
        thActions: '操作',
        loading: '加载中...',
        empty: '暂无应用，点击"添加应用"创建。',
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

        // overview groups
        settingsStreamTitle: '串流参数',
        settingsActionsTitle: '即时操作',
        settingsActionsHint: '立即生效，无需保存',
        settingsHint: '改动会立即应用并保存为下次启动的默认值。',
        agentBudgetTitle: '运行限额',
        agentBudgetHint: '控制单次 Agent run 的最大资源消耗',
        agentBehaviorTitle: 'Provider 与行为',
        agentSaveHint: '默认值会用于后续通过 MCP 或控制台启动的 Agent run。',
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
        overviewAgentRaw: '原始数据',
        overviewJumpProviders: '配置 →',
        // overview tiles
        tileVersion: '版本',
        tileDisplay: '画面',
        tileSessions: '连接',
        tileAgent: 'Agent',
        tileFpsTarget: (fps) => `目标 ${fps} FPS`,
        tileWebRTC: 'WebRTC',
        tileMcpAgentAll: 'mcp · agent-all',
        tileMcpAgent: 'mcp · agent',
        tileNoFeatures: '仅核心',
        tileExclusive: '独占',
        tileIdle: '空闲',
        tileStopRequested: '请求停止',
        tileRunning: '运行中',
        sessionsHelpNone: '当前无观看者',
        sessionsHelpOne: '1 位观看者',
        sessionsHelpMany: (n) => `${n} 位观看者`,
        // agent card
        agentTaskLabel: '当前任务',
        agentNoTaskRunning: 'Agent 正在运行（无任务摘要）',
        agentNoTaskIdle: '当前无任务。通过 MCP 或 Agent 页发起新任务。',
        agentMetricRun: 'Run ID',
        agentMetricExclusive: '独占模式',
        agentMetricStop: '停止请求',
        agentStateRunning: '运行中',
        agentStateIdle: '空闲',
        agentStateExclusive: '独占',
        agentStateStopping: '停止中',
        agentStateDisabled: '未启用',
        yes: '是',
        no: '否',
        // quick actions
        quickActionsTitle: '快捷入口',
        quickApps: '应用',
        quickAppsDesc: '添加或管理桌面应用与后台应用',
        quickSettings: '连接质量',
        quickSettingsDesc: '调整 FPS、码率与剪贴板',
        quickAgent: 'MCP / Agent',
        quickAgentDesc: '运行限额与 Agent 默认参数',
        quickRuns: '日志 / 轨迹',
        quickRunsDesc: '查看最近的 Agent run',
        // provider list
        providerEmptyText: '从左侧选择一个 Provider 查看或编辑配置',
        providerDefaultBadge: '默认',
        providerEndpointHintDefault: (v) => `默认值：${v}`,
        providerModelHintDefault: (v) => `默认值：${v}`,
        providerNoneConfigured: '暂无已配置的 Provider，配置一个以启用 Agent。',
        apiKeyStatusSaved: '已保存',
        apiKeyStatusEmpty: '未设置',
        apiKeyReplace: '替换',
        apiKeyClear: '清除',
        apiKeyCancel: '取消',
        apiKeyUndoClear: '撤销',
        apiKeyPendingText: '将在保存时清除已存 API Key',
        // runs
        runsSearchPlaceholder: '按 run ID 或任务搜索',
        runsFilterAll: '全部',
        runState_running: '运行中',
        runState_done: '完成',
        runState_failed: '失败',
        runState_interrupted: '已中断',
        runState_ask: '等待回答',
        runState_safety: '安全检查',
        runState_budget: '超出预算',
        runState_max_steps: '达到步数上限',
        runState_provider_error: 'Provider 错误',
        runsMetricSteps: '步',
        runsMetricTokens: 'tokens',
        runsCopyId: '复制 ID',
        runsCopyPath: '复制轨迹路径',
        runsNoPath: '未保存轨迹',
        runsIdCopied: 'Run ID 已复制',
        runsPathCopied: '轨迹路径已复制',
        copyFailed: '复制失败',
        emptyRunsTitle: '暂无 Agent run',
        emptyRunsFilterTitle: '无匹配 run',
        runsStartedAt: '开始于',
        runDetailBack: '返回',
        runDetailLoading: '正在加载轨迹…',
        runDetailEmpty: '磁盘上没有步骤记录',
        runDetailEmptyHint: '未找到轨迹文件或文件为空。开启 record_trajectory 才会记录每一步。',
        runDetailStarted: '开始时间',
        runDetailFinished: '结束时间',
        runDetailDuration: '总时长',
        runDetailSteps: '步数',
        runDetailTokens: 'Tokens（入 / 出）',
        runDetailCost: '成本',
        runDetailReason: '结束原因',
        resultTitle: '结果',
        resultCopy: '复制',
        resultCopied: '结果已复制',
        resultEmpty: '无文本结果（任务已完成）',
        resultFailed: '未完成',
        resultRunning: '任务进行中…',
        resultWarnings: '提醒',
        badgeScheduled: '定时',
        newTaskTitle: '新建任务',
        newTaskHint: '一次提交一条任务即可启动 Agent',
        newTaskProvider: 'Provider',
        newTaskMaxSteps: '最大步数',
        newTaskMaxWall: '最长时长 (秒)',
        newTaskStart: '启动',
        newTaskStarting: '启动中…',
        newTaskPlaceholder: '请描述要让 Agent 完成的任务',
        newTaskUseDefault: '使用默认值',
        newTaskTaskRequired: '请填写任务描述',
        newTaskProviderRequired: '请选择 Provider',
        newTaskStarted: '已启动任务',
        newTaskAlreadyActive: '已有运行中的任务',
        newTaskFailed: '启动失败：',
        newTaskRunning: '已有运行中的任务，请先停止再启动',
        schedulesTitle: '定时任务',
        schedulesHint: 'cron 五段表达式，服务器本地时区。运行重叠时跳过本次，不堆积。',
        schedulesEmpty: '暂无定时任务。',
        schedulesNew: '新建',
        scheduleEditTitle: '编辑定时任务',
        scheduleEditNewTitle: '新建定时任务',
        scheduleName: '名称',
        scheduleCron: 'cron (5 段)',
        scheduleTask: '任务描述',
        scheduleProvider: 'Provider',
        scheduleMaxSteps: '最大步数',
        scheduleMaxWall: '最长时长 (秒)',
        scheduleEnabled: '启用',
        scheduleSave: '保存',
        scheduleCancel: '取消',
        scheduleEdit: '编辑',
        scheduleDelete: '删除',
        scheduleRunNow: '立即运行',
        scheduleNextRun: '下次',
        scheduleLastRun: '上次',
        scheduleNever: '—',
        scheduleSaved: '已保存',
        scheduleDeleted: '已删除',
        scheduleRunStarted: '已触发',
        scheduleRunFailed: '触发失败：',
        scheduleSaveFailed: '保存失败：',
        scheduleDeleteFailed: '删除失败：',
        scheduleDeleteConfirm: '确认删除该定时任务？',
        scheduleOutcomeStarted: '已启动',
        scheduleOutcomeSkipped: '已跳过',
        scheduleOutcomeFailed: '失败',
        runDetailFilterAll: '全部动作',
        stepLatency: '延迟',
        stepGap: '间隔',
        stepTokens: 'tokens',
        stepCost: '成本',
        stepResultOk: '成功',
        stepCopyJson: '复制 JSON',
        stepJsonCopied: '步骤 JSON 已复制',
        stepFrame: '截图',
        stepNoFrame: '无截图',
        stepCumulative: '累计',
    }
};

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
        launch: a.launch_command
    })));
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
    setText('nav-agent', 'navAgent');
    setText('nav-providers', 'navProviders');
    setText('nav-runs', 'navRuns');
    setText('nav-schedules', 'navSchedules');
    setText('overview-providers-title', 'providerStatus');
    setText('overview-agent-title', 'agentStatus');
    setText('overview-agent-stop', 'stopAgent');
    setText('overview-jump-providers', 'overviewJumpProviders');
    setText('overview-meta-auto', 'metaAutoRefresh');
    setText('overview-agent-raw-summary', 'overviewAgentRaw');

    setText('settings-stream-title', 'settingsStreamTitle');
    setText('settings-actions-title', 'settingsActionsTitle');
    setText('settings-actions-hint', 'settingsActionsHint');
    setText('settings-hint', 'settingsHint');
    setText('settings-save', 'saveApply');
    setText('settings-dirty', 'dirtyIndicator');
    setText('request-keyframe', 'requestKeyframe');

    setText('agent-budget-title', 'agentBudgetTitle');
    setText('agent-budget-hint', 'agentBudgetHint');
    setText('agent-behavior-title', 'agentBehaviorTitle');
    setText('agent-save-hint', 'agentSaveHint');
    setText('agent-save', 'save');
    setText('agent-dirty', 'dirtyIndicator');

    setText('providers-title', 'navProviders');
    setText('provider-editor-title', 'providerConfig');
    setText('provider-empty-text', 'providerEmptyText');
    setText('provider-default-badge', 'providerDefaultBadge');
    setText('provider-save', 'save');
    setText('provider-reset', 'reset');
    setText('provider-test', 'testButton');
    setText('provider-dirty', 'dirtyIndicator');
    setText('api-key-replace', 'apiKeyReplace');
    setText('api-key-clear', 'apiKeyClear');
    setText('api-key-cancel', 'apiKeyCancel');
    setText('api-key-undo-clear', 'apiKeyUndoClear');
    setText('api-key-pending-text', 'apiKeyPendingText');

    setText('runs-refresh', 'refresh');
    setPlaceholderById('runs-search', t('runsSearchPlaceholder'));
    setText('run-detail-back-label', 'runDetailBack');
    setText('new-task-title', 'newTaskTitle');
    setText('new-task-hint', 'newTaskHint');
    setText('new-task-provider-label', 'newTaskProvider');
    setText('new-task-steps-label', 'newTaskMaxSteps');
    setText('new-task-wall-label', 'newTaskMaxWall');
    setText('new-task-start', 'newTaskStart');
    setPlaceholderById('new-task-task', t('newTaskPlaceholder'));
    setPlaceholderById('new-task-max-steps', t('newTaskUseDefault'));
    setPlaceholderById('new-task-max-wall', t('newTaskUseDefault'));

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
    refreshApiKeyPlaceholder();

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
    if (bgOpt) bgOpt.textContent = t('backgroundOption');
    if (dtOpt) dtOpt.textContent = t('desktopOption');
    setText('label-autostart', 'autostart');
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
    setText('modal-cancel', 'cancel');
    setText('log-close-btn', 'close');
    document.getElementById('modal-close')?.setAttribute('aria-label', t('close'));
    setPlaceholderById('f-name', t('nameInputPlaceholder'));
    setPlaceholderById('f-launch-command', t('backgroundCommandPlaceholder'));
    setPlaceholderById('f-exec', t('desktopCommandPlaceholder'));
    setPlaceholderById('f-launch-cwd', t('cwdPlaceholder'));
    setPlaceholderById('f-launch-timeout', t('timeoutPlaceholder'));
    setPlaceholderById('f-url', t('accessUrlPlaceholder'));
    setPlaceholderById('f-launch-env', t('launchEnvPlaceholder'));
    setPlaceholderById('f-env', t('envPlaceholder'));

    if (!editId) {
        setText('modal-title', 'addModal');
        setText('modal-save', 'addSave');
    }

    const logTitle = document.getElementById('log-title');
    if (logTitle && !logTitle.dataset.appName) {
        logTitle.textContent = t('logsTitle');
    }
    updateSectionHeader();
    renderRunsFilterChips();
    if (activeSection === 'overview') {
        updateOverviewMeta();
        renderOverviewProviders(overviewCache?.providers || []);
        renderOverviewAgent(overviewCache?.agent || {});
        renderQuickActions();
    }
}

function updateSectionHeader() {
    const titleMap = {
        overview: 'navOverview',
        apps: 'navApps',
        settings: 'navSettings',
        agent: 'navAgent',
        providers: 'navProviders',
        runs: 'navRuns',
        schedules: 'navSchedules'
    };
    const descMap = {
        overview: 'sectionOverviewDesc',
        apps: 'sectionAppsDesc',
        settings: 'sectionSettingsDesc',
        agent: 'sectionAgentDesc',
        providers: 'sectionProvidersDesc',
        runs: 'sectionRunsDesc',
        schedules: 'schedulesHint'
    };
    setTextById('section-title', t(titleMap[activeSection]));
    setTextById('section-description', t(descMap[activeSection]));
}

function switchSection(section) {
    activeSection = section;
    document.body.dataset.section = section;
    document.querySelectorAll('.nav-item').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.section === section);
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
    if (section === 'agent') await loadAgentConfig();
    if (section === 'providers') await loadProviders();
    if (section === 'runs') {
        await Promise.all([loadRuns(), loadNewTaskPanel()]);
    }
    if (section === 'schedules') {
        await loadSchedules();
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

// ---------- Overview ----------

async function loadOverview() {
    try {
        const d = await fetchJson(`${CONSOLE_API}/overview`);
        overviewCache = d;
        lastOverviewAt = Date.now();
        renderOverviewTiles(d);
        renderOverviewProviders(d.providers || []);
        renderOverviewAgent(d.agent || {});
        renderQuickActions();
        updateOverviewMeta();
        const rawEl = document.getElementById('overview-agent-json');
        if (rawEl) rawEl.textContent = JSON.stringify(d.agent, null, 2);
    } catch (e) {
        toast(t('fetchFailed') + e, 'err');
    }
}

function renderOverviewTiles(d) {
    const grid = document.getElementById('overview-grid');
    if (!grid) return;
    grid.innerHTML = '';

    const features = d.build?.features || {};
    let versionHelp;
    if (features.agent_all) versionHelp = t('tileMcpAgentAll');
    else if (features.agent || features.mcp) versionHelp = features.agent && features.mcp ? t('tileMcpAgent') : (features.mcp ? 'mcp' : 'agent');
    else versionHelp = t('tileNoFeatures');

    const sessions = d.connections?.webrtc_sessions ?? 0;
    let sessionsHelp;
    if (sessions === 0) sessionsHelp = t('sessionsHelpNone');
    else if (sessions === 1) sessionsHelp = t('sessionsHelpOne');
    else sessionsHelp = t('sessionsHelpMany')(sessions);

    const agent = d.agent || {};
    const agentRunning = !!agent.running_run_id;
    const stopRequested = !!agent.stop_requested;
    const exclusive = !!agent.exclusive;
    let agentValue, agentHelp, agentTone;
    if (stopRequested) {
        agentValue = t('agentStateStopping');
        agentHelp = t('tileStopRequested');
        agentTone = 'tone-warn';
    } else if (agentRunning) {
        agentValue = t('tileRunning');
        agentHelp = exclusive ? t('tileExclusive') : '';
        agentTone = 'tone-info';
    } else if (exclusive) {
        agentValue = t('agentStateExclusive');
        agentHelp = '';
        agentTone = 'tone-warn';
    } else {
        agentValue = t('tileIdle');
        agentHelp = '';
        agentTone = 'tone-muted';
    }

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
        {
            label: t('tileAgent'),
            value: agentValue,
            help: agentHelp,
            tone: agentTone,
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

function renderOverviewProviders(providers) {
    const box = document.getElementById('overview-providers');
    if (!box) return;
    box.innerHTML = '';
    if (!providers.length) {
        const empty = document.createElement('div');
        empty.className = 'muted-hint';
        empty.textContent = t('providerNoneConfigured');
        box.appendChild(empty);
        return;
    }
    providers.forEach(p => {
        const row = document.createElement('div');
        row.className = 'list-row';
        const model = p.settings?.model || p.default_model || '';
        const isDefault = agentDefaultProvider && p.name === agentDefaultProvider;
        const badges = [];
        if (isDefault) badges.push(`<span class="badge-default">${esc(t('providerDefaultBadge'))}</span>`);
        badges.push(p.configured
            ? `<span class="badge-ok">${esc(t('configured'))}</span>`
            : `<span class="badge-warn">${esc(t('missingConfig'))}</span>`);
        row.innerHTML = `<div><strong>${esc(p.name)}</strong><small>${esc(model)}</small></div><div class="provider-row-tags">${badges.join('')}</div>`;
        box.appendChild(row);
    });
}

function renderOverviewAgent(agent) {
    const card = document.getElementById('overview-agent-card');
    if (!card) return;
    const running = !!agent.running_run_id;
    const stopping = !!agent.stop_requested;
    const exclusive = !!agent.exclusive;
    const disabled = agent.enabled === false;

    let stateLabel, stateClass;
    if (disabled) {
        stateLabel = t('agentStateDisabled'); stateClass = 'state-disabled';
    } else if (stopping) {
        stateLabel = t('agentStateStopping'); stateClass = 'state-stopping';
    } else if (running) {
        stateLabel = t('agentStateRunning'); stateClass = 'state-running';
    } else if (exclusive) {
        stateLabel = t('agentStateExclusive'); stateClass = 'state-exclusive';
    } else {
        stateLabel = t('agentStateIdle'); stateClass = 'state-idle';
    }

    const taskText = agent.task || (running ? t('agentNoTaskRunning') : t('agentNoTaskIdle'));
    const runIdShort = agent.running_run_id
        ? String(agent.running_run_id).slice(0, 8)
        : '—';

    card.innerHTML = `
        <div class="agent-card">
            <div class="agent-card-state">
                <span class="agent-state-pill ${stateClass}">${esc(stateLabel)}</span>
            </div>
            <div class="agent-card-task">
                <span class="label">${esc(t('agentTaskLabel'))}</span>
                <span>${esc(taskText)}</span>
            </div>
            <div class="agent-card-metrics">
                <div class="agent-metric">
                    <div class="agent-metric-label">${esc(t('agentMetricRun'))}</div>
                    <div class="agent-metric-value" title="${esc(agent.running_run_id || '')}">${esc(runIdShort)}</div>
                </div>
                <div class="agent-metric">
                    <div class="agent-metric-label">${esc(t('agentMetricExclusive'))}</div>
                    <div class="agent-metric-value">${esc(exclusive ? t('yes') : t('no'))}</div>
                </div>
                <div class="agent-metric">
                    <div class="agent-metric-label">${esc(t('agentMetricStop'))}</div>
                    <div class="agent-metric-value">${esc(stopping ? t('yes') : t('no'))}</div>
                </div>
            </div>
        </div>`;

    const stopBtn = document.getElementById('overview-agent-stop');
    if (stopBtn) stopBtn.disabled = !running || stopping;
}

function renderQuickActions() {
    const box = document.getElementById('overview-quick-actions');
    if (!box) return;
    const cards = [
        { jump: 'apps', icon: '⊞', label: t('quickApps'), desc: t('quickAppsDesc') },
        { jump: 'settings', icon: '⚙', label: t('quickSettings'), desc: t('quickSettingsDesc') },
        { jump: 'agent', icon: '✦', label: t('quickAgent'), desc: t('quickAgentDesc') },
        { jump: 'runs', icon: '☰', label: t('quickRuns'), desc: t('quickRunsDesc') },
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

function formatCostMicros(micros) {
    if (micros == null) return '—';
    const usd = micros / 1e6;
    if (usd === 0) return '$0';
    if (usd < 0.01) return `$${usd.toFixed(4)}`;
    return `$${usd.toFixed(2)}`;
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

async function loadAgentConfig() {
    try {
        const [cfg, providers] = await Promise.all([
            fetchJson(`${CONSOLE_API}/agent-config`),
            fetchJson(`${CONSOLE_API}/providers`)
        ]);
        providersCache = providers.providers || [];
        agentDefaultProvider = cfg.default_provider || null;
        const select = document.getElementById('agent-provider');
        if (select) {
            select.innerHTML = providersCache.map(p => `<option value="${esc(p.name)}">${esc(p.name)}</option>`).join('');
            select.value = cfg.default_provider || (providersCache[0]?.name || 'local');
        }
        const opt = cfg.options || {};
        const budget = opt.budget || {};
        setValue('agent-max-steps', budget.max_steps ?? 50);
        setValue('agent-max-wall', budget.max_wall_seconds ?? 300);
        setValue('agent-screenshot-bytes', opt.screenshot_max_bytes ?? 800000);
        const rec = document.getElementById('agent-record-trajectory');
        const dry = document.getElementById('agent-dry-run');
        if (rec) rec.checked = opt.record_trajectory !== false;
        if (dry) dry.checked = !!opt.dry_run;
        snapshotDirtyGroup('agent');
        markDirty('agent', false);
    } catch (e) {
        toast(t('fetchFailed') + e, 'err');
    }
}

async function saveAgentConfig() {
    const body = {
        default_provider: document.getElementById('agent-provider').value,
        options: {
            budget: {
                max_steps: numberValue('agent-max-steps'),
                max_input_tokens: 200000,
                max_output_tokens: 20000,
                max_wall_seconds: numberValue('agent-max-wall'),
                max_screenshots: Math.max(numberValue('agent-max-steps') + 10, 60)
            },
            action_settle_ms: 250,
            max_actions_per_step: 30,
            max_history_images: 3,
            allow_destructive: false,
            require_confirmation_for: [],
            screenshot_format: 'Jpeg',
            screenshot_max_bytes: numberValue('agent-screenshot-bytes'),
            record_trajectory: document.getElementById('agent-record-trajectory').checked,
            record_frames_to_disk: false,
            dry_run: document.getElementById('agent-dry-run').checked
        }
    };
    await putJson(`${CONSOLE_API}/agent-config`, body);
    agentDefaultProvider = body.default_provider;
    toast(t('agentSaved'), 'ok');
    snapshotDirtyGroup('agent');
    markDirty('agent', false);
}

// ---------- Providers ----------

async function loadProviders() {
    try {
        const [d, cfg] = await Promise.all([
            fetchJson(`${CONSOLE_API}/providers`),
            fetchJson(`${CONSOLE_API}/agent-config`).catch(() => ({}))
        ]);
        providersCache = d.providers || [];
        agentDefaultProvider = cfg.default_provider || agentDefaultProvider;
        renderProviders();
        const selectedName = document.getElementById('provider-name')?.value;
        if (selectedName && providersCache.find(p => p.name === selectedName)) {
            selectProvider(selectedName);
        } else if (providersCache.length) {
            selectProvider(providersCache[0].name);
        } else {
            showProviderEmpty();
        }
    } catch (e) {
        toast(t('fetchFailed') + e, 'err');
    }
}

function renderProviders() {
    const list = document.getElementById('provider-list');
    if (!list) return;
    const selected = document.getElementById('provider-name')?.value;
    list.innerHTML = '';
    const countEl = document.getElementById('provider-count');
    if (countEl) countEl.textContent = String(providersCache.length);
    providersCache.forEach(p => {
        const row = document.createElement('button');
        row.type = 'button';
        row.className = 'list-row provider-row' + (p.name === selected ? ' active' : '');
        const model = p.settings?.model || p.default_model || '';
        const isDefault = agentDefaultProvider && p.name === agentDefaultProvider;
        const badges = [];
        if (isDefault) badges.push(`<span class="badge-default">${esc(t('providerDefaultBadge'))}</span>`);
        badges.push(p.configured
            ? `<span class="badge-ok">${esc(t('configured'))}</span>`
            : `<span class="badge-warn">${esc(t('missingConfig'))}</span>`);
        row.innerHTML = `
            <div class="provider-row-main">
                <strong>${esc(p.name)}</strong>
                <small>${esc(model)}</small>
            </div>
            <div class="provider-row-tags">${badges.join('')}</div>`;
        row.addEventListener('click', () => selectProvider(p.name));
        list.appendChild(row);
    });
}

function showProviderEmpty() {
    const empty = document.getElementById('provider-empty');
    const editor = document.getElementById('provider-editor');
    if (empty) empty.hidden = false;
    if (editor) editor.hidden = true;
}

function selectProvider(name) {
    const p = providersCache.find(item => item.name === name);
    if (!p) return;
    const empty = document.getElementById('provider-empty');
    const editor = document.getElementById('provider-editor');
    if (empty) empty.hidden = true;
    if (editor) editor.hidden = false;

    const endpoint = p.settings?.endpoint || '';
    const model = p.settings?.model || '';
    const apiFormat = p.settings?.api_format || '';
    const coord = p.settings?.coord_space || '';

    document.getElementById('provider-name').value = p.name;
    document.getElementById('provider-endpoint').value = endpoint;
    document.getElementById('provider-model').value = model;
    document.getElementById('provider-api-format').value = apiFormat;
    document.getElementById('provider-api-key').value = '';
    document.getElementById('provider-coord-space').value = coord;
    document.getElementById('provider-clear-key').value = '';

    setTextById('provider-endpoint-hint', p.default_endpoint ? t('providerEndpointHintDefault')(p.default_endpoint) : '');
    setTextById('provider-model-hint', p.default_model ? t('providerModelHintDefault')(p.default_model) : '');

    const badge = document.getElementById('provider-default-badge');
    if (badge) badge.hidden = !(agentDefaultProvider && p.name === agentDefaultProvider);

    providerOriginalSettings = {
        name: p.name,
        endpoint,
        model,
        apiFormat,
        coord,
        keyConfigured: !!p.settings?.api_key_configured,
    };

    setApiKeyMode(providerOriginalSettings.keyConfigured ? 'stored' : 'input');
    renderProviders();
    snapshotDirtyGroup('provider');
    markDirty('provider', false);
}

function setApiKeyMode(mode) {
    apiKeyMode = mode;
    const stored = document.getElementById('api-key-stored');
    const inputWrap = document.getElementById('api-key-input-wrap');
    const pending = document.getElementById('api-key-pending');
    const cancelBtn = document.getElementById('api-key-cancel');
    const input = document.getElementById('provider-api-key');
    const clearField = document.getElementById('provider-clear-key');
    const statusEl = document.getElementById('api-key-status');

    if (statusEl) {
        statusEl.textContent = providerOriginalSettings?.keyConfigured ? t('apiKeyStatusSaved') : t('apiKeyStatusEmpty');
    }

    if (mode === 'stored') {
        if (stored) stored.hidden = false;
        if (inputWrap) inputWrap.hidden = true;
        if (pending) pending.hidden = true;
        if (input) input.value = '';
        if (clearField) clearField.value = '';
    } else if (mode === 'input') {
        if (stored) stored.hidden = true;
        if (inputWrap) inputWrap.hidden = false;
        if (pending) pending.hidden = true;
        if (cancelBtn) cancelBtn.hidden = !providerOriginalSettings?.keyConfigured;
        if (clearField) clearField.value = '';
        refreshApiKeyPlaceholder();
        setTimeout(() => input?.focus(), 0);
    } else if (mode === 'pending-clear') {
        if (stored) stored.hidden = true;
        if (inputWrap) inputWrap.hidden = true;
        if (pending) pending.hidden = false;
        if (input) input.value = '';
        if (clearField) clearField.value = 'true';
    }
}

function refreshApiKeyPlaceholder() {
    const input = document.getElementById('provider-api-key');
    if (!input) return;
    if (apiKeyMode === 'input' && providerOriginalSettings?.keyConfigured) {
        input.placeholder = t('enterNewApiKey');
    } else {
        input.placeholder = t('keepApiKeyPlaceholder');
    }
}

async function saveProvider() {
    const name = document.getElementById('provider-name').value;
    if (!name) return;
    const body = {
        endpoint: document.getElementById('provider-endpoint').value.trim(),
        model: document.getElementById('provider-model').value.trim(),
        api_format: document.getElementById('provider-api-format').value,
        api_key: document.getElementById('provider-api-key').value.trim(),
        clear_api_key: document.getElementById('provider-clear-key').value === 'true',
        coord_space: document.getElementById('provider-coord-space').value
    };
    await putJson(`${CONSOLE_API}/providers/${encodeURIComponent(name)}`, body);
    toast(t('providerSaved'), 'ok');
    document.getElementById('provider-name').value = name;
    await loadProviders();
}

function resetProvider() {
    if (!providerOriginalSettings) return;
    selectProvider(providerOriginalSettings.name);
}

async function testProvider() {
    const name = document.getElementById('provider-name').value;
    if (!name) return;
    const btn = document.getElementById('provider-test');
    const status = document.getElementById('provider-test-status');
    if (status) {
        status.textContent = t('testRunning');
        status.className = 'provider-test-status running';
    }
    if (btn) { btn.disabled = true; }
    const body = {
        endpoint: document.getElementById('provider-endpoint').value.trim(),
        model: document.getElementById('provider-model').value.trim(),
        api_format: document.getElementById('provider-api-format').value,
        api_key: document.getElementById('provider-api-key').value.trim(),
        clear_api_key: document.getElementById('provider-clear-key').value === 'true',
        coord_space: document.getElementById('provider-coord-space').value
    };
    try {
        const r = await fetch(`${CONSOLE_API}/providers/${encodeURIComponent(name)}/test`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
        });
        const d = await parseJsonResponse(r);
        if (!r.ok) {
            const msg = d.error || r.statusText;
            if (status) {
                status.textContent = `${t('testFailed')} ${msg}`;
                status.className = 'provider-test-status err';
            }
            toast(`${t('testFailed')} ${msg}`, 'err');
            return;
        }
        if (d.skipped) {
            if (status) {
                status.textContent = t('testSkipped');
                status.className = 'provider-test-status';
            }
            toast(t('testSkipped'), 'ok');
            return;
        }
        if (d.ok) {
            const parts = [`${t('testOk')} ${d.latency_ms}ms`];
            if (d.preview) parts.push(`"${d.preview}"`);
            const text = parts.join(' · ');
            if (status) {
                status.textContent = text;
                status.className = 'provider-test-status ok';
                status.title = d.preview || '';
            }
            toast(text, 'ok');
        } else {
            const sc = d.status_code ? `HTTP ${d.status_code}` : '';
            const msg = [sc, d.error].filter(Boolean).join(' · ');
            if (status) {
                status.textContent = `${t('testFailed')} ${msg}`;
                status.className = 'provider-test-status err';
                status.title = d.error || '';
            }
            toast(`${t('testFailed')} ${msg}`, 'err');
        }
    } catch (e) {
        if (status) {
            status.textContent = `${t('testFailed')} ${e.message || e}`;
            status.className = 'provider-test-status err';
        }
        toast(`${t('testFailed')} ${e.message || e}`, 'err');
    } finally {
        if (btn) { btn.disabled = false; }
    }
}

// ---------- Runs ----------

async function loadRuns() {
    try {
        const d = await fetchJson(`${CONSOLE_API}/agent-runs`);
        runsCache = d.runs || [];
        renderRunsFilterChips();
        renderRunsList();
        updateNewTaskButtonState();
    } catch (e) {
        toast(t('fetchFailed') + e, 'err');
    }
}

async function loadNewTaskPanel() {
    const select = document.getElementById('new-task-provider');
    if (!select) return;
    try {
        const [providers, agentCfg] = await Promise.all([
            fetchJson(`${CONSOLE_API}/providers`).catch(() => ({ providers: [] })),
            fetchJson(`${CONSOLE_API}/agent-config`).catch(() => ({}))
        ]);
        const list = providers.providers || [];
        const previous = select.value;
        select.innerHTML = list.map(p => `<option value="${esc(p.name)}">${esc(p.name)}</option>`).join('');
        const preferred = previous && list.find(p => p.name === previous)
            ? previous
            : (agentCfg.default_provider || (list[0]?.name || ''));
        if (preferred) select.value = preferred;
    } catch (_) {
        // leave the select empty; the user will see no options
    }
}

function updateNewTaskButtonState() {
    const btn = document.getElementById('new-task-start');
    const statusEl = document.getElementById('new-task-status');
    if (!btn) return;
    if (btn.dataset.pending === '1') return;
    const hasRunning = (runsCache || []).some(r => runStateClass(r) === 'running');
    btn.disabled = hasRunning;
    btn.textContent = t('newTaskStart');
    if (statusEl) statusEl.textContent = hasRunning ? t('newTaskRunning') : '';
}

async function startTask() {
    const btn = document.getElementById('new-task-start');
    const statusEl = document.getElementById('new-task-status');
    if (!btn || btn.dataset.pending === '1') return;
    const taskEl = document.getElementById('new-task-task');
    const providerEl = document.getElementById('new-task-provider');
    const maxStepsEl = document.getElementById('new-task-max-steps');
    const maxWallEl = document.getElementById('new-task-max-wall');
    const task = (taskEl.value || '').trim();
    const provider = providerEl.value || '';
    if (!task) { toast(t('newTaskTaskRequired'), 'err'); taskEl.focus(); return; }
    if (!provider) { toast(t('newTaskProviderRequired'), 'err'); return; }
    const body = { task, provider };
    const maxSteps = parseInt(maxStepsEl.value, 10);
    const maxWall = parseInt(maxWallEl.value, 10);
    const budget = {};
    if (Number.isFinite(maxSteps) && maxSteps > 0) budget.max_steps = maxSteps;
    if (Number.isFinite(maxWall) && maxWall > 0) budget.max_wall_seconds = maxWall;
    if (Object.keys(budget).length) body.budget = budget;

    btn.dataset.pending = '1';
    btn.disabled = true;
    btn.textContent = t('newTaskStarting');
    if (statusEl) statusEl.textContent = '';
    try {
        const r = await fetch(`${CONSOLE_API}/agent-start`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body)
        });
        const d = await parseJsonResponse(r);
        if (r.status === 409) {
            toast(t('newTaskAlreadyActive'), 'err');
            if (statusEl && d.run_id) statusEl.textContent = `run_id: ${d.run_id}`;
            await loadRuns();
            return;
        }
        if (!r.ok || d.error) throw new Error(d.error || r.statusText);
        toast(t('newTaskStarted'), 'ok');
        taskEl.value = '';
        await loadRuns();
        if (d.run_id) {
            const newRun = (runsCache || []).find(rr => rr.run_id === d.run_id);
            if (newRun) openRunDetail(newRun);
        }
    } catch (e) {
        toast(t('newTaskFailed') + (e.message || e), 'err');
    } finally {
        delete btn.dataset.pending;
        updateNewTaskButtonState();
    }
}

function runStateClass(run) {
    const reason = run.finish_reason || { kind: 'running' };
    if (reason.kind === 'running') return 'running';
    if (reason.kind === 'done') return reason.success === false ? 'failed' : 'done';
    if (reason.kind === 'ask') return 'ask';
    if (reason.kind === 'safety') return 'safety';
    if (reason.kind === 'interrupted') return 'interrupted';
    if (reason.kind === 'budget_exceeded') return 'budget';
    if (reason.kind === 'max_steps_reached') return 'max-steps';
    if (reason.kind === 'provider_error') return 'provider-error';
    return 'interrupted';
}

function runStateLabel(run) {
    const cls = runStateClass(run);
    return t('runState_' + cls.replace(/-/g, '_'));
}

function renderRunsFilterChips() {
    const container = document.getElementById('runs-filter-chips');
    if (!container) return;
    const counts = { all: runsCache.length };
    runsCache.forEach(r => {
        const cls = runStateClass(r);
        counts[cls] = (counts[cls] || 0) + 1;
    });
    const order = ['all', 'running', 'done', 'failed', 'ask', 'safety', 'interrupted', 'budget', 'max-steps', 'provider-error'];
    container.innerHTML = order
        .filter(k => k === 'all' || (counts[k] || 0) > 0)
        .map(k => {
            const label = k === 'all' ? t('runsFilterAll') : t('runState_' + k.replace(/-/g, '_'));
            const count = counts[k] || 0;
            const active = k === runsFilter;
            return `<button type="button" class="runs-filter-chip${active ? ' active' : ''}" data-filter="${esc(k)}">
                ${esc(label)}<span class="chip-count">${count}</span>
            </button>`;
        })
        .join('');
    container.querySelectorAll('.runs-filter-chip').forEach(btn => {
        btn.addEventListener('click', () => {
            runsFilter = btn.dataset.filter;
            renderRunsFilterChips();
            renderRunsList();
        });
    });
}

function renderRunsList() {
    const list = document.getElementById('runs-list');
    if (!list) return;
    list.innerHTML = '';

    const search = runsSearchTerm.trim().toLowerCase();
    const filtered = runsCache.filter(r => {
        if (runsFilter !== 'all' && runStateClass(r) !== runsFilter) return false;
        if (search) {
            const hay = `${r.run_id || ''} ${r.task || ''}`.toLowerCase();
            if (!hay.includes(search)) return false;
        }
        return true;
    });

    if (!filtered.length) {
        const isFiltering = runsFilter !== 'all' || !!search;
        list.innerHTML = `<div class="empty-block">
            <div class="empty-icon" aria-hidden="true">∅</div>
            <strong>${esc(isFiltering ? t('emptyRunsFilterTitle') : t('emptyRunsTitle'))}</strong>
            <div>${esc(isFiltering ? t('noRunsMatch') : t('noRunsHint'))}</div>
        </div>`;
        return;
    }

    const fragment = document.createDocumentFragment();
    filtered.forEach(run => renderRunCard(run, fragment));
    list.appendChild(fragment);
}

function renderRunCard(run, parent) {
    const card = document.createElement('div');
    card.className = 'run-card run-card-clickable';
    card.setAttribute('role', 'button');
    card.tabIndex = 0;
    const cls = runStateClass(run);
    const stateLabel = runStateLabel(run);
    const idShort = (run.run_id || '').slice(0, 12);
    const taskText = run.task || '—';
    const wall = formatDuration(run.wall_ms);
    const steps = run.steps_taken ?? 0;
    const tokenIn = run.tokens_in ?? 0;
    const tokenOut = run.tokens_out ?? 0;
    const path = run.trajectory_path || '';
    const startedMs = run.started_at_ms || 0;
    const startedShort = startedMs ? formatDateTimeShort(startedMs) : '';
    const startedFull = startedMs ? formatDateTime(startedMs) : '';

    const reason = (run.finish_reason && run.finish_reason.kind) || 'running';
    let previewHtml = '';
    if (reason !== 'running') {
        const out = (run.output || '').trim();
        const pq = (run.pending_question || '').trim();
        if (out) {
            previewHtml = `<div class="run-card-output"><span class="rco-dot"></span>${esc(truncate(out, 120))}</div>`;
        } else if (pq) {
            previewHtml = `<div class="run-card-output warn"><span class="rco-dot warn"></span>${esc(truncate(pq.split(/\r?\n/)[0], 120))}</div>`;
        }
    }
    const scheduled = run.source && run.source.kind === 'schedule';
    const badgeHtml = scheduled
        ? `<span class="run-badge-scheduled" title="${esc(run.source.id || '')}">⏰ ${esc(t('badgeScheduled'))}</span>`
        : '';

    card.innerHTML = `
        <div class="run-card-status">
            <span class="run-state-pill state-${esc(cls)}">${esc(stateLabel)}</span>
        </div>
        <div class="run-card-main">
            <span class="run-card-id" title="${esc(run.run_id || '')}" data-action="copy-id">
                <span aria-hidden="true">⧉</span>${esc(idShort)}
            </span>
            <div class="run-card-task">${esc(taskText)}${badgeHtml}</div>
            ${previewHtml}
            <div class="run-card-meta">
                ${startedShort ? `<span title="${esc(t('runsStartedAt'))} ${esc(startedFull)}">🕐 ${esc(startedShort)}</span>` : ''}
                <span>⏱ ${esc(wall)}</span>
                <span>↻ ${esc(formatNumber(steps))} ${esc(t('runsMetricSteps'))}</span>
                <span>⇅ ${esc(formatNumber(tokenIn))} / ${esc(formatNumber(tokenOut))} ${esc(t('runsMetricTokens'))}</span>
            </div>
        </div>
        <div class="run-card-actions">
            ${path
                ? `<button type="button" class="btn-link" data-action="copy-path" title="${esc(path)}">${esc(t('runsCopyPath'))}</button>`
                : `<span class="muted-hint">${esc(t('runsNoPath'))}</span>`}
        </div>`;

    card.querySelector('[data-action="copy-id"]')?.addEventListener('click', async (e) => {
        e.stopPropagation();
        try { await copyText(run.run_id || ''); toast(t('runsIdCopied'), 'ok'); }
        catch { toast(t('copyFailed'), 'err'); }
    });
    if (path) {
        card.querySelector('[data-action="copy-path"]')?.addEventListener('click', async (e) => {
            e.stopPropagation();
            try { await copyText(path); toast(t('runsPathCopied'), 'ok'); }
            catch { toast(t('copyFailed'), 'err'); }
        });
    }
    card.addEventListener('click', () => openRunDetail(run));
    card.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openRunDetail(run); }
    });
    parent.appendChild(card);
}

// ---------- Run detail ----------

let runDetailRun = null;
let runDetailSteps = [];
let runDetailActionFilter = 'all';

function truncate(s, n) {
    s = String(s ?? '');
    return s.length > n ? s.slice(0, n) + '…' : s;
}

function frameBasename(p) {
    const parts = String(p).split(/[\\/]/);
    return parts[parts.length - 1] || '';
}

function actionKind(a) { return (a && a.kind) || 'unknown'; }

function actionCategory(kind) {
    if (kind && kind.startsWith('mouse_')) return 'pointer';
    if (kind && kind.startsWith('window_')) return 'window';
    if (['type_text', 'key_chord', 'key_hold', 'clipboard_write', 'clipboard_read'].includes(kind)) return 'input';
    if (['scroll', 'zoom', 'wait', 'screenshot'].includes(kind)) return 'view';
    if (kind === 'done') return 'done';
    if (kind === 'ask') return 'ask';
    return 'other';
}

function actionBrief(a) {
    if (!a) return '—';
    switch (a.kind) {
        case 'mouse_move': return `(${a.x}, ${a.y})${a.label ? ' · ' + a.label : ''}`;
        case 'mouse_click': return `${a.button || ''} (${a.x}, ${a.y})${a.click_count > 1 ? ' ×' + a.click_count : ''}${a.label ? ' · ' + a.label : ''}`;
        case 'mouse_down': return `${a.button || ''} (${a.x}, ${a.y})`;
        case 'mouse_up': return `${a.button || ''} (${a.x}, ${a.y})`;
        case 'mouse_drag': return `${(a.path || []).length} pts`;
        case 'scroll': return `dx=${a.dx} dy=${a.dy}`;
        case 'zoom': return `Δ${a.level_delta}`;
        case 'type_text': return `"${truncate(a.text, 80)}"${a.press_enter ? ' ⏎' : ''}`;
        case 'key_chord': return a.combo || '';
        case 'key_hold': return `${a.key} ${a.ms}ms`;
        case 'clipboard_write': return `"${truncate(a.text, 40)}"`;
        case 'clipboard_read': return '';
        case 'window_focus': return `#${a.id}`;
        case 'window_close': return `#${a.id}`;
        case 'wait': return `${a.ms}ms`;
        case 'screenshot': return '';
        case 'done': return `${a.success ? '✓' : '✗'}${a.reason ? ' · ' + truncate(a.reason, 100) : ''}`;
        case 'ask': return `"${truncate(a.question, 100)}"`;
        default: return '';
    }
}

function resultInfo(r) {
    if (!r) return { ok: true, text: t('stepResultOk') };
    switch (r.kind) {
        case 'ok': return { ok: true, text: t('stepResultOk') };
        case 'out_of_bounds': return { ok: false, text: `out of bounds (${r.x}, ${r.y})` };
        case 'unsupported_action': return { ok: false, text: 'unsupported: ' + truncate(r.message, 60) };
        case 'executor_error': return { ok: false, text: 'error: ' + truncate(r.message, 60) };
        default: return { ok: false, text: r.kind };
    }
}

async function openRunDetail(run) {
    runDetailRun = run;
    runDetailSteps = [];
    runDetailActionFilter = 'all';
    const overlay = document.getElementById('run-detail-overlay');
    if (!overlay) return;
    renderRunDetailHeader(run);
    renderRunDetailResult(run);
    renderRunDetailSummary(run);
    document.getElementById('run-detail-filter').innerHTML = '';
    const stepsEl = document.getElementById('run-detail-steps');
    stepsEl.innerHTML = `<div class="run-detail-loading">${esc(t('runDetailLoading'))}</div>`;
    overlay.classList.add('show');
    try {
        const d = await fetchJson(`${CONSOLE_API}/agent-runs/${encodeURIComponent(run.run_id)}/steps`);
        runDetailSteps = d.steps || [];
        if (d.report) {
            runDetailRun = d.report;
            renderRunDetailHeader(d.report);
        }
        renderRunDetailResult(runDetailRun);
        renderRunDetailSummary(runDetailRun);
        renderRunDetailFilter();
        renderRunDetailSteps();
    } catch (e) {
        stepsEl.innerHTML = `<div class="empty-block">
            <div class="empty-icon" aria-hidden="true">∅</div>
            <strong>${esc(t('runDetailEmpty'))}</strong>
            <div>${esc(t('runDetailEmptyHint'))}</div>
        </div>`;
    }
}

function closeRunDetail() {
    document.getElementById('run-detail-overlay')?.classList.remove('show');
    runDetailRun = null;
    runDetailSteps = [];
}

function renderRunDetailHeader(run) {
    const pill = document.getElementById('run-detail-pill');
    if (pill) {
        pill.className = 'run-state-pill state-' + runStateClass(run);
        pill.textContent = runStateLabel(run);
    }
    const task = document.getElementById('run-detail-task');
    if (task) {
        task.textContent = run.task || '—';
        task.title = `${run.run_id || ''}`;
    }
}

function runIsOk(run) {
    const r = run.finish_reason;
    return r && r.kind === 'done' && r.success !== false;
}

function renderRunDetailResult(run) {
    const el = document.getElementById('run-detail-result');
    if (!el) return;
    const reason = (run.finish_reason && run.finish_reason.kind) || 'running';
    const output = (run.output || '').trim();
    const warnings = Array.isArray(run.warnings) ? run.warnings.filter(Boolean) : [];

    let cls, headHtml, bodyHtml;
    let copyPayload = '';

    if (reason === 'running') {
        cls = 'running';
        headHtml = `<span class="rd-result-title">${esc(t('resultTitle'))}</span>`;
        bodyHtml = `<div class="rd-result-placeholder">${esc(t('resultRunning'))}</div>`;
    } else if (runIsOk(run) && output) {
        cls = 'ok';
        copyPayload = run.output;
        headHtml = `<span class="rd-result-title">${esc(t('resultTitle'))}</span>
            <button type="button" class="rd-result-copy" data-action="copy-result">⧉ ${esc(t('resultCopy'))}</button>`;
        bodyHtml = `<pre class="rd-result-body">${esc(run.output)}</pre>`;
    } else if (runIsOk(run)) {
        cls = 'muted';
        headHtml = `<span class="rd-result-title">${esc(t('resultTitle'))}</span>`;
        bodyHtml = `<div class="rd-result-placeholder">${esc(t('resultEmpty'))}</div>`;
    } else {
        cls = 'warn';
        const msg = (run.pending_question || '').trim() || runStateLabel(run);
        copyPayload = msg;
        headHtml = `<span class="rd-result-title">${esc(t('resultFailed'))}</span>
            <button type="button" class="rd-result-copy" data-action="copy-result">⧉ ${esc(t('resultCopy'))}</button>`;
        const partial = output ? `<pre class="rd-result-body">${esc(run.output)}</pre>` : '';
        bodyHtml = `<div class="rd-result-msg">${esc(msg)}</div>${partial}`;
    }

    let warnHtml = '';
    if (warnings.length) {
        warnHtml = `<div class="rd-result-warnings">
            <span class="rd-result-warntitle">⚠ ${esc(t('resultWarnings'))}</span>
            <ul>${warnings.map(w => `<li>${esc(w)}</li>`).join('')}</ul>
        </div>`;
    }

    el.className = 'run-detail-result rd-result-' + cls;
    el.innerHTML = `<div class="rd-result-head">${headHtml}</div>${bodyHtml}${warnHtml}`;

    el.querySelector('[data-action="copy-result"]')?.addEventListener('click', async () => {
        try { await copyText(copyPayload); toast(t('resultCopied'), 'ok'); }
        catch { toast(t('copyFailed'), 'err'); }
    });
}

function renderRunDetailSummary(run) {
    const el = document.getElementById('run-detail-summary');
    if (!el) return;
    const startedMs = run.started_at_ms || 0;
    const running = run.finish_reason && run.finish_reason.kind === 'running';
    const finishedMs = startedMs ? startedMs + (run.wall_ms || 0) : 0;
    let costMicros = null;
    if (runDetailSteps.length && runDetailSteps.some(s => s.provider_usage && s.provider_usage.cost_usd_micros != null)) {
        costMicros = runDetailSteps.reduce((a, s) => a + ((s.provider_usage && s.provider_usage.cost_usd_micros) || 0), 0);
    }
    const items = [
        [t('runDetailStarted'), startedMs ? formatDateTime(startedMs) : '—'],
        [t('runDetailFinished'), running || !finishedMs ? '—' : formatDateTime(finishedMs)],
        [t('runDetailDuration'), formatDuration(run.wall_ms)],
        [t('runDetailSteps'), formatNumber(run.steps_taken ?? runDetailSteps.length)],
        [t('runDetailTokens'), `${formatNumber(run.tokens_in || 0)} / ${formatNumber(run.tokens_out || 0)}`],
        [t('runDetailCost'), costMicros == null ? '—' : formatCostMicros(costMicros)],
    ];
    el.innerHTML = items.map(([k, v]) =>
        `<div class="rd-stat"><span class="rd-stat-k">${esc(k)}</span><span class="rd-stat-v">${esc(v)}</span></div>`
    ).join('');
}

function renderRunDetailFilter() {
    const el = document.getElementById('run-detail-filter');
    if (!el) return;
    const counts = { all: runDetailSteps.length };
    runDetailSteps.forEach(s => {
        const k = actionKind(s.action);
        counts[k] = (counts[k] || 0) + 1;
    });
    const kinds = ['all', ...Object.keys(counts).filter(k => k !== 'all').sort()];
    el.innerHTML = kinds.map(k => {
        const label = k === 'all' ? t('runDetailFilterAll') : k;
        const active = k === runDetailActionFilter;
        return `<button type="button" class="runs-filter-chip${active ? ' active' : ''}" data-akind="${esc(k)}">${esc(label)}<span class="chip-count">${counts[k] || 0}</span></button>`;
    }).join('');
    el.querySelectorAll('[data-akind]').forEach(b => b.addEventListener('click', () => {
        runDetailActionFilter = b.dataset.akind;
        renderRunDetailFilter();
        renderRunDetailSteps();
    }));
}

function renderRunDetailSteps() {
    const el = document.getElementById('run-detail-steps');
    if (!el) return;
    if (!runDetailSteps.length) {
        el.innerHTML = `<div class="empty-block">
            <div class="empty-icon" aria-hidden="true">∅</div>
            <strong>${esc(t('runDetailEmpty'))}</strong>
            <div>${esc(t('runDetailEmptyHint'))}</div>
        </div>`;
        return;
    }
    const startMs = runDetailRun?.started_at_ms || runDetailSteps[0]?.ts_ms || 0;
    let cumIn = 0, cumOut = 0, cumCost = 0, prevTs = startMs;
    const rows = runDetailSteps.map((s, idx) => {
        const u = s.provider_usage || {};
        cumIn += u.input_tokens || 0;
        cumOut += u.output_tokens || 0;
        cumCost += u.cost_usd_micros || 0;
        const gap = prevTs ? (s.ts_ms - prevTs) : 0;
        prevTs = s.ts_ms;
        return renderStepRow(s, idx, { startMs, cumIn, cumOut, cumCost, gap });
    });
    el.innerHTML = rows.filter(Boolean).join('');

    el.querySelectorAll('.step-row-head').forEach(h => h.addEventListener('click', () => {
        h.closest('.step-row')?.classList.toggle('expanded');
    }));
    el.querySelectorAll('[data-copy-step]').forEach(b => b.addEventListener('click', (e) => {
        e.stopPropagation();
        const idx = +b.dataset.copyStep;
        copyText(JSON.stringify(runDetailSteps[idx], null, 2))
            .then(() => toast(t('stepJsonCopied'), 'ok'))
            .catch(() => toast(t('copyFailed'), 'err'));
    }));
    el.querySelectorAll('[data-frame]').forEach(img => img.addEventListener('click', (e) => {
        e.stopPropagation();
        openFrameLightbox(img.dataset.frame);
    }));
}

function renderStepRow(s, idx, ctx) {
    if (runDetailActionFilter !== 'all' && actionKind(s.action) !== runDetailActionFilter) return '';
    const offset = ctx.startMs ? formatOffset(s.ts_ms - ctx.startMs) : '';
    const absTime = formatDateTime(s.ts_ms);
    const ak = actionKind(s.action);
    const brief = actionBrief(s.action);
    const res = resultInfo(s.result);
    const u = s.provider_usage || {};
    const obs = s.observation || {};
    const sha = (obs.sha256 || '').slice(0, 8);
    const frameName = obs.frame_path ? frameBasename(obs.frame_path) : null;
    const frameUrl = frameName && runDetailRun?.run_id
        ? `${CONSOLE_API}/agent-runs/${encodeURIComponent(runDetailRun.run_id)}/frames/${encodeURIComponent(frameName)}`
        : null;
    const rawJson = esc(JSON.stringify(s, null, 2));
    return `<div class="step-row${res.ok ? '' : ' step-row-err'}">
        <div class="step-row-head">
            <span class="step-idx">#${esc(s.step ?? idx)}</span>
            <span class="step-time" title="${esc(absTime)}">${esc(offset)}</span>
            <span class="step-action-badge cat-${esc(actionCategory(ak))}">${esc(ak)}</span>
            <span class="step-brief">${esc(brief)}</span>
            <span class="step-result ${res.ok ? 'ok' : 'err'}">${esc(res.text)}</span>
            <span class="step-chevron" aria-hidden="true">▸</span>
        </div>
        <div class="step-row-body">
            ${frameUrl ? `<div class="step-frame"><img data-frame="${esc(frameUrl)}" src="${esc(frameUrl)}" loading="lazy" alt="frame"></div>` : ''}
            <div class="step-info">
                <div class="step-metrics">
                    <span title="provider latency">⚡ ${esc(t('stepLatency'))} ${esc(formatDuration(u.provider_latency_ms || 0))}</span>
                    <span title="step elapsed">⏲ ${esc(formatDuration(s.elapsed_ms || 0))}</span>
                    <span title="wall gap">⌛ ${esc(t('stepGap'))} ${esc(formatDuration(ctx.gap))}</span>
                    <span title="tokens in/out">⇅ ${esc(formatNumber(u.input_tokens || 0))} / ${esc(formatNumber(u.output_tokens || 0))}</span>
                    ${u.cost_usd_micros != null ? `<span>💲 ${esc(formatCostMicros(u.cost_usd_micros))}</span>` : ''}
                    ${sha ? `<span class="step-sha" title="screen sha256">⌗ ${esc(sha)}</span>` : ''}
                </div>
                <div class="step-cumulative">${esc(t('stepCumulative'))}: ⇅ ${esc(formatNumber(ctx.cumIn))} / ${esc(formatNumber(ctx.cumOut))}${ctx.cumCost ? ` · ${esc(formatCostMicros(ctx.cumCost))}` : ''}</div>
                <button type="button" class="btn-link step-copy" data-copy-step="${esc(idx)}">${esc(t('stepCopyJson'))}</button>
                <pre class="step-json">${rawJson}</pre>
            </div>
        </div>
    </div>`;
}

function openFrameLightbox(url) {
    const lb = document.getElementById('frame-lightbox');
    const img = document.getElementById('frame-lightbox-img');
    if (!lb || !img) return;
    img.src = url;
    lb.classList.add('show');
}

function closeFrameLightbox() {
    document.getElementById('frame-lightbox')?.classList.remove('show');
}

// ---------- Misc ----------

async function stopAgent() {
    try {
        await fetchJson(`${CONSOLE_API}/agent-stop`, { method: 'POST' });
        toast(t('stopAgent'), 'ok');
        loadOverview();
    } catch (e) {
        toast(t('actionFailed') + e, 'err');
    }
}

async function fetchJson(url, options = {}) {
    const r = await fetch(url, { cache: 'no-store', ...options });
    const d = await parseJsonResponse(r);
    if (!r.ok || d.error) throw new Error(d.error || r.statusText);
    return d;
}

async function putJson(url, body) {
    const r = await fetch(url, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body)
    });
    const d = await parseJsonResponse(r);
    if (!r.ok || d.error) throw new Error(d.error || r.statusText);
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
    ['settings', 'agent', 'provider'].forEach(group => {
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

let schedulesCache = [];
let schedulesEditId = null;

async function loadSchedules() {
    try {
        const d = await fetchJson(`${CONSOLE_API}/schedules`);
        schedulesCache = d.schedules || [];
        renderSchedulesList();
    } catch (e) {
        toast(t('fetchFailed') + (e.message || e), 'err');
    }
}

function renderSchedulesList() {
    const list = document.getElementById('schedules-list');
    if (!list) return;
    if (!schedulesCache.length) {
        list.innerHTML = `<div class="schedules-empty">${esc(t('schedulesEmpty'))}</div>`;
        return;
    }
    list.innerHTML = schedulesCache.map(renderScheduleRow).join('');
    list.querySelectorAll('[data-action="edit"]').forEach(b => b.addEventListener('click', (e) => {
        e.stopPropagation();
        const id = b.dataset.id;
        const st = schedulesCache.find(s => s.id === id);
        if (st) openScheduleEditor(st);
    }));
    list.querySelectorAll('[data-action="delete"]').forEach(b => b.addEventListener('click', async (e) => {
        e.stopPropagation();
        if (!confirm(t('scheduleDeleteConfirm'))) return;
        try {
            const r = await fetch(`${CONSOLE_API}/schedules/${encodeURIComponent(b.dataset.id)}`, { method: 'DELETE' });
            const d = await parseJsonResponse(r);
            if (!r.ok || d.error) throw new Error(d.error || r.statusText);
            toast(t('scheduleDeleted'), 'ok');
            loadSchedules();
        } catch (err) { toast(t('scheduleDeleteFailed') + (err.message || err), 'err'); }
    }));
    list.querySelectorAll('[data-action="run-now"]').forEach(b => b.addEventListener('click', async (e) => {
        e.stopPropagation();
        try {
            const r = await fetch(`${CONSOLE_API}/schedules/${encodeURIComponent(b.dataset.id)}/run-now`, { method: 'POST' });
            const d = await parseJsonResponse(r);
            if (r.status === 409) { toast(t('newTaskAlreadyActive'), 'err'); return; }
            if (!r.ok || d.error) throw new Error(d.error || r.statusText);
            toast(t('scheduleRunStarted'), 'ok');
            loadSchedules();
            loadRuns().catch(() => {});
        } catch (err) { toast(t('scheduleRunFailed') + (err.message || err), 'err'); }
    }));
    list.querySelectorAll('[data-action="toggle"]').forEach(cb => cb.addEventListener('change', async (e) => {
        const id = cb.dataset.id;
        const st = schedulesCache.find(s => s.id === id);
        if (!st) return;
        const updated = { ...st, enabled: cb.checked };
        delete updated.id; delete updated.next_fire_ms; delete updated.last_run_ms;
        delete updated.last_run_id; delete updated.last_outcome; delete updated.last_skip_reason;
        try {
            const r = await fetch(`${CONSOLE_API}/schedules/${encodeURIComponent(id)}`, {
                method: 'PUT', headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(updated)
            });
            const d = await parseJsonResponse(r);
            if (!r.ok || d.error) throw new Error(d.error || r.statusText);
            loadSchedules();
        } catch (err) {
            toast(t('scheduleSaveFailed') + (err.message || err), 'err');
            cb.checked = !cb.checked;
        }
    }));
}

function renderScheduleRow(st) {
    const next = st.next_fire_ms ? formatDateTime(st.next_fire_ms) : t('scheduleNever');
    const last = st.last_run_ms ? formatDateTime(st.last_run_ms) : t('scheduleNever');
    const outcomeLabel = st.last_outcome === 'started' ? t('scheduleOutcomeStarted')
        : st.last_outcome === 'skipped' ? t('scheduleOutcomeSkipped')
        : st.last_outcome === 'failed' ? t('scheduleOutcomeFailed')
        : (st.last_outcome || '');
    const outcomeClass = st.last_outcome === 'failed' ? 'sched-outcome-failed'
        : st.last_outcome === 'skipped' ? 'sched-outcome-skipped'
        : st.last_outcome === 'started' ? 'sched-outcome-started'
        : '';
    const skipReason = st.last_skip_reason ? `<span class="sched-skip-reason" title="${esc(st.last_skip_reason)}">${esc(st.last_skip_reason)}</span>` : '';
    return `
        <div class="schedule-row${st.enabled ? '' : ' disabled'}" data-id="${esc(st.id)}">
            <div class="sched-main">
                <div class="sched-name">${esc(st.name)}</div>
                <div class="sched-meta">
                    <code class="sched-cron">${esc(st.cron)}</code>
                    <span class="sched-provider">${esc(st.provider)}</span>
                </div>
                <div class="sched-task">${esc(truncate(st.task || '', 140))}</div>
            </div>
            <div class="sched-side">
                <div class="sched-times">
                    <div><span class="sched-label">${esc(t('scheduleNextRun'))}</span> ${esc(next)}</div>
                    <div><span class="sched-label">${esc(t('scheduleLastRun'))}</span> ${esc(last)} ${outcomeLabel ? `<span class="sched-outcome ${outcomeClass}">${esc(outcomeLabel)}</span>` : ''}</div>
                    ${skipReason}
                </div>
                <div class="sched-actions">
                    <label class="sched-toggle">
                        <input type="checkbox" data-action="toggle" data-id="${esc(st.id)}" ${st.enabled ? 'checked' : ''}>
                        <span>${esc(t('scheduleEnabled'))}</span>
                    </label>
                    <button class="btn btn-cancel btn-sm" data-action="run-now" data-id="${esc(st.id)}" type="button">${esc(t('scheduleRunNow'))}</button>
                    <button class="btn btn-cancel btn-sm" data-action="edit" data-id="${esc(st.id)}" type="button">${esc(t('scheduleEdit'))}</button>
                    <button class="btn btn-cancel btn-sm sched-delete" data-action="delete" data-id="${esc(st.id)}" type="button">${esc(t('scheduleDelete'))}</button>
                </div>
            </div>
        </div>
    `;
}

async function fillScheduleProviderSelect() {
    const select = document.getElementById('schedule-provider');
    if (!select) return;
    try {
        const providers = await fetchJson(`${CONSOLE_API}/providers`).catch(() => ({ providers: [] }));
        const list = providers.providers || [];
        const prev = select.value;
        select.innerHTML = list.map(p => `<option value="${esc(p.name)}">${esc(p.name)}</option>`).join('');
        if (prev && list.find(p => p.name === prev)) select.value = prev;
    } catch (_) {}
}

function openScheduleEditor(st) {
    const panel = document.getElementById('schedule-edit-panel');
    if (!panel) return;
    panel.classList.remove('hidden');
    schedulesEditId = st && st.id ? st.id : null;
    setTextById('schedule-edit-title', t(schedulesEditId ? 'scheduleEditTitle' : 'scheduleEditNewTitle'));
    document.getElementById('schedule-name').value = st?.name || '';
    document.getElementById('schedule-cron').value = st?.cron || '0 9 * * *';
    document.getElementById('schedule-task').value = st?.task || '';
    document.getElementById('schedule-max-steps').value = st?.budget?.max_steps || '';
    document.getElementById('schedule-max-wall').value = st?.budget?.max_wall_seconds || '';
    document.getElementById('schedule-enabled').checked = st?.enabled !== false;
    document.getElementById('schedule-edit-status').textContent = '';
    fillScheduleProviderSelect().then(() => {
        if (st?.provider) document.getElementById('schedule-provider').value = st.provider;
    });
}

function closeScheduleEditor() {
    document.getElementById('schedule-edit-panel')?.classList.add('hidden');
    schedulesEditId = null;
}

async function saveSchedule() {
    const name = document.getElementById('schedule-name').value.trim();
    const cron = document.getElementById('schedule-cron').value.trim();
    const task = document.getElementById('schedule-task').value.trim();
    const provider = document.getElementById('schedule-provider').value;
    const maxSteps = parseInt(document.getElementById('schedule-max-steps').value, 10);
    const maxWall = parseInt(document.getElementById('schedule-max-wall').value, 10);
    const enabled = document.getElementById('schedule-enabled').checked;
    if (!name || !cron || !task || !provider) {
        document.getElementById('schedule-edit-status').textContent = t('newTaskTaskRequired');
        return;
    }
    const body = { name, cron, task, provider, enabled };
    if (Number.isFinite(maxSteps) && maxSteps > 0 || Number.isFinite(maxWall) && maxWall > 0) {
        const budget = {};
        if (Number.isFinite(maxSteps) && maxSteps > 0) budget.max_steps = maxSteps;
        if (Number.isFinite(maxWall) && maxWall > 0) budget.max_wall_seconds = maxWall;
        body.budget = budget;
    }
    const isUpdate = !!schedulesEditId;
    const url = isUpdate
        ? `${CONSOLE_API}/schedules/${encodeURIComponent(schedulesEditId)}`
        : `${CONSOLE_API}/schedules`;
    try {
        const r = await fetch(url, {
            method: isUpdate ? 'PUT' : 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body)
        });
        const d = await parseJsonResponse(r);
        if (!r.ok || d.error) throw new Error(d.error || r.statusText);
        toast(t('scheduleSaved'), 'ok');
        closeScheduleEditor();
        loadSchedules();
    } catch (e) {
        document.getElementById('schedule-edit-status').textContent = (e.message || e);
        toast(t('scheduleSaveFailed') + (e.message || e), 'err');
    }
}

document.addEventListener('DOMContentLoaded', () => {
    document.body.dataset.section = activeSection;
    document.querySelectorAll('.nav-item').forEach(btn => {
        btn.addEventListener('click', () => switchSection(btn.dataset.section));
    });
    document.getElementById('add-app-btn').addEventListener('click', showAdd);
    document.getElementById('lang-toggle').addEventListener('click', () => {
        setLang(currentLang === 'en' ? 'zh' : 'en');
    });
    onLangChange((lang) => {
        currentLang = lang;
        lastDataHash = '';
        applyTranslations();
        loadSection();
    });
    document.getElementById('f-app-type').addEventListener('change', updateAppTypeVisibility);
    document.getElementById('modal-close').addEventListener('click', hideModal);

    document.querySelectorAll('.btn-cancel').forEach(btn => {
        btn.addEventListener('click', (e) => {
            if (e.target.closest('#log-modal')) hideLogModal();
            else if (e.target.closest('#modal')) hideModal();
        });
    });

    document.getElementById('modal-save').addEventListener('click', saveApp);
    document.getElementById('settings-save').addEventListener('click', () => saveSettings().catch(e => toast(t('actionFailed') + e, 'err')));
    document.getElementById('request-keyframe').addEventListener('click', () => requestKeyframe().catch(e => toast(t('actionFailed') + e, 'err')));
    document.getElementById('agent-save').addEventListener('click', () => saveAgentConfig().catch(e => toast(t('actionFailed') + e, 'err')));
    document.getElementById('provider-save').addEventListener('click', () => saveProvider().catch(e => toast(t('actionFailed') + e, 'err')));
    document.getElementById('provider-reset')?.addEventListener('click', resetProvider);
    document.getElementById('provider-test')?.addEventListener('click', testProvider);
    document.getElementById('runs-refresh').addEventListener('click', loadRuns);
    document.getElementById('new-task-start')?.addEventListener('click', () => startTask().catch(e => toast(t('newTaskFailed') + (e.message || e), 'err')));
    document.getElementById('runs-search')?.addEventListener('input', (e) => {
        runsSearchTerm = e.target.value;
        renderRunsList();
    });
    document.getElementById('run-detail-back')?.addEventListener('click', closeRunDetail);
    document.getElementById('run-detail-close')?.addEventListener('click', closeRunDetail);
    document.getElementById('run-detail-overlay')?.addEventListener('click', (e) => {
        if (e.target.id === 'run-detail-overlay') closeRunDetail();
    });
    document.getElementById('frame-lightbox')?.addEventListener('click', closeFrameLightbox);
    document.addEventListener('keydown', (e) => {
        if (e.key !== 'Escape') return;
        const lb = document.getElementById('frame-lightbox');
        if (lb?.classList.contains('show')) { closeFrameLightbox(); return; }
        const ov = document.getElementById('run-detail-overlay');
        if (ov?.classList.contains('show')) closeRunDetail();
    });
    document.getElementById('overview-agent-stop').addEventListener('click', stopAgent);

    document.getElementById('schedules-refresh')?.addEventListener('click', loadSchedules);
    document.getElementById('schedule-new')?.addEventListener('click', () => openScheduleEditor(null));
    document.getElementById('schedule-edit-save')?.addEventListener('click', () => saveSchedule().catch(e => toast(t('scheduleSaveFailed') + (e.message || e), 'err')));
    document.getElementById('schedule-edit-cancel')?.addEventListener('click', closeScheduleEditor);
    document.getElementById('schedule-edit-close')?.addEventListener('click', closeScheduleEditor);

    // Quick action / overview jump buttons
    document.addEventListener('click', (e) => {
        const btn = e.target.closest('[data-jump]');
        if (btn) switchSection(btn.dataset.jump);
    });

    // API key control
    document.getElementById('api-key-replace')?.addEventListener('click', () => setApiKeyMode('input'));
    document.getElementById('api-key-clear')?.addEventListener('click', () => {
        setApiKeyMode('pending-clear');
        reconcileDirty('provider');
    });
    document.getElementById('api-key-cancel')?.addEventListener('click', () => {
        setApiKeyMode('stored');
        reconcileDirty('provider');
    });
    document.getElementById('api-key-undo-clear')?.addEventListener('click', () => {
        setApiKeyMode('stored');
        reconcileDirty('provider');
    });

    bindDirtyTracking();
    applyTranslations();
    loadSection();
    setInterval(() => {
        if (activeSection === 'apps') load();
        if (activeSection === 'overview') loadOverview();
    }, 5000);
    setInterval(() => {
        if (activeSection === 'overview') updateOverviewMeta();
    }, 1000);
});
