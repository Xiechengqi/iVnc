const API = '/api/apps';
let editId = null;
let lastDataHash = '';

// Helper for XSS safe escaping
function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }

// Generate a simple hash of the data to avoid flicker
function getHash(data) {
    return JSON.stringify(data.apps.map(a => ({ id: a.id, status: a.status, size: a.data_size_human })));
}

async function load() {
    try {
        const r = await fetch(API);
        const d = await r.json();
        
        const currentHash = getHash(d);
        if (currentHash === lastDataHash) return; // Skip update if no change
        lastDataHash = currentHash;

        const tb = document.getElementById('app-list');
        if (!d.apps || !d.apps.length) {
            tb.innerHTML = '<tr><td colspan="6" class="empty">暂无应用，点击上方按钮添加</td></tr>';
            return;
        }

        const fragment = document.createDocumentFragment();
        d.apps.forEach(a => {
            const tr = document.createElement('tr');
            const type = a.app_type === 'desktop' ? '桌面' : '网页';
            const configStr = a.app_type === 'desktop' ? (a.exec_command || '') : (a.url || '');
            const configShort = configStr.length > 30 ? configStr.slice(0, 30) + '...' : configStr;
            
            tr.innerHTML = `
                <td><strong>${esc(a.name)}</strong></td>
                <td><span class="badge">${type}</span></td>
                <td title="${esc(configStr)}">${esc(configShort)}</td>
                <td>
                    <div class="status-wrapper">
                        <span class="status status-${a.status}"></span>
                        <span>${a.status}</span>
                    </div>
                </td>
                <td class="data-size">${a.data_size_human}</td>
                <td class="actions"></td>
            `;

            const actionsCell = tr.querySelector('.actions');
            
            if (a.status === 'running') {
                const stopBtn = createBtn('停止', 'btn-stop btn-sm', () => act(a.id, 'stop'));
                const restartBtn = createBtn('重启', 'btn-restart btn-sm', () => act(a.id, 'restart'));
                actionsCell.append(stopBtn, restartBtn);
            } else {
                const startBtn = createBtn('启动', 'btn-start btn-sm', () => act(a.id, 'start'));
                actionsCell.append(startBtn);
            }

            const editBtn = createBtn('编辑', 'btn-edit btn-sm', () => showEdit(a.id));
            if (a.status === 'running') {
                editBtn.disabled = true;
                editBtn.title = '请先停止应用再编辑';
            }
            
            const logBtn = createBtn('日志', 'btn-log btn-sm', () => showLogs(a.id, a.name));
            const clearBtn = createBtn('清理', 'btn-clear btn-sm', () => clearData(a.id, a.name));
            const delBtn = createBtn('删除', 'btn-delete btn-sm', () => del(a.id, a.name));
            
            actionsCell.append(editBtn, logBtn, clearBtn, delBtn);
            fragment.appendChild(tr);
        });

        tb.innerHTML = '';
        tb.appendChild(fragment);
    } catch (e) { console.error('Load failed:', e); }
}

function createBtn(text, cls, onClick) {
    const b = document.createElement('button');
    b.className = 'btn ' + cls;
    b.textContent = text;
    b.addEventListener('click', onClick);
    return b;
}

function showAdd() {
    editId = null;
    const modal = document.getElementById('modal');
    modal.querySelector('#modal-title').textContent = '添加应用';
    modal.querySelector('#modal-save').textContent = '添加';
    
    document.getElementById('f-name').value = ''; 
    document.getElementById('f-name').disabled = false;
    document.getElementById('f-app-type').value = 'webapp';
    document.getElementById('f-app-type').disabled = false;
    document.getElementById('f-url').value = '';
    document.getElementById('f-mode').value = 'native';
    document.getElementById('f-nav').checked = false;
    document.getElementById('f-debug-port').value = '';
    document.getElementById('f-proxy-server').value = 'socks5://127.0.0.1:1080';
    document.getElementById('f-exec').value = '';
    document.getElementById('f-env').value = '';
    
    updateAppTypeVisibility();
    updateNavVisibility();
    modal.classList.add('show');
}

async function showEdit(id) {
    try {
        const r = await fetch(API + '/' + id); 
        const a = await r.json();
        editId = id;
        const modal = document.getElementById('modal');
        modal.querySelector('#modal-title').textContent = '编辑应用';
        modal.querySelector('#modal-save').textContent = '保存';
        
        document.getElementById('f-name').value = a.name; 
        document.getElementById('f-name').disabled = true;
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
            document.getElementById('f-proxy-server').value = a.proxy_server || '';
        }
        updateAppTypeVisibility();
        updateNavVisibility();
        modal.classList.add('show');
    } catch (e) { toast('获取失败: ' + e, 'err'); }
}

function hideModal() { 
    document.getElementById('modal').classList.remove('show'); 
}

function validateUrl(url) {
    if (!url) return false;
    try { new URL(url); return true; } catch { return false; }
}

async function saveApp() {
    const btn = document.getElementById('modal-save');
    const appType = document.getElementById('f-app-type').value;
    const body = { app_type: appType };
    
    if (!editId) {
        body.name = document.getElementById('f-name').value.trim();
        if (!body.name) return toast('请输入应用名称', 'err');
    }

    if (appType === 'desktop') {
        body.exec_command = document.getElementById('f-exec').value.trim();
        if (!body.exec_command) return toast('请输入启动命令', 'err');
        
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
        body.url = document.getElementById('f-url').value.trim();
        if (!validateUrl(body.url)) return toast('请输入有效的 URL', 'err');
        
        body.mode = document.getElementById('f-mode').value;
        body.show_nav = document.getElementById('f-nav').checked;
        const debugPort = document.getElementById('f-debug-port').value;
        body.remote_debugging_port = debugPort ? parseInt(debugPort) : null;
        const proxyServer = document.getElementById('f-proxy-server').value.trim();
        body.proxy_server = proxyServer || null;
    }

    btn.disabled = true;
    const originalText = btn.textContent;
    btn.textContent = '保存中...';

    try {
        const r = await fetch(editId ? API + '/' + editId : API, {
            method: editId ? 'PUT' : 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body)
        });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        hideModal(); 
        toast(editId ? '已更新' : '已添加', 'ok'); 
        load();
    } catch (e) { toast('操作失败: ' + e, 'err'); } 
    finally {
        btn.disabled = false;
        btn.textContent = originalText;
    }
}

async function act(id, action) {
    try {
        const r = await fetch(`${API}/${id}/${action}`, { method: 'POST' });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        toast(action === 'start' ? '已启动' : action === 'stop' ? '已停止' : '已重启', 'ok'); 
        load();
    } catch (e) { toast('操作失败', 'err'); }
}

async function del(id, name) {
    if (!confirm(`确认删除应用 "${name}"？`)) return;
    try {
        const r = await fetch(API + '/' + id, { method: 'DELETE' });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        toast('已删除', 'ok'); 
        load();
    } catch (e) { toast('删除失败', 'err'); }
}

async function clearData(id, name) {
    if (!confirm(`确认清理 "${name}" 的缓存数据？\n将清除所有Cookie、LocalStorage和缓存。`)) return;
    try {
        const r = await fetch(`${API}/${id}/clear-data`, { method: 'POST' });
        const d = await r.json();
        if (d.error) { toast(d.error, 'err'); return; }
        toast('已清理', 'ok'); 
        load();
    } catch (e) { toast('清理失败', 'err'); }
}

function toast(msg, type) {
    const t = document.getElementById('toast');
    t.textContent = msg; 
    t.className = 'toast show toast-' + type;
    setTimeout(() => t.classList.remove('show'), 2500);
}

async function showLogs(id, name) {
    const modal = document.getElementById('log-modal');
    modal.querySelector('#log-title').textContent = '日志: ' + name;
    const content = document.getElementById('log-content');
    content.textContent = '加载中...';
    modal.classList.add('show');
    try {
        const r = await fetch(`${API}/${id}/logs`); 
        const d = await r.json();
        content.textContent = d.logs || '(空)';
        content.scrollTop = content.scrollHeight;
    } catch (e) { content.textContent = '加载失败: ' + e; }
}

function hideLogModal() { 
    document.getElementById('log-modal').classList.remove('show'); 
}

function updateAppTypeVisibility() {
    const type = document.getElementById('f-app-type').value;
    document.getElementById('webapp-config').style.display = type === 'desktop' ? 'none' : 'block';
    document.getElementById('desktop-config').style.display = type === 'desktop' ? 'block' : 'none';
}

function updateNavVisibility() {
    const mode = document.getElementById('f-mode').value;
    const isWebView = mode === 'webview';
    document.getElementById('nav-row').style.display = isWebView ? 'none' : 'flex';
    document.getElementById('advanced-settings-group').style.display = isWebView ? 'none' : 'block';
    document.getElementById('f-nav').disabled = isWebView;
}

document.addEventListener('DOMContentLoaded', () => {
    // Event listeners for static elements
    document.querySelector('.btn-add').addEventListener('click', showAdd);
    document.getElementById('f-app-type').addEventListener('change', updateAppTypeVisibility);
    document.getElementById('f-mode').addEventListener('change', updateNavVisibility);
    
    // Modal close buttons
    document.querySelectorAll('.btn-cancel').forEach(btn => {
        btn.addEventListener('click', (e) => {
            if (e.target.closest('#log-modal')) hideLogModal();
            else hideModal();
        });
    });

    document.getElementById('modal-save').addEventListener('click', saveApp);

    load();
    setInterval(load, 5000);
});
