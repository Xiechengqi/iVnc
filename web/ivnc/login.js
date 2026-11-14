import { getLang, setLang } from './lib/i18n.js?v=1';
import { checkAuthentication, safeReturnPath } from './auth-client.js';

const text = {
    zh: { title: '登录 · iVnc', heading: '登录远程桌面', username: '用户名', password: '密码', submit: '登录', busy: '正在登录…', invalid_credentials: '用户名或密码不正确。', login_rate_limited: '尝试次数过多，请稍后重试。', cookies: '浏览器未保存登录状态，请允许此站点使用 Cookie 后重试。', network: '连接失败，请检查网络后重试。' },
    en: { title: 'Sign in · iVnc', heading: 'Sign in to your desktop', username: 'Username', password: 'Password', submit: 'Sign in', busy: 'Signing in…', invalid_credentials: 'Incorrect username or password.', login_rate_limited: 'Too many attempts. Please try again later.', cookies: 'Your browser did not save the session. Allow cookies for this site and try again.', network: 'Connection failed. Check your network and try again.' }
};
const form = document.getElementById('login-form'), submit = document.getElementById('submit'), error = document.getElementById('error');
let busy = false, errorKey = '';
const next = safeReturnPath(new URLSearchParams(location.search).get('next'));
const labels = () => text[getLang()];
function render() {
    const t = labels(); document.documentElement.lang = getLang() === 'zh' ? 'zh-CN' : 'en'; document.title = t.title;
    document.getElementById('heading').textContent = t.heading;
    document.getElementById('username-label').textContent = t.username;
    document.getElementById('password-label').textContent = t.password;
    document.getElementById('language').textContent = getLang() === 'zh' ? 'English' : '中文';
    submit.textContent = busy ? t.busy : t.submit; submit.disabled = busy;
    error.textContent = errorKey ? (t[errorKey] || t.network) : '';
}
document.getElementById('language').addEventListener('click', () => { setLang(getLang() === 'zh' ? 'en' : 'zh'); render(); });
form.addEventListener('submit', async event => {
    event.preventDefault(); if (busy) return;
    busy = true; errorKey = ''; render();
    try {
        await window.__ivncAuthReady;
        const response = await fetch('/auth/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username: form.elements.username.value, password: form.elements.password.value }) });
        const result = await response.json();
        if (!response.ok) errorKey = result.error || 'network';
        else {
            const authenticated = await checkAuthentication({ redirect: false });
            if (authenticated === true) { location.replace(next); return; }
            errorKey = authenticated === false ? 'cookies' : 'network';
        }
    } catch { errorKey = 'network'; }
    busy = false; render();
});
render();
// An expired iframe navigates the main page once, rather than displaying a
// second login form inside the console or a remote application's view.
try {
    if (window.top !== window && window.top.location.origin === location.origin) {
        if (typeof window.top.__ivncRequireLogin === 'function') window.top.__ivncRequireLogin();
        else window.top.location.replace('/auth/login?next=' + encodeURIComponent(safeReturnPath(window.top.location.pathname + window.top.location.search)));
    } else {
        checkAuthentication({ redirect: false }).then(authenticated => { if (authenticated === true) location.replace(next); });
    }
} catch { /* the form remains usable in a cross-origin embed */ }
