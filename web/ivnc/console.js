const API = '/api/apps';
let editId = null;

async function load() {
    try {
        const r = await fetch(API);
        const d = await r.json();
        const tb = document.getElementById('app-list');
        if (!d.apps || !d.apps.length) {
            tb.innerHTML = '<tr><td colspan="6" class="empty">暂无应用，点击上方按钮添加</td></tr>';
            return;
        }
        tb.innerHTML = d.apps.map(a => {
            const type = a.app_type === 'desktop' ? '桌面' : '网页';
            const config = a.app_type === 'desktop' ? esc(a.exec_command || '') : esc((a.url || '').length > 30 ? a.url.slice(0, 30) + '...' : a.url || '');
            return `<tr>
      <td><strong>${esc(a.name)}</strong></td>
      <td><span class="badge">${type}</span></td>
      <td title="${esc(a.app_type === 'desktop' ? a.exec_command || '' : a.url || '')}">${config}</td>
      <td>
        <div class="status-wrapper">
          <span class="status status-${a.status}"></span>
          <span>${a.status}</span>
        </div>
      </td>
      <td class="data-size">${a.data_size_human}</td>
      <td class="actions">
        ${a.status === 'running'
                    ? `<button class="btn btn-stop btn-sm" onclick="act('${a.id}','stop')">停止</button>
             <button class="btn btn-restart btn-sm" onclick="act('${a.id}','restart')">重启</button>`
                    : `<button class="btn btn-start btn-sm" onclick="act('${a.id}','start')">启动</button>`}
        <button class="btn btn-edit btn-sm" onclick="showEdit('${a.id}')" ${a.status === 'running' ? 'disabled title="请先停止应用再编辑"' : ''}>编辑</button>
        <button class="btn btn-log btn-sm" onclick="showLogs('${a.id}','${esc(a.name)}')">日志</button>
        <button class="btn btn-clear btn-sm" onclick="clearData('${a.id}','${esc(a.name)}')">清理</button>
        <button class="btn btn-delete btn-sm" onclick="del('${a.id}','${esc(a.name)}')">删除</button>
      </td></tr>`;
        }).join('');
    } catch (e) { toast('加载失败: ' + e, 'err'); }
}

window.showAdd = function() {
    editId = null;
    document.getElementById('modal-title').textContent = '添加应用';
    document.getElementById('modal-save').textContent = '添加';
    document.getElementById('f-name').value = ''; document.getElementById('f-name').disabled = false;
    document.getElementById('f-app-type').value = 'webapp';
    document.getElementById('f-url').value = '';
    document.getElementById('f-mode').value = 'native';
    document.getElementById('f-nav').checked = false;
    document.getElementById('f-debug-port').value = '';
    document.getElementById('f-exec').value = '';
    document.getElementById('f-env').value = '';
    updateAppTypeVisibility();
    updateNavVisibility();
    document.getElementById('modal').classList.add('show');
};

function updateAppTypeVisibility() {
    const type = document.getElementById('f-app-type').value;
    const webappConfig = document.getElementById('webapp-config');
    const desktopConfig = document.getElementById('desktop-config');
    if (type === 'desktop') {
        webappConfig.style.display = 'none';
        desktopConfig.style.display = 'block';
    } else {
        webappConfig.style.display = 'block';
        desktopConfig.style.display = 'none';
    }
}

window.showEdit = async function(id) {
    try {
        const r = await fetch(API + '/' + id); const a = await r.json();
        editId = id;
        document.getElementById('modal-title').textContent = '编辑应用';
        document.getElementById('modal-save').textContent = '保存';
        document.getElementById('f-name').value = a.name; document.getElementById('f-name').disabled = true;
        document.getElementById('f-app-type').value = a.app_type || 'webapp';
        document.getElementById('f-app-type').disabled = true;
        if (a.app_type === 'desktop') {
            document.getElementById('f-exec').value = a.exec_command || '';
            const envStr = a.env_vars ? Object.entries(a.env_vars).map(([k, v]) => `${k}=${v}`).join('\n') : '';
            document.getElementById('f-env').value = envStr;
        } else {
            document.getElementById('f-url').value = a.url || '';
            document.getElementById('f-mode').value = a.mode || 'native';
            document.getElementById('f-nav').checked = a.show_nav || false;
            document.getElementById('f-debug-port').value = a.remote_debugging_port || '';
        }
        updateAppTypeVisibility();
        updateNavVisibility();
        document.getElementById('modal').classList.add('show');
    } catch (e) { toast('获取失败', 'err'); }
};

window.hideModal = function() { document.getElementById('modal').classList.remove('show'); };

window.saveApp = async function() {
    const appType = document.getElementById('f-app-type').value;
    const body = { app_type: appType };
    if (!editId) body.name = document.getElementById('f-name').value;

    if (appType === 'desktop') {
        body.exec_command = document.getElementById('f-exec').value;
        const envText = document.getElementById('f-env').value.trim();
        if (envText) {
            const envVars = {};
            envText.split('\n').forEach(line => {
                const idx = line.indexOf('=');
                if (idx > 0) {
                    const k = line.substring(0, idx).trim();
                    const v = line.substring(idx + 1).trim();
                    if (k) envVars[k] = v;
                }
            });
            body.env_vars = envVars;
        }
    } else {
        body.url = document.getElementById('f-url').value;
        body.mode = document.getElementById('f-mode').value;
        body.show_nav = document.getElementById('f-nav').checked;
        const debugPort = document.getElementById('f-debug-port').value;
        body.remote_debugging_port = debugPort ? parseInt(debugPort) : null;
    }

    try {
        const r = await fetch(editId ? API + '/' + editId : API, {
            method: editId ? 'PUT' : 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body)
        });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        window.hideModal(); toast(editId ? '已更新' : '已添加', 'ok'); load();
    } catch (e) { toast('操作失败', 'err'); }
};

window.act = async function(id, action) {
    try {
        const r = await fetch(`${API}/${id}/${action}`, { method: 'POST' });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        toast(action === 'start' ? '已启动' : action === 'stop' ? '已停止' : '已重启', 'ok'); load();
    } catch (e) { toast('操作失败', 'err'); }
};

window.del = async function(id, name) {
    if (!confirm(`确认删除应用 "${name}"？`)) return;
    try {
        const r = await fetch(API + '/' + id, { method: 'DELETE' });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        toast('已删除', 'ok'); load();
    } catch (e) { toast('删除失败', 'err'); }
};

window.clearData = async function(id, name) {
    if (!confirm(`确认清理 "${name}" 的缓存数据？\n将清除所有Cookie、LocalStorage和缓存。`)) return;
    try {
        const r = await fetch(`${API}/${id}/clear-data`, { method: 'POST' });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        toast('已清理', 'ok'); load();
    } catch (e) { toast('清理失败', 'err'); }
};

function toast(msg, type) {
    const t = document.getElementById('toast');
    t.textContent = msg; t.className = 'toast toast-' + type + ' show';
    setTimeout(() => t.classList.remove('show'), 2500);
}

function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }

window.showLogs = async function(id, name) {
    document.getElementById('log-title').textContent = '日志: ' + name;
    document.getElementById('log-content').textContent = '加载中...';
    document.getElementById('log-modal').classList.add('show');
    try {
        const r = await fetch(`${API}/${id}/logs`); const d = await r.json();
        document.getElementById('log-content').textContent = d.logs || '(空)';
        const el = document.getElementById('log-content');
        el.scrollTop = el.scrollHeight;
    } catch (e) { document.getElementById('log-content').textContent = '加载失败: ' + e; }
};
window.hideLogModal = function() { document.getElementById('log-modal').classList.remove('show'); };

function updateNavVisibility() {
    const mode = document.getElementById('f-mode').value;
    const navRow = document.getElementById('nav-row');
    const debugPortRow = document.getElementById('debug-port-row');
    const navCheckbox = document.getElementById('f-nav');
    if (mode === 'webview') {
        navRow.style.display = 'none';
        debugPortRow.style.display = 'none';
        navCheckbox.disabled = true;
    } else {
        navRow.style.display = 'flex';
        debugPortRow.style.display = 'flex';
        navCheckbox.disabled = false;
    }
}

document.addEventListener('DOMContentLoaded', function() {
    document.getElementById('f-app-type').addEventListener('change', updateAppTypeVisibility);
    document.getElementById('f-mode').addEventListener('change', updateNavVisibility);
    load();
    setInterval(load, 5000);
});
