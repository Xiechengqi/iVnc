import { enhanceSelect } from './select-control.js';
import { getLang, serverErrorText } from './lib/i18n.js?v=1';

const STRINGS = {
    zh: {
        program: '应用', custom: '自定义命令', manual: '手动修改启动命令',
        profile: '用户数据目录', crash: '崩溃日志目录', port: '远程调试端口',
        automatic: '留空自动配置', noPort: '留空关闭远程调试',
        manualHint: '手动模式下，以下配置不参与启动。',
        restore: '切回配置生成将替换当前手动命令，是否继续？',
        portInvalid: '请输入 1–65535 的整数，或留空关闭远程调试。',
        pathInvalid: '请输入不含 .. 的绝对目录路径，或留空自动配置。',
        status: { checking: '检测中…', installed: '已安装', missing: '未安装', not_executable: '不可执行', unknown: '检测失败' },
        previewFailed: '无法预览启动命令，请检查配置。',
    },
    en: {
        program: 'Application', custom: 'Custom command', manual: 'Edit launch command manually',
        profile: 'Profile directory', crash: 'Crash directory', port: 'Remote debugging port',
        automatic: 'Leave empty to configure automatically', noPort: 'Leave empty to disable remote debugging',
        manualHint: 'These settings are inactive in manual command mode.',
        restore: 'Switching to configuration mode replaces the manual command. Continue?',
        portInvalid: 'Enter an integer from 1 to 65535, or leave empty to disable remote debugging.',
        pathInvalid: "Enter an absolute directory without '..', or leave empty for automatic configuration.",
        status: { checking: 'Checking…', installed: 'Installed', missing: 'Not installed', not_executable: 'Not executable', unknown: 'Check failed' },
        previewFailed: 'Cannot preview the launch command. Check the configuration.',
    },
};

// All state belongs to one app dialog. Polls and previews are canceled on close;
// sequence numbers reject responses to older field values in the same dialog.
export function createChromePreset(context, fieldError) {
    const el = id => document.getElementById(id);
    const preset = el('f-desktop-preset'), manual = el('f-chrome-manual'), exec = el('f-exec');
    const control = enhanceSelect(preset, el('label-desktop-preset'));
    const strings = () => STRINGS[getLang()] || STRINGS.en;
    let status = 'checking', timer, debounce, request, inventoryRequest, generation = 0;
    let customCommand = '', generatedCommand = '', manualCommand = '', lastPort = null;
    let activeSession = null, previewPending = false;

    const isChrome = () => preset.value === 'chrome';
    const generated = () => isChrome() && !manual.checked;
    const visible = () => context().session?.isCurrent() && el('f-app-type').value === 'desktop';
    const current = (session, seq) => visible() && context().session === session && seq === generation;
    const identity = () => {
        const { session, id, env } = context();
        return { ...(id ? { id } : { creation_id: session.draftId }), env_vars: env };
    };

    async function post(path, body, signal) {
        const response = await fetch(`/api/apps/${path}`, { method: 'POST',
            headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal });
        const data = await response.json();
        if (!response.ok || data.error) throw new Error(serverErrorText(data) || response.statusText);
        return data;
    }

    function renderStatus() {
        const text = `Chrome · ${strings().status[status] || strings().status.unknown}`;
        if (preset.options[1].textContent !== text) preset.options[1].textContent = text;
    }

    async function inventory() {
        const session = context().session;
        if (!visible() || inventoryRequest) return;
        const controller = new AbortController(); inventoryRequest = controller;
        try {
            const data = await post('desktop-presets', identity(), controller.signal);
            if (session !== context().session || !visible() || inventoryRequest !== controller) return;
            status = data.presets?.find(p => p.id === 'chrome')?.installation?.status || 'unknown';
            renderStatus();
        } catch (error) {
            if (error.name !== 'AbortError' && session === context().session && visible()) { status = 'unknown'; renderStatus(); }
        } finally {
            if (inventoryRequest === controller) {
                inventoryRequest = null;
                if (session === context().session && visible()) timer = setTimeout(inventory, 5000);
            }
        }
    }

    function read() {
        if (!isChrome()) return { desktop_preset: 'custom' };
        const rawPort = el('f-chrome-port').value.trim();
        const port = rawPort === '' ? null : /^\d+$/.test(rawPort) && Number(rawPort) <= 65535 ? Number(rawPort) : rawPort;
        return { desktop_preset: 'chrome', chrome_config: {
            mode: manual.checked ? 'manual' : 'generated',
            profile_dir: el('f-chrome-profile').value.trim() || null,
            crash_dir: el('f-chrome-crash').value.trim() || null,
            debug_port: manual.checked && typeof port === 'string' ? lastPort : port,
        }, proxy_url: el('f-proxy-url').value.trim() || null,
        ...(manual.checked ? { exec_command: exec.value } : {}) };
    }

    function validation(show = true) {
        if (!generated()) return true;
        const port = el('f-chrome-port').value.trim();
        if (port && (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535)) {
            if (show) fieldError('f-chrome-port', strings().portInvalid);
            return false;
        }
        for (const id of ['f-chrome-profile', 'f-chrome-crash']) {
            const path = el(id).value.trim();
            if (path && (!path.startsWith('/') || path.split('/').includes('..') || /[\r\n\0]/.test(path))) {
                if (show) fieldError(id, strings().pathInvalid);
                return false;
            }
        }
        return true;
    }

    function effective(data) {
        el('chrome-profile-effective').textContent = data?.profile_dir || '';
        el('chrome-crash-effective').textContent = data?.crash_dir || '';
    }

    function pending(value) {
        previewPending = value;
        const { session, builtin } = context();
        manual.disabled = builtin || !!session?.saving || !!session?.restarting || value;
    }

    async function preview() {
        clearTimeout(debounce); request?.abort();
        const session = context().session, seq = ++generation;
        if (!visible() || !generated() || !validation(false)) { pending(false); return; }
        pending(true);
        const controller = new AbortController(); request = controller;
        try {
            const data = await post('chrome-preview', { ...identity(), ...read() }, controller.signal);
            if (!current(session, seq) || !generated()) return;
            if (typeof data.exec_command !== 'string' || !data.chrome_config) throw new Error(strings().previewFailed);
            generatedCommand = data.exec_command;
            exec.value = generatedCommand;
            lastPort = data.chrome_config.debug_port;
            effective(data.chrome_effective);
            el('chrome-preview-error').textContent = '';
        } catch (error) {
            if (error.name !== 'AbortError' && current(session, seq) && generated()) {
                el('chrome-preview-error').textContent = error.message || strings().previewFailed;
            }
        } finally { if (request === controller) request = null; if (current(session, seq)) pending(false); }
    }

    function schedule() {
        ++generation; request?.abort(); clearTimeout(debounce);
        pending(visible() && generated() && validation(false));
        if (previewPending) debounce = setTimeout(preview, 180);
    }

    function stop() {
        clearTimeout(timer); clearTimeout(debounce); ++generation;
        request?.abort(); inventoryRequest?.abort(); request = inventoryRequest = null;
        pending(false);
    }

    function sync() {
        const { session, id, builtin } = context();
        const chrome = isChrome(), busy = !!session?.saving || !!session?.restarting;
        el('chrome-settings').hidden = !chrome;
        el('chrome-manual-control').hidden = !chrome;
        el('chrome-manual-hint').hidden = !manual.checked;
        el('chrome-config').disabled = manual.checked || busy || builtin;
        el('proxy-url-line').hidden = !chrome;
        el('desktop-launch-advanced').hidden = chrome;
        exec.readOnly = builtin || generated();
        exec.rows = chrome ? 5 : 3;
        preset.disabled = builtin || busy || id === 'builtin-chrome';
        manual.disabled = builtin || busy || previewPending;
        control.sync();
        if (!visible()) { stop(); activeSession = null; }
        else if (activeSession !== session) {
            activeSession = session; inventory();
            if (generated()) schedule();
        }
    }

    function reset() {
        stop(); activeSession = null;
        preset.value = 'custom'; manual.checked = false;
        customCommand = generatedCommand = manualCommand = ''; lastPort = null;
        ['f-chrome-profile', 'f-chrome-crash', 'f-chrome-port'].forEach(id => { el(id).value = ''; });
        effective(null); el('chrome-preview-error').textContent = '';
        status = 'checking'; translate(); control.close();
    }

    function load(app) {
        const config = app.chrome_config;
        preset.value = app.desktop_preset === 'chrome' || config || app.proxy_configurable ? 'chrome' : 'custom';
        manual.checked = config?.mode === 'manual';
        el('f-chrome-profile').value = config?.profile_dir || '';
        el('f-chrome-crash').value = config?.crash_dir || '';
        el('f-chrome-port').value = config?.debug_port ?? '';
        lastPort = config?.debug_port ?? null;
        el('f-proxy-url').value = app.proxy_url || '';
        if (isChrome()) {
            generatedCommand = manual.checked ? '' : app.exec_command || '';
            manualCommand = manual.checked ? app.exec_command || '' : '';
        } else customCommand = app.exec_command || '';
        effective(app.chrome_effective); sync();
    }

    function translate() {
        const text = strings();
        for (const [id, key] of Object.entries({ 'label-desktop-preset': 'program', 'label-chrome-manual': 'manual',
            'label-chrome-profile': 'profile', 'label-chrome-crash': 'crash', 'label-chrome-port': 'port', 'chrome-manual-hint': 'manualHint' })) el(id).textContent = text[key];
        ['f-chrome-profile', 'f-chrome-crash'].forEach(id => { el(id).placeholder = text.automatic; });
        el('f-chrome-port').placeholder = text.noPort;
        preset.options[0].textContent = text.custom;
        renderStatus(); control.sync();
    }

    preset.addEventListener('change', () => {
        stop(); activeSession = null;
        if (isChrome()) { customCommand = exec.value; exec.value = manual.checked ? manualCommand : generatedCommand; }
        else { if (manual.checked) manualCommand = exec.value; exec.value = customCommand; }
        sync();
        el('f-desktop-compat').disabled = isChrome();
        el('f-desktop-sandbox').disabled = isChrome() || el('f-desktop-compat').value === 'raw';
    });
    manual.addEventListener('change', () => {
        if (!manual.checked && exec.value !== generatedCommand && !confirm(strings().restore)) { manual.checked = true; return; }
        if (manual.checked) { manualCommand = exec.value; stop(); activeSession = null; }
        else { manualCommand = exec.value; exec.value = generatedCommand; }
        el('chrome-preview-error').textContent = ''; sync(); if (generated()) schedule();
    });
    ['f-chrome-profile', 'f-chrome-crash', 'f-chrome-port', 'f-proxy-url'].forEach(id => el(id).addEventListener('input', schedule));
    el('f-env').addEventListener('input', () => { clearTimeout(timer); inventoryRequest?.abort(); inventoryRequest = null; inventory(); });
    reset();
    return { isChrome, generated, read, validate: validation, sync, reset, load, translate, close: () => { stop(); activeSession = null; control.close(); } };
}
