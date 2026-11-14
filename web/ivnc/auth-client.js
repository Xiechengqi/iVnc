// This module is also a build entry: startup and the proxy panel load it before
// issuing API requests. No password or session token is exposed to JavaScript.
import { getLang, onLangChange } from './lib/i18n.js?v=1';
const nativeFetch = window.fetch.bind(window);
const channelName = `ivnc-auth:${location.host}`;
let csrf = null, state = null, checking = null, redirecting = false;
const loginPage = location.pathname === '/auth/login' || location.pathname === '/login.html';
let channel = null;
try { if (typeof BroadcastChannel === 'function') channel = new BroadcastChannel(channelName); }
catch { /* cross-tab storage may be blocked; server-side revocation still applies */ }
const listeners = new Set();

export function safeReturnPath(value) {
    if (!value || !value.startsWith('/') || value.startsWith('//') || /[\\\r\n]/.test(value)) return '/';
    try {
        const url = new URL(value, location.origin);
        if (url.origin !== location.origin || url.pathname.startsWith('/auth/') || url.pathname === '/login.html') return '/';
        return url.pathname + url.search + url.hash;
    } catch { return '/'; }
}

export function redirectToLogin() {
    if (loginPage || redirecting) return;
    redirecting = true;
    window.__ivncAuthRedirecting = true;
    window.dispatchEvent(new CustomEvent('ivnc-auth-ended'));
    try {
        const target = window.top;
        if (target !== window && target.location.origin === location.origin) {
            if (typeof target.__ivncRequireLogin === 'function') target.__ivncRequireLogin();
            else target.location.replace('/auth/login?next=' + encodeURIComponent(safeReturnPath(target.location.pathname + target.location.search)));
            return;
        }
    } catch { /* standalone/cross-origin embedding falls back to this document */ }
    location.replace('/auth/login?next=' + encodeURIComponent(safeReturnPath(location.pathname + location.search + location.hash)));
}
window.__ivncRequireLogin = redirectToLogin;

function announce() {
    try { channel?.postMessage('ended'); } catch { /* a departing document may have closed its channel */ }
    try { localStorage.setItem(channelName, String(Date.now()) + Math.random()); } catch { /* private storage may be unavailable */ }
}

export function onAuthChange(listener) {
    listeners.add(listener);
    if (state) listener(state);
    return () => listeners.delete(listener);
}

export function checkAuthentication({ redirect = true } = {}) {
    if (checking) return checking;
    checking = (async () => {
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 4000);
        try {
            const response = await nativeFetch('/auth/session', { cache: 'no-store', credentials: 'same-origin', signal: controller.signal });
            if (!response.ok || !(response.headers.get('content-type') || '').includes('application/json')) return null;
            const next = await response.json();
            if (typeof next.enabled !== 'boolean') return null;
            state = next; csrf = next.csrf || null;
            listeners.forEach(listener => listener(next));
            const authenticated = !next.enabled || (next.authenticated && next.kind === 'session');
            if (!authenticated && redirect) redirectToLogin();
            return authenticated;
        } catch { return null; /* network failure is handled by the normal reconnect path */ }
        finally { clearTimeout(timeout); checking = null; }
    })();
    return checking;
}

const ready = checkAuthentication({ redirect: !loginPage });
window.__ivncAuthReady = ready;

window.fetch = async function(input, init) {
    const url = new URL(input instanceof Request ? input.url : String(input), location.href);
    if (url.origin !== location.origin) return nativeFetch(input, init);
    const method = (init?.method || (input instanceof Request ? input.method : 'GET')).toUpperCase();
    const unsafe = !['GET', 'HEAD', 'OPTIONS'].includes(method);
    let options = init;
    if (unsafe && url.pathname !== '/auth/login') {
        await ready;
        // Reloads of the embedded Miao panel use its own CSRF token as well.
        if (csrf) {
            const headers = new Headers(init?.headers || (input instanceof Request ? input.headers : undefined));
            headers.set('X-IVNC-CSRF', csrf);
            options = { ...init, headers };
        }
    }
    const response = await nativeFetch(input, options);
    if (response.status === 401 && response.headers.get('x-ivnc-auth') === 'required') redirectToLogin();
    return response;
};

export async function logout() {
    await ready;
    const response = await window.fetch('/auth/logout', { method: 'POST' });
    if (!response.ok) throw Error('Sign out failed');
    announce(); redirectToLogin();
}

export function credentialsChanged() { announce(); redirectToLogin(); }

export function bindLogoutButton(button) {
    const label = () => {
        const text = getLang() === 'zh' ? '退出登录' : 'Sign out';
        button.title = text; button.setAttribute('aria-label', text);
    };
    label(); onLangChange(label);
    onAuthChange(value => { button.hidden = !value.enabled || value.kind !== 'session'; });
    button.addEventListener('click', async event => {
        event.stopPropagation(); button.disabled = true;
        try { await logout(); }
        catch { button.disabled = false; window.alert(getLang() === 'zh' ? '退出失败，请检查网络后重试。' : 'Sign out failed. Check your network and retry.'); }
    });
}

channel?.addEventListener('message', event => { if (event.data === 'ended') redirectToLogin(); });
window.addEventListener('storage', event => { if (event.key === channelName) redirectToLogin(); });
window.addEventListener('pageshow', event => { if (event.persisted && !loginPage) checkAuthentication(); });
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && !loginPage) checkAuthentication();
});
window.addEventListener('pagehide', event => { if (!event.persisted) channel?.close(); });
