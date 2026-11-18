import { getLang, onLangChange } from './lib/i18n.js?v=1';
import { onAuthChange, checkAuthentication } from './auth-client.js';
import styles from './advanced-settings.css?inline';

const STRINGS = {
    zh: {
        advanced: '高级', general: '常规', title: 'IP 白名单免密登录',
        enabled: '启用 IP 白名单免密登录', rules: 'IP 白名单',
        hint: '每行填写一个访问者 IP 或 CIDR 网段。命中后免密登录，名单外仍使用用户名和密码。',
        placeholder: '192.168.1.20\n192.168.1.0/24\n2001:db8::/64',
        client: '当前客户端 IP', unknown: '无法可靠识别', matched: '已命中白名单', unmatched: '未命中白名单',
        authDisabled: '当前已关闭登录认证；白名单配置将在开启认证后生效。',
        sourceUnavailable: '未能从可信代理获取有效的客户端 IP，此连接使用密码登录。',
        ipSession: '当前通过 IP 白名单登录。关闭免密或移除当前 IP 后，此连接将退出登录。',
        save: '保存并应用', saving: '正在保存…', loading: '正在加载…', saved: '已保存并应用',
        dirty: '有未保存的改动', network: '请求失败，请检查网络后重试。',
        invalid_ip_rule: '第 {line} 行不是有效的 IP 或 CIDR 网段。',
        too_many_ip_rules: '最多可配置 256 条不同的 IP 或网段。',
        ip_rules_required: '启用免密登录时，请至少填写一个 IP 或网段。',
        access_save_failed: '保存失败，原有白名单继续有效。',
    },
    en: {
        advanced: 'Advanced', general: 'General', title: 'Passwordless sign-in by IP',
        enabled: 'Enable passwordless sign-in by IP', rules: 'IP allowlist',
        hint: 'Enter one client IP or CIDR network per line. Matching clients sign in automatically; other clients use a username and password.',
        placeholder: '192.168.1.20\n192.168.1.0/24\n2001:db8::/64',
        client: 'Current client IP', unknown: 'Cannot determine reliably', matched: 'Matches the allowlist', unmatched: 'Does not match the allowlist',
        authDisabled: 'Sign-in authentication is disabled. These settings take effect when authentication is enabled.',
        sourceUnavailable: 'The trusted proxy did not supply a valid client IP. This connection uses password sign-in.',
        ipSession: 'Signed in by IP. Disabling passwordless sign-in or removing this IP will sign out this connection.',
        save: 'Save and apply', saving: 'Saving…', loading: 'Loading…', saved: 'Saved and applied',
        dirty: 'Unsaved changes', network: 'Request failed. Check your connection and retry.',
        invalid_ip_rule: 'Line {line} is not a valid IP or CIDR network.',
        too_many_ip_rules: 'Use at most 256 distinct IPs or networks.',
        ip_rules_required: 'Enter at least one IP or network to enable passwordless sign-in.',
        access_save_failed: 'Could not save. The previous allowlist is still in effect.',
    },
};
export const advancedText = key => STRINGS[getLang()][key];

// Shared by /console and /settings.html. Mounting does not fetch or save anything.
export function createAdvancedSettings(element) {
    if (!document.getElementById('ivnc-advanced-styles')) {
        const style = document.createElement('style');
        style.id = 'ivnc-advanced-styles'; style.textContent = styles;
        document.head.appendChild(style);
    }
    element.classList.add('access-settings');
    element.innerHTML = `<form class="access-card allow-native-input">
        <h2 id="access-title"></h2>
        <label class="access-toggle"><input id="access-enabled" type="checkbox"><span id="access-enabled-label"></span></label>
        <p id="access-hint" class="access-hint"></p>
        <label id="access-rules-label" for="access-rules"></label>
        <textarea id="access-rules" rows="7" maxlength="16384" spellcheck="false" autocapitalize="none" autocomplete="off" aria-describedby="access-hint access-message"></textarea>
        <div class="access-current"><span id="access-client-label"></span><code id="access-client-ip"></code><span id="access-match"></span></div>
        <p id="access-auth-disabled" class="access-hint" hidden></p>
        <p id="access-source-unavailable" class="access-hint" hidden></p>
        <p id="access-ip-session" class="access-hint" hidden></p>
        <div class="access-footer"><p id="access-message" role="status" aria-live="polite"></p><button id="access-save" type="submit" disabled></button></div>
    </form>`;
    const find = id => element.querySelector(`#${id}`);
    const form = element.querySelector('form'), enabled = find('access-enabled'), rules = find('access-rules');
    const save = find('access-save'), message = find('access-message');
    let snapshot = null, dirty = false, saving = false, loading = false, status = '', line = null, request = null;
    let authMethod = null;
    const events = new AbortController();

    function render() {
        for (const [id, key] of [
            ['access-title','title'], ['access-enabled-label','enabled'], ['access-hint','hint'],
            ['access-rules-label','rules'], ['access-client-label','client'],
            ['access-auth-disabled','authDisabled'], ['access-source-unavailable','sourceUnavailable'], ['access-ip-session','ipSession'],
        ]) find(id).textContent = advancedText(key);
        rules.placeholder = advancedText('placeholder');
        find('access-client-ip').textContent = snapshot?.client_ip || advancedText('unknown');
        find('access-match').textContent = snapshot ? advancedText(snapshot.matches ? 'matched' : 'unmatched') : '';
        find('access-auth-disabled').hidden = !snapshot || snapshot.authentication_enabled;
        find('access-source-unavailable').hidden = !snapshot || !!snapshot.client_ip;
        find('access-ip-session').hidden = authMethod !== 'ip_whitelist';
        enabled.disabled = rules.disabled = saving || loading || !snapshot;
        save.disabled = saving || loading || !snapshot || !dirty;
        save.textContent = advancedText(saving ? 'saving' : 'save');
        const key = loading ? 'loading' : status || (dirty ? 'dirty' : '');
        message.textContent = key ? advancedText(key).replace('{line}', String(line ?? '')) : '';
        message.classList.toggle('access-error', !!status && !['saved'].includes(status));
        message.classList.toggle('access-success', status === 'saved');
    }
    const values = () => ({ enabled: enabled.checked, ip_allowlist: rules.value.split(/\r?\n/).map(v => v.trim()).filter(Boolean) });
    function changed() {
        dirty = JSON.stringify(values()) !== JSON.stringify({ enabled: snapshot.enabled, ip_allowlist: snapshot.ip_allowlist });
        status = ''; line = null; render();
    }
    enabled.addEventListener('change', changed, { signal: events.signal });
    rules.addEventListener('input', changed, { signal: events.signal });

    async function fetchSettings(method, body) {
        request?.abort();
        const controller = new AbortController(); request = controller;
        const timeout = setTimeout(() => controller.abort(), 10000);
        try {
            await window.__ivncAuthReady;
            const response = await fetch('/api/settings/access', {
                method, cache: 'no-store', signal: controller.signal,
                ...(body ? { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : {}),
            });
            const data = await response.json();
            if (!response.ok) {
                const error = Error(STRINGS.en[data.error] ? data.error : 'network');
                error.line = data.line; throw error;
            }
            if (typeof data.enabled !== 'boolean' || !Array.isArray(data.ip_allowlist)) throw Error('network');
            return data;
        } finally { clearTimeout(timeout); if (request === controller) request = null; }
    }
    function accept(data) {
        snapshot = data; enabled.checked = data.enabled; rules.value = data.ip_allowlist.join('\n');
        dirty = false;
    }
    async function load() {
        if (dirty || saving || loading) return;
        loading = true; status = ''; render();
        try { accept(await fetchSettings('GET')); }
        catch { status = 'network'; }
        finally { loading = false; render(); }
    }
    form.addEventListener('submit', async event => {
        event.preventDefault();
        if (saving || !snapshot || !dirty) return;
        saving = true; status = ''; line = null; render();
        try {
            // Preserve blank lines for the server's validation error indices.
            accept(await fetchSettings('PUT', { enabled: enabled.checked, ip_allowlist: rules.value.split(/\r?\n/) }));
            status = 'saved';
            // Standalone settings has no media socket to notify it of revocation.
            if (authMethod === 'ip_whitelist') await checkAuthentication({ allowAutoLogin: false });
        } catch (error) { status = STRINGS.en[error.message] ? error.message : 'network'; line = error.line; }
        finally { saving = false; render(); }
    }, { signal: events.signal });
    const untranslate = onLangChange(render);
    const unauth = onAuthChange(state => { authMethod = state.method; render(); });
    window.addEventListener('pagehide', event => {
        if (event.persisted) return;
        request?.abort(); events.abort(); untranslate(); unauth();
    });
    render();
    return { load };
}
