# 图片剪贴板 Review

审查日期：2026-11-18。范围包括 Wayland 应用图片读回、浏览器复制辅助面板、实例策略及 worker 同步、远程转发、较大剪贴板传输与本地回归。实现、整体审查和回归已完成，未留下已知的阻断问题。

## 根因与最终行为

原实现已有 PNG 协议，但图片同步默认关闭；浏览器旧缓存可覆盖实例设置，独立应用 worker 也没有持续继承主实例的图片策略。普通 HTTP 的兼容写入只能处理文本，缺少图片手动路径，混合内容降级时还可能将文本成功当成整份内容成功。这些属于策略设计缺口和处理缺陷；普通 HTTP 缺少现代图片写入 API 属于浏览器限制。

未配置图片策略时默认开启，明确保存的 false 继续生效。PNG 校验后保持原始字节，JPEG/WebP 转为 PNG。HTTPS/可信 localhost 使用真实 ClipboardItem 写入；受用户激活或权限限制时保留原图并允许点击重试。HTTP 提供浏览器原生图片预览菜单、触屏长按和下载入口，文本降级会明确说明图片尚未复制。下载成功不等于复制成功。

## 审查发现及处理

| 审查点 | 结果 |
| --- | --- |
| 页面切走时丢失晚到图片 | 当前视图已发起的复制或已收到 receiving 的图片，允许在 30 秒内接收对应结果，仅作为手动快照；返回页面不自动覆盖 OS 剪贴板。切换应用、断线、本地新复制/粘贴和关闭提示取消相关等待 |
| 图片菜单复制后立即粘贴旧内容 | receiving 为当前图片建立等待关系；接收完成前不使用旧快照。明确失败终止等待并保留原因，再次粘贴仍不回放旧 OS 内容 |
| 接收错误静默丢弃图片 | 超时、体积、尺寸、格式、解码、读取和繁忙均有状态；一个 MIME 请求失败时整份快照失败，不报告文本完整成功 |
| 同步关闭时已有解码还在运行 | 发送前再次检查策略，丢弃图片并先说明图片同步已关闭，随后才能发送剩余文本 |
| WebKit 点击复制/下载没有反应 | focusin 会发布状态；重设相同的按钮/链接文字会替换文本节点，WebKit 因此取消本次点击。改为文字有变化时更新，真实鼠标点击回归通过 |
| 数 MB 图片截断 | 原队列容量为 256 个单独消息，逐片广播会丢掉较大 selection 的后半部分。改为整批排队并共享数据，以每连接字节预算限制积压 |
| DTLS receive queue full | TCP 一次读入多条记录时，原驱动连续 handle_input 到整段读完才 poll_output。现逐 TCP 帧处理输出，数 MB 双向图片和大段 Unicode 实测通过 |
| DataChannel 返回未接收却当成发送成功 | 保留 write 的布尔结果，分块和 paste_ack 等控制回复在缓冲可用时发送，未被接收就保留重试 |
| 旧客户端覆盖实例策略 | SETTINGS 不再接受 enable_binary_clipboard；控制台合并保存字段，成功持久化后应用运行时设置，并串行处理更新 |
| worker 与远程来源的设置边界 | 新 worker 使用主进程当前策略，已有 worker 通过认证 loopback 接口更新；启动中或暂不可用的 worker 定期重试。远程应用使用来源实例策略，不修改来源或接入实例设置 |

## 资源、权限及生命周期

- 图片边长最多 8192、像素数最多 16,777,216、解码分配限制 64 MiB；传输体最多 16 MiB。解码在一个后台线程执行，最多另有一个排队任务，不阻塞合成器。
- 完整 start/data/finish 批次以 Arc 共享，每连接控制队列最多 256 个批次、32 MiB 编码内容。达到积压限制时结束拥塞连接；不会无限缓存或继续发送不完整的 selection。待发 DataChannel 缓冲限制为 256 KiB。
- worker 策略接口只在 worker 内生效，要求其认证；app-view 和远程共享代理均不放行该路径。客户端旧设置不能改变所有者策略。
- 预览使用本视图的 PNG Blob URL，不将图片放入 localStorage、父 iframe、HTML 渲染或诊断日志。替换、关闭、断线、销毁会撤销 URL。
- 不增加读取本机剪贴板来验证复制的权限请求。浏览器已经受理的异步写入无法可靠取消；过期回调不会清除新提示或触发旧内容重播。

## 验证

所有浏览器测试均在本地执行，使用独立实例、临时目录、浏览器 profile 和生成的数据。没有更改 GitHub Actions UI 测试范围。

| 检查 | 结果 |
| --- | --- |
| `cargo test --features tls --lib` | 222 项通过，含 PNG 原字节、JPEG/WebP 转换、错误/尺寸限制、策略与批次字节预算 |
| `node --test scripts/clipboard_test.mjs` | 53 项通过 |
| `node scripts/clipboard_ui_smoke.mjs` | 24 项通过，含真实 HTTPS 图片写入后原生粘贴像素、HTTP 预览/下载和 Blob URL 清理 |
| `CHROME_BIN=/usr/bin/google-chrome node scripts/clipboard_image_browser_test.mjs` | 两引擎各 3 个兼容场景通过；Chrome 原生 PNG 粘贴子项通过；WebKit 原生粘贴子项显式跳过，说明见下文 |
| `IVNC_BIN=target/debug/ivnc node scripts/clipboard_image_smoke.mjs` | 9 项通过，覆盖主窗口双向数 MB PNG、普通 HTTP、真实标签切换晚到内容、worker 策略更新/重启和数 MB 图片远程中转；检查尺寸及首尾像素 |
| `IVNC_BIN=target/debug/ivnc node scripts/clipboard_smoke.mjs` | 原有 14 项完整文本链路通过；全屏测试使用真实点击建立鼠标锁定，适配当前横滑导航默认不自动锁定的行为 |
| `CHROME_BIN=/usr/bin/google-chrome node scripts/settings_interaction_test.mjs` | Chromium/WebKit 各 24 个场景通过 |
| 前端生产构建、`cargo build --features tls --bin ivnc` | 通过，先构建前端再嵌入服务端；保留既有的大于 500 kB bundle 提示 |
| 新增 Rust 代码格式、`git diff --check` | 通过 |

当前 Linux 无头 WebKit 环境在最小独立 Clipboard API 页面也出现 write 成功、原生 paste 事件为空的现象；没有伪造 paste 事件来制造通过结果。该环境验证了真实点击、真实写入 API 调用成功、预览、原生菜单未被阻止及下载，OS 原生图片粘贴由 Chrome 完整验证。macOS Safari/iPad 真机的图片复制和原生菜单仍需设备验证，本记录不将 Linux WebKit 等同于 Safari 真机结果。
