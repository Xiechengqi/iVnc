// Available even when console management is disabled. This page does not load
// console assets or request management APIs.
import { getLang, onLangChange } from './lib/i18n.js?v=1';
import { createConsoleDialogs } from './console-dialogs.js';
import { createSettingsActions, settingsText } from './settings-actions.js';

createSettingsActions(document.getElementById('settings-actions'), createConsoleDialogs());
function translate() {
    document.documentElement.lang = getLang() === 'zh' ? 'zh-CN' : 'en';
    document.title = settingsText('settings');
    document.getElementById('settings-title').textContent = settingsText('settings');
    document.getElementById('settings-hint').textContent = settingsText('minimalHint');
}
translate();
onLangChange(translate);
