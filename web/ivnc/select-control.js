// A select-only combobox. The original select remains the value/disabled source.
export function enhanceSelect(select, label) {
    const button = document.createElement('button');
    button.type = 'button'; button.className = 'console-select';
    button.setAttribute('role', 'combobox'); button.setAttribute('aria-haspopup', 'listbox');
    button.setAttribute('aria-expanded', 'false'); button.setAttribute('aria-labelledby', label.id);
    const popup = document.createElement('div');
    popup.className = 'console-select-popup'; popup.id = `${select.id}-listbox`;
    popup.dataset.consoleSelectPopup = ''; popup.setAttribute('role', 'listbox'); popup.hidden = true;
    popup.setAttribute('aria-labelledby', label.id);
    button.setAttribute('aria-controls', popup.id);
    select.insertAdjacentElement('afterend', button);
    // Keep the portal inside its owning modal for focus and backdrop handling,
    // but position it against the viewport to escape the scrolling form.
    select.closest('.modal-bg').appendChild(popup);
    select.hidden = true; select.tabIndex = -1;
    let index = 0, prefix = '', prefixTimer;
    const events = new AbortController();
    const listen = (target, type, handler, options = {}) => target.addEventListener(type, handler, { ...options, signal: events.signal });
    const options = () => [...select.options];

    function position() {
        if (popup.hidden) return;
        const rect = button.getBoundingClientRect();
        const form = select.closest('.modal').getBoundingClientRect();
        if (!rect.width || !rect.height || rect.bottom <= form.top || rect.top >= form.bottom) { close(); return; }
        const below = innerHeight - rect.bottom - 8, above = rect.top - 8;
        const height = Math.min(popup.scrollHeight, Math.max(40, below >= 160 || below >= above ? below : above));
        popup.style.width = `${Math.min(rect.width, innerWidth - 16)}px`;
        popup.style.maxHeight = `${height}px`;
        popup.style.left = `${Math.max(8, Math.min(rect.left, innerWidth - popup.offsetWidth - 8))}px`;
        popup.style.top = `${below >= 160 || below >= above ? rect.bottom + 4 : Math.max(8, rect.top - height - 4)}px`;
    }
    function highlight(next) {
        const all = options();
        index = Math.max(0, Math.min(all.length - 1, next));
        [...popup.children].forEach((node, i) => node.classList.toggle('highlighted', i === index));
        button.setAttribute('aria-activedescendant', `${popup.id}-${index}`);
        popup.children[index]?.scrollIntoView({ block: 'nearest' });
    }
    function sync() {
        button.textContent = select.selectedOptions[0]?.textContent || '';
        button.disabled = select.disabled;
        popup.replaceChildren(...options().map((option, i) => {
            const node = document.createElement('div');
            node.id = `${popup.id}-${i}`; node.setAttribute('role', 'option');
            node.setAttribute('aria-selected', String(option.selected));
            node.setAttribute('aria-disabled', String(option.disabled));
            node.textContent = option.textContent;
            node.addEventListener('pointermove', () => highlight(i));
            node.addEventListener('click', () => choose(i));
            return node;
        }));
        if (select.disabled) close();
        if (!popup.hidden) { highlight(index); position(); }
    }
    function open() {
        if (select.disabled) return;
        sync(); popup.hidden = false; button.setAttribute('aria-expanded', 'true');
        position(); highlight(select.selectedIndex);
    }
    function close() {
        popup.hidden = true; button.setAttribute('aria-expanded', 'false');
        button.removeAttribute('aria-activedescendant');
    }
    function choose(next) {
        if (select.disabled || options()[next]?.disabled) return;
        select.selectedIndex = next;
        select.dispatchEvent(new Event('change', { bubbles: true }));
        close(); sync(); button.focus();
    }
    listen(button, 'click', () => popup.hidden ? open() : close());
    listen(label.closest('label'), 'click', event => { event.preventDefault(); button.focus(); });
    listen(button, 'keydown', event => {
        if (event.isComposing) return;
        if (['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', ' ', 'Escape'].includes(event.key)) {
            if (event.key === 'Escape' && popup.hidden) return;
            event.preventDefault(); event.stopPropagation();
            if (event.key === 'Escape') close();
            else if (event.key === 'Enter' || event.key === ' ') popup.hidden ? open() : choose(index);
            else {
                if (popup.hidden) open();
                highlight(event.key === 'Home' ? 0 : event.key === 'End' ? options().length - 1 : index + (event.key === 'ArrowDown' ? 1 : -1));
            }
        } else if (event.key === 'Tab') close();
        else if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
            if (popup.hidden) open();
            prefix += event.key.toLocaleLowerCase(); clearTimeout(prefixTimer);
            prefixTimer = setTimeout(() => { prefix = ''; }, 600);
            const match = options().findIndex(option => option.textContent.toLocaleLowerCase().startsWith(prefix));
            if (match >= 0) highlight(match);
            event.preventDefault(); event.stopPropagation();
        }
    });
    listen(document, 'pointerdown', event => {
        if (!button.contains(event.target) && !popup.contains(event.target)) close();
    }, { capture: true });
    listen(select, 'change', sync);
    listen(window, 'resize', position);
    listen(document, 'scroll', position, { capture: true, passive: true });
    const observer = new MutationObserver(sync);
    observer.observe(select, { attributes: true, childList: true, subtree: true, characterData: true });
    const modalObserver = new MutationObserver(() => { if (!select.closest('.modal-bg').classList.contains('show')) close(); });
    modalObserver.observe(select.closest('.modal-bg'), { attributes: true, attributeFilter: ['class'] });
    listen(window, 'pagehide', event => {
        if (event.persisted) { close(); return; }
        events.abort(); observer.disconnect(); modalObserver.disconnect(); clearTimeout(prefixTimer);
    });
    sync();
    return { sync, close, button };
}
