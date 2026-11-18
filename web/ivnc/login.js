import { getLang, setLang } from './lib/i18n.js?v=1';
import { checkAuthentication, safeReturnPath, loginByIp, onAuthChange } from './auth-client.js';

const text = {
    zh: { title: '登录 · iVnc', heading: '登录远程桌面', username: '用户名', password: '密码', submit: '登录', busy: '正在登录…', invalid_credentials: '用户名或密码不正确。', login_rate_limited: '尝试次数过多，请稍后重试。', cookies: '浏览器未保存登录状态，请允许此站点使用 Cookie 后重试。', network: '连接失败，请检查网络后重试。' },
    en: { title: 'Sign in · iVnc', heading: 'Sign in to your desktop', username: 'Username', password: 'Password', submit: 'Sign in', busy: 'Signing in…', invalid_credentials: 'Incorrect username or password.', login_rate_limited: 'Too many attempts. Please try again later.', cookies: 'Your browser did not save the session. Allow cookies for this site and try again.', network: 'Connection failed. Check your network and try again.' }
};
const form = document.getElementById('login-form'), submit = document.getElementById('submit'), error = document.getElementById('error');
const password = document.getElementById('password'), eye = document.getElementById('password-toggle');
const ipLogin = document.getElementById('ip-login'), loginCheck = document.getElementById('login-check');
const extraText = {
    zh: { show: '显示密码', hide: '隐藏密码', checking: '正在检查登录状态…', ipLogin: '免密进入' },
    en: { show: 'Show password', hide: 'Hide password', checking: 'Checking sign-in…', ipLogin: 'Continue without password' },
};
let busy = false, errorKey = '';
const returnPath = safeReturnPath(new URLSearchParams(location.search).get('next'));
// A server redirect cannot read fragments; browsers inherit them on /auth/login.
const next = returnPath.includes('#') ? returnPath : returnPath + location.hash;
const labels = () => text[getLang()];
function render() {
    const t = labels(); document.documentElement.lang = getLang() === 'zh' ? 'zh-CN' : 'en'; document.title = t.title;
    document.getElementById('heading').textContent = t.heading;
    document.getElementById('username-label').textContent = t.username;
    document.getElementById('password-label').textContent = t.password;
    document.getElementById('language').textContent = getLang() === 'zh' ? 'English' : '中文';
    submit.textContent = busy ? t.busy : t.submit; submit.disabled = busy;
    error.textContent = errorKey ? (t[errorKey] || t.network) : '';
    const extra = extraText[getLang()], visible = password.type === 'text';
    eye.title = extra[visible ? 'hide' : 'show']; eye.setAttribute('aria-label', eye.title);
    eye.setAttribute('aria-pressed', String(visible));
    document.getElementById('password-eye-slash').toggleAttribute('hidden', !visible);
    loginCheck.textContent = extra.checking;
    ipLogin.textContent = busy ? t.busy : extra.ipLogin; ipLogin.disabled = busy;
}
eye.addEventListener('pointerdown', event => { if (document.activeElement === password) event.preventDefault(); });
eye.addEventListener('click', () => {
    const { selectionStart, selectionEnd } = password;
    password.type = password.type === 'password' ? 'text' : 'password';
    render();
    if (selectionStart !== null) {
        password.setSelectionRange(selectionStart, selectionEnd);
        // Mobile engines may reset selection after the input changes type.
        const value = password.value, type = password.type;
        requestAnimationFrame(() => {
            if (password.type === type && password.value === value) password.setSelectionRange(selectionStart, selectionEnd);
        });
    }
});
const hidePassword = () => { password.type = 'password'; render(); };
window.addEventListener('pageshow', hidePassword);
window.addEventListener('pagehide', hidePassword);
onAuthChange(state => {
    ipLogin.hidden = !state.enabled || !state.auto_login_available;
    if (state.auto_login_error) { errorKey = state.auto_login_error; render(); }
});
ipLogin.addEventListener('click', async () => {
    if (busy) return;
    busy = true; errorKey = ''; render();
    try {
        const authenticated = await loginByIp();
        if (authenticated === true) { location.replace(next); return; }
        errorKey = authenticated === false ? 'cookies' : 'network';
    } catch { errorKey = 'network'; }
    busy = false; render();
});
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
        checkAuthentication({ redirect: false }).then(authenticated => {
            if (authenticated === true) { location.replace(next); return; }
            form.hidden = false; loginCheck.hidden = true;
        });
    }
} catch { form.hidden = false; loginCheck.hidden = true; }
