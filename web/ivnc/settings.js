// Available even when console management is disabled. This page does not load
// console assets or request management APIs.
import { getLang, onLangChange } from './lib/i18n.js?v=1';
import { createConsoleDialogs } from './console-dialogs.js';
import { createSettingsActions, settingsText } from './settings-actions.js';
import { createAdvancedSettings, advancedText } from './advanced-settings.js';

createSettingsActions(document.getElementById('settings-actions'), createConsoleDialogs());
const advancedSettings = createAdvancedSettings(document.getElementById('advanced-settings'));
document.querySelectorAll('.settings-tab').forEach(button => button.addEventListener('click', () => {
    const section = button.dataset.section;
    for (const tab of document.querySelectorAll('.settings-tab')) {
        tab.classList.toggle('active', tab === button);
        if (tab === button) tab.setAttribute('aria-current', 'page'); else tab.removeAttribute('aria-current');
    }
    document.getElementById('section-general').hidden = section !== 'general';
    document.getElementById('section-advanced').hidden = section !== 'advanced';
    if (section === 'advanced') advancedSettings.load();
}));
function translate() {
    document.documentElement.lang = getLang() === 'zh' ? 'zh-CN' : 'en';
    document.title = settingsText('settings');
    document.getElementById('settings-title').textContent = settingsText('settings');
    document.getElementById('settings-hint').textContent = settingsText('minimalHint');
    document.getElementById('nav-general').textContent = advancedText('general');
    document.getElementById('nav-advanced').textContent = advancedText('advanced');
}
translate();
onLangChange(translate);
