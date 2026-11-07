import './remote-apps.css';
// Trusted application views only. Remote applications supply pixels, never HTML.
const zh = () => (localStorage.getItem('ivnc-lang') || document.documentElement.lang || navigator.language).startsWith('zh');
export const isActiveAppView = () => !document.hidden && !window.__ivncRootViewSuspended && (!window.__IVNC_APP_VIEW__ || (window.__ivncViewActive && window.__ivncCanControl));
export const remoteText = (cn, en) => zh() ? cn : en;
async function jsonRequest(url, options) {
    const response = await fetch(url, options);
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error || response.statusText);
    return body;
}
function element(tag, text, className) {
    const node = document.createElement(tag);
    if (text != null) node.textContent = text;
    if (className) node.className = className;
    return node;
}
function button(label, click) {
    const node = element('button', label, 'btn btn-sm');
    node.type = 'button'; node.addEventListener('click', click);
    return node;
}
function textArea(value, label, placeholder) {
    const node = document.createElement('textarea');
    node.rows = 3; node.value = value; node.placeholder = placeholder;
    node.setAttribute('aria-label', label);
    return node;
}
function time(seconds) {
    return seconds ? new Date(seconds * 1000).toLocaleString() : '—';
}
// Uniformly random from a 64-symbol alphabet: 24 symbols carry 144 bits.
function randomPassword() {
    const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_+=@#.';
    const bytes = crypto.getRandomValues(new Uint8Array(24));
    return Array.from(bytes, b => alphabet[b & 63]).join('');
}
// Mount lines are "PATH" (read-only) or "PATH rw".
function parseMounts(text) {
    return text.split('\n').map(line => line.trim()).filter(Boolean).map(line => {
        const writable = /\s+rw$/i.test(line);
        return {path: writable ? line.replace(/\s+rw$/i, '') : line, writable};
    });
}
export async function showSharing(id) {
    const endpoint = '/api/apps/' + encodeURIComponent(id) + '/sharing';
    const [data, serviceData] = await Promise.all([jsonRequest(endpoint), jsonRequest('/api/desktop/services')]);
    const desktop = data.policy.desktop;
    const overlay = element('div', null, 'modal-bg show');
    const panel = element('div', null, 'modal app-modal');
    overlay.setAttribute('role', 'dialog'); overlay.setAttribute('aria-modal', 'true');
    panel.classList.add('remote-sharing');
    overlay.append(panel);
    panel.append(element('h2', remoteText('远程访问与应用隔离', 'Remote access and application isolation')));
    const message = element('p'); message.setAttribute('role', 'status');

    panel.append(element('h3', remoteText('运行隔离（变更后需重启应用）', 'Runtime isolation (restart required)')));
    const isolation = document.createElement('select');
    isolation.setAttribute('aria-label', remoteText('隔离级别', 'Isolation level'));
    for (const [value, cn, en] of [
        ['strict', '严格隔离：私有文件、网络和服务，可共享', 'Strict: private files, network and services; can be shared'],
        ['host', '主机模式：使用主机文件、网络和环境，不能共享', 'Host: uses host files, network and environment; cannot be shared'],
    ]) {
        const option = element('option', remoteText(cn, en)); option.value = value;
        isolation.append(option);
    }
    isolation.value = desktop.isolation || 'strict';
    panel.append(isolation);
    const strictOptions = element('div');
    const mounts = textArea((desktop.mounts || []).map(m => m.path + (m.writable ? ' rw' : '')).join('\n'),
        remoteText('允许访问的主机路径', 'Allowed host paths'), '/data/projects\n/data/output rw');
    const networks = textArea((desktop.networks || []).join('\n'),
        remoteText('允许访问的内网', 'Allowed private networks'), '192.168.1.0/24');
    strictOptions.append(
        element('p', remoteText('允许访问的主机路径，每行一个；行尾加 rw 表示可写。远程控制者也能读写这些路径。', 'Allowed host paths, one per line; append rw for write access. Remote controllers can use these paths too.')),
        mounts,
        element('p', remoteText('允许访问的内网 IPv4 网段（CIDR），每行一个。B 自身地址始终禁止。', 'Allowed private IPv4 networks (CIDR), one per line. B\'s own addresses stay blocked.')),
        networks,
        element('p', remoteText('允许访问的本机后台服务', 'Allowed local background services')));
    const serviceInputs = [];
    for (const service of serviceData.services) {
        const check = document.createElement('input');
        check.type = 'checkbox'; check.value = service.id;
        check.checked = desktop.services.includes(service.id);
        const label = element('label', null, 'checkbox-row');
        label.append(check, document.createTextNode(service.name + ' — ' + service.url));
        serviceInputs.push(check); strictOptions.append(label);
    }
    const x11 = document.createElement('input');
    x11.type = 'checkbox'; x11.checked = desktop.x11;
    const x11Label = element('label', null, 'checkbox-row');
    x11Label.append(x11, document.createTextNode(remoteText('使用 X11 兼容环境', 'Use X11 compatibility environment')));
    panel.append(strictOptions, x11Label);

    panel.append(element('h3', remoteText('远程访问', 'Remote access')));
    const enabled = document.createElement('input');
    enabled.type = 'checkbox'; enabled.checked = data.policy.enabled;
    const enabledLabel = element('label', null, 'checkbox-row');
    enabledLabel.append(enabled, document.createTextNode(remoteText('允许远程访问', 'Allow remote access')));
    panel.append(enabledLabel);
    const passwordRow = element('div', null, 'password-row');
    const password = document.createElement('input');
    password.type = 'password'; password.autocomplete = 'new-password';
    password.placeholder = data.policy.password_set
        ? remoteText('留空保留密码；修改后撤销已有连接', 'Leave blank to keep password; changing it revokes connections')
        : remoteText('设置应用密码，至少 8 个字符', 'Set an application password, at least 8 characters');
    password.setAttribute('aria-label', remoteText('应用密码', 'Application password'));
    const reveal = button(remoteText('显示', 'Show'), () => {
        password.type = password.type === 'password' ? 'text' : 'password';
        reveal.textContent = password.type === 'password' ? remoteText('显示', 'Show') : remoteText('隐藏', 'Hide');
    });
    const generate = button(remoteText('生成强密码', 'Generate'), () => {
        password.value = randomPassword(); password.type = 'text';
        reveal.textContent = remoteText('隐藏', 'Hide');
    });
    passwordRow.append(password, reveal, generate);
    panel.append(passwordRow);
    const sharingUrl = element('input');
    sharingUrl.readOnly = true; sharingUrl.value = new URL(data.share_path, location.origin).href;
    sharingUrl.setAttribute('aria-label', remoteText('应用连接地址', 'Application URL'));
    panel.append(sharingUrl, button(remoteText('复制连接地址', 'Copy connection URL'), async () => {
        try { await navigator.clipboard.writeText(sharingUrl.value); }
        catch { sharingUrl.select(); document.execCommand('copy'); }
    }));
    panel.append(element('p', remoteText('连接者可操作此应用及其已有登录会话。关闭共享或修改密码会立即断开所有远程连接。', 'Viewers can use this application and its existing signed-in sessions. Disabling access or changing the password disconnects all remote sessions.')));
    const syncIsolation = () => {
        const host = isolation.value === 'host';
        strictOptions.hidden = host;
        enabled.disabled = host;
        if (host) enabled.checked = false;
    };
    isolation.addEventListener('change', syncIsolation);
    syncIsolation();

    const connections = element('div');
    async function refreshConnections() {
        const latest = await jsonRequest(endpoint);
        connections.replaceChildren(element('h3', remoteText('已保存的远程连接', 'Saved remote connections')));
        for (const grant of latest.grants || []) {
            const who = [grant.label || remoteText('未命名实例', 'Unnamed instance'), grant.ip].filter(Boolean).join(' · ');
            const row = element('p', who + ' — ' + remoteText('最近使用 ', 'last used ') + time(grant.last_used || grant.created)
                + (grant.active_sessions ? remoteText('，在线 ', ', online ') + grant.active_sessions : '') + ' ');
            row.append(button(remoteText('撤销', 'Revoke'), async () => {
                if (!confirm(remoteText('撤销后该实例需重新输入应用密码。继续？', 'The instance will need the application password again. Continue?'))) return;
                try { await jsonRequest('/api/apps/' + encodeURIComponent(id) + '/grants/' + encodeURIComponent(grant.id), {method:'DELETE'}); await refreshConnections(); }
                catch(error) { message.textContent = error.message; }
            }));
            connections.append(row);
        }
        if (!latest.grants?.length) connections.append(element('p', remoteText('暂无已保存连接', 'No saved connections')));
        connections.append(element('h3', remoteText('当前远程会话', 'Remote sessions')));
        for (const session of latest.sessions || []) {
            const who = [session.client, session.peer].filter(Boolean).join(' · ');
            const row = element('p', who + ' — ' + remoteText('已连接 ', 'connected for ') + session.age_seconds + 's ');
            row.append(button(remoteText('断开', 'Disconnect'), async () => {
                try { await jsonRequest('/api/apps/' + encodeURIComponent(id) + '/sessions/' + encodeURIComponent(session.id), {method:'DELETE'}); await refreshConnections(); }
                catch(error) { message.textContent = error.message; }
            }));
            connections.append(row);
        }
        if (!latest.sessions?.length) connections.append(element('p', remoteText('暂无远程会话', 'No remote sessions')));
        connections.append(element('p', remoteText('断开只结束当前会话，对方可重新连接；撤销连接才会要求重新输入密码。', 'Disconnecting ends the session but the instance may reconnect; revoke its connection to require the password again.')));
    }
    panel.append(connections, button(remoteText('刷新', 'Refresh'), () => refreshConnections().catch(e => {message.textContent=e.message;})));
    const actions = element('div', null, 'modal-actions');
    const save = button(remoteText('保存', 'Save'), async () => {
        save.disabled = true;
        try {
            const host = isolation.value === 'host';
            const result = await jsonRequest(endpoint, {
                method: 'PUT', headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({enabled: enabled.checked, password: password.value, desktop: {
                    x11: x11.checked,
                    isolation: isolation.value,
                    services: host ? [] : serviceInputs.filter(i => i.checked).map(i => i.value),
                    mounts: host ? [] : parseMounts(mounts.value),
                    networks: host ? [] : networks.value.split('\n').map(v => v.trim()).filter(Boolean),
                }}),
            });
            password.value = ''; password.type = 'password';
            const notes = [remoteText('已保存。', 'Saved.')];
            if (result.revoked) notes.push(remoteText('已有远程连接已撤销。', 'Existing remote connections were revoked.'));
            if (result.restart_required) notes.push(remoteText('重启此应用后，新的隔离设置生效；在此之前不接受新的远程连接。', 'Restart this application to apply the isolation settings; new remote connections wait until then.'));
            message.textContent = notes.join(' ');
            await refreshConnections();
        } catch (error) { message.textContent = error.message; }
        finally { save.disabled = false; }
    });
    actions.append(button(remoteText('关闭', 'Close'), () => overlay.remove()), save);
    panel.append(message, actions); document.body.append(overlay);
    if (data.restart_required) message.textContent = remoteText('当前运行环境与已保存设置不同，重启应用后生效。', 'The running environment differs from the saved settings; restart the application to apply them.');
    await refreshConnections();
    isolation.focus();
}

// Each viewer owns its listeners and DOM in a distinct iframe document.
const views = new Map();
const opening = new Map();
let host, tabs, frames, current;
function activate(id) {
    current = id;
    for (const [key, value] of views) {
        value.frame.hidden = key !== id;
        value.tab.classList.toggle('active', key === id);
        value.frame.contentWindow?.postMessage({type:'ivnc-view-active', active:key === id && !host.hidden}, location.origin);
    }
    if (!window.__ivncRootViewSuspended && !host.hidden) window.dispatchEvent(new Event('blur'));
    window.__ivncRootViewSuspended = !host.hidden;
    window.dispatchEvent(new Event('ivnc-root-view-state'));
}
function ensureHost() {
    if (host) return;
    host = element('section');
    host.setAttribute('aria-label', remoteText('应用视图', 'Application views'));
    host.style.cssText = 'position:fixed;inset:12px;z-index:20000;background:#101214;border:1px solid #555;border-radius:8px;display:flex;flex-direction:column;box-shadow:0 12px 60px #0008';
    tabs = element('nav'); tabs.style.cssText = 'display:flex;gap:8px;padding:8px;align-items:center;min-height:42px;overflow-x:auto;background:#202328';
    tabs.append(button(remoteText('返回应用列表', 'Back to applications'), () => {
        host.hidden = true; host.style.display = 'none'; activate(current);
    }));
    frames = element('div'); frames.style.cssText = 'position:relative;flex:1;min-height:0';
    host.append(tabs, frames); document.body.append(host);
}
async function openHosted(app) {
    if (opening.has(app.id)) return opening.get(app.id);
    const promise = createHosted(app).finally(() => opening.delete(app.id));
    opening.set(app.id, promise);
    return promise;
}
async function createHosted(app) {
    ensureHost();
    if (views.has(app.id)) { host.hidden = false; host.style.display = 'flex'; activate(app.id); return; }
    const result = await jsonRequest('/api/apps/' + encodeURIComponent(app.id) + '/view', {method:'POST'});
    const frame = document.createElement('iframe');
    frame.src = result.url + '?app=' + encodeURIComponent(app.id);
    frame.title = app.name; frame.allow = 'autoplay; clipboard-read; clipboard-write; fullscreen';
    frame.style.cssText = 'width:100%;height:100%;border:0;position:absolute;inset:0';
    const tab = element('span'); tab.style.cssText = 'display:inline-flex;gap:4px;white-space:nowrap';
    tab.append(button(app.name, () => activate(app.id)), button('×', () => {
        const url = views.get(app.id)?.url;
        frame.contentWindow?.postMessage({type:'ivnc-view-active',active:false}, location.origin);
        frame.remove(); tab.remove(); views.delete(app.id);
        if (url) fetch(url + 'close', {method:'POST',keepalive:true}).catch(() => {});
        if (views.size) activate(views.keys().next().value);
        else {host.hidden = true;host.style.display = 'none';activate(null);}
    }));
    views.set(app.id, {frame,tab,url:result.url,app});
    frame.addEventListener('load', () => activate(current));
    tabs.append(tab); frames.append(frame);
    host.hidden = false; host.style.display = 'flex'; activate(app.id);
}
window.addEventListener('pagehide', () => {
    for (const {url} of views.values()) fetch(url + 'close', {method:'POST',keepalive:true}).catch(() => {});
});
window.addEventListener('message', async event => {
    if (event.origin !== location.origin || event.data?.type !== 'ivnc-reopen') return;
    const view = [...views.values()].find(v => v.frame.contentWindow === event.source);
    if (!view) return;
    try {
        const result = await jsonRequest('/api/apps/' + encodeURIComponent(view.app.id) + '/view', {method:'POST'});
        fetch(view.url + 'close', {method:'POST'}).catch(() => {});
        view.url = result.url;
        view.frame.src = result.url + '?app=' + encodeURIComponent(view.app.id);
    } catch(error) { view.frame.contentWindow.postMessage({type:'ivnc-reopen-error',message:error.message}, location.origin); }
});
window.__ivncOpenAppView = openHosted;
export async function openAppView(app) {
    let owner = window;
    try { if (window.top.__ivncOpenAppView) owner = window.top; } catch {}
    await owner.__ivncOpenAppView(app);
}

export function attachAppControls(signaling) {
    if (!window.__IVNC_APP_VIEW__) return;
    document.getElementById('app-view-controls')?.remove();
    const bar = element('div'); bar.id = 'app-view-controls';
    bar.style.cssText = 'position:fixed;right:8px;top:8px;z-index:10001;background:#202328ee;color:#eee;display:flex;gap:8px;padding:6px;border-radius:6px;font:13px sans-serif';
    const status = element('span', remoteText('连接中…', 'Connecting…'));
    const send = action => {
        if (signaling._ws_conn?.readyState === WebSocket.OPEN) signaling._ws_conn.send(JSON.stringify({ivnc_control:action}));
    };
    const claim = button(remoteText('取得控制', 'Request control'), () => send('claim'));
    const take = button(remoteText('接管', 'Take over'), () => send('takeover')); take.hidden = true;
    const release = button(remoteText('释放控制', 'Release control'), () => send('release'));
    const download = button(remoteText('下载文件', 'Downloads'), async () => {
        try {
            const base = location.pathname;
            const data = await jsonRequest(base + 'app/files');
            const panel = element('div');
            panel.style.cssText = 'position:fixed;inset:15% 20%;z-index:10002;background:#202328;color:white;padding:20px;overflow:auto;border:1px solid #555';
            panel.append(button(remoteText('关闭', 'Close'), () => panel.remove()));
            for (const file of data.files) {
                const row = element('p'); const link = element('a', file.name + ' (' + file.size + ' B)');
                link.href = base + 'app/files/' + encodeURIComponent(file.name); link.download = file.name; link.style.color = '#a8ccff';
                row.append(link); panel.append(row);
            }
            if (!data.files.length) panel.append(element('p', remoteText('Downloads 目录中暂无文件', 'No files in Downloads')));
            document.body.append(panel);
        } catch (error) { status.textContent = error.message; }
    });
    const retry = button(remoteText('重新连接', 'Reconnect'), async () => {
        if (window.parent !== window) { window.parent.postMessage({type:'ivnc-reopen'},location.origin); return; }
        try {
            const id = new URLSearchParams(location.search).get('app');
            const result = await jsonRequest('/api/apps/' + encodeURIComponent(id) + '/view', {method:'POST'});
            fetch(location.pathname + 'close', {method:'POST',keepalive:true}).catch(() => {});
            location.replace(result.url + '?app=' + encodeURIComponent(id));
        } catch(error) { status.textContent=error.message; }
    });
    let lastError = '';
    const endings = {
        revoked: remoteText('访问已被撤销，请重新输入应用密码', 'Access was revoked; enter the application password again'),
        disconnected: remoteText('应用所有者断开了此连接', 'The application owner ended this session'),
        stopped: remoteText('应用已停止', 'The application stopped'),
    };
    window.addEventListener('ivnc-view-ended', event => {
        lastError = endings[event.detail] || remoteText('连接已结束', 'The session ended');
        status.textContent = lastError;
        claim.hidden = true; release.hidden = true; take.hidden = true;
    });
    window.addEventListener('ivnc-view-error', event => {lastError=String(event.detail);status.textContent=lastError;});
    window.addEventListener('ivnc-view-disconnected', () => {status.textContent=lastError || remoteText('连接中断，正在重连…', 'Disconnected; reconnecting…');});
    window.addEventListener('message', event => {
        if (event.origin === location.origin && event.source === window.parent && event.data?.type === 'ivnc-reopen-error') status.textContent=event.data.message;
    });
    bar.append(status,claim,take,release,download,retry); document.body.append(bar);
    window.addEventListener('ivnc-control', event => {
        lastError = '';
        window.__ivncCanControl = event.detail.controller;
        status.textContent = event.detail.controller ? remoteText('控制中', 'Controlling') : remoteText('仅观看', 'Viewing only');
        claim.hidden = event.detail.controller; release.hidden = !event.detail.controller;
        take.hidden = !event.detail.owner || event.detail.controller;
    });
}
