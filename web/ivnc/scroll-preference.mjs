export const SCROLL_SPEED_KEY = 'ivnc.scroll-speed.v1';
export const DEFAULT_SCROLL_SPEED = 1;
export const MIN_SCROLL_SPEED = .25;
export const MAX_SCROLL_SPEED = 4;
const EVENT = 'ivnc-scroll-speed-change';
const STATE = '__ivncScrollSpeedPreferenceV1';

export function validScrollSpeed(value) {
    return typeof value === 'number' && Number.isFinite(value) && value >= MIN_SCROLL_SPEED && value <= MAX_SCROLL_SPEED;
}
const decode = value => value !== null && value !== '' && validScrollSpeed(Number(value)) ? Number(value) : DEFAULT_SCROLL_SPEED;

function preference() {
    // Settings and app views share one live value even when storage is blocked.
    // Separate tabs synchronize through the browser's localStorage event.
    let owner = window;
    try { while (owner.parent !== owner && owner.parent.location.origin === location.origin) owner = owner.parent; } catch { /* cross-origin host */ }
    if (!owner[STATE]) {
        let value = DEFAULT_SCROLL_SPEED;
        try { value = decode(owner.localStorage.getItem(SCROLL_SPEED_KEY)); } catch { /* memory-only preference */ }
        owner[STATE] = { value };
        owner.addEventListener('storage', event => {
            if (event.key !== null && event.key !== SCROLL_SPEED_KEY) return;
            let next;
            try {
                if (event.storageArea !== owner.localStorage) return;
                next = decode(owner.localStorage.getItem(SCROLL_SPEED_KEY));
            } catch { return; }
            if (owner[STATE].value === next) return;
            owner[STATE].value = next;
            owner.dispatchEvent(new owner.Event(EVENT));
        });
    }
    return owner;
}

export function getScrollSpeed() { return preference()[STATE].value; }
export function setScrollSpeed(value) {
    if (!validScrollSpeed(value)) return;
    const owner = preference();
    try { owner.localStorage.setItem(SCROLL_SPEED_KEY, String(value)); } catch { /* keep working in memory */ }
    if (owner[STATE].value === value) return;
    owner[STATE].value = value;
    owner.dispatchEvent(new owner.Event(EVENT));
}
export function onScrollSpeedChange(callback) {
    const owner = preference(), notify = () => callback(owner[STATE].value);
    owner.addEventListener(EVENT, notify);
    const unsubscribe = () => {
        owner.removeEventListener(EVENT, notify);
        window.removeEventListener('pagehide', leave);
    };
    const leave = event => { if (!event.persisted) unsubscribe(); };
    // A parent's event target must not keep a removed/reloaded iframe alive.
    window.addEventListener('pagehide', leave);
    return unsubscribe;
}
