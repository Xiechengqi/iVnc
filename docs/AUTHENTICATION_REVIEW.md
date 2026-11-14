# Web 登录会话实现与 Review

日期：2026-11-14。基于 `a04e34d`，范围为移动端刷新后的登录保持、认证生命周期及现有桌面功能兼容性。

## 问题与实现

旧实现只校验 HTTP Basic 凭据，网页登录保持依赖浏览器自己的认证缓存。移动浏览器恢复页面、重新建立请求或连接时，缓存不一定继续附带，服务端也没有独立的网页会话。这属于认证设计的兼容性缺口。

浏览器现在通过 `/auth/login` 表单登录，后续页面、API 和 WebSocket 握手使用 HttpOnly Cookie。密码只用于登录，不保存在浏览器存储；服务端只在内存中保存随机会话令牌的 SHA-256 摘要。默认空闲 24 小时、最长 7 天，配置缺省时兼容旧配置。

退出撤销当前会话；改密撤销所有网页会话和旧 Basic 连接。授权状态传入终端、Miao、应用视图及实际 WebRTC 媒体驱动，因此信令已关闭后，退出或超时仍能结束输入和媒体。显式 Basic API、内部 worker 和远程应用 Bearer 认证保留各自的用途。

## 整体 Review

检查了路由与中间件顺序、公开资源范围、默认配置、Cookie 生命周期、CSRF 和完整来源匹配、Basic 兼容、内部 worker 边界、代理凭据过滤、长连接撤销、离线恢复、多标签页和构建产物。

实施与 review 中修正了以下边界问题：

- Cookie 名使用实际请求 Host，包括外部发布端口，避免多个容器共用内部端口时互相覆盖。
- 过期会话和退出标记不回退到浏览器缓存的 Basic 凭据，防止退出后立即恢复旧授权。
- 普通 HTTP IP 来源可能没有 Fetch Metadata，同源 GET 也可能没有 Origin；结合浏览器 User-Agent 和 Cookie 识别，避免在会话失效时重新触发原生 Basic 弹框。
- Cookie 写请求要求同源 Origin 与 CSRF；WebSocket 校验完整来源。Miao 保留自己的 CSRF 协议，管理会话 Cookie 和 CSRF 头不转发到上游，且上游不能设置管理会话 Cookie。
- 信令和媒体分别监听撤销，终端的取消会释放 PTY 会话，应用中继的取消会停止媒体 lease。
- 后台恢复重新检查会话；网络失败不会被误判为退出。BFCache 页面保留跨标签通知能力，存储通道不可用时仍由服务端撤销授权。
- 重连回调等待认证期间若已更换 WebSocket，旧回调不会影响新连接；退出后取消剪贴板、手势和媒体重连。
- 登录页设置根资源基址，修复生产包在 `/auth/login` 下加载相对资源失败的问题。主入口采用模块脚本，确保认证依赖进入正式构建。
- Service Worker 不再缓存或离线重放受保护页面，只清理旧 iVnc 页面缓存；生产包包含 Service Worker、manifest 与图标。
- 浏览器端回归改为真实表单登录，不依靠注入 Basic 请求头；同时修正旧下拉框定位和刷新时误读旧文档的测试竞态。

结合代码审查、真实进程测试与窄屏截图核对，未发现剩余阻断问题。

## 验证结果

| 范围 | 结果 |
| --- | --- |
| Rust 库单元测试，含 9 个认证专项测试 | 157 项通过 |
| 会话集成测试 | 23 项通过；Chromium / WebKit、HTTP / HTTPS、移动视口各连续刷新 10 次，HTTP 另以本机非 loopback IP 完整验证 |
| Cookie 被拒绝、返回地址校验 | 两种浏览器引擎均通过真实表单与导航验证 |
| 退出与过期 | 多标签退出、缓存 Basic 防复活、终端撤销、信令关闭后的媒体撤销、真实共享应用 iframe 和独立中继撤销均通过 |
| 视频、视口与剪贴板状态 | 55 项通过，含等待认证时旧信令回调隔离 |
| 手势识别 | 18 项通过 |
| 原生浏览器工具栏 / 视口交互 | Chromium 和 WebKit 分别通过 31 / 8 项 |
| 真实工具栏 / GTK / 共享应用 | 33 项通过 |
| 真实终端 / PTY | 21 项通过 |
| 远程应用中继 | 64 项通过；B 使用 HTTPS，覆盖独立应用认证、媒体、输入、上传、并行视图及撤销 |
| Miao | HTTP / HTTPS 生命周期、Basic API、CSRF、WebSocket、真实表单控制台与 iframe，以及停止页 Cookie/CSRF 启动均通过 |
| 前后端构建及格式检查 | 通过；前端保留已有的大于 500 KB 产物提示 |

主要复现命令：

```sh
npm --prefix web/ivnc run build
cargo build --features tls --bin ivnc --offline
cargo test --features tls --lib --offline
CHROME_BIN=/usr/bin/google-chrome node scripts/auth_smoke.mjs
node --test scripts/video_playback_test.mjs scripts/viewport_test.mjs scripts/clipboard_test.mjs
node scripts/gesture_navigation_smoke.mjs
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:toolbar
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:viewport
IVNC_BIN=target/debug/ivnc CHROME_BIN=/usr/bin/google-chrome node scripts/toolbar_smoke.mjs
IVNC_BIN=target/debug/ivnc CHROME_BIN=/usr/bin/google-chrome node scripts/terminal_smoke.mjs
CHROME_BIN=/usr/bin/google-chrome python3 scripts/miao_smoke.py --binary target/debug/ivnc --browser
IVNC_BIN="$PWD/target/debug/ivnc" CHROME_BIN=/usr/bin/google-chrome REMOTE_SMOKE_TLS=1 bash scripts/remote_app_smoke.sh
cargo fmt --check
git diff --check
```

设置 `IVNC_TEST_HTTP_HOST` 为本机可达的非 loopback IPv4 地址后运行 `auth_smoke.mjs`，可复测普通 HTTP IP 来源；不设置时默认使用 `127.0.0.1`。

## 使用边界

- 会话保存在内存中，服务重启需要重新登录。网页改密沿用原行为，只作用于当前进程，重启恢复配置中的密码。
- HTTP 访问支持保持登录，传输仍为明文；Cookie 自身没有端口安全隔离。来源与 Secure 属性依据真实连接，不信任转发头。
- 浏览器验证使用 Linux Chromium / WebKit 和移动视口，未宣称完成实体 iPhone、iPad 或 macOS Safari 测试。
- 在线升级只验证未认证请求会被拒绝，没有执行真实升级、部署或远程 CI。
