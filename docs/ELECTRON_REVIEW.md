# Electron 原生 Wayland 实现 Review

审查日期：2026-11-12。范围为本轮基于 `1e9a8d1` 的应用启动、窗口管理、Wayland 扩展、中文输入、多 MIME 剪贴板、光标和使用文档。仅支持原生 Wayland，没有加入 X11/Xwayland。配置方法见 [ELECTRON.md](ELECTRON.md)。

本轮已完成约定的实现和回归，未发现未处理的阻断功能问题。确认了一项 Smithay 在弹出菜单销毁后的上游清理日志问题，具体证据和影响见下文。当前结论针对实测运行环境，不表示所有 Electron 发行软件及桌面集成都已兼容。

## 实现与审查结果

| 审查点 | 最终处理 |
| --- | --- |
| 宿主机 X11 会话影响 Electron 自动选择后端 | 本地与独立应用共用环境统一设置 `XDG_SESSION_TYPE=wayland`；提供不覆盖已有命令、环境的 Electron 启动模板 |
| 同进程第二个主窗口被当成对话框 | 仅按 xdg parent / xdg-foreign 分组，使用完整 ObjectId 区分不同客户端的相同 wire ID |
| 默认 Fullscreen、固定尺寸窗口收到桌面尺寸 | 主窗口默认使用 Maximized；首个 configure 延迟到初次 commit，读取 min/max 限制；保存和恢复普通窗口几何 |
| 对话框每次重绘回到中央或抢焦点 | 仅首次有效几何居中；拖动后保留位置；parent 变化双向更新工具栏，parent 销毁后存活子窗口继承祖父或恢复独立入口 |
| 最小化后工具栏入口丢失、背景窗口最小化改变当前焦点 | 保存整个父子组及其层次，保留入口，恢复时父先子后升起；仅在原焦点属于该组时切换焦点；恢复位置约束到当前桌面 |
| 应用取消 close 仍发生焦点跳转 | close 只发送请求，实际销毁后才选择后续焦点 |
| text-input-v3 使用固定 done 序号或未启用实例 | 按实例记录 enable/commit 与最新 serial，只提交到焦点客户端已启用的实例；焦点切换清理状态 |
| IME 回退和后续输入串序、错误使用终端快捷键 | 复用绑定实际焦点、不可变 selection、EOF 确认的 Ctrl+V 事务；后续输入等待交付；内部 IME 事务期限为 2 秒 |
| Electron 激活请求省略可选 serial | 同焦点客户端且最近 5 秒有真实输入时允许省略；有 serial 时校验 seat/序号；token 单次使用、5 秒过期，执行时再次校验来源焦点 |
| 鼠标锁定丢失越界增量 | relative-pointer 传送原始 delta，锁定时停止发送绝对 motion；约束按 region/focus 生效，重置和换组失焦释放 |
| 位图光标、缩放和重连状态 | 常规 SHM ARGB/XRGB 转 PNG，处理预乘 alpha、buffer scale 和热点，支持光标帧回调；重连直接重播缓存；不支持的变换、裁剪或尺寸回默认光标 |
| HTML/PNG 与文本来自不同剪贴板操作 | paste v1 增加可选 MIME 描述，所有表示保存在同一 revision 的不可变 snapshot 中，旧文本请求继续兼容 |
| 远端多 MIME 读取阻塞或混合新旧 selection | 在合成器线程延后请求，使用非阻塞管道并行读取；所有表示结束后一次回传，新 selection/超时/超限/错误丢弃 |
| 图片异步读取导致粘贴与键鼠输入重排 | 同步获取文本/HTML 后等待 PNG File，队列继续保留该次操作及后续输入顺序 |
| 二进制剪贴板开关需要重连才生效 | 运行时广播新能力，提交和实际执行时再次检查开关；旧二进制入口直接进入合成器队列，删除外部 wl-copy 路径 |
| 前端分块内容积压及非法 MIME | 拼接/解码前限制累计 base64 长度；两端校验最多 3 种、白名单、去重、长度；服务端校验 UTF-8 和 PNG 签名，总上限 16 MiB |

输出保持 scale=1，视频和桌面尺寸已经使用物理像素，fractional-scale 的 preferred scale 为 120。不会再次乘本机浏览器 DPR。模板保持应用自己的 sandbox 策略；需要 D-Bus 的应用可自行使用 `dbus-run-session`，Docker 运行依赖提供 `dbus`，不自动启动桌面服务。

## 验证

运行环境：Linux、Electron **45.3.1**、Google Chrome **146.0.7680.153**、Rust **1.94.1**。端到端脚本使用独立 HOME、runtime、端口、浏览器 profile 和应用进程，不连接或重启现有用户实例。

| 检查 | 结果 |
| --- | --- |
| `CARGO_INCREMENTAL=0 cargo test --features tls --lib --bin ivnc -- --test-threads=4` | 库 113、二进制 116 项通过；二者包含共用模块测试，不按独立用例相加 |
| `node --test scripts/clipboard_test.mjs scripts/viewport_test.mjs` | 35 项通过，包含异步 PNG 保序及 MIME 解码边界 |
| `scripts/electron_smoke.mjs` | 27 项通过，实际 Electron → iVnc → WebRTC |
| `scripts/clipboard_smoke.mjs` | 12 项通过，真实 Chrome 文本复制粘贴及事务回归 |
| `scripts/clipboard_ui_smoke.mjs` | 12 项通过，权限、HTTP/HTTPS、iframe、手动复制及生命周期 |
| `scripts/window_ui_smoke.mjs` | 20 项通过，包含模板填入、不覆盖命令/环境、选中程序路径 |
| `scripts/viewport_smoke.mjs` | 14 项通过 |
| `scripts/toolbar_smoke.mjs` | 28 项通过 |
| `scripts/desktop_smoke.sh` | 56 项通过 |
| `scripts/remote_app_smoke.sh` | 64 项通过，实际 GTK、独立运行环境和双实例中继 |
| 前端生产构建、带 TLS 的服务端构建、格式与补丁空白检查 | 通过 |

Electron 专项覆盖自动选择 Wayland、多主窗、固定窗、最小化恢复、普通/最大化窗口进出全屏、连续 Unicode 与大文本 IME、原生菜单、HTML/PNG 双向复制、重连光标、鼠标锁定越界增量、文件上传与应用读取、GTK 原生文件选择器打开/Esc 取消。配套 C 客户端覆盖 Electron 未发送的显式 parent、窗口组恢复、拖动重绘、缩桌面后恢复、跨客户端相同 wire ID、解除/恢复 parent 及 parent 销毁。

全屏验证等待实际 Wayland configure/ack/commit 和普通几何提交，避免 Electron API 提前报告状态导致误判。菜单验证包含 configure、确认、帧回调、浏览器视频像素变化、Esc 关闭回调和画面恢复；PNG 粘贴先确认 renderer 收到含 Files 的原生 paste 事件，再读取 clipboard 检查内容。重连光标在新通道打开后 1.2 秒内验证，不依赖周期广播。

Rust 管道测试曾在同时运行多个浏览器、默认测试并发时触发一次既有 3 秒超时；单独复测及限制为 4 线程的完整库/二进制测试均通过，保留原超时与断言。

## 已确认的上游边界

**Electron 父子窗口。** Electron 45.3.1 的 Linux `BrowserWindow.parent` 实现在 X11 分支设置父子关系；实测 Wayland 请求没有 `xdg_toplevel.set_parent`。因此未上报 parent 的 Electron modal 窗口保留独立入口。无法根据同进程关系区分 modal 与第二个主窗口；实际发送 parent 的原生协议客户端已验证正确分组。

**菜单销毁日志。** 固定依赖 Smithay `3d3f9e359352d95cffd1e53287d57df427fcbd34` 会在 `xdg_popup.destroy → xdg_surface.destroy → wl_surface.attach(null)/commit` 清理顺序下输出 `surface missing from known popups`。`popup` 的 destroyed 回调已移除 known_popups 并重置角色，先前注册的 `PopupSurface::pre_commit_hook` 仍在最后一次空缓冲提交中检查 parent，因而产生该日志。实测菜单已收到 configure/ack、提交缓冲和帧回调，并在 WebRTC 视频中出现；Esc 后关闭回调和画面恢复正常，后续输入、剪贴板及窗口测试继续通过，没有协议断线。当前保留原诊断，后续可随 Smithay 钩子生命周期修复更新依赖。

**渲染及桌面服务。** 当前 Pixman/SHM 软件渲染，不包含 DMA-BUF/EGL 零拷贝、硬件 WebGL、portal 屏幕共享、Secret Service、通知或系统托盘服务。D-Bus 为按应用选择的启动配置；实际 Electron 软件的专属服务、AppImage 依赖和 GPU 要求仍需分别验证。

**光标、文件与浏览器。** 位图光标仅覆盖普通变换、无裁剪的 ARGB8888/XRGB8888 SHM，逻辑尺寸上限 128×128。文件选择器访问服务端文件系统；已验证上传文件由 Electron 读取以及选择器取消，没有实现合成文件拖放或把文件 URI 转为本机文件读取。没有对 Firefox、Safari 或所有 Electron 应用作兼容保证。

**构建与部署。** Docker 完整镜像和容器运行未在本轮验收；新依赖声明的 MSRV 已核对不高于 Rust 1.88，实际完整构建使用 Rust 1.94.1，未额外完成 1.88 全量构建。开发验证完成不等于已部署到现有实例。
