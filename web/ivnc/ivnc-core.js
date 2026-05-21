/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

import webrtc from "./ivnc-wr-core.js?v=50";

const STATUS_UPDATE_INTERVAL_MS = 10000;
const DEFAULT_TITLE = "iVNC";

function renderFavicon(state) {
	const canvas = document.createElement("canvas");
	canvas.width = 64;
	canvas.height = 64;
	const ctx = canvas.getContext("2d");
	if (!ctx) return "/icons/icon.svg";

	const colors = {
		active: "#22c55e",
		viewer_quiet_idle: "#f59e0b",
		no_viewer_idle: "#ef4444",
		offline: "#64748b",
	};
	const color = colors[state] || colors.offline;

	ctx.clearRect(0, 0, 64, 64);
	ctx.fillStyle = "#111827";
	ctx.fillRect(8, 10, 48, 34);
	ctx.fillStyle = color;
	ctx.fillRect(12, 14, 40, 26);
	ctx.fillStyle = "#111827";
	ctx.fillRect(28, 44, 8, 6);
	ctx.fillRect(20, 50, 24, 4);

	if (state === "viewer_quiet_idle") {
		ctx.fillStyle = "#111827";
		ctx.fillRect(23, 20, 6, 16);
		ctx.fillRect(35, 20, 6, 16);
	} else if (state === "no_viewer_idle") {
		ctx.strokeStyle = "#111827";
		ctx.lineWidth = 5;
		ctx.beginPath();
		ctx.moveTo(22, 20);
		ctx.lineTo(42, 36);
		ctx.moveTo(42, 20);
		ctx.lineTo(22, 36);
		ctx.stroke();
	}

	return canvas.toDataURL("image/png");
}

function setFavicon(state) {
	const href = renderFavicon(state);
	let link = document.querySelector('link[rel="icon"]');
	if (!link) {
		link = document.createElement("link");
		link.rel = "icon";
		document.head.appendChild(link);
	}
	link.href = href;
}

function titleStateLabel(state) {
	if (state === "viewer_quiet_idle") return "idle";
	if (state === "no_viewer_idle") return "sleep";
	if (state === "offline") return "offline";
	return "";
}

function setStatusTitle(state, cpuPercent) {
	const label = titleStateLabel(state);
	if (state === "offline") {
		document.title = `${DEFAULT_TITLE} - offline`;
		return;
	}
	const cpu = Number.isFinite(cpuPercent) ? Math.round(cpuPercent) : 0;
	document.title = label ? `${DEFAULT_TITLE} - ${label} - ${cpu}% cpu` : `${DEFAULT_TITLE} - ${cpu}% cpu`;
}

async function updateBrowserStatus() {
	try {
		const response = await fetch(`/api/render-status?ts=${Date.now()}`, {
			cache: "no-store",
			credentials: "same-origin",
		});
		if (!response.ok) throw new Error(`status ${response.status}`);
		const status = await response.json();
		const state = status.render_state || "offline";
		const cpu = Number(status.ivnc_cpu_percent);
		setFavicon(state);
		setStatusTitle(state, cpu);
	} catch (error) {
		setFavicon("offline");
		setStatusTitle("offline", 0);
	}
}

function startBrowserStatusUpdates() {
	updateBrowserStatus();
	window.setInterval(updateBrowserStatus, STATUS_UPDATE_INTERVAL_MS);
}

startBrowserStatusUpdates();

const mode = webrtc();
mode.initialize();
