import { getLang, onLangChange } from './lib/i18n.js?v=1';
import { onAuthChange, checkAuthentication } from './auth-client.js';
import { getScrollSpeed, setScrollSpeed, onScrollSpeedChange, DEFAULT_SCROLL_SPEED, MIN_SCROLL_SPEED, MAX_SCROLL_SPEED } from './scroll-preference.mjs';
import styles from './advanced-settings.css?inline';

const STRINGS = {
    zh: {
        advanced: '高级', general: '常规', title: 'IP 白名单免密登录',
        scrollTitle: '滚动速度', scrollHint: '调整鼠标滚轮和双指滚动的速度。仅对当前浏览器生效，立即应用。',
        scrollReset: '恢复默认', scrollValue: '{value} 倍',
        enabled: '启用 IP 白名单免密登录', rules: 'IP 白名单',
        hint: '每行填写一个访问者 IP 或 CIDR 网段。命中后免密登录，名单外仍使用用户名和密码。',
        placeholder: '192.168.1.20\n192.168.1.0/24\n2001:db8::/64',
        client: '当前客户端 IP', unknown: '无法可靠识别', matched: '已命中白名单', unmatched: '未命中白名单',
        addCurrent: '加入当前 IP', covered: '已在白名单', adding: '正在添加…',
        inactive: '免密未启用',
        added: '当前 IP 已加入白名单。', addedInactive: '当前 IP 已加入白名单，开启免密后生效。',
        access_source_unknown: '无法可靠识别当前连接的 IP。',
        access_changed: '白名单已更新，已保留你的编辑。请核对后再保存。',
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
        scrollTitle: 'Scroll speed', scrollHint: 'Adjust mouse wheel and two-finger scrolling. Applies immediately to this browser only.',
        scrollReset: 'Restore default', scrollValue: '{value} times',
        enabled: 'Enable passwordless sign-in by IP', rules: 'IP allowlist',
        hint: 'Enter one client IP or CIDR network per line. Matching clients sign in automatically; other clients use a username and password.',
        placeholder: '192.168.1.20\n192.168.1.0/24\n2001:db8::/64',
        client: 'Current client IP', unknown: 'Cannot determine reliably', matched: 'Matches the allowlist', unmatched: 'Does not match the allowlist',
        addCurrent: 'Add current IP', covered: 'Already on the list', adding: 'Adding…',
        inactive: 'Passwordless sign-in is off',
        added: 'Current IP added to the allowlist.', addedInactive: 'Current IP added. Enable passwordless sign-in to use it.',
        access_source_unknown: 'Cannot reliably determine this connection’s IP.',
        access_changed: 'The allowlist changed. Your edits are preserved. Review them before saving.',
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
    element.innerHTML = `<section class="access-card scroll-card allow-native-input" aria-labelledby="scroll-title">
        <h2 id="scroll-title"></h2>
        <p id="scroll-hint" class="access-hint"></p>
        <div class="scroll-control"><input id="scroll-speed" type="range" min="${MIN_SCROLL_SPEED}" max="${MAX_SCROLL_SPEED}" step="0.25" aria-labelledby="scroll-title" aria-describedby="scroll-hint"><output id="scroll-value" for="scroll-speed"></output></div>
        <div class="scroll-footer"><span>${MIN_SCROLL_SPEED}× — ${MAX_SCROLL_SPEED.toFixed(1)}×</span><button id="scroll-reset" type="button"></button></div>
    </section>
    <form class="access-card access-ip-form allow-native-input">
        <h2 id="access-title"></h2>
        <label class="access-toggle"><input id="access-enabled" type="checkbox"><span id="access-enabled-label"></span></label>
        <p id="access-hint" class="access-hint"></p>
        <label id="access-rules-label" for="access-rules"></label>
        <textarea id="access-rules" rows="7" maxlength="16384" spellcheck="false" autocapitalize="none" autocomplete="off" aria-describedby="access-hint access-message"></textarea>
        <div class="access-current"><span id="access-client-label"></span><code id="access-client-ip"></code><span id="access-match"></span><button id="access-add-current" type="button"></button></div>
        <p id="access-auth-disabled" class="access-hint" hidden></p>
        <p id="access-source-unavailable" class="access-hint" hidden></p>
        <p id="access-ip-session" class="access-hint" hidden></p>
        <div class="access-footer"><p id="access-message" role="status" aria-live="polite"></p><button id="access-save" type="submit" disabled></button></div>
    </form>`;
    const find = id => element.querySelector(`#${id}`);
    const form = element.querySelector('.access-ip-form'), enabled = find('access-enabled'), rules = find('access-rules');
    const save = find('access-save'), message = find('access-message');
    const addCurrent = find('access-add-current');
    const scrollSpeed = find('scroll-speed'), scrollReset = find('scroll-reset');
    let snapshot = null, dirty = false, saving = false, loading = false, status = '', line = null, request = null;
    let adding = false;
    let authMethod = null;
    const events = new AbortController();

    function render() {
        const speed = getScrollSpeed(), value = speed.toFixed(2).replace(/0$/, '');
        find('scroll-title').textContent = advancedText('scrollTitle');
        find('scroll-hint').textContent = advancedText('scrollHint');
        scrollSpeed.value = String(speed);
        scrollSpeed.setAttribute('aria-valuetext', advancedText('scrollValue').replace('{value}', value));
        find('scroll-value').value = value + '×';
        scrollReset.textContent = advancedText('scrollReset');
        scrollReset.disabled = speed === DEFAULT_SCROLL_SPEED;
        for (const [id, key] of [
            ['access-title','title'], ['access-enabled-label','enabled'], ['access-hint','hint'],
            ['access-rules-label','rules'], ['access-client-label','client'],
            ['access-auth-disabled','authDisabled'], ['access-source-unavailable','sourceUnavailable'], ['access-ip-session','ipSession'],
        ]) find(id).textContent = advancedText(key);
        rules.placeholder = advancedText('placeholder');
        find('access-client-ip').textContent = snapshot?.client_ip || advancedText('unknown');
        const match = snapshot?.covered && !snapshot.enabled ? 'inactive' : snapshot?.matches ? 'matched' : 'unmatched';
        find('access-match').textContent = snapshot ? advancedText(match) : '';
        find('access-auth-disabled').hidden = !snapshot || snapshot.authentication_enabled;
        find('access-source-unavailable').hidden = !snapshot || !!snapshot.client_ip;
        find('access-ip-session').hidden = authMethod !== 'ip_whitelist';
        const busy = saving || adding || loading || !snapshot;
        enabled.disabled = rules.disabled = busy;
        save.disabled = busy || !dirty;
        addCurrent.disabled = busy || !snapshot?.client_ip || snapshot.covered === true;
        addCurrent.textContent = advancedText(adding ? 'adding' : snapshot?.covered ? 'covered' : 'addCurrent');
        save.textContent = advancedText(saving ? 'saving' : 'save');
        const key = loading ? 'loading' : status || (dirty ? 'dirty' : '');
        message.textContent = key ? advancedText(key).replace('{line}', String(line ?? '')) : '';
        const success = ['saved', 'added', 'addedInactive', 'covered'].includes(status);
        if (dirty && success) message.textContent += ' ' + advancedText('dirty');
        message.classList.toggle('access-error', !!status && !success);
        message.classList.toggle('access-success', success);
    }
    const values = () => ({ enabled: enabled.checked, ip_allowlist: rules.value.split(/\r?\n/).map(v => v.trim()).filter(Boolean) });
    function changed() {
        dirty = JSON.stringify(values()) !== JSON.stringify({ enabled: snapshot.enabled, ip_allowlist: snapshot.ip_allowlist });
        status = ''; line = null; render();
    }
    enabled.addEventListener('change', changed, { signal: events.signal });
    rules.addEventListener('input', changed, { signal: events.signal });
    scrollSpeed.addEventListener('input', () => setScrollSpeed(Number(scrollSpeed.value)), { signal: events.signal });
    scrollReset.addEventListener('click', () => setScrollSpeed(DEFAULT_SCROLL_SPEED), { signal: events.signal });
    const unscroll = onScrollSpeedChange(render);

    async function fetchSettings(method, body) {
        request?.abort();
        const controller = new AbortController(); request = controller;
        const timeout = setTimeout(() => controller.abort(), 10000);
        try {
            await window.__ivncAuthReady;
            const response = await fetch(method === 'POST' ? '/api/settings/access/current-ip' : '/api/settings/access', {
                method, cache: 'no-store', signal: controller.signal,
                ...(body ? { headers: { 'Content-Type': 'application/json',
                    ...(method === 'PUT' && snapshot?.etag ? { 'If-Match': snapshot.etag } : {}) }, body: JSON.stringify(body) } : {}),
            });
            const data = await response.json();
            if (!response.ok) {
                const error = Error(STRINGS.en[data.error] ? data.error : 'network');
                error.line = data.line;
                if (response.status === 412 && data.error === 'access_changed') error.current = data;
                throw error;
            }
            if (typeof data.enabled !== 'boolean' || !Array.isArray(data.ip_allowlist)) throw Error('network');
            return data;
        } finally { clearTimeout(timeout); if (request === controller) request = null; }
    }
    function accept(data) {
        snapshot = data; enabled.checked = data.enabled; rules.value = data.ip_allowlist.join('\n');
        dirty = false;
    }
    // Keep local edits (including blank or invalid lines) while applying rules
    // added or removed since this form was loaded.
    function mergeSnapshot(data) {
        if (!dirty) { accept(data); return; }
        const previous = new Set(snapshot.ip_allowlist), current = new Set(data.ip_allowlist);
        const lines = rules.value.split(/\r?\n/).filter(line => !previous.has(line.trim()) || current.has(line.trim()));
        const draft = new Set(lines.map(line => line.trim()));
        for (const rule of current) {
            if (!previous.has(rule) && !draft.has(rule)) lines.push(rule);
        }
        if (enabled.checked === snapshot.enabled) enabled.checked = data.enabled;
        rules.value = lines.join('\n');
        snapshot = data;
        changed();
    }
    addCurrent.addEventListener('click', async () => {
        if (addCurrent.disabled) return;
        adding = true; status = ''; line = null; render();
        try {
            const data = await fetchSettings('POST', {});
            mergeSnapshot(data);
            status = !data.changed ? 'covered' : data.enabled ? 'added' : 'addedInactive';
        } catch (error) {
            status = STRINGS.en[error.message] ? error.message : 'network';
            if (status === 'access_source_unknown') snapshot = { ...snapshot, client_ip: null, matches: false, covered: false };
        } finally { adding = false; render(); }
    }, { signal: events.signal });
    async function load() {
        if (dirty || saving || adding || loading) return;
        loading = true; status = ''; render();
        try { accept(await fetchSettings('GET')); }
        catch { status = 'network'; }
        finally { loading = false; render(); }
    }
    form.addEventListener('submit', async event => {
        event.preventDefault();
        if (saving || adding || !snapshot || !dirty) return;
        saving = true; status = ''; line = null; render();
        try {
            // Preserve blank lines for the server's validation error indices.
            accept(await fetchSettings('PUT', { enabled: enabled.checked, ip_allowlist: rules.value.split(/\r?\n/) }));
            status = 'saved';
            // Standalone settings has no media socket to notify it of revocation.
            if (authMethod === 'ip_whitelist') await checkAuthentication({ allowAutoLogin: false });
        } catch (error) {
            if (error.current) mergeSnapshot(error.current);
            status = STRINGS.en[error.message] ? error.message : 'network'; line = error.line;
        }
        finally { saving = false; render(); }
    }, { signal: events.signal });
    const untranslate = onLangChange(render);
    const unauth = onAuthChange(state => { authMethod = state.method; render(); });
    window.addEventListener('pagehide', event => {
        if (event.persisted) return;
        request?.abort(); events.abort(); untranslate(); unauth(); unscroll();
    });
    render();
    return { load };
}
