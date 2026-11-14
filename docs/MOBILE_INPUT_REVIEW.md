# 移动端键盘输入修复与 Review

日期：2026-11-14。基础提交：`7dbc2a2`。范围为主桌面、共享／远程应用视图的系统键盘入口、文字提交、焦点生命周期和键盘遮挡适配。使用行为与协议见 [MOBILE_INPUT.md](MOBILE_INPUT.md)。

## 根因与实现

远端应用有输入焦点，不代表浏览器中存在可编辑元素。原有画面覆盖层为 readonly input，触摸只向远端发送鼠标操作，无法可靠打开移动端键盘；IME textarea 又只处理组合事件，丢弃普通 input。另一个输入辅助控件调用了不存在的方法。这是输入设计的兼容性缺口与实现 bug 共同造成的问题。

现在每个输入视图共用一个可编辑代理。移动端工具栏提供键盘按钮，在原始用户点击的调用栈内同步聚焦；同源嵌入视图直接调用当前 Input，不通过异步消息唤起。英文、数字、Unicode 和组合输入提交至已有文字通道，删除、回车和导航保留按键语义。未启用 text-input-v3 的应用保留兼容路径。

服务端按 Wayland commit 语义报告当前编辑能力、输入用途和可可靠转换的光标坐标，不发送文档、选区或周边文本。顶层页面处理 Visual Viewport，预留键盘和固定工具栏；子视图继承工作区，维持每个客户端的本地显示适配。

## 整体 Review

检查了浏览器原生编辑事件、物理键盘与快捷键、Wayland 状态的双缓冲、共享视图的用户手势传递、焦点与认证生命周期、视口变化、资源销毁、旧版本兼容和 CI 可移植性。

实施与 review 中修正了以下边界：

- Safari 中取消工具栏 pointerdown 会抑制后续触摸 click。保留浏览器点击行为，在 pointerdown 记录显示／收起意图，避免按钮先获焦导致状态反转；取消指针操作后清理该记录。
- 普通 input、beforeinput、compositionend 统一处理。最终组合文字与紧随的 input 去重；取消组合后晚到的提交不能进入当前或另一个应用。
- 远端已知编辑框关闭、应用切换、覆盖弹窗、页面隐藏、断线和退出登录会清理相应输入状态。窗口失焦期间拒绝文字提交，后台视图不能自动夺取焦点；重连不沿用旧编辑元数据。
- 桌面 IME 模式下，鼠标点击画面后的默认动作不能让 readonly 覆盖层抢回焦点；远端鼠标事件仍正常传递。
- 未支持原生文字协议的编辑器，单字符 ASCII 使用成对按键与必要的 Shift，Unicode／多字符文字沿用现有文字兼容通道。
- 移动设备识别不强制要求 maxTouchPoints 大于零，兼容 iPad 桌面模式及 WebKit 的移动设备模拟。移动端不从桌面 IME 偏好恢复自动弹键盘。
- 键盘收起恢复视口时同步复位代理与按钮；缩放期间保留上一次可靠的键盘工作区，不把 pinch 当成键盘遮挡变化。原始大小模式在有可靠坐标时将光标滚动到可见区域。
- 输入代理、光标 DOM、事件监听、Visual Viewport 监听和定时器均有销毁路径。测试默认使用 Playwright 浏览器，避免依赖 Linux 特定 Chrome 路径；移动交互套件接入现有 Linux/macOS CI。

本轮功能范围内未发现剩余阻断问题。没有修改现有实例的数据或重启运行中的用户实例。

## 验证结果

实测环境为 Linux、Google Chrome、Playwright WebKit、Electron 45.3.1。端到端使用隔离的运行目录、端口、应用与浏览器配置。

| 检查 | 结果 |
| --- | --- |
| Rust 库，含编辑元数据双缓冲测试 | 158 项通过 |
| 移动输入生产模块，Chromium／WebKit | 每种引擎 24 个交互场景通过，执行器输出共 51 项（包含分组） |
| 主桌面与独立共享运行时的真实 WebRTC → Electron | 22 项输入检查通过；两种引擎均覆盖文字、删除、回车、组合去重、工具栏关闭与退出清理；Chromium 另验证浏览器原生 IME 事件 |
| 原有 toolbar／viewport 浏览器交互 | 执行器输出 84 项通过 |
| 原有剪贴板、视频、视口和手势状态回归 | 73 项通过 |
| Electron 综合回归 | 隔离复测 28 项通过 |
| 登录会话 | 23 项通过，含 Chromium／WebKit、HTTP／HTTPS、移动刷新、多标签退出及共享视图撤销 |
| 终端／PTY | 21 项通过，含中文、粘贴、选区复制、弹窗与重新连接 |
| 剪贴板弹层 | 21 项通过，含 HTTP／HTTPS、iframe、焦点与断线清理 |
| 生产前端、嵌入前端的 TLS 二进制构建 | 通过；保留已有的 bundle 大于 500 KB 提示 |
| 格式、补丁空白与脚本语法 | 通过 |

Electron 综合脚本首次与其他浏览器套件并发运行时，在新窗口立即最小化的等待处超时；保持原脚本与断言进行隔离复测后全部通过。未将首次失败计作通过，也未为通过测试放宽生产行为。

主要复测命令：

```sh
npm --prefix web/ivnc run build
cargo build --features tls --bin ivnc --offline
cargo test --lib --features tls --offline
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:mobile-input
CHROME_BIN=/usr/bin/google-chrome ELECTRON_BIN=/path/to/electron node scripts/mobile_input_smoke.mjs
CHROME_BIN=/usr/bin/google-chrome ELECTRON_BIN=/path/to/electron node scripts/electron_smoke.mjs
CHROME_BIN=/usr/bin/google-chrome node --test scripts/toolbar_interaction_test.mjs scripts/viewport_interaction_test.mjs
node --test scripts/clipboard_test.mjs scripts/video_playback_test.mjs scripts/viewport_test.mjs scripts/gesture_navigation_smoke.mjs
CHROME_BIN=/usr/bin/google-chrome node scripts/auth_smoke.mjs
CHROME_BIN=/usr/bin/google-chrome node scripts/terminal_smoke.mjs
CHROME_BIN=/usr/bin/google-chrome node scripts/clipboard_ui_smoke.mjs
cargo fmt --check
git diff --check
```

## 验证边界

- 没有实体 iPhone、iPad 或 Android 设备。移动视口、触摸事件、同步用户激活和模拟的 Visual Viewport 变化已验证，但不能替代系统键盘的真机验收。iOS 对第一次远端点击后经网络往返才发生的自动聚焦仍可能拒绝，工具栏按钮提供同步入口。
- Chromium 端到端等待并确认了实际视频尺寸。Linux 无头 WebKit 的主视图报告收到并解码视频帧，但 video 仍为 readyState 0；共享视图也未确认画面展示。本轮单独验证了 WebKit 的真实文字／DataChannel 链路，没有把它记作视频或 Safari 真机验收。端到端脚本输出这一诊断，`REQUIRE_VIDEO=1` 可要求两种引擎都必须展示视频。
- 未实现本地完整文档编辑、自动纠错、任意远端选区同步或完整移动端桌面模拟。旧应用的 Unicode 兼容输入仍取决于其粘贴能力。
- 已更新 CI 配置，未执行远端工作流、部署或推送。
