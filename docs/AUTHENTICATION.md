# Web 登录与会话

iVnc 的浏览器登录使用服务端会话和 HttpOnly Cookie，用户名和密码继续由 `[http]` 的 `basic_auth_user` / `basic_auth_password` 配置。`--basic-auth-enabled false` 仍关闭外层认证；参数名称为兼容旧部署而保留。

未登录时打开桌面或控制台会进入 `/auth/login`。登录成功后恢复原来的同源地址，普通刷新、应用切换和后台恢复不要求重新输入密码。浏览器禁止 Cookie 时，登录页会说明无法保存登录状态。登录页支持中文、英文、密码管理器自动填充及输入框内的显示／隐藏密码按钮。明文显示状态不持久化，重新加载及页面恢复时隐藏密码。

## 生命周期

```toml
[http]
basic_auth_enabled = true
basic_auth_user = "user"
basic_auth_password = "change-me"
auth_session_idle_seconds = 86400
auth_session_max_seconds = 604800
```

- 默认空闲 24 小时、最长 7 天；合法请求更新空闲时间，最长寿命不会滑动延长。
- Cookie 的有效期为最长寿命。服务端只在内存中保存随机令牌的 SHA-256 摘要和会话元数据，**服务重启后需要重新登录**。
- 每实例最多 64 个浏览器会话，达到上限时撤销最久未使用的会话。密码登录会话不绑定 IP，因此移动网络切换不会结束这类登录；IP 白名单会话绑定实际识别的客户端 IP。
- 底部 toolbar 的齿轮打开“设置”，顶部右侧提供修改密码和退出按钮；认证关闭时隐藏这两个账户操作。退出当前浏览器会话会通知同一地址下的其他标签页，并终止该会话的桌面媒体、输入、应用视图、终端和代理 WebSocket。控制台管理关闭时，精简设置页仍保留账户与服务操作，详见 [设置](SETTINGS.md)。
- 修改密码撤销所有网页会话和旧 Basic 连接。沿用现有改密行为：网页修改只影响当前服务进程，重启后恢复配置文件中的密码；需要永久修改时更新配置并重启。

会话的授权生命周期独立于 WebSocket。即使信令已经关闭，退出、改密或超时仍会终止实际 WebRTC 输入和媒体连接。

## IP 白名单免密登录

“设置 → 高级”提供免密开关与 IP 白名单，默认关闭。每行填写一个 IPv4、IPv6 或 CIDR 网段，例如 `192.168.1.20`、`192.168.1.0/24`、`2001:db8::/64`。匹配访问者来源 IP，获得与当前配置账户相同的管理权限；名单外仍可通过用户名和密码登录。关闭开关可保留规则，启用时至少需要一条规则。

保存会规范化网段、去重并立即生效，持久化到 `$IVNC_HOME/access.json`（默认 `$HOME/.ivnc/access.json`）。最多 256 条不同规则；无效规则使整次保存失败，并标明输入行。文件以 0600 权限原子写入，失败时保留当前策略；启动时文件损坏或无法读取会关闭免密能力，密码认证继续可用。

登录页先检查现有会话，再通过同源 `POST /auth/auto-login` 建立 IP 会话，成功后恢复原访问地址。检查期间不显示密码表单。状态查询不创建会话，有效会话不会重复签发 Cookie。IP 会话继续使用原有 Cookie、CSRF、Origin、空闲／绝对超时及实例隔离机制，`kind` 仍为 `session`，`method` 为 `ip_whitelist`。

IP 会话在每次 HTTP 请求和 WebSocket 握手时核验绑定 IP。来源变化后，仍在名单内可重新自动建立会话，否则要求密码。关闭免密或删除规则会撤销不再匹配的 IP 会话，并终止其信令、实际媒体／输入、应用视图、终端和代理 WebSocket；仍匹配的 IP 会话及密码会话保留。重启会结束内存会话，但保留白名单并允许重新自动登录。

主动退出保留退出标记，即使 IP 命中也不会刷新后自动重入；登录页提供“免密进入”按钮，明确点击后才替换标记并建立新会话。重复 Cookie 被拒绝，免密接口不绕过现有远程应用共享密码或令牌。

### 来源 IP 与可信代理

直连默认使用 TCP 对端 IP，忽略 `X-Forwarded-For`、`X-Real-IP` 等声明。高级页展示最终识别的客户端 IP 和当前匹配状态。

确实由代理转发 HTTP 请求时，可在服务端配置中显式声明代理的来源 IP 或 CIDR：

```toml
[http]
trusted_proxy_cidrs = ["127.0.0.1", "10.0.0.0/24"]
```

只有实际 TCP 对端命中该列表时，才解析单个 `X-Forwarded-For` 头。代理必须覆盖或正确追加来源链；服务器从右向左剥离可信代理，取首个不可信来源。缺失、无效、重复或过长的转发链使来源不可用，该连接不能免密；错误的可信代理配置也会禁用 IP 免密。IPv4 映射 IPv6 地址统一为 IPv4 匹配。

该配置只控制客户端 IP 的识别，不改变实际 TLS、Cookie Secure 或 Origin 校验，也不改变现有媒体入口要求。可信代理配置修改后需重启，网页白名单修改不需重启。

## HTTP、HTTPS 与来源

HTTP IP:端口访问支持会话。Cookie 使用 `HttpOnly; SameSite=Lax; Path=/`；实际 TLS 连接额外使用 `Secure`。Cookie 名根据外部请求的 Host（包含发布端口）区分，避免多个 Docker 实例都监听内部 8007 时互相覆盖。Cookie 本身没有端口隔离，不应把不同端口视为浏览器安全隔离。

HTTP 兼容不会提供传输加密；HTTPS 才会加密登录凭据和会话通信。

不信任客户端提供的 `Forwarded` / `X-Forwarded-Proto` 来决定 Cookie 或来源。来源校验使用实际 HTTP/TLS 连接和 Host，包括协议与端口。此行为适用于直接连接及文档支持的透明四层入口；仅转发 HTTP 的反向代理原本也无法承载 iVnc 的同端口 ICE-TCP 媒体。

Cookie 写请求要求同源 Origin 和 `X-IVNC-CSRF`。浏览器模块自动附加 CSRF 令牌，Cookie 令牌不会暴露给 JavaScript。WebSocket 使用同源 Origin 检查。Miao 的写操作继续采用它自己的 CSRF 协议，外层同时检查来源；iVnc 会话 Cookie 和 CSRF 头不转发给 Miao。

登录、状态、管理 API 和受保护页面使用 `Cache-Control: no-store`。Service Worker 只清理旧 iVnc 页面缓存，不拦截请求、不离线重放页面，也不清理同源其他应用的缓存。

## 兼容性

显式 Basic Auth 继续支持脚本、管理 API、内部 worker 与旧工具，不会自动签发浏览器会话。浏览器缺少会话时通过登录页认证；普通 API 认证失败返回 `401` 和 `X-IVNC-Auth: required`，浏览器请求不收到 Basic challenge，避免弹出原生用户名/密码框。

已过期或主动退出的浏览器 Cookie 不回退到缓存的 Basic 凭据。退出响应保留一个无授权能力的退出标记，新登录会替换它。远程应用的 `/remote/v1/apps/` 保持独立的应用级密码和 Bearer 令牌，不能用网页登录会话替代。

## 接口

| 接口 | 行为 |
|---|---|
| `GET /auth/login?next=/...` | 登录页；登录后的返回地址限于本站，拒绝外站和登录页循环 |
| `POST /auth/login` | JSON `{username,password}`；同源 Origin；成功签发 Cookie 和 CSRF 令牌 |
| `GET /auth/session` | 返回 `enabled`、`authenticated`、`kind` 和当前会话的 `csrf`；不创建会话 |
| `POST /auth/auto-login` | JSON `{manual:false}`；同源 Origin、匹配来源 IP；创建或复用会话。退出标记要求显式 `{manual:true}` |
| `POST /auth/logout` | 同源 Origin、有效会话的 CSRF；撤销当前会话并设置退出标记 |
| `GET /api/settings/access` | 认证后读取免密配置、当前客户端 IP 和匹配结果 |
| `PUT /api/settings/access` | JSON `{enabled,ip_allowlist}`；认证和会话 CSRF 后校验、持久化并应用 |

状态接口另返回 `method`（`password` 或 `ip_whitelist`）、`auto_login_available` 和 `auto_login_suppressed`；未认证时不返回白名单内容。

登录接口限制请求体大小；同一实际来源地址在 15 分钟内失败 8 次后返回 `429`，不锁住其他来源。认证失败不区分未知用户名和密码错误。日志不输出凭据、Cookie 或 CSRF。

## 验证

先构建前端及启用 TLS 的 iVnc，再运行：

```sh
cargo test --features tls --lib web::auth --offline
node scripts/auth_smoke.mjs
```

集成测试使用独立数据目录和真实表单，在 Chromium / WebKit 的移动视口下验证 HTTP/HTTPS 连续刷新、Cookie 属性、无 Basic 头登录、CSRF、WebSocket 来源、跨标签退出、后台恢复、Cookie 拒绝提示、返回地址、共享应用 iframe、密码撤销、会话过期及服务重启。测试不会执行真实在线升级；只验证升级端点的认证拒绝行为。完整回归与 review 记录见 [AUTHENTICATION_REVIEW.md](AUTHENTICATION_REVIEW.md)。
