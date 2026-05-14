const STORAGE_KEY = 'ivnc.lang';

function detect() {
    try {
        const saved = localStorage.getItem(STORAGE_KEY);
        if (saved === 'zh' || saved === 'en') return saved;
    } catch (_) {}
    const browser = (navigator.language || 'en').toLowerCase();
    return browser.startsWith('zh') ? 'zh' : 'en';
}

let _lang = detect();
const _listeners = new Set();

function _notify() {
    _listeners.forEach(fn => {
        try { fn(_lang); } catch (e) { console.error('[i18n] listener failed:', e); }
    });
}

export function getLang() {
    return _lang;
}

export function setLang(lang) {
    if (lang !== 'zh' && lang !== 'en') return;
    if (lang === _lang) return;
    _lang = lang;
    try { localStorage.setItem(STORAGE_KEY, lang); } catch (_) {}
    _notify();
}

export function onLangChange(fn) {
    _listeners.add(fn);
    return () => _listeners.delete(fn);
}

export function t(table, key, ...args) {
    const set = table[_lang] || table.en || {};
    const entry = set[key];
    if (typeof entry === 'function') return entry(...args);
    return entry !== undefined ? entry : key;
}

if (typeof window !== 'undefined') {
    window.addEventListener('storage', (e) => {
        if (e.key !== STORAGE_KEY) return;
        if (e.newValue !== 'zh' && e.newValue !== 'en') return;
        if (e.newValue === _lang) return;
        _lang = e.newValue;
        _notify();
    });
}
