/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * This file incorporates work covered by the following copyright and
 * permission notice:
 *
 *   Copyright 2019 Google LLC
 *
 *   Licensed under the Apache License, Version 2.0 (the "License");
 *   you may not use this file except in compliance with the License.
 *   You may obtain a copy of the License at
 *
 *        http://www.apache.org/licenses/LICENSE-2.0
 *
 *   Unless required by applicable law or agreed to in writing, software
 *   distributed under the License is distributed on an "AS IS" BASIS,
 *   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *   See the License for the specific language governing permissions and
 *   limitations under the License.
 */

import { WebRTCDemo } from "./lib/webrtc.js?v=26";
import { WebRTCDemoSignaling } from "./lib/signaling.js?v=1";
import { stringToBase64 } from "./lib/util.js?v=1";
import { Input } from "./lib/input2.js?v=21";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import terminalStyles from "@xterm/xterm/css/xterm.css?inline";
import { t, onLangChange, getLang } from "./lib/i18n.js?v=1";
import { createAppStreams } from "./app-streams.js";
import { createToolbar } from "./toolbar.js";
import { createGestureNavigation } from "./gesture-navigation.mjs";
import { createClipboard } from "./clipboard.js";
import { createClipboardUI } from "./clipboard-ui.js";
import { createViewport } from "./viewport.js";
import { createFloatingWindow } from "./floating-window.js";
import { installOverlayGuard, minimizeOtherOverlays, createConsoleShield, releaseOverlayInput } from "./overlay-manager.js";

const MAIN_I18N = {
	zh: {
		videoConnecting: '正在连接画面…',
		videoWaiting: '已连接，正在等待画面…',
		videoBlocked: '浏览器暂停了画面，请点击播放。',
		videoNoData: '已连接，但尚未收到画面，请检查服务端或重新连接。',
		videoDecodeError: '画面解码失败，请重新连接；若持续出现，请检查视频编码兼容性。',
		videoPlaybackError: '画面播放失败，请点击重试。',
		videoConnectionError: '画面连接失败，请检查网络或重新连接。',
		videoUnsupported: '当前浏览器无法接收此视频格式或不支持画面连接。',
		videoPlay: '播放画面',
		videoRetry: '重试',
		// toolbar tooltips
		connectionsCount: n => `连接数: ${n}`,
		connectionMode: '连接模式',
		unpinTaskbar: '取消固定',
		pinTaskbar: '固定任务栏',
		imeOn: '中文输入 (Ctrl+Shift+Space)',
		imeOff: '关闭中文输入 (Ctrl+Shift+Space)',
		changePassword: '修改密码',
		uploadFile: '上传文件到桌面',
		restartOrUpgrade: '重启或升级',
		proxyTooltip: '代理',
		terminalTooltip: '终端',
		consoleTooltip: '控制台',
		// floating windows
		minimize: '最小化',
		terminalBackgroundTranslucent: '半透明',
		terminalBackgroundOpaque: '不透明',
		terminalTranslucentBackground: '半透明背景',
		terminalSwitchToOpaque: '切换为不透明背景',
		terminalSwitchToTranslucent: '切换为半透明背景',
		terminalPin: '置顶终端',
		terminalUnpin: '取消终端置顶',
		terminalPinned: '终端置顶',
		terminalRestore: '还原窗口位置和大小',
		consoleTitle: '控制台',
		proxyTitle: '代理',
		consoleFrameTitle: 'iVnc 控制台',
		proxyFrameTitle: 'iVnc 代理',
		// password modal
		pwdHeader: '修改密码',
		pwdNewPlaceholder: '新密码 (至少4位)',
		pwdConfirmPlaceholder: '确认新密码',
		cancel: '取消',
		confirm: '确定',
		pwdTooShort: '密码至少需要4个字符',
		pwdMismatch: '两次输入的密码不一致',
		pwdChanged: '密码已修改，下次请求将使用新密码',
		pwdChangeFailed: '修改失败',
		networkError: '网络错误',
		// upgrade modal
		upgradeHeader: '重启或升级',
		upgradeNotice: '将重启 iVNC 服务，连接会短暂中断。',
		upgradeOption: '升级到最新',
		upgradeWarning: '勾选后会先更新 iVNC binary，然后重启服务。',
		restartService: '重启服务',
		upgradeAndRestart: '升级并重启',
		close: '关闭',
		upgradePreparing: '准备升级...',
		restartPreparing: '准备重启...',
		restarting: '重启中...',
		failed: '失败',
		upgradeDoneWaiting: '更新完成，等待服务重启...',
		websocketFailed: 'WebSocket 连接失败',
		requestingRestart: '正在请求重启服务...',
		restartSent: '重启请求已发送，等待服务恢复...',
		restartRequestFailed: err => `重启请求失败: ${err}`,
		serviceRecovered: '服务已恢复，即将刷新页面...',
		serviceNotRecovered: '服务未恢复，请稍后手动刷新',
		// no-window overlay
		waitingAppTitle: '等待应用启动',
		noAppRunning: '当前没有应用在运行',
		// connection management
		connectionMgmtTitle: '连接管理',
		clientFeatures: '客户端浏览器特征',
		clientFeatureBrowser: '浏览器',
		clientFeaturePlatform: '平台',
		clientFeatureLanguage: '语言',
		clientFeatureViewport: '视口',
		clientFeatureScreen: '屏幕',
		clientFeatureDpr: 'DPR',
		clientFeatureCpu: 'CPU 线程',
		clientFeatureMemory: '内存',
		clientFeatureTouch: '触控点',
		clientFeatureTimezone: '时区',
		clientFeatureNetwork: '网络',
		clientFeatureOnline: '在线',
		clientFeatureWebgl: 'WebGL',
		clientFeatureUnknown: '未知',
		networkQualityGood: '网络：优',
		networkQualityFair: '网络：良',
		networkQualityPoor: '网络：差',
		networkQualityOffline: '网络：断开',
		networkQualityUnknown: '网络：检测中',
		networkQualityTooltip: (rtt, loss, fps, bitrate, jitter, type) => `RTT: ${rtt}\n丢包: ${loss}\nFPS: ${fps}\n码率: ${bitrate}\n抖动: ${jitter}\n类型: ${type}`,
		currentConnectionsCount: n => `当前连接数: ${n}`,
		connTime: '连接时间',
		connDuration: '持续',
		connType: '类型',
		noConnections: '暂无连接',
		durationHM: (h, m) => `${h}小时${m}分钟`,
		durationM: m => `${m}分钟`,
		// upload toast
		uploadFileTitle: '上传文件',
		uploadSuccess: '✓ 上传成功',
		uploadSavedTo: path => `文件已保存到: ${path}`,
		uploadFailed: '✗ 上传失败',
		unknownError: '未知错误',
	},
	en: {
		videoConnecting: 'Connecting video…',
		videoWaiting: 'Connected. Waiting for video…',
		videoBlocked: 'The browser paused video. Click to play.',
		videoNoData: 'Connected, but no video has arrived. Check the server or reconnect.',
		videoDecodeError: 'Video decoding failed. Reconnect, or check video codec compatibility if it persists.',
		videoPlaybackError: 'Video playback failed. Click to retry.',
		videoConnectionError: 'Video connection failed. Check the network or reconnect.',
		videoUnsupported: 'This browser cannot receive the video format or establish a video connection.',
		videoPlay: 'Play video',
		videoRetry: 'Retry',
		connectionsCount: n => `Connections: ${n}`,
		connectionMode: 'Connection mode',
		unpinTaskbar: 'Unpin taskbar',
		pinTaskbar: 'Pin taskbar',
		imeOn: 'Chinese IME (Ctrl+Shift+Space)',
		imeOff: 'Disable Chinese IME (Ctrl+Shift+Space)',
		changePassword: 'Change password',
		uploadFile: 'Upload file to desktop',
		restartOrUpgrade: 'Restart or upgrade',
		proxyTooltip: 'Proxy',
		terminalTooltip: 'Terminal',
		consoleTooltip: 'Console',
		minimize: 'Minimize',
		terminalBackgroundTranslucent: 'Translucent',
		terminalBackgroundOpaque: 'Opaque',
		terminalTranslucentBackground: 'Translucent background',
		terminalSwitchToOpaque: 'Switch to opaque background',
		terminalSwitchToTranslucent: 'Switch to translucent background',
		terminalPin: 'Pin terminal on top',
		terminalUnpin: 'Unpin terminal',
		terminalPinned: 'Terminal always on top',
		terminalRestore: 'Restore window position and size',
		consoleTitle: 'Console',
		proxyTitle: 'Proxy',
		consoleFrameTitle: 'iVnc Console',
		proxyFrameTitle: 'iVnc Proxy',
		pwdHeader: 'Change Password',
		pwdNewPlaceholder: 'New password (min 4 chars)',
		pwdConfirmPlaceholder: 'Confirm new password',
		cancel: 'Cancel',
		confirm: 'OK',
		pwdTooShort: 'Password must be at least 4 characters',
		pwdMismatch: 'Passwords do not match',
		pwdChanged: 'Password changed. The next request will use the new password.',
		pwdChangeFailed: 'Change failed',
		networkError: 'Network error',
		upgradeHeader: 'Restart or Upgrade',
		upgradeNotice: 'iVNC service will restart, the connection will be briefly interrupted.',
		upgradeOption: 'Upgrade to latest',
		upgradeWarning: 'If checked, the iVNC binary will be updated before restarting.',
		restartService: 'Restart',
		upgradeAndRestart: 'Upgrade & restart',
		close: 'Close',
		upgradePreparing: 'Preparing upgrade...',
		restartPreparing: 'Preparing restart...',
		restarting: 'Restarting...',
		failed: 'Failed',
		upgradeDoneWaiting: 'Update complete, waiting for service to restart...',
		websocketFailed: 'WebSocket connection failed',
		requestingRestart: 'Requesting service restart...',
		restartSent: 'Restart request sent, waiting for service to recover...',
		restartRequestFailed: err => `Restart request failed: ${err}`,
		serviceRecovered: 'Service recovered, refreshing page...',
		serviceNotRecovered: 'Service not recovered, please refresh manually later',
		waitingAppTitle: 'Waiting for App',
		noAppRunning: 'No app is currently running',
		connectionMgmtTitle: 'Connection Management',
		clientFeatures: 'Client Browser Features',
		clientFeatureBrowser: 'Browser',
		clientFeaturePlatform: 'Platform',
		clientFeatureLanguage: 'Language',
		clientFeatureViewport: 'Viewport',
		clientFeatureScreen: 'Screen',
		clientFeatureDpr: 'DPR',
		clientFeatureCpu: 'CPU threads',
		clientFeatureMemory: 'Memory',
		clientFeatureTouch: 'Touch points',
		clientFeatureTimezone: 'Timezone',
		clientFeatureNetwork: 'Network',
		clientFeatureOnline: 'Online',
		clientFeatureWebgl: 'WebGL',
		clientFeatureUnknown: 'Unknown',
		networkQualityGood: 'Network: Good',
		networkQualityFair: 'Network: Fair',
		networkQualityPoor: 'Network: Poor',
		networkQualityOffline: 'Network: Offline',
		networkQualityUnknown: 'Network: Checking',
		networkQualityTooltip: (rtt, loss, fps, bitrate, jitter, type) => `RTT: ${rtt}\nLoss: ${loss}\nFPS: ${fps}\nBitrate: ${bitrate}\nJitter: ${jitter}\nType: ${type}`,
		currentConnectionsCount: n => `Current connections: ${n}`,
		connTime: 'Connected at',
		connDuration: 'Duration',
		connType: 'Type',
		noConnections: 'No connections',
		durationHM: (h, m) => `${h}h ${m}m`,
		durationM: m => `${m}m`,
		uploadFileTitle: 'Upload File',
		uploadSuccess: '✓ Upload succeeded',
		uploadSavedTo: path => `Saved to: ${path}`,
		uploadFailed: '✗ Upload failed',
		unknownError: 'Unknown error',
	},
};

const tt = (key, ...args) => t(MAIN_I18N, key, ...args);

const _liveTitles = new Set();
const _liveTexts = new Set();
function setLiveTitle(el, key, ...args) {
	if (!el) return;
	el._titleKey = key;
	el._titleArgs = args;
	el.title = tt(key, ...args);
	_liveTitles.add(el);
}
function setLiveText(el, key, ...args) {
	if (!el) return;
	el._textKey = key;
	el._textArgs = args;
	el.textContent = tt(key, ...args);
	_liveTexts.add(el);
}
onLangChange(() => {
	for (const el of Array.from(_liveTitles)) {
		if (!el.isConnected) { _liveTitles.delete(el); continue; }
		el.title = tt(el._titleKey, ...(el._titleArgs || []));
	}
	for (const el of Array.from(_liveTexts)) {
		if (!el.isConnected) { _liveTexts.delete(el); continue; }
		el.textContent = tt(el._textKey, ...(el._textArgs || []));
	}
});

function InitUI() {
	let style = document.createElement('style');
	style.textContent = `
	body {
		background-color: #000000;
		font-family: sans-serif;
		margin: 0;
		padding: 0;
		overflow: hidden;
		background-color: #000;
		color: #fff;
	}
	html.ivnc-gesture-navigation,
	html.ivnc-gesture-navigation body { overscroll-behavior-x: none; }

	#app {
		display: flex;
		flex-direction: column;
		height: calc(var(--vh, 1vh) * 100 - var(--ivnc-toolbar-height, 0px));
		height: calc(100dvh - var(--ivnc-toolbar-height, 0px));
		width: 100%;
	}

	.video-container {
		min-height: 0;
		min-width: 0;
		flex-grow: 1;
		flex-shrink: 1;
		display: flex;
		flex-direction: column;
		justify-content: center;
		align-items: center;
		height: 100%;
		width: 100%;
		position: relative;
		overflow: hidden;
	}
	.video-container.manual-exact { overflow: auto; }
	.video-container:fullscreen { width: 100%; height: 100%; }

	.video-container video,
	.video-container #overlayInput{
		position: absolute;
		top: 0;
		left: 0;
		width: 100%;
		height: 100%;
	}

	.video-container video {
		object-fit: fill;
	}

	.video-container #overlayInput {
		opacity: 0;
		z-index: 3;
		touch-action: none;
		caret-color: transparent;
		background-color: transparent;
		color: transparent;
		pointer-events: auto;
		-webkit-user-select: none;
		border: none;
		outline: none;
		padding: 0;
		margin: 0;
	}

	.video-container #playButton {
		position: absolute;
		top: 50%;
		left: 50%;
		transform: translate(-50%, -50%);
		z-index: 1002;
	}

	.video-container .status-bar {
		position: absolute;
		bottom: 0;
		left: 0;
		width: 100%;
		padding: 5px;
		background-color: rgba(0, 0, 0, 0.7);
		color: #fff;
		text-align: center;
		z-index: 1001;
	}

	.loading-text {
		margin-top: 1em;
	}

	.hidden {
		display: none !important;
	}

	#playButton {
		padding: 15px 30px;
		font-size: 1.5em;
		cursor: pointer;
		background-color: rgba(0, 0, 0, 0.5);
		color: white;
		border: 1px solid rgba(255, 255, 255, 0.3);
		border-radius: 3px;
		backdrop-filter: blur(5px);
	}
	.no-window-overlay {
		position: absolute;
		top: 0;
		left: 0;
		width: 100%;
		height: 100%;
		background: #f5f5f5;
		display: flex;
		justify-content: center;
		align-items: center;
		z-index: 1000;
		color: #333;
		font-family: system-ui, sans-serif;
	}
	.no-window-overlay.hidden { display: none; }
	.no-window-content { text-align: center; }
	.no-window-content h2 { font-size: 24px; margin-bottom: 10px; }
	.no-window-content p { font-size: 14px; color: #666; }

	.taskbar {
		position: fixed;
		bottom: 0;
		left: 0;
		width: 100%;
		height: 36px;
		background: rgba(30, 30, 30, 0.85);
		backdrop-filter: blur(8px);
		display: flex;
		align-items: center;
		padding: 0 4px;
		z-index: 1000;
		box-sizing: border-box;
		transform: translateY(100%);
		transition: transform 0.15s ease;
		pointer-events: auto;
	}
	.taskbar.visible {
		transform: translateY(0);
	}
	.taskbar.pinned {
		transform: translateY(0);
	}
	.taskbar-pin {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		margin: 0 4px 0 2px;
		border-radius: 4px;
		background: rgba(255, 255, 255, 0.08);
		color: #888;
		font-size: 14px;
		cursor: pointer;
		flex-shrink: 0;
		border: 1px solid transparent;
		transition: background 0.1s;
	}
	.taskbar-pin:hover {
		background: rgba(255, 255, 255, 0.15);
	}
	.taskbar-pin.active {
		color: #4c86e6;
		background: rgba(76, 134, 230, 0.2);
		border-color: rgba(76, 134, 230, 0.4);
	}
	.taskbar-pin.disabled {
		opacity: 0.4;
		cursor: not-allowed;
		pointer-events: none;
	}
	.taskbar-pin.uploading {
		animation: pulse 1.5s ease-in-out infinite;
	}
	@keyframes pulse {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.5; }
	}
	.upload-toast {
		position: fixed;
		top: 20px;
		right: 20px;
		min-width: 300px;
		max-width: 400px;
		background: rgba(30, 30, 30, 0.95);
		border: 1px solid rgba(255, 255, 255, 0.2);
		border-radius: 8px;
		padding: 16px;
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
		z-index: 10000;
		color: #fff;
		font-family: system-ui, sans-serif;
		animation: slideIn 0.3s ease-out;
	}
	@keyframes slideIn {
		from {
			transform: translateX(400px);
			opacity: 0;
		}
		to {
			transform: translateX(0);
			opacity: 1;
		}
	}
	.upload-toast.success {
		border-color: rgba(76, 175, 80, 0.5);
	}
	.upload-toast.error {
		border-color: rgba(244, 67, 54, 0.5);
	}
	.upload-toast-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin-bottom: 8px;
	}
	.upload-toast-title {
		font-size: 14px;
		font-weight: 600;
		color: #fff;
	}
	.upload-toast-close {
		background: none;
		border: none;
		color: rgba(255, 255, 255, 0.6);
		cursor: pointer;
		font-size: 18px;
		padding: 0;
		width: 20px;
		height: 20px;
		line-height: 20px;
		text-align: center;
	}
	.upload-toast-close:hover {
		color: #fff;
	}
	.upload-toast-filename {
		font-size: 12px;
		color: rgba(255, 255, 255, 0.8);
		margin-bottom: 8px;
		word-break: break-all;
	}
	.upload-toast-progress {
		width: 100%;
		height: 4px;
		background: rgba(255, 255, 255, 0.1);
		border-radius: 2px;
		overflow: hidden;
		margin-bottom: 8px;
	}
	.upload-toast-progress-bar {
		height: 100%;
		background: linear-gradient(90deg, #4c86e6, #5ea3f5);
		border-radius: 2px;
		transition: width 0.3s ease;
	}
	.upload-toast-info {
		font-size: 11px;
		color: rgba(255, 255, 255, 0.6);
		display: flex;
		justify-content: space-between;
	}
	.upload-toast-path {
		font-size: 12px;
		color: #4caf50;
		margin-top: 8px;
		padding: 8px;
		background: rgba(76, 175, 80, 0.1);
		border-radius: 4px;
		word-break: break-all;
	}
	.upload-toast-error {
		font-size: 12px;
		color: #f44336;
		margin-top: 8px;
	}
	.taskbar-conn {
		position: absolute;
		right: 8px;
		top: 0;
		padding: 0 4px;
		font-size: 11px;
		line-height: 36px;
		color: rgba(255, 255, 255, 0.7);
		white-space: nowrap;
		user-select: none;
		pointer-events: none;
	}
	.taskbar-network-quality {
		position: absolute;
		right: 42px;
		top: 6px;
		height: 24px;
		padding: 0 8px;
		border-radius: 4px;
		font-size: 11px;
		line-height: 24px;
		font-family: system-ui, sans-serif;
		font-weight: 600;
		white-space: nowrap;
		user-select: none;
		pointer-events: auto;
		cursor: default;
		color: rgba(255, 255, 255, 0.86);
		background: rgba(148, 163, 184, 0.22);
		border: 1px solid rgba(148, 163, 184, 0.35);
	}
	.taskbar-network-quality.good {
		background: rgba(34, 197, 94, 0.22);
		border-color: rgba(34, 197, 94, 0.45);
		color: #bbf7d0;
	}
	.taskbar-network-quality.fair {
		background: rgba(234, 179, 8, 0.22);
		border-color: rgba(234, 179, 8, 0.45);
		color: #fef3c7;
	}
	.taskbar-network-quality.poor {
		background: rgba(239, 68, 68, 0.24);
		border-color: rgba(239, 68, 68, 0.52);
		color: #fecaca;
	}
	.taskbar-network-quality.offline {
		background: rgba(107, 114, 128, 0.22);
		border-color: rgba(107, 114, 128, 0.35);
		color: rgba(255, 255, 255, 0.58);
	}
	.taskbar-trigger {
		position: fixed;
		bottom: 0;
		left: 0;
		width: 100%;
		height: 6px;
		z-index: 999;
	}
	.taskbar-item {
		display: inline-flex;
		align-items: center;
		display: flex;
		align-items: center;
		height: 28px;
		padding: 0 12px;
		margin: 0 2px;
		border-radius: 4px;
		background: rgba(255, 255, 255, 0.08);
		color: #ccc;
		font-size: 12px;
		font-family: system-ui, sans-serif;
		cursor: pointer;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		min-width: 120px;
		max-width: 200px;
		user-select: none;
		border: 1px solid transparent;
		transition: background 0.1s;
	}
	.taskbar-item:hover {
		background: rgba(255, 255, 255, 0.15);
	}
	.taskbar-item.focused {
		background: rgba(76, 134, 230, 0.35);
		color: #fff;
		border-color: rgba(76, 134, 230, 0.6);
	}
	.taskbar-app-dot {
		flex-shrink: 0;
		width: 7px;
		height: 7px;
		margin-right: 6px;
		border-radius: 50%;
		background: #64748b;
	}
	.taskbar-app-dot.connecting { background: #f59e0b; }
	.taskbar-app-dot.connected { background: #22c55e; }
	/* Stream tabs: laid over the video, below the taskbar and overlays. */
	.app-streams {
		position: fixed;
		z-index: 990;
		visibility: hidden;
		pointer-events: none;
	}
	.app-stream {
		position: absolute;
		inset: 0;
		background: #000;
	}
	.app-stream.active {
		visibility: visible;
		pointer-events: auto;
	}
	.app-stream iframe {
		width: 100%;
		height: 100%;
		border: 0;
		display: block;
	}
	.app-stream-status {
		position: absolute;
		inset: 0;
		display: flex;
		align-items: center;
		justify-content: center;
		color: rgba(255, 255, 255, 0.75);
		font: 14px system-ui, sans-serif;
		pointer-events: none;
	}
	.web-terminal-modal {
		position: fixed;
		left: 50%;
		top: 50%;
		width: var(--ivnc-terminal-width, min(920px, calc(100vw - 48px)));
		height: var(--ivnc-terminal-height, min(560px, calc(100vh - 96px)));
		min-width: 320px;
		min-height: 240px;
		z-index: 1800;
		display: flex;
		flex-direction: column;
		transform: translate(-50%, -50%) scale(var(--ivnc-modal-scale, 1));
		transform-origin: center center;
		background: #101214;
		border: 1px solid rgba(255, 255, 255, 0.18);
		border-radius: 8px;
		box-shadow: 0 18px 48px rgba(0, 0, 0, 0.55);
		overflow: hidden;
	}
	.web-terminal-modal.minimized {
		display: none;
	}
	.web-terminal-floating { box-sizing: border-box; }
	.web-terminal-floating > .web-terminal-header { cursor: grab; touch-action: none; user-select: none; }
	.web-terminal-floating.window-interacting > .web-terminal-header { cursor: grabbing; }
	.web-terminal-floating .web-terminal-actions { position: relative; z-index: 4; }
	.web-terminal-floating[data-terminal-pinned="true"] { z-index: 1900; }
	.web-terminal-action:focus-visible { outline: 2px solid #8bbcff; outline-offset: 1px; }
	.web-terminal-resize { position: absolute; z-index: 3; touch-action: none; }
	.resize-n, .resize-s { left: var(--terminal-resize-corner); right: var(--terminal-resize-corner); height: var(--terminal-resize-hit); cursor: ns-resize; }
	.resize-n { top: 0; } .resize-s { bottom: 0; }
	.resize-e, .resize-w { top: var(--terminal-resize-corner); bottom: var(--terminal-resize-corner); width: var(--terminal-resize-hit); cursor: ew-resize; }
	.resize-e { right: 0; } .resize-w { left: 0; }
	.resize-ne, .resize-nw, .resize-se, .resize-sw { width: var(--terminal-resize-corner); height: var(--terminal-resize-corner); }
	.resize-ne { right: 0; top: 0; cursor: nesw-resize; }
	.resize-nw { left: 0; top: 0; cursor: nwse-resize; }
	.resize-se { right: 0; bottom: 0; cursor: nwse-resize; }
	.resize-sw { left: 0; bottom: 0; cursor: nesw-resize; }
	.resize-se::after { content: ''; position: absolute; right: 3px; bottom: 3px; width: 4px; height: 4px; border-right: 1px solid #718096; border-bottom: 1px solid #718096; }
	.console-dialog-shield { position: fixed; inset: 0; z-index: 1940; touch-action: none; }
	.console-dialog-shield[hidden] { display: none; }
	.web-console-modal.has-child-dialog { z-index: 1950; }
	.web-terminal-header {
		height: 34px;
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0 8px 0 12px;
		background: #1b1f23;
		border-bottom: 1px solid rgba(255, 255, 255, 0.1);
		color: #e5e7eb;
		font: 12px system-ui, sans-serif;
		flex-shrink: 0;
	}
	.web-terminal-title {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}
	.web-terminal-status {
		color: #8bd18b;
		white-space: nowrap;
	}
	.web-terminal-status.error {
		color: #ff8f8f;
	}
	.web-terminal-actions {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	.web-terminal-action {
		width: 24px;
		height: 24px;
		border: 0;
		border-radius: 4px;
		background: rgba(255, 255, 255, 0.08);
		color: #d1d5db;
		cursor: pointer;
		line-height: 1;
	}
	.web-terminal-action:hover {
		background: rgba(255, 255, 255, 0.16);
		color: #fff;
	}
	.web-terminal-modal[data-terminal-background] .web-terminal-action {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		padding: 0;
		flex: 0 0 24px;
	}
	.web-terminal-background[aria-pressed="true"] .web-terminal-background-solid { display: none; }
	.web-terminal-pin { transition: background-color 150ms ease, color 150ms ease; }
	.web-terminal-pin > svg { transform: rotate(-35deg); transition: transform 150ms ease; }
	.web-terminal-pin[aria-pressed="true"] { color: #a6ccff; background: #2b4c76; }
	.web-terminal-pin[aria-pressed="true"] > svg { transform: rotate(0deg); }
	.web-terminal-pin[aria-pressed="true"]:hover,
	.web-terminal-pin[aria-pressed="true"]:focus-visible { color: #c4deff; background: #355d8e; }
	.web-terminal-pin[aria-pressed="true"]:active { color: #c4deff; background: #244268; }
	@media (prefers-reduced-motion: reduce) {
		.web-terminal-pin, .web-terminal-pin > svg { transition: none; }
	}
	.web-terminal-body {
		flex: 1;
		min-height: 0;
		padding: 8px;
		background: #050607;
	}
	.web-terminal-body .xterm {
		height: 100%;
	}
	/* Only the PTY terminal carries this attribute; console/proxy share the
	   base modal styles. Paint the background once to avoid stacked alpha. */
	.web-terminal-modal[data-terminal-background] {
		background: #050607;
	}
	.web-terminal-modal[data-terminal-background="translucent"] {
		background: rgba(5, 6, 7, 0.85);
	}
	.web-terminal-modal[data-terminal-background="translucent"] > .web-terminal-header {
		background: rgba(27, 31, 35, 0.35);
	}
	.web-terminal-modal[data-terminal-background] > .web-terminal-body,
	.web-terminal-modal[data-terminal-background] .xterm-viewport {
		background: transparent;
	}
	.web-terminal-modal[data-terminal-background] .web-terminal-title {
		overflow: hidden;
	}
	.web-terminal-modal[data-terminal-background] .web-terminal-title > svg,
	.web-terminal-modal[data-terminal-background] .web-terminal-title > span:first-of-type,
	.web-terminal-modal[data-terminal-background] .web-terminal-actions {
		flex-shrink: 0;
	}
	.web-terminal-modal[data-terminal-background] .web-terminal-status {
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.web-console-modal {
		width: var(--ivnc-console-width, min(1080px, calc(100vw - 48px)));
		height: var(--ivnc-console-height, min(720px, calc(100vh - 96px)));
		min-width: 360px;
		min-height: 300px;
		background: #101214;
		border-color: transparent;
	}
	.web-console-body {
		padding: 0;
		background: #101214;
	}
	.web-console-frame {
		width: 100%;
		height: 100%;
		border: 0;
		display: block;
		background: #101214;
	}
	.pwd-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0,0,0,0.6);
		z-index: 2000;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.pwd-dialog {
		background: #1e1e1e;
		border: 1px solid #444;
		border-radius: 8px;
		padding: 24px;
		min-width: 300px;
		color: #eee;
		font-family: system-ui, sans-serif;
	}
	.pwd-dialog h3 {
		margin: 0 0 16px;
		font-size: 15px;
		font-weight: 600;
	}
	.pwd-dialog input {
		display: block;
		width: 100%;
		padding: 8px;
		margin-bottom: 10px;
		border: 1px solid #555;
		border-radius: 4px;
		background: #2a2a2a;
		color: #eee;
		font-size: 13px;
		box-sizing: border-box;
	}
	.pwd-dialog input:focus {
		outline: none;
		border-color: #4c86e6;
	}
	.pwd-dialog .pwd-btns {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 14px;
	}
	.pwd-dialog button {
		padding: 6px 16px;
		border: none;
		border-radius: 4px;
		font-size: 13px;
		cursor: pointer;
	}
	.pwd-dialog .pwd-cancel {
		background: #444;
		color: #ccc;
	}
	.pwd-dialog .pwd-cancel:hover {
		background: #555;
	}
	.pwd-dialog .pwd-ok {
		background: #4c86e6;
		color: #fff;
	}
	.pwd-dialog .pwd-ok:hover {
		background: #5a94f0;
	}
	.pwd-msg {
		font-size: 12px;
		margin-top: 8px;
		min-height: 16px;
	}
	.pwd-msg.error { color: #e85959; }
	.pwd-msg.ok { color: #5cb85c; }

	/* Force update modal styles */
	.update-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0,0,0,0.6);
		z-index: 2000;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.update-dialog {
		background: #1e1e1e;
		border: 1px solid #444;
		border-radius: 8px;
		padding: 24px;
		min-width: 400px;
		max-width: 600px;
		max-height: 80vh;
		color: #eee;
		font-family: system-ui, sans-serif;
		display: flex;
		flex-direction: column;
	}
	.update-dialog h3 {
		margin: 0 0 16px;
		font-size: 15px;
		font-weight: 600;
	}
	.update-info {
		margin-bottom: 16px;
		font-size: 13px;
	}
	.update-info p {
		margin: 8px 0;
	}
	.update-info strong {
		color: #4c86e6;
	}
	.update-warning {
		color: #f59e0b;
		margin-top: 12px !important;
	}
	.update-option {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 12px;
		color: #ddd;
		user-select: none;
	}
	.update-option input {
		width: 16px;
		height: 16px;
	}
	.update-ok-msg {
		color: #10b981;
	}
	.update-error {
		color: #ef4444;
	}
	.update-progress {
		margin-bottom: 16px;
	}
	.progress-bar {
		width: 100%;
		height: 8px;
		background: #2a2a2a;
		border-radius: 4px;
		overflow: hidden;
		margin-bottom: 8px;
	}
	.progress-fill {
		height: 100%;
		background: #4c86e6;
		transition: width 0.3s ease;
	}
	.progress-text {
		font-size: 12px;
		color: #999;
		text-align: center;
	}
	.update-logs {
		max-height: 300px;
		overflow-y: auto;
		background: #0a0a0a;
		border: 1px solid #333;
		border-radius: 4px;
		padding: 12px;
		margin-bottom: 16px;
		font-family: 'Courier New', monospace;
		font-size: 12px;
	}
	.log-entry {
		margin-bottom: 4px;
		line-height: 1.4;
	}
	.log-info { color: #94a3b8; }
	.log-success { color: #10b981; }
	.log-error { color: #ef4444; }
	.log-progress { color: #60a5fa; }
	.update-btns {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}
	.update-dialog button {
		padding: 6px 16px;
		border: none;
		border-radius: 4px;
		font-size: 13px;
		cursor: pointer;
	}
	.update-dialog button:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.update-cancel {
		background: #444;
		color: #ccc;
	}
	.update-cancel:hover:not(:disabled) {
		background: #555;
	}
	.update-ok {
		background: #4c86e6;
		color: #fff;
	}
	.update-ok:hover:not(:disabled) {
		background: #5a94f0;
	}
	.connect-page {
		position: fixed;
		inset: 0;
		background: #1a1a2e;
		color: #fff;
		padding: 20px;
		overflow-y: auto;
		z-index: 3000;
	}
	.connect-header {
		display: flex;
		align-items: center;
		margin-bottom: 20px;
		font-size: 20px;
		font-weight: 600;
	}
	.connect-back {
		margin-right: 12px;
		cursor: pointer;
		font-size: 24px;
	}
	.connection-list {
		max-width: 800px;
	}
	.connection-item {
		background: rgba(255,255,255,0.05);
		border-radius: 8px;
		padding: 16px;
		margin-bottom: 12px;
	}
	.ipv4-display {
		display: inline-flex;
		align-items: center;
		margin-right: 4px;
		padding: 5px 10px;
		border-radius: 4px;
		background: rgba(255, 255, 255, 0.08);
		color: #888;
		font-size: 12px;
		cursor: pointer;
		font-family: monospace;
		flex-shrink: 0;
	}
	.conn-ip {
		font-size: 16px;
		font-weight: 600;
		color: #4c86e6;
		margin-bottom: 8px;
		font-family: monospace;
	}
	.conn-info {
		font-size: 12px;
		color: rgba(255,255,255,0.7);
		margin-bottom: 6px;
		display: flex;
		gap: 12px;
		flex-wrap: wrap;
	}
	.client-features {
		margin-top: 10px;
		padding-top: 10px;
		border-top: 1px solid rgba(255,255,255,0.08);
	}
	.client-features-title {
		margin-bottom: 8px;
		font-size: 12px;
		font-weight: 600;
		color: rgba(255,255,255,0.86);
	}
	.client-features-grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
		gap: 6px 12px;
		font-size: 12px;
		color: rgba(255,255,255,0.72);
	}
	.client-feature-label {
		color: rgba(255,255,255,0.48);
		margin-right: 6px;
	}
	.client-feature-value {
		word-break: break-word;
	}
	`;
  document.head.appendChild(style);
}

function updateIPv4Display(ip) {
	let el = window._ipv4Element;
	if (!el) {
		const connEl = document.getElementById('conn-indicator');
		if (!connEl || !connEl.parentNode) return;
		el = document.createElement('div');
		el.className = 'ipv4-display';
		connEl.parentNode.insertBefore(el, connEl);
		window._ipv4Element = el;
	}
	if (ip && ip !== '--' && ip !== 'null') {
		el.textContent = ip;
		el.style.display = 'inline-flex';
		el.onclick = () => window.open(`https://ping.pe/${ip}`, '_blank');
		window._currentIPv4 = ip;
		console.log('[IPv4] Display updated:', ip);
	} else {
		el.style.display = 'none';
	}
}

async function fetchInitialIPv4() {
	try {
		const resp = await fetch('/api/ipv4');
		if (!resp.ok) {
			console.warn('[IPv4] API returned:', resp.status);
			return;
		}
		const data = await resp.json();
		console.log('[IPv4] API response:', data);
		if (data.ipv4) updateIPv4Display(data.ipv4);
	} catch (e) {
		console.warn('[IPv4] Failed to fetch:', e);
	}
}

function getBasePath() {
	const pathname = window.location.pathname;
	return pathname.slice(0, pathname.lastIndexOf("/") + 1);
}

function showChangePasswordModal() {
	// Remove existing modal if any
	const existing = document.querySelector('.pwd-overlay');
	if (existing) existing.remove();

	const overlay = document.createElement('div');
	overlay.className = 'pwd-overlay';

	const dialog = document.createElement('div');
	dialog.className = 'pwd-dialog';
	dialog.innerHTML = `
		<h3>${tt('pwdHeader')}</h3>
		<input type="password" id="pwd-new" placeholder="${tt('pwdNewPlaceholder')}" autocomplete="new-password" />
		<input type="password" id="pwd-confirm" placeholder="${tt('pwdConfirmPlaceholder')}" autocomplete="new-password" />
		<div class="pwd-msg" id="pwd-msg"></div>
		<div class="pwd-btns">
			<button class="pwd-cancel" id="pwd-cancel">${tt('cancel')}</button>
			<button class="pwd-ok" id="pwd-ok">${tt('confirm')}</button>
		</div>
	`;
	overlay.appendChild(dialog);
	document.body.appendChild(overlay);
	const close = registerTransientOverlay(overlay);

	const newInput = document.getElementById('pwd-new');
	const confirmInput = document.getElementById('pwd-confirm');
	// Allow native keyboard input (bypass VNC key capture)
	newInput.classList.add('allow-native-input');
	confirmInput.classList.add('allow-native-input');
	const msg = document.getElementById('pwd-msg');
	const okBtn = document.getElementById('pwd-ok');
	const cancelBtn = document.getElementById('pwd-cancel');

	newInput.focus();

	overlay.addEventListener('click', (e) => { if (e.target === overlay) close(); });
	cancelBtn.addEventListener('click', close);

	okBtn.addEventListener('click', async () => {
		const np = newInput.value;
		const cp = confirmInput.value;
		msg.className = 'pwd-msg';
		msg.textContent = '';

		if (np.length < 4) {
			msg.className = 'pwd-msg error';
			msg.textContent = tt('pwdTooShort');
			return;
		}
		if (np !== cp) {
			msg.className = 'pwd-msg error';
			msg.textContent = tt('pwdMismatch');
			return;
		}

		okBtn.disabled = true;
		okBtn.textContent = '...';
		try {
			const resp = await fetch('/api/change-password', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ new_password: np }),
			});
			if (resp.ok) {
				msg.className = 'pwd-msg ok';
				msg.textContent = tt('pwdChanged');
				setTimeout(close, 1500);
			} else {
				const data = await resp.json().catch(() => ({}));
				msg.className = 'pwd-msg error';
				msg.textContent = data.error || tt('pwdChangeFailed');
			}
		} catch (e) {
			msg.className = 'pwd-msg error';
			msg.textContent = tt('networkError');
		}
		okBtn.disabled = false;
		okBtn.textContent = tt('confirm');
	});
}

function showForceUpdateModal() {
	// Remove existing modal if any
	const existing = document.querySelector('.update-overlay');
	if (existing) existing.remove();

	const overlay = document.createElement('div');
	overlay.className = 'update-overlay';

	const dialog = document.createElement('div');
	dialog.className = 'update-dialog';
	dialog.innerHTML = `
		<h3>${tt('upgradeHeader')}</h3>
		<div class="update-info" id="update-info">
			<p>${tt('upgradeNotice')}</p>
			<label class="update-option">
				<input type="checkbox" id="upgrade-latest">
				<span>${tt('upgradeOption')}</span>
			</label>
			<p class="update-warning">${tt('upgradeWarning')}</p>
		</div>
		<div class="update-progress" id="update-progress" style="display:none;">
			<div class="progress-bar">
				<div class="progress-fill" id="progress-fill"></div>
			</div>
			<div class="progress-text" id="progress-text">0%</div>
		</div>
		<div class="update-logs" id="update-logs" style="display:none;"></div>
		<div class="update-btns" id="update-btns">
			<button class="update-cancel" id="update-cancel">${tt('cancel')}</button>
			<button class="update-ok" id="update-ok">${tt('restartService')}</button>
		</div>
	`;
	overlay.appendChild(dialog);
	document.body.appendChild(overlay);
	const close = registerTransientOverlay(overlay);

	const infoDiv = document.getElementById('update-info');
	const progressDiv = document.getElementById('update-progress');
	const logsDiv = document.getElementById('update-logs');
	const progressFill = document.getElementById('progress-fill');
	const progressText = document.getElementById('progress-text');
	const btnsDiv = document.getElementById('update-btns');
	const okBtn = document.getElementById('update-ok');
	const cancelBtn = document.getElementById('update-cancel');
	const upgradeCheckbox = document.getElementById('upgrade-latest');

	let isUpdating = false;

	const closeIfIdle = () => {
		if (!isUpdating) close();
	};
	overlay.addEventListener('click', (e) => { if (e.target === overlay && !isUpdating) closeIfIdle(); });
	cancelBtn.addEventListener('click', closeIfIdle);
	upgradeCheckbox.addEventListener('change', () => {
		okBtn.textContent = upgradeCheckbox.checked ? tt('upgradeAndRestart') : tt('restartService');
	});

	okBtn.addEventListener('click', () => {
		isUpdating = true;
		okBtn.disabled = true;
		cancelBtn.disabled = true;
		infoDiv.style.display = 'none';
		progressDiv.style.display = 'block';
		logsDiv.style.display = 'block';
		logsDiv.innerHTML = '';
		progressFill.style.backgroundColor = '#4c86e6';
		progressFill.style.width = '5%';
		progressText.textContent = upgradeCheckbox.checked ? tt('upgradePreparing') : tt('restartPreparing');

		if (!upgradeCheckbox.checked) {
			restartService();
			return;
		}

		// Connect to WebSocket
		const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
		const wsUrl = `${wsProtocol}//${window.location.host}/api/upgrade/ws`;
		const ws = new WebSocket(wsUrl);

		ws.onmessage = (event) => {
			try {
				const entry = JSON.parse(event.data);
				const logEntry = document.createElement('div');
				logEntry.className = `log-entry log-${entry.level}`;
				logEntry.textContent = `[${entry.step}/${entry.total_steps}] ${entry.message}`;
				logsDiv.appendChild(logEntry);
				logsDiv.scrollTop = logsDiv.scrollHeight;

				const progress = Math.round((entry.step / entry.total_steps) * 100);
				progressFill.style.width = progress + '%';
				progressText.textContent = progress + '%';

				if (entry.level === 'error') {
					progressFill.style.backgroundColor = '#ef4444';
					isUpdating = false;
					cancelBtn.disabled = false;
					cancelBtn.textContent = tt('close');
				}
			} catch (err) {
				console.error('Failed to parse upgrade log:', err);
			}
		};

		ws.onclose = () => {
			const hasError = Array.from(logsDiv.querySelectorAll('.log-error')).length > 0;
			if (!hasError) {
				progressFill.style.backgroundColor = '#10b981';
				const logEntry = document.createElement('div');
				logEntry.className = 'log-entry log-success';
				logEntry.textContent = tt('upgradeDoneWaiting');
				logsDiv.appendChild(logEntry);
				logsDiv.scrollTop = logsDiv.scrollHeight;
				waitForRestart();
			} else {
				isUpdating = false;
				cancelBtn.disabled = false;
				cancelBtn.textContent = tt('close');
			}
		};

		ws.onerror = () => {
			const logEntry = document.createElement('div');
			logEntry.className = 'log-entry log-error';
			logEntry.textContent = tt('websocketFailed');
			logsDiv.appendChild(logEntry);
			progressFill.style.backgroundColor = '#ef4444';
			isUpdating = false;
			cancelBtn.disabled = false;
			cancelBtn.textContent = tt('close');
		};
	});

	async function restartService() {
		const logEntry = document.createElement('div');
		logEntry.className = 'log-entry log-info';
		logEntry.textContent = tt('requestingRestart');
		logsDiv.appendChild(logEntry);

		try {
			const resp = await fetch('/api/restart', { method: 'POST' });
			if (!resp.ok) throw new Error(`HTTP ${resp.status}`);

			progressFill.style.width = '50%';
			progressText.textContent = tt('restarting');

			const okEntry = document.createElement('div');
			okEntry.className = 'log-entry log-success';
			okEntry.textContent = tt('restartSent');
			logsDiv.appendChild(okEntry);
			logsDiv.scrollTop = logsDiv.scrollHeight;
			waitForRestart();
		} catch (err) {
			progressFill.style.backgroundColor = '#ef4444';
			progressText.textContent = tt('failed');
			const errEntry = document.createElement('div');
			errEntry.className = 'log-entry log-error';
			errEntry.textContent = tt('restartRequestFailed', err.message || err);
			logsDiv.appendChild(errEntry);
			logsDiv.scrollTop = logsDiv.scrollHeight;
			isUpdating = false;
			cancelBtn.disabled = false;
			cancelBtn.textContent = tt('close');
		}
	}

	function waitForRestart() {
		let attempts = 0;
		const maxAttempts = 30;
		const checkInterval = setInterval(() => {
			attempts++;
			fetch('/health')
				.then(resp => {
					if (resp.ok) {
						clearInterval(checkInterval);
						const logEntry = document.createElement('div');
						logEntry.className = 'log-entry log-success';
						logEntry.textContent = tt('serviceRecovered');
						logsDiv.appendChild(logEntry);
						setTimeout(() => window.location.reload(), 1000);
					}
				})
				.catch(() => {
					// Still waiting
				});

			if (attempts >= maxAttempts) {
				clearInterval(checkInterval);
				const logEntry = document.createElement('div');
				logEntry.className = 'log-entry log-error';
				logEntry.textContent = tt('serviceNotRecovered');
				logsDiv.appendChild(logEntry);
				isUpdating = false;
				cancelBtn.disabled = false;
				cancelBtn.textContent = tt('close');
			}
		}, 1000);
	}
}

function clampNumber(value, min, max) {
	return Math.max(min, Math.min(max, value));
}

function updateOverlayModalScale() {
	// Browser controls use CSS pixels, independently of the remote video. CSS
	// transforms also break xterm's cell-based pointer/selection coordinates.
	const scale = 1;
	const horizontalMargin = 48 * scale;
	const verticalMargin = 96 * scale;
	const terminalWidth = Math.min(920, Math.max(320, (window.innerWidth - horizontalMargin) / scale));
	const terminalHeight = Math.min(560, Math.max(240, (window.innerHeight - verticalMargin) / scale));
	const consoleWidth = Math.min(1080, Math.max(360, (window.innerWidth - horizontalMargin) / scale));
	const consoleHeight = Math.min(720, Math.max(300, (window.innerHeight - verticalMargin) / scale));
	const rootStyle = document.documentElement.style;
	rootStyle.setProperty('--ivnc-modal-scale', scale.toFixed(3));
	rootStyle.setProperty('--ivnc-terminal-width', `${terminalWidth.toFixed(1)}px`);
	rootStyle.setProperty('--ivnc-terminal-height', `${terminalHeight.toFixed(1)}px`);
	rootStyle.setProperty('--ivnc-console-width', `${consoleWidth.toFixed(1)}px`);
	rootStyle.setProperty('--ivnc-console-height', `${consoleHeight.toFixed(1)}px`);
	window.dispatchEvent(new Event('ivnc-overlay-layout'));
}

function installOverlayModalInputGuard() {
    installOverlayGuard(updateOverlayModalScale);
}

function registerOverlayController(controller) {
	installOverlayModalInputGuard();
	if (!window.__ivncOverlayControllers) window.__ivncOverlayControllers = [];
	window.__ivncOverlayControllers.push(controller);
	return controller;
}

function unregisterOverlayController(controller) {
	const controllers = window.__ivncOverlayControllers;
	if (!controllers) return;
	const index = controllers.indexOf(controller);
	if (index >= 0) controllers.splice(index, 1);
}

function minimizeOverlayControllersExcept(activeController = null) {
    minimizeOtherOverlays(activeController);
}

function registerTransientOverlay(overlay) {
	let controller = null;
	const close = () => {
		overlay.remove();
		unregisterOverlayController(controller);
	};
	controller = registerOverlayController({
		minimize: close,
		isOpen: () => overlay.isConnected,
		contains: target => overlay.contains(target),
	});
	minimizeOverlayControllersExcept(controller);
	releaseOverlayInput();
	return close;
}

function createWebTerminalController() {
	installOverlayModalInputGuard();
	let modal = null;
	let terminal = null;
	let fitAddon = null;
	let socket = null;
	let resizeObserver = null;
	let resizeTimer = null;
	let floating = null;
	let lastResize = null;
	let terminalBtn = null;
	let intentionalRestart = false;
	let destroyed = false;
	let reconnectTimer = null;
	let reconnectAttempt = 0;
	let socketGeneration = 0;
	let controller = null;
	const backgroundStorageKey = 'ivnc.terminal.background';
	let backgroundMode = readBackgroundMode();
	const unsubscribeTerminalLanguage = onLangChange(renderTerminalActions);
	window.addEventListener('storage', handleBackgroundStorage);

	const terminalSvgSmall = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>`;

	function readBackgroundMode() {
		try {
			return window.localStorage.getItem(backgroundStorageKey) === 'opaque' ? 'opaque' : 'translucent';
		} catch (_) {
			return 'translucent';
		}
	}

	function renderTerminalActions() {
		if (!modal) return;
		modal.dataset.terminalBackground = backgroundMode;
		const translucent = backgroundMode === 'translucent';
		const button = modal.querySelector('#web-terminal-background');
		button.title = `${tt(translucent ? 'terminalBackgroundTranslucent' : 'terminalBackgroundOpaque')} · ${tt(translucent ? 'terminalSwitchToOpaque' : 'terminalSwitchToTranslucent')}`;
		// Keep the accessible name stable while aria-pressed describes the mode.
		button.setAttribute('aria-label', tt('terminalTranslucentBackground'));
		button.setAttribute('aria-pressed', String(translucent));
		const minimizeButton = modal.querySelector('#web-terminal-minimize');
		minimizeButton.title = tt('minimize');
		minimizeButton.setAttribute('aria-label', tt('minimize'));
		const pinButton = modal.querySelector('#web-terminal-pin');
		pinButton.title = tt(floating?.isPinned() ? 'terminalUnpin' : 'terminalPin');
		pinButton.setAttribute('aria-label', tt('terminalPinned'));
		pinButton.setAttribute('aria-pressed', String(!!floating?.isPinned()));
		const restoreButton = modal.querySelector('#web-terminal-restore');
		restoreButton.title = tt('terminalRestore');
		restoreButton.setAttribute('aria-label', tt('terminalRestore'));
	}

	function handleBackgroundStorage(event) {
		if (event.key !== backgroundStorageKey && event.key !== null) return;
		try {
			if (event.storageArea !== window.localStorage) return;
		} catch (_) {
			return;
		}
		backgroundMode = readBackgroundMode();
		renderTerminalActions();
	}

	function setButton(btn) {
		terminalBtn = btn;
	}

	function setStatus(text, isError = false) {
		const status = modal?.querySelector('.web-terminal-status');
		if (!status) return;
		status.textContent = text;
		status.classList.toggle('error', isError);
	}

	function clearReconnectTimer() {
		clearTimeout(reconnectTimer);
		reconnectTimer = null;
	}

	function ensureModal() {
		if (destroyed) return;
		if (modal) return;
		// index.html loads this entry dynamically, so Vite cannot attach its
		// extracted CSS. Install the complete xterm styles before opening it.
		if (!document.getElementById('ivnc-terminal-styles')) {
			const style = document.createElement('style');
			style.id = 'ivnc-terminal-styles';
			style.textContent = terminalStyles;
			document.head.appendChild(style);
		}
		updateOverlayModalScale();

		modal = document.createElement('div');
		modal.className = 'web-terminal-modal minimized';
		modal.innerHTML = `
			<div class="web-terminal-header">
				<div class="web-terminal-title">${terminalSvgSmall}<span>Terminal</span><span class="web-terminal-status">connecting</span></div>
				<div class="web-terminal-actions">
					<button class="web-terminal-action web-terminal-background" id="web-terminal-background" type="button"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true" focusable="false"><path d="M6 4h6v16H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z" fill="currentColor" stroke="none"/><path class="web-terminal-background-solid" d="M12 4h6a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-6Z" fill="currentColor" stroke="none"/><rect x="4" y="4" width="16" height="16" rx="2"/></svg></button>
					<button class="web-terminal-action web-terminal-pin" id="web-terminal-pin" type="button"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M9 3h6l-1 6 4 4v2H6v-2l4-4-1-6Z"/><path d="M12 15v7"/></svg></button>
					<button class="web-terminal-action" id="web-terminal-restore" type="button"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg></button>
					<button class="web-terminal-action" id="web-terminal-minimize" type="button" title="${tt('minimize')}">_</button>
				</div>
			</div>
			<div class="web-terminal-body" id="web-terminal-body"></div>
		`;
		renderTerminalActions();
		document.body.appendChild(modal);
		modal.querySelector('#web-terminal-pin').addEventListener('click', event => {
			event.stopPropagation();
			floating.setPinned(!floating.isPinned()); renderTerminalActions();
			if (event.detail > 0) terminal?.focus();
		});
		modal.querySelector('#web-terminal-restore').addEventListener('click', event => {
			event.stopPropagation();
			floating.resetGeometry();
			if (event.detail > 0) terminal?.focus();
		});
		modal.querySelector('#web-terminal-background').addEventListener('click', (event) => {
			event.stopPropagation();
			backgroundMode = backgroundMode === 'translucent' ? 'opaque' : 'translucent';
			try {
				window.localStorage.setItem(backgroundStorageKey, backgroundMode);
			} catch (_) {
				// Keep the in-memory choice when browser storage is unavailable.
			}
			renderTerminalActions();
			// Mouse users can keep typing; keyboard users retain the button focus.
			if (event.detail > 0) terminal?.focus();
		});
		modal.querySelector('.web-terminal-actions').addEventListener('keydown', (event) => {
			event.stopPropagation();
		});
		modal.querySelector('#web-terminal-minimize').addEventListener('click', (e) => {
			e.stopPropagation();
			minimize();
		});

		terminal = new Terminal({
			allowTransparency: true,
			cursorBlink: true,
			convertEol: true,
			screenReaderMode: false,
			fontFamily: '"JetBrains Mono", "Cascadia Mono", "SFMono-Regular", Consolas, monospace',
			fontSize: 13,
			theme: {
				background: 'rgba(5, 6, 7, 0)',
				foreground: '#e5e7eb',
				cursor: '#f4f4f5',
				selectionBackground: '#315a8c',
				black: '#0b0f10',
				red: '#ef4444',
				green: '#22c55e',
				yellow: '#eab308',
				blue: '#3b82f6',
				magenta: '#d946ef',
				cyan: '#06b6d4',
				white: '#e5e7eb',
			},
		});
		fitAddon = new FitAddon();
		terminal.loadAddon(fitAddon);
		terminal.open(modal.querySelector('#web-terminal-body'));
		floating = createFloatingWindow({
			modal,
			getScale: () => Number(getComputedStyle(document.documentElement).getPropertyValue('--ivnc-modal-scale')) || 1,
			getBounds: () => {
				const area = document.querySelector('.video-container')?.getBoundingClientRect();
				return { left: Math.max(0, area?.left || 0) + 8, top: Math.max(0, area?.top || 0) + 8,
					right: Math.min(innerWidth, area?.right || innerWidth) - 8, bottom: Math.min(innerHeight, area?.bottom || innerHeight) - 8 };
			},
			getLimits: () => {
				const screen = modal.querySelector('.xterm-screen');
				const cellWidth = screen?.clientWidth / terminal.cols || 8;
				const cellHeight = screen?.clientHeight / terminal.rows || 17;
				return { maxWidth: cellWidth * 400 + 18, maxHeight: cellHeight * 160 + 52 };
			},
			onResize: () => { clearTimeout(resizeTimer); resizeTimer = null; fitAndResize(); },
			onInteraction: () => { releaseOverlayInput(); terminal?.focus(); },
		});
		renderTerminalActions();
		modal.addEventListener('pointerdown', (event) => {
			if (!event.target.closest('.web-terminal-actions')) terminal?.focus();
		});
		modal.addEventListener('keydown', handleTerminalShortcut);
		modal.addEventListener('paste', handleTerminalPaste);
		terminal.onData((data) => {
			send({ type: 'input', data });
		});

		resizeObserver = new ResizeObserver(() => queueResize());
		resizeObserver.observe(modal.querySelector('#web-terminal-body'));
		connect();
	}

	async function copySelectionToClipboard() {
		const selection = terminal?.getSelection();
		if (!selection) return;
		try {
			await navigator.clipboard.writeText(selection);
			setStatus('copied');
			setTimeout(() => setStatus(socket?.readyState === WebSocket.OPEN ? 'ready' : 'disconnected', socket?.readyState !== WebSocket.OPEN), 900);
		} catch (err) {
			setStatus('copy failed', true);
			console.warn('Terminal copy failed:', err);
		}
	}

	async function pasteClipboardToTerminal() {
		try {
			const text = await navigator.clipboard.readText();
			if (text) send({ type: 'input', data: text });
		} catch (err) {
			setStatus('paste failed', true);
			console.warn('Terminal paste failed:', err);
		}
	}

	function handleTerminalShortcut(event) {
		const modifier = event.ctrlKey || event.metaKey;
		if (!modifier || !event.shiftKey) return;
		const key = event.key.toLowerCase();
		if (key === 'c') {
			event.preventDefault();
			event.stopPropagation();
			copySelectionToClipboard();
		} else if (key === 'v') {
			event.preventDefault();
			event.stopPropagation();
			pasteClipboardToTerminal();
		}
	}

	function handleTerminalPaste(event) {
		const text = event.clipboardData?.getData('text/plain');
		if (!text) return;
		event.preventDefault();
		event.stopPropagation();
		send({ type: 'input', data: text });
	}

	function show() {
		ensureModal();
		if (!modal) return;
		minimizeOverlayControllersExcept(controller);
		if (!intentionalRestart && (!socket || socket.readyState === WebSocket.CLOSED || socket.readyState === WebSocket.CLOSING)) {
			clearReconnectTimer();
			connect();
		}
		updateOverlayModalScale();
		modal.classList.remove('minimized');
		floating?.layout();
		releaseOverlayInput();
		terminalBtn?.classList.add('active');
		setTimeout(() => {
			fitAndResize();
			terminal?.focus();
		}, 0);
	}

	function minimize(reason = 'manual') {
		if (reason === 'auto' && floating?.isPinned()) return;
		if (!modal) return;
		floating?.cancel();
		modal.classList.add('minimized');
		terminalBtn?.classList.remove('active');
	}

	function toggle() {
		ensureModal();
		if (!modal) return;
		if (modal.classList.contains('minimized')) {
			show();
		} else {
			minimize();
		}
	}

	function connect() {
		if (destroyed) return;
		if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
			return;
		}
		clearReconnectTimer();
		const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
		const generation = ++socketGeneration;
		lastResize = null;
		socket = new WebSocket(`${protocol}//${window.location.host}/terminal/ws`);
		setStatus('connecting');

		socket.onopen = () => {
			if (generation !== socketGeneration || destroyed) return;
			clearReconnectTimer();
			reconnectAttempt = 0;
			setStatus('connected');
			fitAndResize();
		};

		socket.onmessage = (event) => {
			if (generation !== socketGeneration || destroyed) return;
			let message;
			try {
				message = JSON.parse(event.data);
			} catch (err) {
				console.warn('Invalid terminal message:', err);
				return;
			}
			handleServerMessage(message);
		};

		socket.onerror = () => {
			if (generation !== socketGeneration || destroyed) return;
			setStatus('connection error', true);
		};

		socket.onclose = () => {
			if (generation !== socketGeneration || destroyed) return;
			if (socket && generation === socketGeneration) {
				socket = null;
			}
			if (intentionalRestart) return;
			setStatus('disconnected', true);
			scheduleReconnect();
		};
	}

	function scheduleReconnect() {
		if (destroyed || !modal || reconnectTimer) return;
		const delays = [300, 800, 1500, 3000, 5000, 8000];
		const delay = delays[Math.min(reconnectAttempt, delays.length - 1)];
		reconnectAttempt += 1;
		const seconds = Math.max(1, Math.ceil(delay / 1000));
		setStatus(`reconnecting in ${seconds}s`, true);
		reconnectTimer = setTimeout(() => {
			reconnectTimer = null;
			if (destroyed || !modal) return;
			connect();
		}, delay);
	}

	function handleServerMessage(message) {
		switch (message.type) {
			case 'ready':
				setStatus(message.shell || 'ready');
				break;
			case 'output':
				terminal?.write(Uint8Array.from(atob(message.data || ''), c => c.charCodeAt(0)));
				break;
			case 'exit':
				terminal?.writeln('');
				terminal?.writeln('[terminal exited, starting a new shell]');
				restart();
				break;
			case 'error':
				setStatus('error', true);
				terminal?.writeln(`\r\n[terminal error] ${message.message || 'unknown error'}`);
				break;
			case 'pong':
				break;
			default:
				console.warn('Unhandled terminal message:', message);
		}
	}

	function restart() {
		intentionalRestart = true;
		clearReconnectTimer();
		try {
			socket?.close();
		} catch (_) {}
		socket = null;
		socketGeneration += 1;
		setStatus('restarting');
		show();
		setTimeout(() => {
			if (destroyed) return;
			intentionalRestart = false;
			connect();
		}, 250);
	}

	function send(message) {
		if (!socket || socket.readyState !== WebSocket.OPEN) return;
		socket.send(JSON.stringify(message));
	}

	function fitAndResize() {
		resizeTimer = null;
		if (!terminal || !fitAddon || !modal || modal.classList.contains('minimized')) return;
		try {
			const dims = fitAddon.proposeDimensions();
			if (dims && dims.cols > 0 && dims.rows > 0) {
				const cols = clampNumber(dims.cols, 20, 400), rows = clampNumber(dims.rows, 5, 160);
				if (cols === dims.cols && rows === dims.rows) fitAddon.fit();
				else if (terminal.cols !== cols || terminal.rows !== rows) { terminal.resize(cols, rows); terminal.refresh(0, rows - 1); }
				const size = `${cols}:${rows}`;
				if (socket?.readyState === WebSocket.OPEN && lastResize !== size) {
					send({ type: 'resize', cols, rows }); lastResize = size;
				}
			}
		} catch (err) {
			console.warn('Terminal fit failed:', err);
		}
	}

	function queueResize() {
		if (!resizeTimer) resizeTimer = setTimeout(fitAndResize, 60);
	}

	function destroy() {
		if (destroyed) return;
		destroyed = true;
		clearTimeout(resizeTimer);
		clearReconnectTimer();
		resizeObserver?.disconnect();
		floating?.destroy();
		try {
			socket?.close();
		} catch (_) {}
		terminal?.dispose();
		modal?.remove();
		unsubscribeTerminalLanguage();
		window.removeEventListener('storage', handleBackgroundStorage);
		window.removeEventListener('beforeunload', destroy);
		unregisterOverlayController(controller);
	}

	window.addEventListener('beforeunload', destroy);

	function isOpen() {
		return !!modal && !modal.classList.contains('minimized');
	}

	function contains(target) {
		return !!modal && modal.contains(target);
	}

	controller = registerOverlayController({ setButton, toggle, show, minimize, destroy, isOpen, contains, isPinned: () => !!floating?.isPinned() });
	return controller;
}

function createConsoleModalController() {
	installOverlayModalInputGuard();
	let modal = null;
	let frame = null;
	let consoleBtn = null;
	let destroyed = false;
	let controller = null;
	let childBridge = null;

	const consoleSvgSmall = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z"/><path d="M19.4 15a1.8 1.8 0 0 0 .36 1.98l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06A1.8 1.8 0 0 0 15 19.4a1.8 1.8 0 0 0-1 .6 1.8 1.8 0 0 0-.5 1.3V21a2 2 0 1 1-4 0v-.1A1.8 1.8 0 0 0 8.5 19.4a1.8 1.8 0 0 0-1.98.36l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.8 1.8 0 0 0 4.6 15a1.8 1.8 0 0 0-.6-1 1.8 1.8 0 0 0-1.3-.5H2.6a2 2 0 1 1 0-4h.1A1.8 1.8 0 0 0 4.6 8.5a1.8 1.8 0 0 0-.36-1.98l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.8 1.8 0 0 0 9 4.6a1.8 1.8 0 0 0 1-.6 1.8 1.8 0 0 0 .5-1.3V2.6a2 2 0 1 1 4 0v.1A1.8 1.8 0 0 0 15.5 4.6a1.8 1.8 0 0 0 1.98-.36l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.8 1.8 0 0 0 19.4 9c.2.37.57.6 1 .6h.1a2 2 0 1 1 0 4h-.1a1.8 1.8 0 0 0-1 .5z"/></svg>`;

	function setButton(btn) {
		consoleBtn = btn;
	}

	function ensureModal() {
		if (destroyed) return;
		if (modal) return;
		updateOverlayModalScale();

		modal = document.createElement('div');
		modal.className = 'web-terminal-modal web-console-modal minimized';
		modal.innerHTML = `
			<div class="web-terminal-header">
				<div class="web-terminal-title">${consoleSvgSmall}<span>${tt('consoleTitle')}</span></div>
				<div class="web-terminal-actions">
					<button class="web-terminal-action" id="web-console-minimize" type="button" title="${tt('minimize')}">_</button>
				</div>
			</div>
			<div class="web-terminal-body web-console-body">
				<iframe class="web-console-frame" title="${tt('consoleFrameTitle')}"></iframe>
			</div>
		`;
		document.body.appendChild(modal);
		frame = modal.querySelector('.web-console-frame');
		childBridge = createConsoleShield(modal, frame);
		setLiveText(modal.querySelector('.web-terminal-title span'), 'consoleTitle');
		setLiveTitle(modal.querySelector('#web-console-minimize'), 'minimize');
		setLiveTitle(frame, 'consoleFrameTitle');
		frame.src = `${getBasePath()}console`;
		modal.querySelector('#web-console-minimize').addEventListener('click', (e) => {
			e.stopPropagation();
			minimize();
		});
	}

	function show() {
		ensureModal();
		if (!modal) return;
		minimizeOverlayControllersExcept(controller);
		updateOverlayModalScale();
		modal.classList.remove('minimized');
		childBridge?.syncChild();
		releaseOverlayInput();
		consoleBtn?.classList.add('active');
	}

	function minimize() {
		if (!modal) return;
		modal.classList.add('minimized');
		childBridge?.syncChild();
		consoleBtn?.classList.remove('active');
	}

	function toggle() {
		ensureModal();
		if (!modal) return;
		if (modal.classList.contains('minimized')) {
			show();
		} else {
			minimize();
		}
	}

	function destroy() {
		if (destroyed) return;
		destroyed = true;
		childBridge?.destroy();
		unregisterOverlayController(controller);
		frame?.removeAttribute('src');
		modal?.remove();
		frame = null;
		modal = null;
	}

	function isOpen() {
		return !!modal && !modal.classList.contains('minimized');
	}

	function contains(target) {
		return !!modal && modal.contains(target);
	}

	controller = registerOverlayController({ setButton, toggle, show, minimize, destroy, isOpen, contains,
		childState: () => childBridge?.childState(), syncChild: source => childBridge?.syncChild(source) });
	return controller;
}

function createProxyModalController() {
	installOverlayModalInputGuard();
	let modal = null;
	let frame = null;
	let proxyBtn = null;
	let destroyed = false;
	let controller = null;

	const proxySvgSmall = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a7.8 7.8 0 0 0 0-6"/><path d="M4.6 9a7.8 7.8 0 0 0 0 6"/><path d="M16.2 4.9a12 12 0 0 1 0 14.2"/><path d="M7.8 19.1a12 12 0 0 1 0-14.2"/></svg>`;

	function setButton(btn) {
		proxyBtn = btn;
	}

	function ensureModal() {
		if (destroyed) return;
		if (modal) return;
		updateOverlayModalScale();

		modal = document.createElement('div');
		modal.className = 'web-terminal-modal web-console-modal minimized';
		modal.innerHTML = `
			<div class="web-terminal-header">
				<div class="web-terminal-title">${proxySvgSmall}<span>${tt('proxyTitle')}</span></div>
				<div class="web-terminal-actions">
					<button class="web-terminal-action" id="web-proxy-minimize" type="button" title="${tt('minimize')}">_</button>
				</div>
			</div>
			<div class="web-terminal-body web-console-body">
				<iframe class="web-console-frame" title="${tt('proxyFrameTitle')}"></iframe>
			</div>
		`;
		document.body.appendChild(modal);
		frame = modal.querySelector('.web-console-frame');
		setLiveText(modal.querySelector('.web-terminal-title span'), 'proxyTitle');
		setLiveTitle(modal.querySelector('#web-proxy-minimize'), 'minimize');
		setLiveTitle(frame, 'proxyFrameTitle');
		frame.src = `${getBasePath()}proxy/`;
		modal.querySelector('#web-proxy-minimize').addEventListener('click', (e) => {
			e.stopPropagation();
			minimize();
		});
	}

	function show() {
		ensureModal();
		if (!modal) return;
		minimizeOverlayControllersExcept(controller);
		updateOverlayModalScale();
		modal.classList.remove('minimized');
		releaseOverlayInput();
		proxyBtn?.classList.add('active');
	}

	function minimize() {
		if (!modal) return;
		modal.classList.add('minimized');
		proxyBtn?.classList.remove('active');
	}

	function toggle() {
		ensureModal();
		if (!modal) return;
		if (modal.classList.contains('minimized')) {
			show();
		} else {
			minimize();
		}
	}

	function destroy() {
		if (destroyed) return;
		destroyed = true;
		unregisterOverlayController(controller);
		frame?.removeAttribute('src');
		modal?.remove();
		frame = null;
		modal = null;
	}

	function isOpen() {
		return !!modal && !modal.classList.contains('minimized');
	}

	function contains(target) {
		return !!modal && modal.contains(target);
	}

	controller = registerOverlayController({ setButton, toggle, show, minimize, destroy, isOpen, contains });
	return controller;
}

export default function webrtc() {
	// Connection page functions (must be defined before early return)
	function initConnectionPage() {
		const page = document.createElement('div');
		page.id = 'connect-page';
		page.className = 'connect-page';
		document.body.appendChild(page);
		loadConnections();
		setInterval(loadConnections, 3000);
	}

	async function loadConnections() {
		try {
			const resp = await fetch('/api/connections');
			const data = await resp.json();
			renderConnections(data.connections);
		} catch (err) {
			console.error('Failed to load connections:', err);
		}
	}

	function escapeHtml(value) {
		return String(value ?? '').replace(/[&<>"']/g, (ch) => ({
			'&': '&amp;',
			'<': '&lt;',
			'>': '&gt;',
			'"': '&quot;',
			"'": '&#39;'
		}[ch]));
	}

	function formatClientFeature(value) {
		if (value === null || value === undefined || value === '') return tt('clientFeatureUnknown');
		if (typeof value === 'boolean') return value ? 'true' : 'false';
		if (Array.isArray(value)) return value.length ? value.join(', ') : tt('clientFeatureUnknown');
		return String(value);
	}

	function renderClientFeatures(info) {
		if (!info || typeof info !== 'object') return '';
		const viewport = info.viewport ? `${info.viewport.width || 0}x${info.viewport.height || 0}` : '';
		const screen = info.screen ? `${info.screen.width || 0}x${info.screen.height || 0}` : '';
		const network = info.network ? [
			info.network.effectiveType,
			info.network.downlink ? `${info.network.downlink}Mbps` : '',
			info.network.rtt ? `${info.network.rtt}ms` : ''
		].filter(Boolean).join(' / ') : '';
		const webgl = info.webgl ? [info.webgl.vendor, info.webgl.renderer].filter(Boolean).join(' / ') : '';
		const fields = [
			[tt('clientFeatureBrowser'), info.browser],
			[tt('clientFeaturePlatform'), info.platform],
			[tt('clientFeatureLanguage'), info.languages || info.language],
			[tt('clientFeatureViewport'), viewport],
			[tt('clientFeatureScreen'), screen],
			[tt('clientFeatureDpr'), info.devicePixelRatio],
			[tt('clientFeatureCpu'), info.hardwareConcurrency],
			[tt('clientFeatureMemory'), info.deviceMemory ? `${info.deviceMemory}GB` : ''],
			[tt('clientFeatureTouch'), info.maxTouchPoints],
			[tt('clientFeatureTimezone'), info.timezone],
			[tt('clientFeatureNetwork'), network],
			[tt('clientFeatureOnline'), info.online],
			[tt('clientFeatureWebgl'), webgl]
		];
		return `
			<div class="client-features">
				<div class="client-features-title">${tt('clientFeatures')}</div>
				<div class="client-features-grid">
					${fields.map(([label, value]) => `
						<div>
							<span class="client-feature-label">${escapeHtml(label)}:</span>
							<span class="client-feature-value">${escapeHtml(formatClientFeature(value))}</span>
						</div>
					`).join('')}
				</div>
			</div>
		`;
	}

	function renderConnections(connections) {
		const page = document.getElementById('connect-page');
		if (!page) return;

		const now = Math.floor(Date.now() / 1000);
		const items = connections.map(c => {
			const duration = now - c.connected_at;
			const hours = Math.floor(duration / 3600);
			const minutes = Math.floor((duration % 3600) / 60);
			const durationText = hours > 0 ? tt('durationHM', hours, minutes) : tt('durationM', minutes);
			const connTime = new Date(c.connected_at * 1000).toLocaleString(getLang() === 'zh' ? 'zh-CN' : 'en-US');

			return `
				<div class="connection-item">
					<div class="conn-ip">${c.peer_ip}</div>
					<div class="conn-info">
						<span>${tt('connTime')}: ${connTime}</span>
						<span>${tt('connDuration')}: ${durationText}</span>
						<span>${tt('connType')}: ${c.connection_type.toUpperCase()}</span>
					</div>
					${renderClientFeatures(c.client_info)}
				</div>
			`;
		}).join('');

		page.innerHTML = `
			<div class="connect-header">
				<span class="connect-back" onclick="window.close()">←</span>
				<span>${tt('connectionMgmtTitle')}</span>
			</div>
			<div style="margin-bottom:12px;color:rgba(255,255,255,0.7);">${tt('currentConnectionsCount', connections.length)}</div>
			<div class="connection-list">${items || `<div style="color:rgba(255,255,255,0.5);">${tt('noConnections')}</div>`}</div>
		`;
	}

	// Check if this is the connection management page
	if (window.location.pathname === '/connect') {
		InitUI();
		initConnectionPage();
		return;
	}

	let appName;
	let videoBitRate = 8000;
	let videoFramerate = 60;
	let audioBitRate = 96000;
	let showStart = false;
	let videoPlaybackState = 'connecting';
	let showDrawer = false;
	// TODO: how do we want to handle the log and debug entries
	let logEntries = [];
	let debugEntries = [];
	let status = 'connecting';
	const toolbarFeatures = (window.__IVNC_UI_CONFIG__ && window.__IVNC_UI_CONFIG__.features) || {};
	const toolbarFeatureEnabled = (name) => toolbarFeatures[name] !== false;
	const terminalController = createWebTerminalController();
	const proxyController = createProxyModalController();
	const consoleController = createConsoleModalController();
	let encoderLabel = "";
	let encoder = ""

	let connectionStat = {
		connectionStatType: "unknown",
		connectionLatency: 0,
		connectionVideoLatency: 0,
		connectionAudioLatency: 0,
		connectionAudioCodecName: "NA",
		connectionAudioBitrate: 0,
		connectionPacketsReceived: 0,
		connectionPacketsLost: 0,
		connectionBytesReceived: 0,
		connectionBytesSent: 0,
		connectionCodec: "unknown",
		connectionVideoDecoder: "unknown",
		connectionResolution: "",
		connectionFrameRate: 0,
		connectionVideoBitrate: 0,
		connectionAvailableBandwidth: 0
	};

	var videoElement = null;
	var audioElement = null;
	let appStreams = null;
	let toolbarController = null;
	let gestureController = null;
	let viewportController = null;
	let displayMode = 'fit', displayModeButton = null;
	// An active stream tab covers the main desktop.
	let mainCovered = false;
	let lastTaskbarData = {};
	let serverLatency = 0;
	let scaleLocal = false;
	let debug = false;
	let playButtonElement = null;
	let statusDisplayElement = null;
	let resizeTimer = null;
	let lastSentResolution = null;
	window.isManualResolutionMode = false;
	window.fps = 0;

	var videoConnected = "";
	var audioConnected = "";
	var statWatchEnabled = false;
	var statsLoopId = null;
	var metricsLoopId = null;
	var webrtc = null;
	var input = null;
	let useCssScaling = true;
	let networkQualityBadge = null;
	let networkQualityLevel = 'unknown';
	let networkQualityMetrics = null;
	let networkQualityPendingLevel = null;
	let networkQualityPendingCount = 0;
	let networkQualityFailures = 0;
	let networkQualitySamples = [];
	let networkQualityStartedAt = 0;

	const UPLOAD_CHUNK_SIZE = 64 * 1024  - 1; // 64KiB, excluding a byte for prefix

	// Set storage key based on URL; app views share one key instead of one per view token.
	const urlForKey = window.__IVNC_APP_VIEW__ ? `${window.location.origin}/app-view/` : window.location.href.split('#')[0];
	const storageAppName = urlForKey.replace(/[^a-zA-Z0-9.-_]/g, '_');
	const _urlParams = new URLSearchParams(window.location.search);

	const getIntParam = (key, default_value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		const value = window.localStorage.getItem(prefixedKey);
		return (value === null || value === undefined) ? default_value : parseInt(value);
	};
	const setIntParam = (key, value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		if (value === null || value === undefined) {
				window.localStorage.removeItem(prefixedKey);
		} else {
				window.localStorage.setItem(prefixedKey, value.toString());
		}
	};
	const getBoolParam = (key, default_value) => {
		if (_urlParams.has(key)) {
			return _urlParams.get(key).toLowerCase() === 'true';
		}
		const prefixedKey = `${storageAppName}_${key}`;
		const v = window.localStorage.getItem(prefixedKey);
		if (v === null) {
				return default_value;
		}
		return v.toString().toLowerCase() === 'true';
	};
	const setBoolParam = (key, value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		if (value === null || value === undefined) {
				window.localStorage.removeItem(prefixedKey);
		} else {
				window.localStorage.setItem(prefixedKey, value.toString());
		}
	};
	const getStringParam = (key, default_value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		const value = window.localStorage.getItem(prefixedKey);
		return (value === null || value === undefined) ? default_value : value;
	};
	const setStringParam = (key, value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		if (value === null || value === undefined) {
				window.localStorage.removeItem(prefixedKey);
		} else {
				window.localStorage.setItem(prefixedKey, value.toString());
		}
	};

	function formatNetworkMetric(value, suffix, digits = 0) {
		if (value === null || value === undefined || !Number.isFinite(Number(value))) return 'NA';
		return `${Number(value).toFixed(digits)}${suffix}`;
	}

	function renderNetworkQualityBadge() {
		if (!networkQualityBadge) return;
		const key = {
			good: 'networkQualityGood',
			fair: 'networkQualityFair',
			poor: 'networkQualityPoor',
			offline: 'networkQualityOffline',
			unknown: 'networkQualityUnknown'
		}[networkQualityLevel] || 'networkQualityUnknown';
		networkQualityBadge.className = `taskbar-network-quality ${networkQualityLevel}`;
		networkQualityBadge.textContent = tt(key);
		const metrics = networkQualityMetrics;
		if (metrics) {
			networkQualityBadge.title = tt(
				'networkQualityTooltip',
				formatNetworkMetric(metrics.rttMs, 'ms'),
				formatNetworkMetric(metrics.lossRatePct, '%', 1),
				formatNetworkMetric(metrics.fps, ''),
				formatNetworkMetric(metrics.bitrateMbps, 'Mbps', 2),
				formatNetworkMetric(metrics.jitterMs, 'ms'),
				metrics.connectionType || 'NA'
			);
		} else {
			networkQualityBadge.title = tt(key);
		}
		networkQualityBadge.setAttribute('aria-label', `${tt(key)}: ${networkQualityBadge.title}`);
	}

	function setNetworkQuality(level, metrics = null) {
		networkQualityLevel = level || 'unknown';
		networkQualityMetrics = metrics;
		renderNetworkQualityBadge();
	}

	function resetNetworkQuality(level = 'unknown') {
		networkQualityPendingLevel = null;
		networkQualityPendingCount = 0;
		networkQualityFailures = 0;
		networkQualitySamples = [];
		networkQualityStartedAt = 0;
		setNetworkQuality(level, null);
	}

	function classifyNetworkSample(metrics) {
		const rttMs = Number(metrics.rttMs) || 0;
		const lossRatePct = Number(metrics.lossRatePct) || 0;
		const jitterMs = Number(metrics.jitterMs) || 0;
		if (rttMs >= 300 || lossRatePct >= 5 || (rttMs >= 200 && lossRatePct >= 3)) return 'poor';
		if (rttMs >= 120 || lossRatePct >= 1 || (rttMs >= 100 && jitterMs >= 150)) return 'fair';
		return 'good';
	}

	function applyNetworkQualitySample(metrics) {
		networkQualityFailures = 0;
		if (!networkQualityStartedAt) networkQualityStartedAt = Date.now();
		networkQualitySamples.push({
			at: Date.now(),
			received: Math.max(Number(metrics.deltaPacketsReceived) || 0, 0),
			lost: Math.max(Number(metrics.deltaPacketsLost) || 0, 0)
		});
		const cutoff = Date.now() - 6000;
		networkQualitySamples = networkQualitySamples.filter((sample) => sample.at >= cutoff);
		const packets = networkQualitySamples.reduce((acc, sample) => {
			acc.received += sample.received;
			acc.lost += sample.lost;
			return acc;
		}, { received: 0, lost: 0 });
		const totalPackets = packets.received + packets.lost;
		const windowLossRatePct = totalPackets > 0 ? (packets.lost / totalPackets) * 100 : 0;
		const smoothedMetrics = {
			...metrics,
			lossRatePct: windowLossRatePct
		};

		if (Date.now() - networkQualityStartedAt < 3000) {
			setNetworkQuality('unknown', smoothedMetrics);
			return;
		}

		const nextLevel = classifyNetworkSample(smoothedMetrics);
		const rank = { unknown: 0, good: 1, fair: 2, poor: 3, offline: 4 };
		const currentRank = rank[networkQualityLevel] ?? 0;
		const nextRank = rank[nextLevel] ?? 0;

		if (networkQualityLevel === 'unknown') {
			networkQualityPendingLevel = null;
			networkQualityPendingCount = 0;
			setNetworkQuality(nextLevel, smoothedMetrics);
			return;
		}

		if (nextLevel === networkQualityLevel) {
			networkQualityPendingLevel = null;
			networkQualityPendingCount = 0;
			setNetworkQuality(nextLevel, smoothedMetrics);
			return;
		}

		if (nextRank > currentRank) {
			if (networkQualityPendingLevel === nextLevel) {
				networkQualityPendingCount += 1;
			} else {
				networkQualityPendingLevel = nextLevel;
				networkQualityPendingCount = 1;
			}
			if (networkQualityPendingCount >= 2) {
				networkQualityPendingLevel = null;
				networkQualityPendingCount = 0;
				setNetworkQuality(nextLevel, smoothedMetrics);
			} else {
				networkQualityMetrics = smoothedMetrics;
				renderNetworkQualityBadge();
			}
			return;
		}

		if (nextRank < currentRank) {
			if (networkQualityPendingLevel === nextLevel) {
				networkQualityPendingCount += 1;
			} else {
				networkQualityPendingLevel = nextLevel;
				networkQualityPendingCount = 1;
			}
			if (networkQualityPendingCount >= 3) {
				networkQualityPendingLevel = null;
				networkQualityPendingCount = 0;
				setNetworkQuality(nextLevel, smoothedMetrics);
			} else {
				networkQualityMetrics = smoothedMetrics;
				renderNetworkQualityBadge();
			}
		}
	}

	function markNetworkQualitySampleFailed() {
		networkQualityFailures += 1;
		if (networkQualityFailures >= 3) {
			resetNetworkQuality('offline');
		}
	}

	onLangChange(renderNetworkQualityBadge);

	// Function to add timestamp to logs.
	var applyTimestamp = (msg) => {
		var now = new Date();
		var ts = now.getHours() + ":" + now.getMinutes() + ":" + now.getSeconds();
		return "[" + ts + "]" + " " + msg;
	}

	function getWebglClientInfo() {
		try {
			const canvas = document.createElement('canvas');
			const gl = canvas.getContext('webgl') || canvas.getContext('experimental-webgl');
			if (!gl) return null;
			const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
			return {
				vendor: debugInfo ? gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
				renderer: debugInfo ? gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER)
			};
		} catch (_) {
			return null;
		}
	}

	async function collectClientBrowserFeatures() {
		const connection = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
		let browser = navigator.userAgent;
		let uaHighEntropy = null;
		if (navigator.userAgentData) {
			const brands = (navigator.userAgentData.brands || []).map((brand) => `${brand.brand} ${brand.version}`).join(', ');
			browser = brands || browser;
			try {
				uaHighEntropy = await navigator.userAgentData.getHighEntropyValues(['architecture', 'bitness', 'model', 'platformVersion', 'uaFullVersion']);
			} catch (_) {
				uaHighEntropy = null;
			}
		}
		return {
			browser: String(browser || '').slice(0, 240),
			userAgent: String(navigator.userAgent || '').slice(0, 360),
			platform: navigator.userAgentData?.platform || navigator.platform || '',
			language: navigator.language || '',
			languages: Array.isArray(navigator.languages) ? navigator.languages.slice(0, 6) : [],
			timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || '',
			online: navigator.onLine,
			cookieEnabled: navigator.cookieEnabled,
			hardwareConcurrency: navigator.hardwareConcurrency || null,
			deviceMemory: navigator.deviceMemory || null,
			maxTouchPoints: navigator.maxTouchPoints || 0,
			devicePixelRatio: window.devicePixelRatio || 1,
			viewport: {
				width: window.innerWidth,
				height: window.innerHeight
			},
			screen: {
				width: window.screen?.width || 0,
				height: window.screen?.height || 0,
				availWidth: window.screen?.availWidth || 0,
				availHeight: window.screen?.availHeight || 0,
				colorDepth: window.screen?.colorDepth || 0
			},
			network: connection ? {
				effectiveType: connection.effectiveType || '',
				downlink: connection.downlink || null,
				rtt: connection.rtt || null,
				saveData: Boolean(connection.saveData)
			} : null,
			webgl: getWebglClientInfo(),
			uaHighEntropy
		};
	}

	async function sendClientBrowserFeatures() {
		if (!webrtc || !webrtc._send_channel || webrtc._send_channel.readyState !== 'open') return;
		try {
			const features = await collectClientBrowserFeatures();
			webrtc.sendDataChannelMessage(`_client_info,${JSON.stringify(features)}`);
		} catch (err) {
			console.warn("Failed to send client browser features:", err);
		}
	}

	let clientActivityLoopId = null;
	let lastClientActivityInputAt = 0;

	function sendClientActivity(event = 'heartbeat', input = false) {
		if (!webrtc || !webrtc._send_channel || webrtc._send_channel.readyState !== 'open') return;
		const payload = {
			event,
			// Hidden stream tab (embedded view), or main desktop covered by an
			// active stream tab: the compositor behind it may idle.
			visible: document.visibilityState !== 'hidden' && window.__ivncViewActive !== false && !appStreams?.activeId(),
			focused: document.hasFocus(),
			input: Boolean(input),
			ts: Date.now(),
		};
		if (input) {
			lastClientActivityInputAt = payload.ts;
		}
		try {
			webrtc.sendDataChannelMessage(`_client_activity,${JSON.stringify(payload)}`);
		} catch (err) {
			console.warn("Failed to send client activity:", err);
		}
	}

	function sendClientInputActivity(event = 'input') {
		const now = Date.now();
		if (now - lastClientActivityInputAt < 1000) return;
		sendClientActivity(event, true);
	}

	function startClientActivityReporting() {
		sendClientActivity('open');
		if (clientActivityLoopId) {
			clearInterval(clientActivityLoopId);
		}
		clientActivityLoopId = setInterval(() => {
			sendClientActivity('heartbeat');
		}, 10000);
	}

	function stopClientActivityReporting() {
		if (clientActivityLoopId) {
			clearInterval(clientActivityLoopId);
			clientActivityLoopId = null;
		}
	}

	window.addEventListener('focus', () => sendClientActivity('focus'));
	window.addEventListener('blur', () => sendClientActivity('blur'));
	window.addEventListener('beforeunload', () => sendClientActivity('beforeunload'));
	document.addEventListener('visibilitychange', () => sendClientActivity('visibilitychange'));
	['pointerdown', 'keydown', 'wheel', 'touchstart'].forEach((eventName) => {
		window.addEventListener(eventName, () => sendClientInputActivity(eventName), {
			capture: true,
			passive: true,
		});
	});

	const roundDownToEven = (num) => {
		return Math.floor(num / 2) * 2;
	};

	function playStream() {
		showStart = false;
		if (playButtonElement) playButtonElement.classList.add('hidden');
		if (['no-data', 'decode-error', 'connection-error'].includes(videoPlaybackState)) webrtc.reset();
		else webrtc.playStream();
		webrtc.unmuteAudio();
	}

	function updateStatusDisplay() {
        syncDisplayModeButton();
		const key = { connecting: 'videoConnecting', waiting: 'videoWaiting', blocked: 'videoBlocked',
			'no-data': 'videoNoData', 'decode-error': 'videoDecodeError', 'playback-error': 'videoPlaybackError',
			'connection-error': 'videoConnectionError', unsupported: 'videoUnsupported' }[videoPlaybackState];
		if (statusDisplayElement) {
			statusDisplayElement.textContent = key ? tt(key) : '';
			statusDisplayElement.classList.toggle('hidden', mainCovered || videoPlaybackState === 'playing');
		}
		if (playButtonElement) {
			playButtonElement.classList.toggle('hidden', mainCovered || !showStart);
			playButtonElement.textContent = tt(videoPlaybackState === 'blocked' ? 'videoPlay' : 'videoRetry');
		}
	}
	const stopPlaybackLanguageUpdates = onLangChange(updateStatusDisplay);

	function updateVideoImageRendering() {
        if (!videoElement?.videoWidth) return;
        const rect = videoElement.getBoundingClientRect();
        const scale = rect.width * (window.devicePixelRatio || 1) / videoElement.videoWidth;
        videoElement.style.imageRendering = scale >= 1 && Math.abs(scale - Math.round(scale)) < 0.01 ? 'pixelated' : 'auto';
    }

	function sanitizeAndStoreSettings(serverSettings) {
		console.log("Sanitizing and storing settings based on server payload.");
		const changes = {};

		for (const key in serverSettings) {
            if (["is_manual_resolution_mode", "manual_width", "manual_height"].includes(key)) continue;
			if (!serverSettings.hasOwnProperty(key)) continue;
			const setting = serverSettings[key];
			let sanitizedValue;
			if (setting.min !== undefined && setting.max !== undefined) {
				const clientValue = getIntParam(key, setting.default);
				if (clientValue < setting.min || clientValue > setting.max) {
					sanitizedValue = setting.default;
					console.log(`Sanitizing '${key}': value ${clientValue} is out of range [${setting.min}-${setting.max}]. Resetting to default ${sanitizedValue}.`);
					changes[key] = sanitizedValue;
				} else {
					sanitizedValue = clientValue;
				}
				window[key] = sanitizedValue;
				setIntParam(key, sanitizedValue);
			}
			else if (setting.allowed !== undefined) {
				const isNumericEnum = !isNaN(parseFloat(setting.allowed[0]));
				let clientValueStr;

				if (isNumericEnum) {
					clientValueStr = getIntParam(key, parseInt(setting.value, 10)).toString();
				} else {
					clientValueStr = getStringParam(key, setting.value);
				}

				if (!setting.allowed.includes(clientValueStr)) {
					sanitizedValue = setting.value;
					console.log(`Sanitizing '${key}': value "${clientValueStr}" is not in allowed list [${setting.allowed.join(', ')}]. Resetting to default "${sanitizedValue}".`);
					changes[key] = sanitizedValue;
				} else {
					sanitizedValue = clientValueStr;
				}

				if (isNumericEnum) {
					const numericValue = parseInt(sanitizedValue, 10);
					window[key] = numericValue;
					setIntParam(key, numericValue);
				} else {
					window[key] = sanitizedValue;
					setStringParam(key, sanitizedValue);
				}
			}
			else if (typeof setting.value === 'boolean') {
				const serverValue = setting.value;
				const isLocked = !!setting.locked;
				if (isLocked) {
					const clientValue = getBoolParam(key, !serverValue);
				if (clientValue !== serverValue) {
					console.log(`Sanitizing '${key}': setting is locked by server. Client value ${clientValue} is being overwritten with ${serverValue}.`);
					changes[key] = serverValue;
				}
				window[key] = serverValue;
				setBoolParam(key, serverValue);
				} else {
					const prefixedKey = `${storageAppName}_${key}`;
					const wasUnset = window.localStorage.getItem(prefixedKey) === null;
					const clientValue = getBoolParam(key, serverValue);
					if (wasUnset) {
						console.log(`Initializing unlocked setting '${key}' for the first time with server default: ${serverValue}. Flagging as a change.`);
						changes[key] = serverValue;
					}
					window[key] = clientValue;
					setBoolParam(key, clientValue);
				}
			}
		}
		return changes;
	}

	function sendClientPersistedSettings() {
		const settingsPrefix = `${storageAppName}_`;
		const settingsToSend = {};
		const dpr = useCssScaling ? 1 : (window.devicePixelRatio || 1);

		const knownSettings = [
			'framerate', 'encoder_rtc',
			'audio_bitrate', 'video_bitrate', 'scaling_dpi', 'enable_binary_clipboard'
		];
		const booleanSettingKeys = [
			'enable_binary_clipboard'
		];
		const integerSettingKeys = [
			'framerate', 'audio_bitrate', 'scaling_dpi', 'video_bitrate'
		];

		for (const key in localStorage) {
			if (Object.hasOwnProperty.call(localStorage, key) && key.startsWith(settingsPrefix)) {
				const unprefixedKey = key.substring(settingsPrefix.length);;
				const baseKey = unprefixedKey;
				if (knownSettings.includes(baseKey)) {
					let value = localStorage.getItem(key);
					if (booleanSettingKeys.includes(baseKey)) {
						value = (value === 'true');
					} else if (integerSettingKeys.includes(baseKey)) {
						value = parseInt(value, 10);
						if (isNaN(value)) continue;
					}
					settingsToSend[baseKey] = value;
				}
			}
		}

		try {
			const settingsJson = JSON.stringify(settingsToSend);
			webrtc.sendDataChannelMessage(`SETTINGS,${settingsJson}`);
		
			console.log('Sent initial settings to server:', settingsToSend);
		} catch (e) {
			console.error('Error constructing or sending initial settings:', e);
		}
	}

    function setDisplayMode(value, persist = true) {
        displayMode = value === 'native' ? 'native' : 'fit';
        window.isManualResolutionMode = displayMode === 'native';
        viewportController?.mode(displayMode);
        if (persist) setStringParam('display_mode', displayMode);
        syncDisplayModeButton();
    }

    function syncDisplayModeButton() {
        if (!displayModeButton) return;
        displayModeButton.title = getLang() === 'en' ? (displayMode === 'fit' ? 'Fit window' : 'Original size') : (displayMode === 'fit' ? '适应窗口' : '原始大小');
        displayModeButton.setAttribute('aria-label', displayModeButton.title);
        const label = displayModeButton.querySelector('.toolbar-tool-label');
        if (label) label.textContent = displayModeButton.title;
        displayModeButton.classList.toggle('active', displayMode === 'native');
        displayModeButton.setAttribute('aria-pressed', String(displayMode === 'native'));
    }

    function reportViewport() {
        if (!input || webrtc?._send_channel?.readyState !== 'open') return;
        const [width, height] = input.getWindowResolution();
        if (width < 1 || height < 1) return;
        const report = JSON.stringify({ width, height, dpr: window.devicePixelRatio || 1, mode: displayMode });
        if (report === lastSentResolution) return;
        webrtc.sendDataChannelMessage(`_viewport,${report}`);
        lastSentResolution = report;
    }
    function resizeStart() {
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(reportViewport, 150);
    }
    function loadLastSessionSettings() { setDisplayMode(displayMode, false); reportViewport(); }

	// callback invoked when "message" event is triggerd
	function handleMessage(event) {
		if (event.origin !== window.location.origin) return;
		let message = event.data;
		if (!message) return;
		switch(message.type) {
            case "setDisplayMode":
                setDisplayMode(message.value);
                break;
            case "setScaleLocally":
                if (typeof message.value === 'boolean') {
                    scaleLocal = message.value;
                    setDisplayMode(scaleLocal ? 'fit' : 'native');
                }
                break;
            case "resetResolutionToWindow":
                setDisplayMode('fit');
                break;
            case "setManualResolution":
                // Older dashboards can still select fit/original locally, but
                // only the authenticated console can change desktop dimensions.
                setDisplayMode(scaleLocal ? 'fit' : 'native');
                break;
			case "setUseCssScaling":
				console.warn("Skipping cssScaling since hidpi needs to be implemented")
				break;
			case "clipboardUpdateFromUI":
				console.log("Received clipboard from UI, sending it to server");
				webrtc.sendDataChannelMessage(`cw,${stringToBase64(message.text)}`);
				break;
			case "settings":
				console.log("Received settings msg from dashboard:", message.settings);
				handleSettingsMessage(message.settings);
				break;
			case "command":
				if (message.value !== null && message.value !== undefined) {
					const commandString = message.value;
					console.log(`Received 'command' message with value: "${commandString}"`);
					webrtc.sendDataChannelMessage(`cmd,${commandString}`);
				} else {
					console.warn(`Received invalid command from dashboard: ${message.value}`)
				}
				break;
		}
	}

	function handleSettingsMessage(settings) {
		if (settings.video_bitrate !== undefined) {
			videoBitRate = parseInt(settings.video_bitrate);
			webrtc.sendDataChannelMessage(`vb,${videoBitRate}`);
			setIntParam('video_bitrate', videoBitRate);
		}
		if (settings.framerate !== undefined) {
			videoFramerate = parseInt(settings.framerate);
			webrtc.sendDataChannelMessage(`_arg_fps,${videoFramerate}`);
			setIntParam('framerate', videoFramerate);
		}
		if (settings.audio_bitrate !== undefined) {
			audioBitRate = parseInt(settings.audio_bitrate);
			webrtc.sendDataChannelMessage(`ab,${audioBitRate}`);
			setIntParam('audio_bitrate', audioBitRate);
		}
		if (settings.encoder !== undefined) {
			console.log("Received encoder setting from dashboard:", settings.encoder);
			encoder = settings.encoder;
			console.warn("Changing of encoder on the fly is not yet supported");
			// setIntParam('encoder_rtc', encoder);
		}
		if (settings.SCALING_DPI !== undefined) {
			const dpi = parseInt(settings.SCALING_DPI, 10);
			webrtc.sendDataChannelMessage(`s,${dpi}`)
		}
	}

	function handleRequestFileUpload() {
		const hiddenInput = document.getElementById('globalFileInput');
		if (!hiddenInput) {
			console.error("Global file input not found!");
			return;
		}
		console.log("Triggering click on hidden file input.");
		hiddenInput.click();
	}

	async function handleFileInputChange(event) {
		const files = Array.from(event.target.files || []);
		event.target.value = null;
		if (files.length === 0) return;
		// Files picked while a stream tab is active go to that application.
		if (appStreams && appStreams.upload(files)) return;
		await uploadFiles(files);
	}

	async function uploadFiles(files) {
		// For every user action 'upload' an auxiliary data is dynamically created.
		// Currently only one aux channel is allowed to operate at a given time, since the backend
		// doesn't support simultaneous reception of multiple files, yet.
		if (!webrtc.createAuxDataChannel()) {
			console.warn("Simultaneous uploading of files with distinct upload operations is not supported yet");
			const errorMsg = "Please let the ongoing upload complete";
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'warning',
				fileName: '_N/A_',
				message: errorMsg
				}
			}, window.location.origin);
			return;
		}
		console.log(`File input changed, processing ${files.length} files sequentially.`);
		try {
			await webrtc.waitForAuxChannelOpen();
			for (let i = 0; i < files.length; i++) {
				const file = files[i];
				const pathToSend = file.name;
				console.log(`Uploading file ${i + 1}/${files.length}: ${pathToSend}`);
				await uploadFileObject(file, pathToSend);
			}
			console.log("Finished processing all files from input.");
		} catch (error) {
			const errorMsg = `An error occurred during the file input upload process: ${error.message || error}`;
			console.error(errorMsg);
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'error',
				fileName: 'N/A',
				message: errorMsg
				}
			}, window.location.origin);
		} finally {
			webrtc.closeAuxDataChannel();
		}
	}

	// Upload progress toast management
	let currentUploadToast = null;
	const UPLOAD_DIR = '~/Desktop'; // From config.toml

	function showUploadToast(fileName, fileSize) {
		// Remove existing toast if any
		if (currentUploadToast) {
			currentUploadToast.remove();
		}

		const toast = document.createElement('div');
		toast.className = 'upload-toast';
		toast.innerHTML = `
			<div class="upload-toast-header">
				<div class="upload-toast-title">${tt('uploadFileTitle')}</div>
				<button class="upload-toast-close" onclick="this.parentElement.parentElement.remove()">×</button>
			</div>
			<div class="upload-toast-filename">${fileName}</div>
			<div class="upload-toast-progress">
				<div class="upload-toast-progress-bar" style="width: 0%"></div>
			</div>
			<div class="upload-toast-info">
				<span class="upload-toast-percent">0%</span>
				<span class="upload-toast-size">${formatFileSize(fileSize)}</span>
			</div>
		`;
		document.body.appendChild(toast);
		currentUploadToast = toast;
		return toast;
	}

	function updateUploadProgress(toast, progress, fileName) {
		if (!toast) return;
		const progressBar = toast.querySelector('.upload-toast-progress-bar');
		const percentText = toast.querySelector('.upload-toast-percent');
		if (progressBar) progressBar.style.width = progress + '%';
		if (percentText) percentText.textContent = progress + '%';
	}

	function showUploadSuccess(toast, fileName) {
		if (!toast) return;
		toast.className = 'upload-toast success';
		const uploadPath = UPLOAD_DIR + '/' + fileName;
		toast.innerHTML = `
			<div class="upload-toast-header">
				<div class="upload-toast-title">${tt('uploadSuccess')}</div>
				<button class="upload-toast-close" onclick="this.parentElement.parentElement.remove()">×</button>
			</div>
			<div class="upload-toast-filename">${fileName}</div>
			<div class="upload-toast-path">${tt('uploadSavedTo', uploadPath)}</div>
		`;
		// Auto-remove after 5 seconds
		setTimeout(() => {
			if (toast && toast.parentElement) {
				toast.remove();
				if (currentUploadToast === toast) {
					currentUploadToast = null;
				}
			}
		}, 5000);
	}

	function showUploadError(toast, fileName, errorMessage) {
		if (!toast) {
			toast = document.createElement('div');
			toast.className = 'upload-toast error';
			document.body.appendChild(toast);
		} else {
			toast.className = 'upload-toast error';
		}
		toast.innerHTML = `
			<div class="upload-toast-header">
				<div class="upload-toast-title">${tt('uploadFailed')}</div>
				<button class="upload-toast-close" onclick="this.parentElement.parentElement.remove()">×</button>
			</div>
			<div class="upload-toast-filename">${fileName}</div>
			<div class="upload-toast-error">${errorMessage}</div>
		`;
		// Auto-remove after 8 seconds
		setTimeout(() => {
			if (toast && toast.parentElement) {
				toast.remove();
				if (currentUploadToast === toast) {
					currentUploadToast = null;
				}
			}
		}, 8000);
	}

	function formatFileSize(bytes) {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
	}

	// Listen for upload events
	window.addEventListener('message', (event) => {
		if (event.origin !== window.location.origin) return;
		if (event.data.type !== 'fileUpload') return;

		const { status, fileName, progress, fileSize, message } = event.data.payload;

		if (status === 'start') {
			showUploadToast(fileName, fileSize);
		} else if (status === 'progress') {
			updateUploadProgress(currentUploadToast, progress, fileName);
		} else if (status === 'end') {
			showUploadSuccess(currentUploadToast, fileName);
		} else if (status === 'error') {
			showUploadError(currentUploadToast, fileName, message || tt('unknownError'));
		}
	});

	function uploadFileObject(file, pathToSend) {
		return new Promise((resolve, reject) => {
			// Start a heartbeat to prevent connection timeout during upload
			const heartbeatInterval = setInterval(() => {
				try {
					if (webrtc && webrtc._send_channel && webrtc._send_channel.readyState === 'open') {
						// Send a lightweight heartbeat message
						webrtc.sendDataChannelMessage('pong');
					}
				} catch (err) {
					console.warn('Heartbeat failed:', err);
				}
			}, 5000); // Send heartbeat every 5 seconds

			const cleanup = () => {
				clearInterval(heartbeatInterval);
			};

			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'start',
				fileName: pathToSend,
				fileSize: file.size
				}
			}, window.location.origin);
			webrtc.sendDataChannelMessage(`FILE_UPLOAD_START:${pathToSend}:${file.size}`)

			let offset = 0;
			const reader = new FileReader();
			reader.onload = async function(e) {
				if (e.target.error) {
					const readErrorMsg = `File read error for ${pathToSend}: ${e.target.error}`;
					window.postMessage({ type: 'fileUpload', payload: { status: 'error', fileName: pathToSend, message: readErrorMsg }}, window.location.origin);
					webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:File read error`)
					reject(e.target.error);
					return;
				}
				try {
					const prefixedView = new Uint8Array(1 + e.target.result.byteLength);
					prefixedView[0] = 0x01; // Data prefix for file chunk
					prefixedView.set(new Uint8Array(e.target.result), 1);

					// Check if auxiliary channel is still open before sending
					if (!webrtc._aux_channel || webrtc._aux_channel.readyState !== 'open') {
						throw new Error('Auxiliary data channel closed during upload');
					}

				// Send data and check if it succeeded
				const sendSuccess = webrtc.sendAuxChannelData(prefixedView.buffer);
				if (!sendSuccess) {
					// If send failed (buffer full), retry after a delay
					console.warn(`[Upload] Send failed for ${pathToSend}, retrying chunk at offset ${offset}...`);
					setTimeout(() => reader.readAsArrayBuffer(slice), 100);
					return;
				}

				offset += e.target.result.byteLength;
					const progress = file.size > 0 ? Math.round((offset / file.size) * 100) : 100;
					window.postMessage({
						type: 'fileUpload',
						payload: {
						status: 'progress',
						fileName: pathToSend,
						progress: progress,
						fileSize: file.size
							}
					}, window.location.origin);
					if (offset < file.size) {
						if(webrtc.isAuxBufferNearThreshold()) {
							setTimeout(() => readChunk(offset), 50);
						} else {
							readChunk(offset)
						}
					} else {
						// Data channels work asynchronously due to their underlying implementation,
						// so we need to wait for its buffer to drain before sending the end message.
						try {
						console.log(`[Upload] All chunks sent for ${pathToSend}, waiting for buffer to drain...`);
					console.log(`[Upload] Current buffer amount: ${webrtc._aux_channel ? webrtc._aux_channel.bufferedAmount : 'N/A'} bytes`);
							await webrtc.awaitForAuxBufferToDrain(30000);
						console.log(`[Upload] Buffer drained for ${pathToSend}, sending end message...`);
							// Check if connection is still alive before sending end message
							if (!webrtc || !webrtc._send_channel || webrtc._send_channel.readyState !== 'open') {
								throw new Error('Connection lost during upload');
							}
							webrtc.sendDataChannelMessage(`FILE_UPLOAD_END:${pathToSend}`);
						console.log(`[Upload] End message sent for ${pathToSend}, posting success...`);
							window.postMessage({
							type: 'fileUpload',
							payload: {
								status: 'end',
								fileName: pathToSend,
								fileSize: file.size
							}
							}, window.location.origin);
						console.log(`[Upload] Upload completed successfully for ${pathToSend}`);
				cleanup();
							resolve();
						} catch (endError) {
						console.error(`[Upload] Error completing upload for ${pathToSend}:`, endError);
							const endErrorMsg = `Failed to complete upload of ${pathToSend}: ${endError.message || endError}`;
							window.postMessage({ type: 'fileUpload', payload: { status: 'error', fileName: pathToSend, message: endErrorMsg }}, window.location.origin);
				cleanup();
							reject(endError);
						}
						}
				} catch (error) {
					const sendErrorMsg = `error during upload of ${pathToSend}: ${error.message || error}`;
					window.postMessage({ type: 'fileUpload', payload: { status: 'error', fileName: pathToSend, message: sendErrorMsg }}, window.location.origin);
					webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:send error`);
			cleanup();
					reject(error);
				}
			};
			reader.onerror = function(e) {
				const generalReadError = `General file reader error for ${pathToSend}: ${e.target.error}`;
				window.postMessage({ type: 'fileUpload', payload: { status: 'error', fileName: pathToSend, message: generalReadError }}, window.location.origin);
				webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:General file reader error`)
		cleanup();
				reject(e.target.error);
			};

			function readChunk(startOffset) {
				const slice = file.slice(startOffset, Math.min(startOffset + UPLOAD_CHUNK_SIZE, file.size));
				reader.readAsArrayBuffer(slice);
			}
			readChunk(0);
		});
	}

	function handleDragOver(ev) {
		ev.preventDefault();
		ev.dataTransfer.dropEffect = 'copy';
	}

	async function handleDrop(ev) {
		ev.preventDefault();
		ev.stopPropagation();
		const entriesToProcess = [];
		if (!webrtc.createAuxDataChannel()) {
			console.warn("Simultaneous uploading of files with distinct upload operations is not supported yet");
			const errorMsg = "Please let the ongoing upload complete";
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'warning',
				fileName: '_N/A_',
				message: errorMsg
				}
			}, window.location.origin);
			return;
		}
		if (ev.dataTransfer.items) {
			for (let i = 0; i < ev.dataTransfer.items.length; i++) {
				const item = ev.dataTransfer.items[i];
			  // Only care about file-kind items
				if (item.kind !== 'file') continue;
				let entry = null;
				if (typeof item.webkitGetAsEntry === 'function') entry = item.webkitGetAsEntry();
				else if (typeof item.getAsEntry === 'function') entry = item.getAsEntry();
				if (entry) entriesToProcess.push(entry);
			}
		} else if (ev.dataTransfer.files.length > 0) {
			for (let i = 0; i < ev.dataTransfer.files.length; i++) {
				await uploadFileObject(ev.dataTransfer.files[i], ev.dataTransfer.files[i].name);
			}
			webrtc.closeAuxDataChannel();
			return;
		}

		// Process the nested entries
		try {
			for (const entry of entriesToProcess) await handleDroppedEntry(entry);
		} catch (error) {
			const errorMsg = `Error during sequential upload: ${error.message || error}`;
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'error',
				fileName: 'N/A',
				message: errorMsg
				}
			}, window.location.origin);
			webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:GENERAL:Processing failed`)
		}
		webrtc.closeAuxDataChannel();
	}

	function getFileFromEntry(fileEntry) {
		return new Promise((resolve, reject) => fileEntry.file(resolve, reject));
	}

	async function handleDroppedEntry(entry, basePathFallback = "") { // basePathFallback is for non-fullPath scenarios
		let pathToSend;
		if (entry.fullPath && typeof entry.fullPath === 'string' && entry.fullPath !== entry.name && (entry.fullPath.includes('/') || entry.fullPath.includes('\\'))) {
			pathToSend = entry.fullPath;
			if (pathToSend.startsWith('/')) {
				pathToSend = pathToSend.substring(1);
			}
			console.log(`Using entry.fullPath: "${pathToSend}" for entry.name: "${entry.name}"`);
		} else {
			pathToSend = basePathFallback ? `${basePathFallback}/${entry.name}` : entry.name;
			console.log(`Constructed path: "${pathToSend}" for entry.name: "${entry.name}" (basePathFallback: "${basePathFallback}")`);
		}

		if (entry.isFile) {
			try {
				const file = await getFileFromEntry(entry);
				await uploadFileObject(file, pathToSend);
			} catch (err) {
				console.error(`Error processing file ${pathToSend}: ${err}`);
				window.postMessage({
				type: 'fileUpload',
				payload: { status: 'error', fileName: pathToSend, message: `Error processing file: ${err.message || err}` }
				}, window.location.origin);
				webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:Client-side file processing error`)
			}
		} else if (entry.isDirectory) {
			console.log(`Processing directory: ${pathToSend}`);
			const dirReader = entry.createReader();
			let entries;
			do {
				entries = await new Promise((resolve, reject) => dirReader.readEntries(resolve, reject));
				for (const subEntry of entries) {
					await handleDroppedEntry(subEntry, pathToSend);
				}
			} while (entries.length > 0);
		}
	}

	// TODO: How do we want to render rudimentary metrics?
		function enableStatWatch() {
			// Clear any previous stats loop to prevent timer leaks on reconnect
			if (statsLoopId) {
				clearInterval(statsLoopId);
				statsLoopId = null;
			}
			// Start watching stats
			var videoBytesReceivedStart = 0;
			var audioBytesReceivedStart = 0;
			var previousVideoJitterBufferDelay = 0.0;
			var previousVideoJitterBufferEmittedCount = 0;
			var previousAudioJitterBufferDelay = 0.0;
			var previousAudioJitterBufferEmittedCount = 0;
			var previousPacketsReceived = null;
			var previousPacketsLost = null;
			var previousNetworkVideoBytesReceived = null;
			var statsStart = new Date().getTime() / 1000;
			var lastSessionCount = null;
			var sessionCountPending = false;

			async function refreshSessionCount() {
				if (sessionCountPending || window.__IVNC_EMBED__) return;
				sessionCountPending = true;
				try {
					const resp = await fetch('/clients', { cache: 'no-store' });
					if (resp.ok) {
						const data = await resp.json();
						if (typeof data.webrtc_sessions === 'number') {
							lastSessionCount = data.webrtc_sessions;
						}
					}
				} catch (err) {
					// Ignore fetch errors; keep last known count.
				} finally {
					sessionCountPending = false;
				}
			}
			statsLoopId = setInterval(async () => {
				refreshSessionCount();
				webrtc.getConnectionStats().then((stats) => {
					statWatchEnabled = true;
					var now = new Date().getTime() / 1000;
					const intervalSeconds = Math.max(now - statsStart, 0.001);
					connectionStat = {};

				// Connection latency in milliseconds
				const rtt = (stats.general.currentRoundTripTime !== null) ? (stats.general.currentRoundTripTime * 1000.0) : (serverLatency)

				// Connection stats
				connectionStat.connectionPacketsReceived = stats.general.packetsReceived;
				connectionStat.connectionPacketsLost = stats.general.packetsLost;
				connectionStat.connectionStatType = stats.general.connectionType

				var connEl = document.getElementById('conn-indicator');
					if (connEl) {
						if (lastSessionCount && lastSessionCount > 0) {
							connEl.textContent = String(lastSessionCount);
							connEl.style.display = 'block';
							connEl.style.color = '#4caf50';
							setLiveTitle(connEl, 'connectionsCount', lastSessionCount);
						} else {
							connEl.style.display = 'none';
						}
					}

				connectionStat.connectionBytesReceived = (stats.general.bytesReceived * 1e-6).toFixed(2) + " MBytes";
				connectionStat.connectionBytesSent = (stats.general.bytesSent * 1e-6).toFixed(2) + " MBytes";
				connectionStat.connectionAvailableBandwidth = (parseInt(stats.general.availableReceiveBandwidth) / 1e+6).toFixed(2) + " mbps";

				// Video stats
				connectionStat.connectionCodec = stats.video.codecName;
				connectionStat.connectionVideoDecoder = stats.video.decoder;
				connectionStat.connectionResolution = stats.video.frameWidth + "x" + stats.video.frameHeight;
				connectionStat.connectionFrameRate = stats.video.framesPerSecond;
				connectionStat.connectionVideoBitrate = (((stats.video.bytesReceived - videoBytesReceivedStart) / intervalSeconds) * 8 / 1e+6).toFixed(2);
				videoBytesReceivedStart = stats.video.bytesReceived;

				// Audio stats
				connectionStat.connectionAudioCodecName = stats.audio.codecName;
				connectionStat.connectionAudioBitrate = (((stats.audio.bytesReceived - audioBytesReceivedStart) / intervalSeconds) * 8 / 1e+3).toFixed(2);
				audioBytesReceivedStart = stats.audio.bytesReceived;

				// Latency stats
				const videoJitterCountDelta = stats.video.jitterBufferEmittedCount - previousVideoJitterBufferEmittedCount;
				const videoJitterMs = videoJitterCountDelta > 0 ? (1000.0 * (stats.video.jitterBufferDelay - previousVideoJitterBufferDelay) / videoJitterCountDelta) : 0;
				connectionStat.connectionVideoLatency = parseInt(Math.round(rtt + (videoJitterMs || 0)));
				previousVideoJitterBufferDelay = stats.video.jitterBufferDelay;
				previousVideoJitterBufferEmittedCount = stats.video.jitterBufferEmittedCount;
				connectionStat.connectionAudioLatency = parseInt(Math.round(rtt + (1000.0 * (stats.audio.jitterBufferDelay - previousAudioJitterBufferDelay) / (stats.audio.jitterBufferEmittedCount - previousAudioJitterBufferEmittedCount) || 0)));
				previousAudioJitterBufferDelay = stats.audio.jitterBufferDelay;
				previousAudioJitterBufferEmittedCount = stats.audio.jitterBufferEmittedCount;

				const deltaPacketsReceived = previousPacketsReceived === null ? 0 : Math.max(stats.general.packetsReceived - previousPacketsReceived, 0);
				const deltaPacketsLost = previousPacketsLost === null ? 0 : Math.max(stats.general.packetsLost - previousPacketsLost, 0);
				const deltaPacketsTotal = deltaPacketsReceived + deltaPacketsLost;
				const lossRatePct = deltaPacketsTotal > 0 ? (deltaPacketsLost / deltaPacketsTotal) * 100 : 0;
				const deltaVideoBytes = previousNetworkVideoBytesReceived === null ? 0 : Math.max(stats.video.bytesReceived - previousNetworkVideoBytesReceived, 0);
				const bitrateMbps = deltaVideoBytes * 8 / intervalSeconds / 1e+6;
				applyNetworkQualitySample({
					rttMs: rtt,
					lossRatePct,
					deltaPacketsReceived,
					deltaPacketsLost,
					fps: stats.video.framesPerSecond || 0,
					bitrateMbps,
					jitterMs: Math.max(videoJitterMs || 0, 0),
					connectionType: stats.general.connectionType || 'NA'
				});
				previousPacketsReceived = stats.general.packetsReceived;
				previousPacketsLost = stats.general.packetsLost;
				previousNetworkVideoBytesReceived = stats.video.bytesReceived;

				// Format latency
				connectionStat.connectionLatency =  Math.max(connectionStat.connectionVideoLatency, connectionStat.connectionAudioLatency);

				statsStart = now;
				window.fps = connectionStat.connectionFrameRate

				if (webrtc._send_channel !== null && webrtc._send_channel.readyState === 'open') {
					// Send compact stats summary instead of full allReports
					// (allReports can be 5-15KB, exceeding DTLS/SCTP frame limits)
					var summary = {
						video: {
							bytesReceived: stats.video.bytesReceived,
							packetsReceived: stats.video.packetsReceived,
							packetsLost: stats.video.packetsLost,
							framesPerSecond: stats.video.framesPerSecond,
							frameWidth: stats.video.frameWidth,
							frameHeight: stats.video.frameHeight,
							codecName: stats.video.codecName,
							decoder: stats.video.decoder,
						},
						audio: {
							bytesReceived: stats.audio.bytesReceived,
							packetsReceived: stats.audio.packetsReceived,
							packetsLost: stats.audio.packetsLost,
							codecName: stats.audio.codecName,
						},
						general: {
							bytesReceived: stats.general.bytesReceived,
							bytesSent: stats.general.bytesSent,
							currentRoundTripTime: stats.general.currentRoundTripTime,
							connectionType: stats.general.connectionType,
						}
					};
					webrtc.sendDataChannelMessage(`_stats_video,${JSON.stringify(summary)}`);
				}
			}).catch((err) => {
				markNetworkQualitySampleFailed();
				if (debug) console.warn("[webrtc] Failed to collect connection stats:", err);
			});
		// Stats refresh interval (1000 ms)
		}, 1000);
	}

	let clipboardController = null;
	let clipboardUI = null;
	let clipboardNotice = null, clipboardNoticeTimer = null;
	function reportClipboardError(reason) {
		const zh = getLang() === 'zh';
		const messages = {
			unsupported: zh ? '服务端尚不支持可靠粘贴，请更新服务端后重新连接。' : 'Update the server and reconnect to enable clipboard paste.',
			text_required: zh ? '此次剪贴板没有可粘贴的文本。' : 'The clipboard does not contain text.',
			limit: zh ? '剪贴板内容过大或待处理操作过多。' : 'Clipboard size or pending operation limit exceeded.',
			busy: zh ? '远端正在处理另一次粘贴，请稍后再试。' : 'Another paste is still being delivered. Try again shortly.',
			remote_copy_timeout: zh ? '未收到远端复制文本，请重新复制。' : 'No remote copied text was received. Copy again.',
		};
		const message = messages[reason] || (zh ? '本次粘贴未能确认完成，请检查连接和输入焦点。' : 'Paste could not be confirmed. Check the connection and input focus.');
		if (!clipboardNotice) {
			clipboardNotice = document.createElement('div');
			clipboardNotice.className = 'upload-toast error';
			clipboardNotice.setAttribute('role', 'alert');
			document.body.appendChild(clipboardNotice);
		}
		clipboardNotice.textContent = message;
		clearTimeout(clipboardNoticeTimer);
		clipboardNoticeTimer = setTimeout(() => { clipboardNotice?.remove(); clipboardNotice = null; }, 6000);
	}
	function handleWindowFocus() {
		if (webrtc?._send_channel?.readyState === 'open') webrtc.sendDataChannelMessage('kr');
	}
	function handleWindowBlur() {
		clipboardController?.suspend();
	}

	function setupKeyBoardAssisstant() {
		const keyboardInputAssist = document.getElementById('keyboard-input-assist');
		if (keyboardInputAssist && input) {
			keyboardInputAssist.addEventListener('input', (event) => {
				const typedString = keyboardInputAssist.value;
				if (typedString) {
				input._typeString(typedString);
				keyboardInputAssist.value = '';
				}
			});
		keyboardInputAssist.addEventListener('keydown', (event) => {
			if (event.key === 'Enter' || event.keyCode === 13) {
			const enterKeysym = 0xFF0D;
			input._guac_press(enterKeysym);
			setTimeout(() => input._guac_release(enterKeysym), 5);
			event.preventDefault();
			keyboardInputAssist.value = '';
			} else if (event.key === 'Backspace' || event.keyCode === 8) {
			const backspaceKeysym = 0xFF08;
			input._guac_press(backspaceKeysym);
			setTimeout(() => input._guac_release(backspaceKeysym), 5);
			event.preventDefault();
			}
		});
		console.log("Added 'input' and 'keydown' listeners to #keyboard-input-assist.");
		} else {
		console.error(" Could not add listeners to keyboard assist: Element or Input handler instance not found.");
		}
	}

	return {
		initialize() {
			InitUI();
			// Create the nodes and configure its attributes
			const appDiv = document.getElementById('app');
			let videoContainer = document.createElement("div");
			videoContainer.className = "video-container";

			playButtonElement = document.createElement('button');
			playButtonElement.id = 'playButton';
			playButtonElement.type = 'button';
			playButtonElement.textContent = tt('videoPlay');
			playButtonElement.classList.add('hidden');
			playButtonElement.addEventListener("click", playStream);

			statusDisplayElement = document.createElement('div');
			statusDisplayElement.id = 'status-display';
			statusDisplayElement.className = 'status-bar';
			statusDisplayElement.setAttribute('role', 'status');
			statusDisplayElement.textContent = tt('videoConnecting');

			let overlayInput = document.createElement('input');
			overlayInput.type = 'text';
			overlayInput.readOnly = true;
			overlayInput.id = 'overlayInput';

			// prepare the video and audio elements
			videoElement = document.createElement('video');
			videoElement.id = 'stream';
			videoElement.className = 'video';
			videoElement.autoplay = true;
			videoElement.playsInline = true;
			videoElement.muted = true;
			videoElement.setAttribute('muted', '');
			videoElement.setAttribute('autoplay', '');
			videoElement.setAttribute('playsinline', '');
			// NOTE: Do NOT set contentEditable on the video element.
			// When a system IME is active (even in English mode), contentEditable
			// causes the browser to report keyCode===229 for ALL keystrokes,
			// which breaks keyboard input entirely.

			const hiddenFileInput = document.createElement('input');
			hiddenFileInput.type = 'file';
			hiddenFileInput.id = 'globalFileInput';
			hiddenFileInput.multiple = true;
			hiddenFileInput.style.display = 'none';
			document.body.appendChild(hiddenFileInput);
			hiddenFileInput.addEventListener('change', handleFileInputChange);

			videoContainer.appendChild(videoElement);
			videoContainer.appendChild(playButtonElement);

			// No-window overlay (shown when no X11 windows are running)
			const noWindowOverlay = document.createElement('div');
			noWindowOverlay.className = 'no-window-overlay hidden';
			noWindowOverlay.innerHTML = `<div class="no-window-content"><h2>${tt('waitingAppTitle')}</h2><p>${tt('noAppRunning')}</p></div>`;
			if (!window.__IVNC_EMBED__) videoContainer.appendChild(noWindowOverlay);

			// Taskbar: trigger zone + bar
			const taskbarTrigger = document.createElement('div');
			taskbarTrigger.className = 'taskbar-trigger';
			const taskbar = document.createElement('div');
			taskbar.className = 'taskbar';
			taskbar.id = 'taskbar';
			toolbarController = createToolbar({
				element: taskbar,
				canReorder: !window.__IVNC_APP_VIEW__ && toolbarFeatureEnabled('console'),
				canRename: !window.__IVNC_APP_VIEW__ && toolbarFeatureEnabled('console'),
				activateWindow: (id, key) => {
					if (!webrtc.sendDataChannelMessage(key ? `focus-key,${key}` : `focus,${id}`)) return false;
					if (appStreams?.activeId()) appStreams.activate(null);
					return true;
				},
				onInteraction: (active) => active ? showTaskbar() : hideTaskbar(),
			});
			// Pin button (first element in taskbar)
			let taskbarPinned = getBoolParam('taskbar_pinned', true);
			// SVG icons for pin states
			// Unpinned: tilted pin (📌 style)
			const pinSvgUnpinned = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><g transform="rotate(-45 12 12)"><path d="M12 17v5"/><path d="M9 11V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v7"/><path d="M5 15h14"/><path d="M9 11l-2 4h10l-2-4"/></g></svg>`;
			// Pinned: straight down pin
			const pinSvgPinned = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17v5"/><path d="M9 11V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v7"/><path d="M5 15h14"/><path d="M9 11l-2 4h10l-2-4"/></svg>`;

			const pinBtn = document.createElement('div');
			pinBtn.className = 'taskbar-pin' + (taskbarPinned ? ' active' : '');
			pinBtn.innerHTML = taskbarPinned ? pinSvgPinned : pinSvgUnpinned;
			setLiveTitle(pinBtn, taskbarPinned ? 'unpinTaskbar' : 'pinTaskbar');
			if (taskbarPinned) taskbar.classList.add('pinned');
			pinBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				taskbarPinned = !taskbarPinned;
				pinBtn.classList.toggle('active', taskbarPinned);
				taskbar.classList.toggle('pinned', taskbarPinned);
				pinBtn.innerHTML = taskbarPinned ? pinSvgPinned : pinSvgUnpinned;
				setLiveTitle(pinBtn, taskbarPinned ? 'unpinTaskbar' : 'pinTaskbar');
				setBoolParam('taskbar_pinned', taskbarPinned);
				viewportController?.layout();
			});
			toolbarController.addTool(pinBtn, true);
			if (!window.__IVNC_EMBED__) {
				gestureController = createGestureNavigation({
					navigate: direction => toolbarController?.navigate(direction),
					canHandle: source => {
						if (document.hidden || toolbarController?.interacting() || document.documentElement.hasAttribute('data-ivnc-modal-blocked')) return false;
						if (source === window) return !appStreams?.activeId();
						return !!appStreams?.isActiveFrame(source);
					},
				});
				gestureController.setEnabled(getBoolParam('gesture_navigation', true));
				window.__ivncGestureNavigation = gestureController;
				const syncGestureStyle = doc => doc?.documentElement?.classList.toggle('ivnc-gesture-navigation', gestureController.enabled());
				const syncGestureStyles = () => {
					syncGestureStyle(document);
					for (const frame of appStreams?.frames() || []) {
						try { syncGestureStyle(frame.contentDocument); } catch (_) {}
					}
				};
				const releaseGesturePointerLock = doc => {
					if (doc?.pointerLockElement?.id === 'overlayInput') doc.exitPointerLock();
				};
				toolbarController.addGestureToggle(gestureController, enabled => {
					if (enabled) {
						releaseGesturePointerLock(document);
						for (const frame of appStreams?.frames() || []) {
							try { releaseGesturePointerLock(frame.contentDocument); } catch (_) {}
						}
					}
					setBoolParam('gesture_navigation', enabled);
					syncGestureStyles();
				});
				syncGestureStyles();
			}

            displayModeButton = document.createElement('button');
            displayModeButton.type = 'button'; displayModeButton.className = 'taskbar-pin toolbar-display-mode';
            displayModeButton.innerHTML = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M7 11V7h4m6 6v4h-4"/></svg>';
            displayModeButton.addEventListener('click', () => {
                setDisplayMode(displayMode === 'fit' ? 'native' : 'fit');
                document.querySelector('.app-stream.active iframe')?.contentWindow?.postMessage({type:'setDisplayMode', value:displayMode}, location.origin);
            });
            syncDisplayModeButton();
            toolbarController.addTool(displayModeButton);


			// IME toggle button
			let imeModeActive = getBoolParam('ime_mode', false);
			const imeSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="16" rx="2"/><text x="12" y="16" text-anchor="middle" font-size="11" fill="currentColor" stroke="none" font-weight="bold">中</text></svg>`;
			const imeBtn = document.createElement('div');
			imeBtn.className = 'taskbar-pin' + (imeModeActive ? ' active' : '');
			imeBtn.innerHTML = imeSvg;
			setLiveTitle(imeBtn, imeModeActive ? 'imeOff' : 'imeOn');
			const syncImeButton = () => {
				const active = appStreams?.imeState() ?? !!input?._imeModeActive;
				imeBtn.classList.toggle('active', active);
				setLiveTitle(imeBtn, active ? 'imeOff' : 'imeOn');
			};
			imeBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				// The active stream tab answers with ivnc-ime-state.
				if (appStreams?.toggleIme()) return;
				if (input) {
					const active = input.toggleImeMode();
					imeBtn.classList.toggle('active', active);
					setLiveTitle(imeBtn, active ? 'imeOff' : 'imeOn');
					setBoolParam('ime_mode', active);
				}
			});
			toolbarController.addTool(imeBtn, true);

			// Change password button
			const pwdSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>`;
			const pwdBtn = document.createElement('div');
			pwdBtn.className = 'taskbar-pin';
			pwdBtn.innerHTML = pwdSvg;
			setLiveTitle(pwdBtn, 'changePassword');
			pwdBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				showChangePasswordModal();
			});
			if (!window.__IVNC_APP_VIEW__) toolbarController.addTool(pwdBtn);

			// File upload button
			const uploadSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>`;
			const uploadBtn = document.createElement('div');
			uploadBtn.className = 'taskbar-pin disabled';  // 初始为禁用状态
			uploadBtn.id = 'upload-btn';
			uploadBtn.innerHTML = uploadSvg;
			setLiveTitle(uploadBtn, 'uploadFile');
			uploadBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				// Check if WebRTC data channel is open
				if (!appStreams?.activeId() && (!webrtc || !webrtc._send_channel || webrtc._send_channel.readyState !== 'open')) {
					console.warn('Cannot upload: data channel not ready');
					return;
				}
				// Trigger file input
				const hiddenInput = document.getElementById('globalFileInput');
				if (hiddenInput) {
					hiddenInput.click();
				}
			});
			toolbarController.addTool(uploadBtn, true);

			// Restart / upgrade button
			const updateSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2"/></svg>`;
			const updateBtn = document.createElement('div');
			updateBtn.className = 'taskbar-pin';
			updateBtn.id = 'update-btn';
			updateBtn.innerHTML = updateSvg;
			setLiveTitle(updateBtn, 'restartOrUpgrade');
			updateBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				showForceUpdateModal();
			});
			if (!window.__IVNC_APP_VIEW__) toolbarController.addTool(updateBtn);

			if (!window.__IVNC_APP_VIEW__ && toolbarFeatureEnabled('proxy')) {
				const proxySvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a7.8 7.8 0 0 0 0-6"/><path d="M4.6 9a7.8 7.8 0 0 0 0 6"/><path d="M16.2 4.9a12 12 0 0 1 0 14.2"/><path d="M7.8 19.1a12 12 0 0 1 0-14.2"/></svg>`;
				const proxyBtn = document.createElement('div');
				proxyBtn.className = 'taskbar-pin';
				proxyBtn.id = 'proxy-btn';
				proxyBtn.innerHTML = proxySvg;
				setLiveTitle(proxyBtn, 'proxyTooltip');
				proxyBtn.addEventListener('click', (e) => {
					e.stopPropagation();
					terminalController.minimize('auto');
					consoleController.minimize();
					proxyController.toggle();
				});
				proxyController.setButton(proxyBtn);
				toolbarController.addTool(proxyBtn);
			}

			if (!window.__IVNC_APP_VIEW__ && toolbarFeatureEnabled('terminal')) {
				const terminalSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>`;
				const terminalBtn = document.createElement('div');
				terminalBtn.className = 'taskbar-pin';
				terminalBtn.id = 'terminal-btn';
				terminalBtn.innerHTML = terminalSvg;
				setLiveTitle(terminalBtn, 'terminalTooltip');
				terminalBtn.addEventListener('click', (e) => {
					e.stopPropagation();
					proxyController.minimize();
					consoleController.minimize();
					terminalController.toggle();
				});
				terminalController.setButton(terminalBtn);
				toolbarController.addTool(terminalBtn);
			}

			if (!window.__IVNC_APP_VIEW__ && toolbarFeatureEnabled('console')) {
				const consoleSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z"/><path d="M19.4 15a1.8 1.8 0 0 0 .36 1.98l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06A1.8 1.8 0 0 0 15 19.4a1.8 1.8 0 0 0-1 .6 1.8 1.8 0 0 0-.5 1.3V21a2 2 0 1 1-4 0v-.1A1.8 1.8 0 0 0 8.5 19.4a1.8 1.8 0 0 0-1.98.36l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.8 1.8 0 0 0 4.6 15a1.8 1.8 0 0 0-.6-1 1.8 1.8 0 0 0-1.3-.5H2.6a2 2 0 1 1 0-4h.1A1.8 1.8 0 0 0 4.6 8.5a1.8 1.8 0 0 0-.36-1.98l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.8 1.8 0 0 0 9 4.6a1.8 1.8 0 0 0 1-.6 1.8 1.8 0 0 0 .5-1.3V2.6a2 2 0 1 1 4 0v.1A1.8 1.8 0 0 0 15.5 4.6a1.8 1.8 0 0 0 1.98-.36l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.8 1.8 0 0 0 19.4 9c.2.37.57.6 1 .6h.1a2 2 0 1 1 0 4h-.1a1.8 1.8 0 0 0-1 .5z"/></svg>`;
				const consoleBtn = document.createElement('div');
				consoleBtn.className = 'taskbar-pin';
				consoleBtn.id = 'console-btn';
				consoleBtn.innerHTML = consoleSvg;
				setLiveTitle(consoleBtn, 'consoleTooltip');
				consoleBtn.addEventListener('click', (e) => {
					e.stopPropagation();
					proxyController.minimize();
					terminalController.minimize('auto');
					consoleController.toggle();
				});
				consoleController.setButton(consoleBtn);
				toolbarController.addTool(consoleBtn);
			}

			const connIndicator = document.createElement('div');
			connIndicator.className = 'taskbar-conn';
			connIndicator.id = 'conn-indicator';
			connIndicator.textContent = '—';
			setLiveTitle(connIndicator, 'connectionMode');
			connIndicator.style.cursor = 'pointer';
			connIndicator.style.pointerEvents = 'auto';
			connIndicator.addEventListener('click', (e) => {
				e.stopPropagation();
				window.open('/connect', '_blank');
			});
			networkQualityBadge = document.createElement('div');
			networkQualityBadge.className = 'taskbar-network-quality unknown';
			networkQualityBadge.id = 'network-quality';
			toolbarController.status.appendChild(networkQualityBadge);
			networkQualityBadge.tabIndex = 0;
			networkQualityBadge.setAttribute('role', 'button');
			resetNetworkQuality('unknown');
			toolbarController.status.appendChild(connIndicator);

			// Embedded stream tabs use the parent page's toolbar.
			if (!window.__IVNC_EMBED__) {
				document.body.appendChild(taskbarTrigger);
				document.body.appendChild(taskbar);
			}

			let taskbarHideTimer = null;
			const showTaskbar = () => {
				clearTimeout(taskbarHideTimer);
				taskbar.classList.add('visible');
			};
			const hideTaskbar = () => {
				if (taskbarPinned || toolbarController?.interacting()) return;
				taskbarHideTimer = setTimeout(() => {
					if (!toolbarController?.interacting()) taskbar.classList.remove('visible');
				}, 400);
			};
			// overlayInput captures all pointer events, so taskbar-trigger
			// never receives mouseenter. Detect bottom-edge hover via
			// mousemove on the document instead.
			let atBottomEdge = false;
			document.addEventListener('mousemove', (e) => {
				if (window.__IVNC_EMBED__) {
					const edge = e.clientY >= window.innerHeight - 6;
					if (edge && !atBottomEdge) window.parent.postMessage({ type: 'ivnc-edge-hover' }, window.location.origin);
					atBottomEdge = edge;
				} else if (e.clientY >= window.innerHeight - 6) {
					showTaskbar();
				} else if (e.clientY < window.innerHeight - taskbar.getBoundingClientRect().height - 6) {
					hideTaskbar();
				}
			});
			taskbar.addEventListener('mouseenter', showTaskbar);
			taskbar.addEventListener('mouseleave', hideTaskbar);

			if (!window.__IVNC_APP_VIEW__) {
				appStreams = createAppStreams({
					viewport: videoContainer,
                    displayMode: () => displayMode,
					onFrameReady: doc => doc?.documentElement?.classList.toggle('ivnc-gesture-navigation', !!gestureController?.enabled()),
					onChange: () => {
						renderTaskbar();
						syncImeButton();
						// Report the main desktop hidden while a stream tab covers it.
						const streamActive = !!appStreams?.activeId();
						if (streamActive !== mainCovered) {
							mainCovered = streamActive;
							updateStatusDisplay();
							webrtc?.playback?.setActive(!streamActive);
							clipboardController?.cancel();
							sendClientActivity('view-switch');
						}
					},
					// Pointer over a stream iframe never reaches this document.
					onEdgeHover: () => {
						showTaskbar();
						hideTaskbar();
					},
				});
			}

			videoContainer.appendChild(statusDisplayElement);
			videoContainer.appendChild(overlayInput);
			appDiv.appendChild(videoContainer);
			viewportController?.destroy();
			viewportController = createViewport({
				root: appDiv, container: videoContainer, video: videoElement, overlay: overlayInput,
				toolbar: taskbar, isPinned: () => taskbarPinned, embedded: !!window.__IVNC_EMBED__,
				onChange: ({ changed }) => {
					input?.resize();
					appStreams?.layout();
					updateVideoImageRendering();
					updateOverlayModalScale();
					if (changed) resizeStart();
				},
			});

			if (!document.getElementById('keyboard-input-assist')) {
				const keyboardInputAssist = document.createElement('input');
				keyboardInputAssist.type = 'text';
				keyboardInputAssist.id = 'keyboard-input-assist';
				keyboardInputAssist.style.position = 'absolute';
				keyboardInputAssist.style.left = '-9999px';
				keyboardInputAssist.style.top = '-9999px';
				keyboardInputAssist.style.width = '1px';
				keyboardInputAssist.style.height = '1px';
				keyboardInputAssist.style.opacity = '0';
				keyboardInputAssist.style.border = '0';
				keyboardInputAssist.style.padding = '0';
				keyboardInputAssist.style.caretColor = 'transparent';
				keyboardInputAssist.setAttribute('aria-hidden', 'true');
				keyboardInputAssist.setAttribute('autocomplete', 'off');
				keyboardInputAssist.setAttribute('autocorrect', 'off');
				keyboardInputAssist.setAttribute('autocapitalize', 'off');
				keyboardInputAssist.setAttribute('spellcheck', 'false');
				document.body.appendChild(keyboardInputAssist);
				console.log("Dynamically added #keyboard-input-assist element.");
			}
			// Fetch locally stored application data
			appName = window.location.pathname.endsWith("/") && (window.location.pathname.split("/")[1]) || "webrtc";
			debug = getBoolParam('debug', false);
			setBoolParam('debug', debug);
			// TCP-only: ignore legacy relay switch setting entirely.
            scaleLocal = getBoolParam('scaleLocallyManual', true);
			videoBitRate = getIntParam('video_bitrate', videoBitRate);
			setIntParam('video_bitrate', videoBitRate);
			videoFramerate = getIntParam('framerate', videoFramerate);
			setIntParam('framerate', videoFramerate);
			audioBitRate = getIntParam('audio_bitrate', audioBitRate);
			setIntParam('audio_bitrate', audioBitRate);
            // Migrate only the local fit/original preference; discard old global dimensions.
            const legacyNative = getBoolParam('is_manual_resolution_mode', false) && !getBoolParam('scaleLocallyManual', true);
            displayMode = getStringParam('display_mode', legacyNative ? 'native' : 'fit');
            setIntParam('manual_width', null); setIntParam('manual_height', null);
            setBoolParam('is_manual_resolution_mode', false);
            setDisplayMode(displayMode);
			encoder = getStringParam('encoder_rtc', 'x264enc');
			setStringParam('encoder_rtc', encoder)
			useCssScaling = getBoolParam('useCssScaling', true);  // TODO: need to handle hiDPI
			setBoolParam('useCssScaling', useCssScaling);

			// listen for dashboard messages (Dashboard -> core client)
			window.addEventListener("message", handleMessage);
			// listen for file upload event
			window.addEventListener('requestFileUpload', handleRequestFileUpload);
			// handlers to handle the drop in files/directories for upload
			overlayInput.addEventListener('dragover', handleDragOver);
			overlayInput.addEventListener('drop', handleDrop);

			// WebRTC entrypoint, connect to the signaling server
			var pathname = window.location.pathname;
			pathname = pathname.slice(0, pathname.lastIndexOf("/") + 1);
			var protocol = (location.protocol == "http:" ? "ws://" : "wss://");
			var signaling = new WebRTCDemoSignaling(new URL(protocol + window.location.host + pathname + (window.__IVNC_APP_VIEW__ ? "webrtc" : appName + "/signaling/")));
			webrtc = new WebRTCDemo(signaling, videoElement, 1);
            window.__ivncWebRTC = webrtc;
			// Embedded stream tab: the parent page drives activation, audio, upload and IME.
			// `view` lets the parent ignore messages from a view it already replaced.
			const postToParent = (message) => {
				if (window.__IVNC_EMBED__) window.parent.postMessage({ ...message, view: window.location.pathname }, window.location.origin);
			};
			webrtc.onplaybackstatechange = ({ state }) => {
				videoPlaybackState = state;
				showStart = ['blocked', 'no-data', 'decode-error', 'playback-error', 'connection-error'].includes(state);
				updateStatusDisplay();
				postToParent({ type: 'ivnc-view-state', state: state === 'playing' ? 'connected' : 'connecting', playback: state });
			};
			if (window.__IVNC_EMBED__) {
				window.addEventListener('ivnc-view-ended', (event) => {
					postToParent({ type: 'ivnc-view-state', state: 'ended', reason: event.detail });
				});
				window.addEventListener('message', (event) => {
					if (event.origin !== window.location.origin || event.source !== window.parent) return;
					const message = event.data || {};
					const open = webrtc._send_channel?.readyState === 'open';
					if (message.type === 'ivnc-view-active') {
						window.__ivncViewActive = !!message.active;
						webrtc.playback.setActive(!!message.active);
						// Release held keys when the tab is switched away.
						if (!message.active) clipboardController?.cancel();
						sendClientActivity('view-switch');
						if (message.active) window.focus();
					} else if (message.type === 'ivnc-unmute') {
						webrtc.unmuteAudio();
					} else if (message.type === 'ivnc-upload' && Array.isArray(message.files)) {
						if (open) uploadFiles(message.files);
					} else if (message.type === 'ivnc-ime' && input) {
						input.onimetoggle(input.toggleImeMode());
					}
				});
				// Messages the parent sent on iframe load may predate the listener above.
				postToParent({ type: 'ivnc-view-ready' });
			}
			const send = (data) => {
				if (clipboardController) clipboardController.sendInput(data);
				else webrtc.sendDataChannelMessage(data);
			}
			input = new Input(overlayInput, send, false, useCssScaling=useCssScaling);
			input.gestureNavigation = () => {
				if (!window.__IVNC_EMBED__) return gestureController;
				try { return window.parent.__ivncGestureNavigation || null; } catch (_) { return null; }
			};

			setupKeyBoardAssisstant();

			// assign the handlers to respective objects
			// TODO: Need to handle the logEntries and DebugEntries list
			signaling.onstatus = (message) => {
				logEntries.push(applyTimestamp("[signaling] " + message));
				console.log("[signaling] " + message);
			};
			signaling.onerror = (message) => {
				logEntries.push(applyTimestamp("[signaling] [ERROR] " + message))
				console.log("[signaling ERROR] " + message);
				if (message.includes('IVNC_VIDEO_UNSUPPORTED')) webrtc.playback.fail('unsupported', message);
				else if (message.includes('SESSION_ERROR')) webrtc.playback.fail('connection-error', message);
			};

			signaling.ondisconnect = (reconnect) => {
				postToParent(reconnect
					? { type: 'ivnc-view-state', state: 'connecting' }
					: { type: 'ivnc-view-state', state: 'ended', reason: window.__ivncViewEnded || 'closed' });
				videoElement.style.cursor = "auto";
				if (reconnect) {
					// If WebRTC media is already flowing, don't tear it down
					// just because the signaling WebSocket was closed by a proxy.
					// Only reset when the peer connection is actually dead.
					var pc = webrtc.peerConnection;
					if (!window.__IVNC_APP_VIEW__ && pc && (pc.connectionState === 'connected' || pc.connectionState === 'connecting')) {
						console.log("[signaling] WebSocket closed but WebRTC still alive, reconnecting signaling only");
						status = 'connected';
						signaling.connect();
					} else {
						status = 'connecting';
						webrtc.reset();
					}
				} else {
					status = 'disconnected';
				}
				updateStatusDisplay();
			};

			// Send webrtc status and error messages to logs.
			webrtc.onstatus = (message) => {
				logEntries.push(applyTimestamp("[webrtc] " + message));
				console.log("[webrtc] " + message);
			};
			webrtc.onerror = (message) => {
				logEntries.push(applyTimestamp("[webrtc] [ERROR] " + message));
				console.log("[webrtc] [ERROR] " + message);
			};

			if (debug) {
				signaling.ondebug = (message) => { debugEntries.push("[signaling] " + message); };
				webrtc.ondebug = (message) => { debugEntries.push(applyTimestamp("[webrtc] " + message)) };
			}

			webrtc.ongpustats = async (stats) => {
				// Gpu stats for the Dashboard to render
				window.gpu_stats = stats;
			}

			webrtc.onconnectionstatechange = (state) => {
				videoConnected = state;
				if (['failed', 'disconnected', 'closed'].includes(state)) {
					gestureController?.reset();
					toolbarController?.resetNavigation();
				}
				if (videoConnected === "connected") {
					// Repeatedly emit minimum latency target
					webrtc.peerConnection.getReceivers().forEach((receiver) => {
						let intervalLoop = setInterval(async () => {
							if (receiver.track.readyState !== "live" || receiver.transport.state !== "connected") {
								clearInterval(intervalLoop);
								return;
							} else {
								receiver.jitterBufferTarget = receiver.jitterBufferDelayHint = receiver.playoutDelayHint = 0;
							}
						}, 15);
					});
					status = state;
					if (!statWatchEnabled) {
						enableStatWatch();
					}
					resetNetworkQuality('unknown');
				} else if (videoConnected === "failed" && !window.__FORCE_DISCONNECTED__) {
					// WebRTC connection died — reset and reconnect
					console.log("[webrtc] Connection failed, resetting");
					status = 'connecting';
					resetNetworkQuality('offline');
					webrtc.reset();
				} else if (videoConnected === "disconnected" || videoConnected === "closed") {
					resetNetworkQuality('offline');
				}
				updateStatusDisplay();
			};

			webrtc.ondatachannelopen = () => {
				lastSentResolution = null;
				console.log("Data channel opened");
				// Bind input handlers.
				input.attach();
				clipboardController?.connected();
				input.setDisplayState(null);
				webrtc.sendDataChannelMessage("_display,1");
				loadLastSessionSettings();
				fetchInitialIPv4();
				sendClientPersistedSettings();
				sendClientBrowserFeatures();
				startClientActivityReporting();

				// Restore IME mode from localStorage
				if (imeModeActive && input) {
					postToParent({ type: 'ivnc-ime-state', active: input.toggleImeMode() });
				}

				// Enable upload button
				const uploadBtn = document.getElementById('upload-btn');
				if (uploadBtn) {
					uploadBtn.classList.remove('disabled');
				}

				// Send client-side metrics over data channel every 5 seconds
				if (metricsLoopId) {
					clearInterval(metricsLoopId);
				}
				metricsLoopId = setInterval(async () => {
					if (connectionStat.connectionFrameRate === parseInt(connectionStat.connectionFrameRate, 10))webrtc.sendDataChannelMessage(`_f,${connectionStat.connectionFrameRate}`);
					if (connectionStat.connectionLatency === parseInt(connectionStat.connectionLatency, 10)) webrtc.sendDataChannelMessage(`_l,${connectionStat.connectionLatency}`);
				}, 5000)
			}

			webrtc.ondatachannelclose = () => {
				gestureController?.reset();
				toolbarController?.resetNavigation();
				lastSentResolution = null;
				clearTimeout(resizeTimer); resizeTimer = null;
				resetNetworkQuality('offline');
				stopClientActivityReporting();
				clipboardController?.disconnected();
				input.detach();
				// Disable upload button
				const uploadBtn = document.getElementById('upload-btn');
				if (uploadBtn) {
					uploadBtn.classList.add('disabled');
				}
			}

			input.onmenuhotkey = () => {
				showDrawer = !showDrawer;
			}

			input.onimetoggle = (active) => {
				imeBtn.classList.toggle('active', active);
				setLiveTitle(imeBtn, active ? 'imeOff' : 'imeOn');
				setBoolParam('ime_mode', active);
				postToParent({ type: 'ivnc-ime-state', active });
			};

			webrtc.onplaystreamrequired = () => {
				showStart = true;
				updateStatusDisplay();
			}

			// Unmute audio on first user interaction (registered once)
			if (!webrtc._unmuteListenersBound) {
				webrtc._unmuteListenersBound = true;
				const unmuteOnInteraction = () => {
					webrtc.unmuteAudio();
					document.removeEventListener('click', unmuteOnInteraction);
					document.removeEventListener('keydown', unmuteOnInteraction);
				};
				document.addEventListener('click', unmuteOnInteraction);
				document.addEventListener('keydown', unmuteOnInteraction);
			}

			// Actions to take whenever window changes focus
			window.addEventListener('focus', handleWindowFocus);
			window.addEventListener('blur', handleWindowBlur);

			clipboardController?.destroy();
			clipboardUI?.destroy();
			clipboardUI = createClipboardUI({
				getController: () => clipboardController,
				restoreFocus: () => {
					if (window.__ivncViewActive !== false && !appStreams?.activeId() && document.hasFocus()) {
						(input?._imeModeActive ? input._imeTextarea : overlayInput)?.focus({ preventScroll: true });
					}
				},
			});
			clipboardController = createClipboard({
				webrtc, overlay: overlayInput, imeTarget: () => input?._imeTextarea,
				isActive: () => window.__ivncViewActive !== false && !appStreams?.activeId() && document.visibilityState !== 'hidden',
				getTarget: () => lastTaskbarData?.windows?.find(w => w.focused)?.id,
				report: reportClipboardError,
				onCopyState: state => clipboardUI?.update(state),
				diagnostic: details => console.debug('[clipboard]', details),
			});
			input.clipboard = clipboardController;
			window.__ivncClipboard = clipboardController;
			webrtc.onclipboardcontent = clipboardController.receive;
			webrtc.onclipboardformats = clipboardController.receiveFormats;
			webrtc.onclipboardmessage = clipboardController.message;

			webrtc.oncursorchange = (cursorData) => {
				input.updateServerCursor(cursorData);
			}

			webrtc.ontaskbarupdate = (data) => {
				lastTaskbarData = data || {};
				if (appStreams) appStreams.update(lastTaskbarData.apps || []);
				renderTaskbar(true);
			}

			// Window tabs come from the compositor, stream tabs from app-streams.js.
			function renderTaskbar(reveal = false) {
				if (!toolbarController) return;
				const streams = appStreams ? appStreams.tabs() : [];
				toolbarController.update(lastTaskbarData, streams, !!appStreams?.activeId());
				if (!(lastTaskbarData.windows || []).length && !streams.length) {
					taskbar.classList.remove('visible');
				} else if (reveal && !taskbarPinned) {
					showTaskbar();
				}
			}

			webrtc.onsystemaction = (action) => {
				webrtc._setStatus("Executing system action: " + action);
				if (action === 'reload') {
					setTimeout(() => {
						// trigger webrtc.reset() by disconnecting from the signaling server.
						signaling.disconnect();
					}, 700);
				} else {
					webrtc._setStatus('Server sent acknowledgement for ' + action);
				}
			}

			webrtc.onlatencymeasurement = (latency_ms) => {
				serverLatency = latency_ms * 2.0;
			}

			webrtc.onsystemstats = async (stats) => {
				// Dashboard takes care of data validation
				window.system_stats = stats;
			}

            webrtc.ondisplaystate = state => {
                if (state?.protocol !== 1) return;
                window.__ivncDisplayState = state;
                input?.setDisplayState(state);
                viewportController?.layout();
            };
            webrtc.onserversettings = (obj) => {
				console.log("Received server settings payload:", obj.settings);
				const changes = sanitizeAndStoreSettings(obj.settings);
				window.postMessage({ type: 'serverSettings', payload: obj.settings }, window.location.origin);
				if (Object.keys(changes).length > 0) {
						// TODO: server-side handling of settings updates
						// console.log('Client settings were sanitized by server rules. Sending updates back to server:', changes);
						handleSettingsMessage(changes);
				}

			}


			// TCP-only: directly connect using the SDP answer's TCP candidate.
			const windowResolution = input.getWindowResolution();
			signaling.currRes = windowResolution;
			webrtc.connect();
		},
		cleanup() {
			stopPlaybackLanguageUpdates();
			webrtc?.destroy();
			clearTimeout(resizeTimer); resizeTimer = null;
			viewportController?.destroy(); viewportController = null;
			clipboardController?.destroy(); clipboardController = null;
			clipboardUI?.destroy(); clipboardUI = null;
			window.__ivncClipboard = null;
			clipboardNotice?.remove(); clipboardNotice = null;
			clearTimeout(clipboardNoticeTimer);
			input?.detach();
			// reset the data
			window.isManualResolutionMode = false;
			window.fps = 0;

			// remove the listeners
			window.removeEventListener("message", handleMessage);
			window.removeEventListener("resize", resizeStart);
			window.removeEventListener("requestFileUpload", handleRequestFileUpload);
			window.removeEventListener("focus", handleWindowFocus);
			window.removeEventListener("blur", handleWindowBlur);

			// temporary workaround to nullify/reset the variables
			appName = null;
			videoBitRate = 8000;
			videoFramerate = 60;
			audioBitRate = 128000;
			showStart = false;
			videoPlaybackState = 'connecting';
			showDrawer = false;
			logEntries = [];
			debugEntries = [];
			status = 'connecting';
			encoderLabel = "";
			encoder = ""
			connectionStat = {
					connectionStatType: "unknown",
					connectionLatency: 0,
					connectionVideoLatency: 0,
					connectionAudioLatency: 0,
					connectionAudioCodecName: "NA",
					connectionAudioBitrate: 0,
					connectionPacketsReceived: 0,
					connectionPacketsLost: 0,
					connectionBytesReceived: 0,
					connectionBytesSent: 0,
					connectionCodec: "unknown",
					connectionVideoDecoder: "unknown",
					connectionResolution: "",
					connectionFrameRate: 0,
					connectionVideoBitrate: 0,
					connectionAvailableBandwidth: 0
			};
			serverLatency = 0;
			scaleLocal = false;
			debug = false;
			playButtonElement = null;
			statusDisplayElement = null;
			displayModeButton = null;
			window.__ivncDisplayState = null;
			lastSentResolution = null;
			videoConnected = "";
			audioConnected = "";
			statWatchEnabled = false;
			if (statsLoopId) { clearInterval(statsLoopId); statsLoopId = null; }
			if (metricsLoopId) { clearInterval(metricsLoopId); metricsLoopId = null; }
			resetNetworkQuality('unknown');
			networkQualityBadge = null;
			appStreams?.destroy(); appStreams = null;
			gestureController?.reset(); gestureController = null;
			if (!window.__IVNC_EMBED__) window.__ivncGestureNavigation = null;
			document.documentElement.classList.remove('ivnc-gesture-navigation');
			toolbarController?.destroy(); toolbarController = null;
			window._ipv4Element = null;
			terminalController.destroy();
			proxyController.destroy();
			consoleController.destroy();
			webrtc = null;
			input = null;
			useCssScaling = true;
		}
	}

	// Connection management page
	initRouter();
}
