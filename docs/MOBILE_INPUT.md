# 移动端键盘与文字输入

移动端工具栏的键盘图标用于显示／收起当前应用的系统键盘。移动输入默认可用，页面加载和重连时不主动弹出键盘。桌面端保留中文 IME 按钮和 Ctrl+Shift+Space。

## 操作与兼容

- 点击键盘按钮，在本次用户点击中同步聚焦本地编辑代理。共享应用和远程应用的同源嵌入视图使用同步调用，避免异步消息丢失 iOS 的键盘唤起机会。
- 支持 text-input-v3 的应用会报告当前编辑状态。短促单指点击可辅助打开键盘；拖动、长按、双指滚动和应用切换手势不触发这一动作。用户手动收起后保持收起，直到再次点击键盘按钮。
- 第一次点击远端输入框，编辑状态需要网络往返。iOS 可能拒绝稍后才发生的聚焦，此时使用工具栏键盘按钮。不承诺所有应用的第一次点击都能自动弹出系统键盘。
- 英文、数字、中文候选、emoji 共用一个文字输入代理。文字通过已存在的 `t,` 通道提交，回车、删除和导航键保持按键语义。支持原生文字协议的应用直接接收提交。
- 未启用文字协议的旧应用，单字符 ASCII 使用按键兼容路径，避免每个字都发生剪贴板事务；Unicode 和较长文字沿用已有串行文字兼容通道，其支持范围仍取决于应用是否接受粘贴。
- 默认关闭本地自动纠错、自动大小写和拼写检查。本地代理不持有远端完整文档，也不读取远端密码或周边文本。

## 键盘遮挡与应用切换

顶层页面同时观察布局视口和 Visual Viewport，预留系统键盘与固定工具栏空间。嵌入视图继承父页面工作区，不再次扣除工具栏。缩放手势不会按键盘遮挡处理。原始大小模式在有可用光标坐标时滚动到输入位置；适应窗口模式按本地可用区域缩放，不更改其他客户端的远端桌面尺寸。

切换应用、失去页面焦点、打开覆盖弹窗、断开连接及退出登录时，取消未提交的组合输入并清理按键状态。后台视图不能自动夺取编辑焦点。系统收起键盘造成可视区域恢复时，输入代理与按钮状态随之复位。

## 编辑状态协议

服务器通过现有 DataChannel 下发：

```json
{"type":"text_input","data":{"protocol":1,"enabled":true,"focus":"window-uuid","purpose":0,"cursor":[100,200,1,18]}}
```

`enabled` 来自当前焦点客户端提交的 text-input-v3 状态。`purpose` 沿用 Wayland content purpose，用于数字、电话、URL、邮箱等键盘布局。`cursor` 使用远端桌面逻辑坐标；无法可靠转换的 popup/subsurface 返回 null。消息不包含 surrounding text、选区内容或输入文字。状态变化和新 DataChannel 建立时发送。

前后端仍接受原来的文字／按键消息。旧客户端可以忽略新增状态；旧服务器下仍可使用手动键盘入口。

## 验证

```sh
CHROME_BIN=/usr/bin/google-chrome node scripts/mobile_input_test.mjs
CHROME_BIN=/usr/bin/google-chrome ELECTRON_BIN=/path/to/electron node scripts/mobile_input_smoke.mjs
```

前者验证生产输入、视口和共享应用模块；后者通过真实 WebRTC 将移动浏览器输入送入 Electron Wayland 编辑框，并覆盖独立共享运行时。Linux Chromium／WebKit 自动化验证不等于 iPhone、iPad 或 Android 真机系统键盘验收。

未设置 `CHROME_BIN` 时使用 Playwright 安装的 Chromium。CI 在 Linux 和 macOS 上运行移动输入交互套件。端到端脚本对 Chromium 强制等待画面；Linux 无头 WebKit 的画面展示可能受运行环境限制，脚本单独输出媒体诊断后继续检查文字链路。设置 `REQUIRE_VIDEO=1` 可对两种引擎都要求实际画面。
