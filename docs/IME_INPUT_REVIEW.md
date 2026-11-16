# Toolbar 与中文输入：实现和整体 Review

日期：2026-11-16。基础提交：`b004635`。本轮实现和整体 review 已完成，已验证范围内没有剩余阻断问题。使用方式和协议边界见 [桌面中文输入](IME_INPUT.md)。

## 根因与最终实现

- 显示按钮原先不是 primary 工具，窄屏会被收进更多菜单；恢复宽屏只按父节点变化追加按钮，也会打乱顺序。现在“固定 → 适应窗口／铺满窗口 → 中文输入”保持可见和稳定顺序，默认 fit 并保留个人选择，fill 在 hover／focus 时持续高亮。
- `hide(false)` 原先同样关闭 IME，运行状态回调又把关闭写入 storage；刷新、切换应用或断线因此覆盖用户偏好。现在临时停止输入与用户开关分离，只由按钮／快捷键保存偏好。主页面拥有桌面偏好，嵌入应用继承并回传用户操作，不写独立 app-view 偏好。
- 重连原先使用初始化捕获的状态加隐式 toggle，可能重复反转。现在明确设置 desired state，在 load、ready、激活及连接恢复时校准；移动端仍必须通过用户操作唤起键盘。
- 代理只在开启时计算一次光标位置，且 Visual Viewport 更新会把无参数 position 重置到角落。现在持续处理编辑元数据、工作区、视频尺寸、DPR、全屏和可视视口变化；按 fit 留白或 fill 独立比例映射，在没有可靠元数据时保留当前目标的最近位置。
- 当前版本的远程架构已有可用中文通道，不能仅凭现象判定远程应用完全不支持中文。原有链路缺少提交结果，兼容粘贴失败时只有服务端 debug 日志。本轮增加协商后的连接级回执、窗口 UUID 围栏、取消和失败提示；旧服务端保留 `t,`，旧编辑器保留粘贴兼容路径。

## 整体审查

| 审查点 | 结果 |
| --- | --- |
| 偏好生命周期 | 刷新和多次重连不会反转；临时隐藏、弹窗、失焦和应用切换不覆盖开关；显式关闭可保存 |
| 父子视图 | 主桌面、共享和远程视图使用同一桌面偏好；嵌入快捷键更新父页面；来源窗口和 view 路径验证阻止过期状态／偏好污染 |
| 焦点 | 只有当前可输入视图恢复代理；WebKit 返回主页面时显式归还焦点；ready 重复同步不能打断本地设置／终端编辑；后台页面不主动聚焦 |
| 移动端 | 不从桌面偏好自动弹键盘；保留原始点击栈中的同步聚焦、按钮显隐意图和 Visual Viewport 遮挡适配 |
| 光标映射 | fit 计入居中留白；fill 分别计算横纵缩放；位置更新和 resize 不依赖再次开启；清理其他窗口的旧坐标 |
| 原生与兼容提交 | text-input-v3 优先；未启用的应用保留串行粘贴，单字符 ASCII 保留按键；更新前的远程运行时不会收到新协议 |
| 回执语义 | 原生回执仅表示协议已发送；兼容回执表示数据送达，不承诺应用接受编辑；结果不广播，也不含输入文字 |
| 目标与取消 | 目标必须仍为当前窗口；每连接 ID 递增、不可重放；断线／切换取消待处理请求，先到的取消也阻止迟到的同 ID 文本 |
| 队列顺序 | 回执计时从实际发送开始，不把前序剪贴板传输等待计作超时；取消和按键释放不被旧粘贴队列阻挡 |
| 容量和失败 | 最多 32 个待处理文字提交，单次 48 KiB，服务端有效期 2 秒；浏览器发送后等待 3 秒，不盲目重发，晚到回执不会提示到其他应用 |
| 清理与兼容 | 组合输入代次、去重、定时器、监听器、代理 DOM 和断线清理保留；剪贴板队列发送／取消结果可追踪，原有按键消息不变 |
| CI 与实例 | UI 回归全部本地执行，没有添加 Actions 测试入口；使用隔离数据目录、端口与应用，未重启生产 PID 797063，未部署或推送 |

review 中额外修正了 WebKit 对隐藏 iframe 的 focus 保留、子页面 active 消息绕过父页面焦点保护、以及文字排在粘贴后过早超时的边界，并增加对应回归。

## 验证结果

按独立场景计数，不重复计算 Node 父级分组：

| 验证 | 结果 |
| --- | --- |
| Rust 库，含提交过期／取消／重放／连接隔离 | 163 项通过 |
| 桌面 IME 与生产按钮／偏好入口 | Chromium／WebKit 各 11，共 22 项通过 |
| 移动输入 | 两引擎各 25，共 50 项通过 |
| 显示模式与父子视图 | 两引擎各 9，共 18 项通过 |
| Toolbar 交互与应用排序／切换 | 两引擎各 33，共 66 项通过 |
| 独立视口与输入坐标 | 两引擎各 15，共 30 项通过 |
| 剪贴板及文字提交状态逻辑 | 46 项通过 |
| 真实主桌面／共享旧编辑器／A→B Electron | 30 项通过，覆盖原生输入、兼容粘贴、重连、真实光标、目标拒绝、失败提示、延迟粘贴队列和焦点保护 |
| 原有移动端真实 WebRTC→Electron | 22 项通过，含删除、回车、组合去重、同步键盘入口和认证结束 |
| 前端生产构建及含嵌入资源的 TLS 二进制构建 | 通过；保留既有大于 500 KB 的 bundle 提示 |
| Rust 格式、脚本语法和补丁空白 | 通过 |

浏览器交互共 186 个独立场景；最终组合批次的 Node 输出为 214/214（包含 12 个父级分组和 46 项状态逻辑测试），独立视口批次为 33/33（包含 3 个父级分组）。真实实例检查共 52 项，未计入浏览器 fixture 数量。

主要命令：

```sh
cargo test --lib --features tls --offline
npm --prefix web/ivnc run build
cargo build --features tls --bin ivnc --offline
CHROME_BIN=/usr/bin/google-chrome node --test scripts/ime_input_test.mjs scripts/mobile_input_test.mjs scripts/display_mode_interaction_test.mjs scripts/toolbar_interaction_test.mjs scripts/clipboard_test.mjs scripts/text_commit_test.mjs
CHROME_BIN=/usr/bin/google-chrome node scripts/viewport_interaction_test.mjs
ELECTRON_BIN=/path/to/electron IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/ime_input_smoke.mjs
ELECTRON_BIN=/path/to/electron IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/mobile_input_smoke.mjs
cargo fmt --check
git diff --check
```

最终真实实例记录：`/tmp/ivnc-ime-e2e-2nVNFZ`、`/tmp/ivnc-mobile-e2e-0fqNS9`。对应汇总日志为 `/tmp/ivnc-ime-smoke.log`、`/tmp/ivnc-ime-mobile-smoke.log`；浏览器与 Rust 结果为 `/tmp/ivnc-ime-final-tests.log`、`/tmp/ivnc-ime-viewport.log`、`/tmp/ivnc-ime-rust.log`。

## 验证边界

- 实测 Electron 45.3.1、Linux Chromium 和 Playwright WebKit。Chromium 的真实测试确认视频及光标映射；Linux 无头 WebKit 的真实测试未确认视频展示，单独输出诊断，仅计入文字／DataChannel 检查。WebKit 的几何行为由有固定视频尺寸的浏览器 fixture 验证，不等同于 macOS Safari 真机候选窗口验收。
- 没有实体 macOS 外接显示器、中文系统候选窗口或手机／平板键盘验收。操作系统最终如何放置候选窗口仍由设备决定。
- 缺少光标元数据的应用只能使用位置回退；旧服务端没有新回执和目标围栏。旧编辑器仍依赖粘贴能力；应用可以拒绝已经送达的编辑。这些边界没有被记录为已解决。
