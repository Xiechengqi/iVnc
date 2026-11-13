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
import { appText, remoteStateText, serverErrorText } from "./lib/i18n.js?v=1";

// Taskbar `status` at which a view can be opened.
const READY = { shared: 'running', remote: 'connected' };
const RETRY_MIN_MS = 1000;
const RETRY_MAX_MS = 30000;
const STARTED_SELECT_MS = 60000;

// For the console page, which runs in an iframe of the desktop page: selects
// the app's stream tab once it appears after the user started it.
export function notifyAppStarted(id) {
	if (window.parent !== window) {
		window.parent.postMessage({ type: 'ivnc-app-started', id }, window.location.origin);
	}
}

const post = (stream, message) => stream.frame?.contentWindow?.postMessage(message, window.location.origin);
const closeView = (url) => fetch(`${url}close`, { method: 'POST', keepalive: true }).catch(() => {});
const fromConsole = (source) => Array.from(document.querySelectorAll('iframe.web-console-frame'))
	.some((frame) => frame.contentWindow === source);

export function createAppStreams({ viewport, onChange, onEdgeHover, onFrameReady = () => {} }) {
	const streams = new Map();
	let order = [];
	let activeId = null;
	let pendingSelect = null;
	let disposed = false;

	const layer = document.createElement('div');
	layer.className = 'app-streams';
	document.body.appendChild(layer);

	// The parent work area already reserves its toolbar. Embedded views use
	// this entire box and must not subtract the toolbar a second time.
	function layout() {
		if (disposed) return;
		const rect = viewport.getBoundingClientRect();
		layer.style.left = `${rect.left}px`;
		layer.style.top = `${rect.top}px`;
		layer.style.width = `${rect.width}px`;
		layer.style.height = `${rect.height}px`;
	}
	window.addEventListener('resize', layout);
	const resizeObserver = new ResizeObserver(layout);
	resizeObserver.observe(viewport);
	layout();

	const ready = (s) => s.app.status === READY[s.app.kind];

	function entry(app) {
		const wrap = document.createElement('div');
		wrap.className = 'app-stream';
		const status = document.createElement('div');
		status.className = 'app-stream-status';
		wrap.appendChild(status);
		layer.appendChild(wrap);
		// state: null (never opened) | 'connecting' | 'connected' | 'ended'
		// error: the server's answer when a view could not be opened.
		return { id: app.id, app, wrap, status, frame: null, url: null, state: null, error: null, withdrawn: false, minting: false, timer: null, delay: 0, unlocked: false, ime: false };
	}

	// Mints a view token for this page and (re)loads the iframe with it.
	async function open(s) {
		clearTimeout(s.timer);
		s.timer = null;
		if (disposed || s.minting || !ready(s)) return;
		s.minting = true;
		s.state = 'connecting';
		s.error = null;
		s.playback = null;
		refresh();
		try {
			const response = await fetch(`/api/apps/${encodeURIComponent(s.id)}/view`, { method: 'POST' });
			const body = await response.json().catch(() => ({}));
			if (!response.ok || !body.url) throw Object.assign(new Error(body.error || response.statusText), { body });
			if (streams.get(s.id) !== s) {
				closeView(body.url);
				return;
			}
			if (s.url) closeView(s.url);
			s.url = body.url;
			if (s.frame) {
				s.frame.src = `${s.url}?embed=1`;
			} else {
				// src is set before insertion so no about:blank load is observed.
				s.frame = document.createElement('iframe');
				s.frame.title = s.app.name;
				s.frame.allow = 'autoplay; clipboard-read; clipboard-write; fullscreen';
				s.frame.src = `${s.url}?embed=1`;
				s.frame.addEventListener('load', () => loaded(s));
				s.wrap.prepend(s.frame);
			}
		} catch (error) {
			s.minting = false;
			end(s, 'error', error.body?.error ? error.body : { error: error.message });
		} finally {
			s.minting = false;
		}
	}

	function loaded(s) {
		if (disposed || streams.get(s.id) !== s) return;
		// An expired token answers with an error document instead of the client.
		let client = false;
		try {
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
		post(s, { type: 'ivnc-view-active', active: s.id === activeId });
		// unlocked: the user has activated this tab, so audio may play.
		if (s.unlocked) post(s, { type: 'ivnc-unmute' });
		if (s.id === activeId) s.frame.contentWindow.focus();
	}

	function end(s, reason, error = null) {
		if (disposed || streams.get(s.id) !== s) return;
		s.state = 'ended';
		s.error = error;
		s.playback = null;
		// Withdrawn access is not retried until the user clicks the tab or the app recovers.
		if (reason === 'revoked') s.withdrawn = true;
		if (!s.withdrawn && !s.timer && !s.minting && ready(s)) {
			s.delay = Math.min(Math.max(s.delay * 2, RETRY_MIN_MS), RETRY_MAX_MS);
			s.timer = setTimeout(() => open(s), s.delay);
		}
		refresh();
	}

	function remove(s) {
		clearTimeout(s.timer);
		streams.delete(s.id);
		s.wrap.remove();
		if (s.url) closeView(s.url);
		if (activeId === s.id) activate(null);
	}

	function describe(s) {
		const { kind, status } = s.app;
		const statusText = kind === 'remote' ? remoteStateText(status) : appText(status);
		if (s.state === 'connected') return { dot: 'connected', text: remoteStateText('connected') };
		if (s.state === 'ended' && s.withdrawn) return { dot: 'ended', text: appText('streamRevoked') };
		if (!ready(s)) return { dot: status === 'starting' || status === 'connecting' ? 'connecting' : 'ended', text: statusText };
		if (s.state === 'ended') return { dot: 'ended', text: serverErrorText(s.error) || appText('streamReconnecting') };
		if (s.state === 'connecting') return { dot: 'connecting', text: remoteStateText('connecting') };
		return { dot: 'connected', text: statusText };
	}

	function paint() {
		for (const s of streams.values()) {
			s.status.textContent = describe(s).text;
			// The embedded client owns playback/codec error UI once loaded.
			s.status.classList.toggle('hidden', s.state === 'connected' || !!s.playback);
		}
	}

	function refresh() {
		if (disposed) return;
		paint();
		onChange();
	}

	function activate(id) {
		if (disposed) return;
		const next = (id && streams.get(id)) || null;
		activeId = next ? next.id : null;
		for (const s of streams.values()) {
			s.wrap.classList.toggle('active', s === next);
			post(s, { type: 'ivnc-view-active', active: s === next });
		}
		if (next) {
			// Activation follows a user gesture, so the view may play audio.
			next.unlocked = true;
			if (next.state === null || next.state === 'ended') {
				next.withdrawn = false;
				next.delay = 0;
				open(next);
			}
			post(next, { type: 'ivnc-unmute' });
			next.frame?.contentWindow?.focus();
		} else if (document.activeElement?.closest?.('.app-streams')) {
			document.activeElement.blur();
		}
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
			// The app came back (remote reachable again, worker restarted).
			if (!wasReady && ready(s) && (s.state === 'ended' || (s.state === null && s.id === activeId))) {
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
		paint();
	}

	function tabs() {
		return order.map((id) => streams.get(id)).filter(Boolean).map((s) => {
			const { dot, text } = describe(s);
			return {
				key: `app:${s.id}`, appId: s.id, dot,
				label: s.app.name || s.id, title: `${s.app.name} · ${text}`,
				appName: s.app.name, nameRevision: s.app.name_revision, canRename: s.app.can_rename,
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
					refresh();
				}
				break;
			case 'ivnc-reopen':
				end(s, 'expired');
				break;
			case 'ivnc-ime-state':
				s.ime = !!message.active;
				if (s.id === activeId) onChange();
				break;
			case 'ivnc-edge-hover':
				if (s.id === activeId) onEdgeHover();
				break;
		}
	};
	window.addEventListener('message', receiveMessage);

	// Each page closes only the view tokens it minted.
	const closePage = () => {
		for (const s of streams.values()) {
			if (s.url) closeView(s.url);
		}
	};
	window.addEventListener('pagehide', closePage);

	const active = () => (activeId && streams.get(activeId)) || null;
	return {
		destroy() {
			disposed = true;
			window.removeEventListener('resize', layout);
			window.removeEventListener('message', receiveMessage);
			window.removeEventListener('pagehide', closePage);
			resizeObserver.disconnect();
			closePage();
			for (const s of streams.values()) clearTimeout(s.timer);
			streams.clear(); activeId = null; layer.remove();
		},
		update,
		tabs,
		activate,
		layout,
		activeId: () => activeId,
		isActiveFrame: (frame) => !!activeId && streams.get(activeId)?.frame?.contentWindow === frame,
		frames: () => Array.from(streams.values()).map(s => s.frame).filter(Boolean),
		// null when the main desktop has the keyboard.
		imeState: () => active()?.ime ?? null,
		toggleIme() {
			const s = active();
			if (!s) return false;
			s.frame?.contentWindow?.focus();
			post(s, { type: 'ivnc-ime' });
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
