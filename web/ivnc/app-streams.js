/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

// Toolbar stream tabs for desktop apps that do not draw into the main
// compositor: shared apps running in a worker and started remote apps
// (taskbar `apps`). Each app gets one same-origin embedded view
// (/app-view/{token}/?embed=1) laid over the main video. Hidden views stay
// connected so their audio keeps playing. Only the top-level desktop page
// owns stream tabs; embedded views never create them.
import { appText, onLangChange, remoteStateText, serverErrorText } from "./lib/i18n.js?v=1";
import { createMultiAppView, multiAppText } from './multi-app-view.js';

// Taskbar `status` at which a view can be opened.
const READY = { shared: 'running', remote: 'connected' };
const RETRY_MIN_MS = 1000;
const RETRY_MAX_MS = 30000;
const STARTED_SELECT_MS = 60000;
const PREVIEW_RELEASE_MS = 15000;

// For the console page, which runs in an iframe of the desktop page: selects
// the app's stream tab once it appears after the user started it.
export function notifyAppStarted(id) {
	if (window.parent !== window) {
		window.parent.postMessage({ type: 'ivnc-app-started', id }, window.location.origin);
	}
}

const post = (stream, message) => stream.frame?.contentWindow?.postMessage({ ...message, view: stream.url }, window.location.origin);
const closeView = (url) => fetch(`${url}close`, { method: 'POST', keepalive: true }).catch(() => {});
const fromConsole = (source) => Array.from(document.querySelectorAll('iframe.web-console-frame'))
	.some((frame) => frame.contentWindow === source);

export function createAppStreams({ viewport, onChange, onEdgeHover, onFrameReady = () => {}, displayMode = () => 'fit', imeMode = () => null, onImePreference = () => {}, onImeError = () => {} }) {
	const streams = new Map();
	let order = [];
	let activeId = null;
	let previews = [];
	let beforePreview = null;
	const placeholders = new Map();
	let pendingSelect = null;
	let disposed = false;

	const layer = document.createElement('div');
	layer.className = 'app-streams';
	document.body.appendChild(layer);
	const multiApp = createMultiAppView({
		layer,
		onShow(members) {
			if (!previews.length) beforePreview = activeId;
			activeId = null;
			pendingSelect = null;
			previews = members;
			leaveFrameFocus();
			updatePresentation();
			refresh();
		},
		onExit: () => activate(beforePreview),
		onSingle: activate,
		beforeEdit() {
			try { active()?.frame?.contentWindow?.dispatchEvent(new Event('ivnc-overlay-interaction')); } catch (_) {}
		},
	});

	// The parent work area already reserves its toolbar. Embedded views use
	// this entire box and must not subtract the toolbar a second time.
	function layout() {
		if (disposed) return;
		const rect = viewport.getBoundingClientRect();
		layer.style.left = `${rect.left}px`;
		layer.style.top = `${rect.top}px`;
		layer.style.width = `${rect.width}px`;
		layer.style.height = `${rect.height}px`;
		layer.classList.toggle('preview-narrow', rect.width < 700);
	}
	window.addEventListener('resize', layout);
	const resizeObserver = new ResizeObserver(layout);
	resizeObserver.observe(viewport);
	layout();

	const ready = (s) => s.app.status === READY[s.app.kind];
	const canPreview = s => s.app.kind === 'shared' || s.app.preview_supported === true;
	const slot = id => previews.findIndex(member => member.id === id);
	const inPreview = s => slot(s.id) >= 0 && canPreview(s);
	const wantsView = s => s.unlocked || (inPreview(s) && !document.hidden);

	function tile(parent) {
		const wrap = document.createElement('div');
		wrap.className = 'app-stream';
		const status = document.createElement('div');
		status.className = 'app-stream-status';
		wrap.append(status);
		const button = document.createElement('button');
		button.className = 'app-preview-open';
		button.type = 'button';
		const caption = document.createElement('span');
		button.append(caption);
		wrap.append(button);
		button.hidden = true;
		parent.append(wrap);
		return { wrap, status, button, caption };
	}

	function entry(app) {
		const parts = tile(layer);
		parts.button.onclick = () => activate(app.id);
		// state: null (never opened) | 'connecting' | 'connected' | 'ended'
		// error: the server's answer when a view could not be opened.
		return { id: app.id, app, ...parts, frame: null, url: null, state: null, error: null, withdrawn: false,
			request: null, timer: null, releaseTimer: null, delay: 0, unlocked: false, ime: false };
	}

	// Mints a view token for this page and (re)loads the iframe with it.
	async function open(s) {
		clearTimeout(s.timer);
		s.timer = null;
		if (disposed || s.request || !ready(s) || !wantsView(s)) return;
		const request = s.request = {};
		s.state = 'connecting';
		s.error = null;
		s.playback = null;
		refresh();
		try {
			const response = await fetch(`/api/apps/${encodeURIComponent(s.id)}/view`, { method: 'POST' });
			const body = await response.json().catch(() => ({}));
			if (!response.ok || !body.url) throw Object.assign(new Error(body.error || response.statusText), { body });
			if (s.request !== request) {
				closeView(body.url);
				return;
			}
			if (!wantsView(s) || !ready(s)) {
				closeView(body.url);
				release(s);
				refresh();
				return;
			}
			if (s.url) closeView(s.url);
			s.url = body.url;
			if (s.frame) {
				s.frame.src = viewSource(s);
			} else {
				// src is set before insertion so no about:blank load is observed.
				s.frame = document.createElement('iframe');
				s.frame.title = s.app.name;
				s.frame.allow = 'autoplay; clipboard-read; clipboard-write; fullscreen';
				const frame = s.frame;
				frame.inert = s.id !== activeId;
				frame.tabIndex = s.id === activeId ? 0 : -1;
				frame.src = viewSource(s);
				frame.addEventListener('load', () => { if (s.frame === frame) loaded(s); });
				s.wrap.prepend(s.frame);
			}
		} catch (error) {
			if (s.request === request) {
				s.request = null;
				end(s, 'error', error.body?.error ? error.body : { error: error.message });
			}
		} finally {
			if (s.request === request) s.request = null;
		}
	}

	function viewSource(s) {
		return `${s.url}?embed=1${s.id !== activeId && (inPreview(s) || !s.unlocked) ? '&preview=1' : ''}`;
	}

	function loaded(s) {
		if (disposed || streams.get(s.id) !== s) return;
		// An expired token answers with an error document instead of the client.
		let client = false;
		try {
			const path = s.frame.contentWindow?.location.pathname;
			if (path !== s.url) return; // A load from the replaced document.
			client = !!s.frame.contentDocument?.getElementById('app');
		} catch (_) {}
		if (!client) {
			end(s, 'expired');
			return;
		}
		onFrameReady(s.frame.contentDocument);
		sync(s);
	}

	// Sends the view its current state. Runs on load and again on
	// ivnc-view-ready, since the view's listener may not exist at load yet.
	function sync(s) {
		syncPresentation(s, true);
		sendImeMode(s);
		// unlocked: the user has activated this tab, so audio may play.
		if (s.unlocked) post(s, { type: 'ivnc-unmute' });
		if (s.id === activeId && s.state !== 'ended') focusFrame(s);
	}

	function syncPresentation(s, force = false) {
		if (!s.frame) return;
		const single = s.id === activeId;
		const preview = inPreview(s);
		const state = {
			type: 'ivnc-view-active',
			// Input attaches only after the DataChannel is ready. Playback may
			// still be waiting for a frame or a manual play gesture.
			active: single && s.state !== 'ended',
			visible: single || (preview && ready(s) && !document.hidden && !s.withdrawn),
			preview: !s.unlocked || preview,
			muted: previews.length > 0,
		};
		const key = JSON.stringify(state);
		if (!force && s.presentation === key) return;
		s.presentation = key;
		s.frame.inert = !single;
		s.frame.tabIndex = single ? 0 : -1;
		try {
			const view = s.frame.contentWindow;
			if (view.location.pathname === s.url) view.__ivncInput?.setViewActive(state.active);
		} catch (_) {}
		sendDisplayMode(s);
		post(s, state);
	}

    function focusFrame(s) {
        if (document.hidden || !document.hasFocus() || multiApp.isEditing()
            || document.documentElement.hasAttribute('data-ivnc-modal-blocked')) return;
        const target = document.activeElement;
        if (target?.closest?.('.web-terminal-modal, .clipboard-assist, .pwd-overlay, .update-overlay, .toolbar-rename')) return;
        s.frame?.contentWindow?.focus();
    }

	function leaveFrameFocus() {
		if (!document.activeElement?.closest?.('.app-streams iframe')) return;
		const hadFocus = !document.hidden && document.hasFocus();
		document.activeElement.blur();
		if (hadFocus) window.focus();
	}

    function sendImeMode(s, focus = false) {
        const enabled = imeMode();
        if (typeof enabled !== 'boolean') return; // Mobile keyboards need a tap.
        const controlling = s.id === activeId && s.state !== 'ended';
        if (focus && controlling) focusFrame(s);
        try {
            const view = s.frame?.contentWindow;
            if (view?.location.pathname === s.url) {
                view.__ivncInput?.setImeMode(enabled, false);
                if (controlling) view.__ivncInput?.restoreImeFocus();
            }
        } catch (_) {}
        post(s, { type: 'ivnc-ime-mode', enabled, view: s.url });
    }

	// Display changes must also reach background views, without activation,
	// audio or focus side effects. Load/ready repair messages sent too early.
	function sendDisplayMode(s) {
		post(s, { type: 'setDisplayMode', value: inPreview(s) ? 'fit' : displayMode() });
	}

	function end(s, reason, error = null) {
		if (disposed || streams.get(s.id) !== s) return;
		s.state = 'ended';
		s.error = error;
		s.playback = null;
		// Withdrawn access is not retried until the user clicks the tab or the app recovers.
		if (reason === 'revoked') s.withdrawn = true;
		if (!s.withdrawn && !s.timer && !s.request && ready(s) && wantsView(s)) {
			s.delay = Math.min(Math.max(s.delay * 2, RETRY_MIN_MS), RETRY_MAX_MS);
			s.timer = setTimeout(() => open(s), s.delay);
		}
		syncPresentation(s);
		refresh();
	}

	function release(s) {
		clearTimeout(s.timer);
		clearTimeout(s.releaseTimer);
		s.timer = s.releaseTimer = null;
		s.request = null;
		s.frame?.remove();
		if (s.url) closeView(s.url);
		s.frame = s.url = s.presentation = null;
		s.state = s.playback = s.error = null;
	}

	function remove(s) {
		release(s);
		streams.delete(s.id);
		s.wrap.remove();
		if (activeId === s.id) activate(null);
	}

	function describe(s) {
		const { kind, status } = s.app;
		const statusText = kind === 'remote' ? remoteStateText(status) : appText(status);
		if (s.state === 'ended' && s.withdrawn) return { dot: 'ended', text: appText('streamRevoked') };
		if (!ready(s)) return { dot: status === 'starting' || status === 'connecting' ? 'connecting' : 'ended', text: statusText };
		if (s.state === 'connected') return { dot: 'connected', text: remoteStateText('connected') };
		if (s.state === 'ended') return { dot: 'ended', text: serverErrorText(s.error) || appText('streamReconnecting') };
		if (s.state === 'connecting') return { dot: 'connecting', text: remoteStateText('connecting') };
		return { dot: 'connected', text: statusText };
	}

	function paint() {
		for (const s of streams.values()) {
			const index = slot(s.id);
			const fallback = index >= 0 && !canPreview(s);
			const normal = s.id === activeId;
			s.wrap.classList.toggle('active', normal);
			paintTile(s, index, s.app.name, ready(s) && !s.withdrawn);
			if (s.frame) s.frame.hidden = fallback || (!normal && index >= 0 && (!ready(s) || s.withdrawn));
			s.status.textContent = fallback && ready(s) ? multiAppText('unsupported') : describe(s).text;
			// The embedded client owns playback/codec error UI once loaded.
			s.status.classList.toggle('hidden', !fallback && (normal || (ready(s) && !s.withdrawn))
				&& (s.state === 'connected' || !!s.playback));
		}
		for (const [id, parts] of placeholders) {
			if (slot(id) >= 0 && !streams.has(id)) continue;
			parts.wrap.remove(); placeholders.delete(id);
		}
		previews.forEach((member, index) => {
			if (streams.has(member.id)) return;
			let parts = placeholders.get(member.id);
			if (!parts) {
				parts = tile(layer); placeholders.set(member.id, parts);
			}
			paintTile(parts, index, member.name || member.id, false);
			parts.status.textContent = multiAppText('unavailable');
		});
	}

	function paintTile(parts, index, name, available) {
		parts.wrap.classList.toggle('preview', index >= 0);
		parts.button.hidden = index < 0;
		if (index < 0) return;
		parts.wrap.dataset.slot = String(index);
		parts.wrap.style.setProperty('--preview-column', String(Math.max(1, index)));
		parts.caption.textContent = name;
		parts.button.disabled = !available;
		parts.button.setAttribute('aria-label', multiAppText('open', name));
	}

	function updatePresentation() {
		multiApp.setShowing(previews.length > 0);
		if (previews.length) {
			layer.dataset.previewCount = String(previews.length);
			layer.style.setProperty('--preview-columns', String(previews.length - 1));
		} else delete layer.dataset.previewCount;

		for (const s of streams.values()) {
			syncPresentation(s);
			if (wantsView(s)) {
				clearTimeout(s.releaseTimer);
				s.releaseTimer = null;
				if (inPreview(s) && ready(s) && !document.hidden && !s.withdrawn && !s.timer
					&& (s.state === null || s.state === 'ended')) open(s);
			} else if (!s.releaseTimer && (s.url || s.request)) {
				s.releaseTimer = setTimeout(() => {
					s.releaseTimer = null;
					if (!wantsView(s)) { release(s); refresh(); }
				}, PREVIEW_RELEASE_MS);
			}
		}
	}

	function refresh() {
		if (disposed) return;
		refreshCatalog();
		onChange();
	}

	function activate(id) {
		if (disposed) return;
		const next = (id && streams.get(id)) || null;
		previews = [];
		activeId = next ? next.id : null;
		if (next) next.unlocked = true;
		updatePresentation();
		if (next) {
			sendDisplayMode(next);
			sendImeMode(next);
			// Activation follows a user gesture, so the view may play audio.
			if (next.state === null || next.state === 'ended') {
				next.withdrawn = false;
				next.delay = 0;
				open(next);
			}
			post(next, { type: 'ivnc-unmute' });
			focusFrame(next);
		} else leaveFrameFocus();
		refresh();
	}

	// Taskbar `apps` push. Tabs appear without stealing focus unless the
	// user just started the app from this page's console.
	function update(apps) {
		order = [];
		for (let app of apps) {
			if (!app || typeof app.id !== 'string' || order.includes(app.id)) continue;
			order.push(app.id);
			let s = streams.get(app.id);
			if (!s) {
				s = entry(app);
				streams.set(app.id, s);
			}
			const wasReady = ready(s);
			// A delayed taskbar snapshot may carry an older alias, while its
			// runtime status is still useful. Preserve only the newer name.
			if ((s.app?.name_revision ?? -1) > (app.name_revision ?? -1)) {
				app = { ...app, name: s.app.name, name_revision: s.app.name_revision };
			}
			s.app = app;
			if (s.frame) s.frame.title = app.name;
			if (!ready(s) && !s.unlocked && (s.url || s.request)) release(s);
			// The app came back (remote reachable again, worker restarted).
			if (!wasReady && ready(s) && wantsView(s) && (s.state === 'ended' || s.state === null)) {
				s.withdrawn = false;
				s.delay = 0;
				open(s);
			}
		}
		for (const s of Array.from(streams.values())) {
			if (!order.includes(s.id)) remove(s);
		}
		if (pendingSelect && Date.now() > pendingSelect.until) pendingSelect = null;
		if (pendingSelect && streams.has(pendingSelect.id)) {
			const id = pendingSelect.id;
			pendingSelect = null;
			activate(id);
		}
		updatePresentation();
		refreshCatalog();
	}

	function refreshCatalog() {
		multiApp.update(order.map(id => streams.get(id)).filter(Boolean).map(s => ({
			id: s.id, name: s.app.name, available: canPreview(s) && ready(s) && !s.withdrawn,
		})));
		paint();
	}

	function tabs() {
		return order.map((id) => streams.get(id)).filter(Boolean).map((s) => {
			const { dot, text } = describe(s);
			return {
				key: `app:${s.id}`, appId: s.id, dot,
				label: s.app.name || s.id, title: `${s.app.name} · ${text}`,
				appName: s.app.name, nameRevision: s.app.name_revision, canRename: s.app.can_rename,
				canManage: s.app.can_rename !== false, kind: s.app.kind,
				running: s.app.status === 'running' || s.app.status === 'starting' || s.app.status === 'connected' || s.app.status === 'connecting',
				statusText: text,
				available: ready(s) && !s.withdrawn,
				active: s.id === activeId, activate: () => activate(s.id),
			};
		});
	}

	const receiveMessage = (event) => {
		if (event.origin !== window.location.origin) return;
		const message = event.data || {};
		if (message.type === 'ivnc-app-started') {
			if (typeof message.id !== 'string' || !fromConsole(event.source)) return;
			if (previews.length) return;
			if (streams.has(message.id)) activate(message.id);
			else pendingSelect = { id: message.id, until: Date.now() + STARTED_SELECT_MS };
			return;
		}
		const s = Array.from(streams.values()).find((v) => v.frame && v.frame.contentWindow === event.source);
		// Messages from a view this page already replaced are stale.
		if (!s || (message.view && message.view !== s.url)) return;
		switch (message.type) {
			case 'ivnc-view-ready':
				sync(s);
				break;
			case 'ivnc-view-state':
				if (s.state === 'ended') break;
				s.playback = message.playback || null;
				if (message.state === 'ended') {
					end(s, String(message.reason || 'closed'));
				} else if (message.state === 'connected') {
					s.state = 'connected';
					s.delay = 0;
					refresh();
				} else if (message.state === 'connecting' && s.state !== 'ended') {
					s.state = 'connecting';
					syncPresentation(s);
					refresh();
				}
				break;
			case 'ivnc-reopen':
				end(s, 'expired');
				break;
			case 'ivnc-ime-state':
				if (message.view !== s.url) break;
				s.ime = !!message.active;
				if (s.id === activeId) onChange();
				break;
            case 'ivnc-ime-preference':
                if (message.view === s.url && s.id === activeId && typeof message.enabled === 'boolean') onImePreference(message.enabled);
                break;
            case 'ivnc-ime-error':
                if (message.view === s.url && s.id === activeId && typeof message.reason === 'string') onImeError(message.reason);
                break;
			case 'ivnc-edge-hover':
				if (s.id === activeId) onEdgeHover();
				break;
		}
	};
	window.addEventListener('message', receiveMessage);
	const visibilityChanged = () => updatePresentation();
	document.addEventListener('visibilitychange', visibilityChanged);
	const untranslate = onLangChange(paint);

	// Each page closes only the view tokens it minted.
	const closePage = () => { for (const s of streams.values()) release(s); };
	window.addEventListener('pagehide', closePage);
	const resumePage = event => {
		if (!event.persisted) return;
		if (activeId) activate(activeId);
		else { updatePresentation(); refresh(); }
	};
	window.addEventListener('pageshow', resumePage);

	const active = () => (activeId && streams.get(activeId)) || null;
	return {
		destroy() {
			disposed = true;
			window.removeEventListener('resize', layout);
			window.removeEventListener('message', receiveMessage);
			window.removeEventListener('pagehide', closePage);
			window.removeEventListener('pageshow', resumePage);
			document.removeEventListener('visibilitychange', visibilityChanged);
			untranslate();
			resizeObserver.disconnect();
			closePage();
			multiApp.destroy();
			streams.clear(); activeId = null; layer.remove();
		},
		multiAppButton: multiApp.button,
		update,
		tabs,
		activate,
		layout,
		syncDisplayMode() {
			if (!disposed) for (const s of streams.values()) sendDisplayMode(s);
		},
        syncImeMode(focus = false) {
            if (!disposed) for (const s of streams.values()) sendImeMode(s, focus);
        },
		activeId: () => activeId,
		coversDesktop: () => !!activeId || previews.length > 0,
		isPreviewing: () => previews.length > 0,
		isActiveFrame: (frame) => !!activeId && streams.get(activeId)?.frame?.contentWindow === frame,
		frames: () => Array.from(streams.values()).map(s => s.frame).filter(Boolean),
		// null when the main desktop has the keyboard.
		imeState: () => active()?.ime ?? null,
		toggleIme(desired) {
			const s = active();
			if (!s) return false;
			if (s.state === 'ended') return true;
			focusFrame(s);
			// Same-origin call preserves the toolbar tap's user activation on iOS.
			try {
				const input = s.frame?.contentWindow?.__ivncInput;
				if (input) { s.ime = input.toggleImeMode(desired); onChange(); return true; }
			} catch (_) {}
			// A loading view cannot accept keyboard input yet. Do not queue a
			// deferred focus which could steal focus from another application.
			return true;
		},
		upload(files) {
			const s = active();
			if (!s) return false;
			post(s, { type: 'ivnc-upload', files });
			return true;
		},
	};
}
