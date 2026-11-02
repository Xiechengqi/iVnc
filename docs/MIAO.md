# Miao 后台应用

Miao 以内置后台应用 `builtin-miao` 出现在控制台应用列表，可启动、停止、重启、查看日志、打开代理配置，并可在运行期间编辑自启动选项。首次注册默认自启动，后续启动和升级保留用户选择。自启动指随 **iVnc 服务启动**；宿主机开机启动仍由 iVnc 的 systemd 服务或 Docker restart 策略提供。

应用列表与 `/api/proxy/status`、`/api/proxy/start`、`/api/proxy/restart` 共用同一个进程管理器。应用运行状态表示 Miao 面板进程；sing-box 内核是否运行由 Miao 自己保存和恢复，两者独立。Miao 意外退出后由 iVnc 自动恢复，手动停止后保持停止；打开 `/proxy/`、连接 WebSocket 均不会隐式启动。关闭自启动后，即使五分钟内重启 iVnc，也不会通过旧的应用运行状态恢复 Miao。

节点、订阅、代理模式和内核运行偏好继续由 Miao 面板管理。iVnc 默认以 `--nopasswd` 启动 Miao，打开代理配置即可进入面板，无需再次输入 Miao 密码；原有密码文件保留，独立普通启动 Miao 时仍然有效。iVnc 只允许修改此内置应用的自启动选项，不允许替换启动命令、删除应用或通过通用“清除数据”删除代理配置。管理员的 `ui_features.proxy_enabled=false` / `--proxy-enabled false` 优先于自启动和手动启动。

## 目录与端口

| 路径/变量 | 用途 |
|---|---|
| `$HOME/.miao` | 现有 Miao 配置、认证、缓存、内核运行偏好；保留真实 HOME |
| `$IVNC_HOME/miao/miao-rust-linux-amd64` | iVnc 提取的配套 Miao 可执行文件 |
| `$IVNC_HOME/miao/miao.log` | 面板和内核日志，应用列表提供末尾日志 |
| `$IVNC_HOME/miao/runtime` | 此 iVnc 实例的 sing-box 及临时配置 |
| `$IVNC_HOME/miao/chrome-proxy` | 根据实时内核状态生成的 Chrome 启动代理参数 |
| `IVNC_MIAO_PORT` | 面板内部回环端口，默认 6161；不改写 Miao 已保存的面板端口 |

同一份 `$HOME/.miao` 只能由一个 iVnc 实例管理。部署多个实例时，应分别设置 HOME、IVNC_HOME 和端口。Miao 原有配置路径兼容规则保持有效，旧的二进制目录配置或 `/etc/miao/config.yaml` 可继续被识别。

嵌入面板仅监听 `127.0.0.1`，浏览器经 `/proxy/` 访问，外部访问由 iVnc 的认证配置控制。网页自动建立 Miao 会话，保留 Cookie、同源、CSRF 和 WebSocket 校验；隐藏修改密码和退出登录按钮，初始化及改密接口禁用，只读状态接口不创建会话。嵌入模式允许同源 iframe，独立运行仍使用 `DENY`。嵌入模式不打开首次配置浏览器，升级随 iVnc 分发；面板自身的升级入口被禁用。Chrome 新启动时读取实际运行内核的 SOCKS 地址，支持用户自定义端口；已有 Chrome 进程需重启才会应用新地址。

VPS 部署调用运行环境中的 OpenSSH 客户端。Docker 运行时已安装 `openssh-client` 并在构建时执行 `ssh -V`；直接部署二进制时也需安装该软件包。若旧镜像出现 `SSH client unavailable`，需更新或重建镜像，仅更新嵌入二进制不会补齐系统依赖。

## 配套构建与发布

此功能需要 **iVnc 与 Miao 同时更新**。Miao 必须提供嵌入协议 v2，支持 `--nopasswd` 与 `password_required` 状态字段；`miao-rust --ivnc-integration-version` 应输出 `2`。iVnc 构建时和运行就绪检测时都会验证协议，就绪状态还要求 `password_required: false`，拒绝旧二进制。

本地推荐把两个仓库并列检出：

```bash
# iVnc/ 和 miao/ 位于同一父目录
bash build.sh --release

# 或只构建配套 Miao 后直接使用 cargo
bash scripts/build-miao.sh --release
IVNC_MIAO_BIN="$PWD/target/release/miao-rust" cargo build --release
```

`MIAO_SOURCE_DIR` 可指定其他 Miao 源码目录，`CARGO_TARGET_DIR` 可指定共享产物目录；Miao 的 `embedded/` 内核和规则文件须按 Miao 自身构建说明准备。已有配套产物时，设置绝对路径 `IVNC_MIAO_BIN` 可跳过 Miao 源码构建。构建脚本不会重启现有服务。

纯 cargo、Docker 与 CI 默认获取 Miao 的 latest 发布产物，并校验协议。发布时应先发布包含协议 v2 的 Miao，再构建 iVnc。可用 `IVNC_MIAO_URL` 指定配套版本的下载地址；CI 使用同名仓库变量，Docker 使用同名 build arg。`IVNC_REFRESH_MIAO=1` 刷新本地缓存。若 latest 尚未包含该接口，构建会明确失败，应使用本地 `IVNC_MIAO_BIN`，避免把不兼容面板打包进去。

## 验证

```bash
# 后端回归
IVNC_MIAO_BIN=/absolute/path/to/miao-rust cargo test --features tls
# 独立 HOME、数据目录及端口；测试 HTTP 与 HTTPS（需要 tls feature）
python3 scripts/miao_smoke.py --binary /absolute/path/to/ivnc
# 可选：另验证控制台编辑和认证 iframe，需要 Chrome、Node 20+ 及 web/ivnc 的 npm ci
python3 scripts/miao_smoke.py --binary /absolute/path/to/ivnc --browser
```

隔离测试只启动面板，验证默认自启动、同一 PID、免密码自动进入、认证/CSRF/WebSocket、原密码与配置保留、停止不复活、崩溃恢复、重启后的自启动偏好和管理员总开关，不启动 TUN。浏览器测试从没有 Miao Cookie 的状态打开实际 iframe，检查自动进入和密码按钮隐藏。通用 `desktop_smoke.sh` 显式禁用代理，以免接触运行环境中的代理配置。

本次实施的审查发现、修复与验证记录见 [实施 Review](MIAO_REVIEW.md)。
