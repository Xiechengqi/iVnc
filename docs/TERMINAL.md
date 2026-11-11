# Web 终端样式与会话

toolbar 的终端弹窗使用 xterm，通过 `/terminal/ws` 连接服务端 PTY。每条 `output` 消息携带 Base64 编码的原始字节，由 xterm 解析并显示。

## 重复字符问题

xterm 为测量字宽，会在 `.xterm-char-measure-element` 中放入重复 32 次的字符；DOM 字体测量回退路径还会创建重复的 `W`。这些节点由官方 CSS 隐藏并移出正常布局，仍保留可测量宽度。

桌面入口由 `index.html` 在取得配置后动态创建 module script 加载。此前普通 CSS import 被 Vite 提取到独立文件，但动态入口没有加载该文件，导致测量节点可见。初始 shell 提示符中的 `#` 因此显示为一串 `################################`。WebSocket 中并没有这串异常输出。

`ivnc-wr-core.js` 现在使用 `@xterm/xterm/css/xterm.css?inline` 引入完整官方样式，在终端创建前注入 `#ivnc-terminal-styles`，同一 document 只注入一次。开发环境与生产资源使用相同路径，不依赖构建文件名，也不复制依赖库的 CSS。

测量节点不能用 `display:none` 隐藏，否则宽度为零。输入 textarea、IME composition view 和辅助功能节点均由官方样式管理；应用只负责弹窗外观和尺寸。

## 显示与会话行为

- 首次打开、重连和重新显示不会自动清屏，启动阶段的真实输出保留。原有连接后 300ms 的自动清屏已移除。
- 最小化隐藏弹窗，保留终端、WebSocket、PTY 和滚动历史。重新显示后适配可见容器尺寸。
- 连接断开后沿用现有重连机制，服务端为新 WebSocket 创建新的 shell；前端保留原有显示历史。
- 真实命令输出的重复字符照常显示。修复只涉及测量节点的样式，不过滤输出数据。

## 验证

先构建前端，再构建包含这些资源的服务端：

```bash
npm --prefix web/ivnc run build
CARGO_INCREMENTAL=0 cargo build --features tls --bin ivnc
IVNC_BIN="$PWD/target/debug/ivnc" node scripts/terminal_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" node scripts/toolbar_smoke.mjs
```

终端 smoke 使用独立运行目录、端口、浏览器配置和 shell，默认清理成功运行的临时文件；失败保留日志、诊断数据和截图，可用 `KEEP_TMP=1` 保留成功运行的目录。测试 shell 禁用用户启动配置及历史写入。

覆盖官方样式、隐藏且可测量的辅助节点、启动输出、真实重复字符、Unicode 与字体变体、会话复用、尺寸与 DPR 变化、主桌面手动缩放、中文输入法预编辑与提交、复制粘贴、滚动历史、重连及 DOM 字体测量回退路径。

本次前端与启用 TLS 的服务端构建均通过；Chromium 终端 smoke 12 项、toolbar smoke 28 项全部通过。原生粘贴使用 Shift+Insert，另行验证 Ctrl+Shift+V 与 Ctrl+Shift+C。Vite 仍有已有的单文件体积提示。

整体 review 核对了样式注入时机和去重、官方辅助节点样式、真实输出保留、IME 与输入隔离、滚动和连接生命周期，未发现阻塞问题。
