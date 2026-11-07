# 远程桌面应用

iVnc 可以把 B 上正在运行的一个桌面应用添加到 A 的应用列表。用户浏览器连接 A，A 中继 B 的信令、ICE-TCP 媒体和文件传输；B 的应用运行环境编码画面，A 不重新编码。

## 使用

1. 在 B 设置非默认的实例管理密码，启动要共享的桌面应用。
2. 在应用的“远程访问与应用隔离”中启用共享，设置独立的应用密码（8–256 字节，可用“生成强密码”），复制连接地址。
3. 在 A 新增“远程应用”，填写连接地址和应用密码，可选填本地名称；“测试连接”会显示目标应用状态。
4. 打开应用即可连接原有进程。本地桌面应用也使用同一套应用视图。

共享默认关闭。B 保存 Argon2 校验值，保存后不回显原密码。A 保存加密的连接凭据，应用列表不返回凭据。公网地址必须使用 HTTPS；明文 HTTP 仅接受回环地址和私有 IP 地址，用于可信内网或本地测试。A 不能把自身实例添加为远程应用。

| 操作 | 行为 |
| --- | --- |
| A 关闭视图或返回列表 | 关闭视图会断开连接；返回列表保留视图并停止其键鼠、剪贴板和声音输出 |
| A 删除远程应用 | 删除连接配置并尝试撤销对应凭据，B 的应用继续运行 |
| A 重启 | 保留连接配置，打开时重新连接，不改变 B 的进程 |
| B 关闭共享或修改应用密码 | 撤销所有远程凭据，断开远程信令、媒体和下载；B 本地访问保留 |
| B 修改隔离设置（X11、路径、网段、后台服务） | 不撤销凭据；应用重启前，B 拒绝新的远程连接并提示需要重启 |
| B 撤销某个已保存连接 | 只撤销该凭据并结束使用它的会话；该实例需重新输入密码，其他连接不受影响 |
| B 断开某个会话 | 结束当前会话，对方视图停止自动重连，但可手动重新连接 |
| B 停止或重启应用 | 会话结束并提示应用已停止；应用恢复后用“重新连接”打开新视图 |
| B 的应用未运行 | 返回明确错误，A 不提供远端启停功能 |

A 的应用列表显示远程应用的最近状态：已连接、B 上未运行、B 上需重启应用、需重新输入密码或 B 无法访问。状态在列表刷新时于后台更新，最长缓存 30 秒。

同一应用的多个视图共享同一个进程。首个连接取得控制权，其余连接只能观看；控制者可释放控制，B 所有者可接管。服务端限制观看者的输入、剪贴板写入、上传和分辨率修改。切换活动视图会释放按键和鼠标按钮。下载面向所有已授权观看者，文件仅来自应用 HOME 的 `Downloads` 目录。

共享 Chrome 包括它已有的标签页、登录状态和允许访问的后台服务。远程用户在应用内部关闭窗口、退出程序等操作，会实际作用于 B 上的应用。

## 运行隔离

每个桌面应用有自己的合成器、PID 命名空间、HOME、D-Bus 会话总线和 PulseAudio。隔离级别逐应用设置，修改后重启应用生效：

| 级别 | 文件 | 网络 | 环境变量 | 可共享 |
| --- | --- | --- | --- | --- |
| 严格隔离（默认） | 系统程序只读；应用数据可写；仅额外允许的主机路径可见 | 独立网络命名空间，阻止内网和 B 自身地址，仅额外允许的内网网段可达 | 仅语言、时区和输入法变量 | 是 |
| 主机模式 | 主机文件系统 | 主机网络 | 管理进程的环境（排除 iVnc、显示、音频和会话总线变量） | 否 |

严格隔离使用 bubblewrap、slirp4netns 和 nftables；依赖或权限不足时启动失败，不回退到主机环境，控制台直接显示原因及需要调整的系统或容器设置。原因只从 iVnc 在应用启动前收到的沙箱及辅助进程输出中识别，应用自己的输出和日志文件不参与，因此应用无法伪造。主机模式只需要 bubblewrap，适合需要访问主机文件和内网、且不会共享的应用；iVnc 有 `CAP_SYS_ADMIN` 时（通常是宿主机上的 root），它的权限与以前直接在主机运行时相同。没有时（例如在容器中），主机模式应用在只映射 iVnc 自身用户的 user namespace 中运行，权限因此变小：属于其他用户的文件显示为 `nobody`，root 也不能越过其权限位；不能使用容器的 `NET_ADMIN` 等 capabilities（例如创建 TUN、修改路由）；set-user-ID 程序不提升权限。需要这些权限的应用不能在这种环境下以主机模式运行。

严格隔离的应用及其 worker 还受 iVnc 安装的系统调用过滤约束，过滤一旦安装不能解除：

- 不能创建或进入命名空间，也不能挂载（`unshare`、`setns`、带命名空间标志的 `clone`、`mount` 及新挂载接口）。`clone3` 返回“不支持”，C 库会自动改用 `clone`。
- 拒绝 io_uring、bpf、userfaultfd、keyctl、perf_event_open、syslog、kexec、内核模块、配额、交换分区、系统时间等桌面应用不需要的内核接口，以及 x32 调用方式。
- 32 位 x86 程序（例如 32 位 Wine 程序、Steam 客户端）发起系统调用时被内核直接结束（`SIGSYS`）：过滤按 64 位调用号检查，无法覆盖另一套编号。
- 应用自己的 `/proc` 中，`acpi`、`asound`、`fs`、`kcore`、`keys`、`scsi`、`timer_list` 等内核接口被隐藏，`sys`、`sysrq-trigger`、`irq`、`bus` 只读。

因此依赖自身命名空间沙箱的程序无法在严格隔离中运行：Flatpak、Podman，以及 Chromium 和基于 Electron 的应用（VS Code、Slack、Obsidian 等）。后者启动时报 `No usable sandbox!`，在启动命令中加 `--no-sandbox` 即可，这时由 iVnc 的沙箱隔离应用；内置 Chrome 已使用该参数。允许嵌套命名空间会让应用接触到 nf_tables 等仅管理员可用的内核代码，这是内核提权漏洞最常见的入口，因此不提供开关。主机模式不安装此过滤。

严格隔离的附加设置：

- **允许访问的主机路径**：每行一个，行尾加 `rw` 表示可写。路径在启动时解析符号链接后挂载。包含 iVnc 数据目录、Miao 配置、TLS 私钥的路径，`/`、`/tmp` 本身以及 `/proc`、`/sys`、`/dev`、`/run` 不能共享；`/usr`、`/etc`、`/opt` 等系统目录只能只读。远程控制者同样可以读写这些路径。
- **允许访问的内网网段**：IPv4 CIDR，前缀 8–32。回环、链路本地、组播和运行环境内部网段不能放行；B 自身地址始终禁止，即使落在放行网段内。
- **允许访问的后台服务**：只有已注册、具有 HTTP(S) URL、监听 B 回环地址的后台服务可授权。应用重启后，用该服务在 B 的回环地址和端口访问它，例如 `http://127.0.0.1:PORT/`。iVnc 管理端口和运行环境保留端口不能开放。授权后台服务意味着同时授予该服务本身的能力，例如免密码的 Miao 面板仍可更改 B 的代理设置。

B 自身地址包括网卡地址、配置的公网 ICE 地址、iVnc 最近探测到的公网 IPv4（保存在 `$IVNC_HOME/public-ipv4`），以及 `IVNC_DESKTOP_DENY_IPS=IP1,IP2` 指定的额外地址。网络地址变化后需重启应用以更新规则。IPv6 出站当前禁用。

内置 Chrome 经 Miao 的 SOCKS5 桥接访问网络。桥接用 B 的系统 DNS 解析域名，所有结果都必须是公网地址，再把检查过的 IP 交给 Miao；解析失败的域名直接拒绝。这样 Miao 不会再次解析出不同的地址（DNS 重绑定），应用无法借代理访问 B 的回环或内网服务。代价是 Miao 只能按 IP 和嗅探到的 TLS/HTTP 域名分流；若 B 的系统 DNS 会被污染，应让系统解析经过可信的 DNS（例如 Miao 的 TUN 模式或加密 DNS），否则被污染的域名在 Chrome 中无法正常访问。支持 IPv4 与 IPv6 目标。

应用由启动命令的整个进程树组成：启动命令在一个专用的回收进程下运行，启动器派生进程后退出不会结束应用，最后一个应用进程退出时应用才停止；运行环境自身的辅助进程不计入。意外退出的应用由看护进程重启；运行不足 30 秒即退出视为启动失败，重试间隔从 6 秒起倍增，连续 5 次失败后停止重试并显示为崩溃。运行环境不允许建立沙箱（例如缺少下文的容器参数）时直接停止重试。手动启动时立即失败的应用直接报错，不自动重试。

需要 X11 的应用可勾选“X11 兼容环境”，使用本应用的 Xwayland 和 Openbox。特殊 GPU、宿主机设备或桌面协议依赖仍需逐项适配；当前运行环境不挂载宿主机 GPU、摄像头、麦克风或 USB 设备。

应用日志位于 `apps/<id>/app.log`，以追加方式写入，超过 8 MB 时在下次启动前轮换为 `app.log.1`。

文件上传和下载拒绝目录穿越、符号链接和硬链接。下载为附件响应，传输中 120 秒没有数据会中止。关闭视图、共享撤销或凭据失效会取消仍在等待响应或传输中的文件请求。

## 访问控制与审计

- 应用密码错误按来源限流（IPv6 按 /64 计）：同一来源对同一应用 15 分钟内失败 8 次、对所有应用失败 30 次后暂时拒绝；进行中的尝试先按失败计数，认证成功后扣除。某个来源的失败不会锁住其他来源。同一应用的密码校验串行进行，每次失败额外占用 1 秒，因此不论来源多少，针对单个应用的猜测约为每秒一次。未共享的应用直接拒绝，不消耗密码校验资源。所有来源都经反向代理时会共享同一计数，因此应直接或经四层入口访问。
- B 的“远程访问”面板列出已保存的远程连接（报告的实例名称、来源地址、最近使用时间、在线会话数）和当前会话，可以逐个撤销或断开。
- 认证成功与失败、限流、远程连接、断开、撤销和共享策略变更写入 `$IVNC_HOME/remote-audit.log`（`0600`，超过 4 MB 时轮换为 `.1`）。

## 部署与证书

主机需要 Linux 5.6+（文件限制使用 `openat2`）及 user、mount、PID、network namespace 支持。Ubuntu/Debian 额外依赖：

```bash
sudo apt-get install bubblewrap slirp4netns nftables iproute2 dbus-daemon \
  xwayland openbox x11-xkb-utils pulseaudio pulseaudio-utils
bash build.sh --release --features tls
```

Docker 镜像包含这些运行依赖、音频捕获及 TLS。容器需要以下参数，`docker-run.sh` 已全部包含，且都是 Docker 内置选项，不需要额外文件：

| 参数 | 原因 |
| --- | --- |
| `--security-opt seccomp=unconfined` | Docker 默认 seccomp 禁止没有 `CAP_SYS_ADMIN` 的容器创建命名空间和挂载；系统调用过滤改由 iVnc 安装，见下文 |
| `--security-opt apparmor=unconfined` | Docker 默认 AppArmor 配置禁止挂载 |
| `--security-opt systempaths=unconfined` | Docker 遮蔽或只读覆盖 `/proc/kcore`、`/proc/sys` 等路径；内核只允许新 user namespace 在已有 `/proc` 完全可见时挂载新的 `/proc`，否则沙箱报 `Can't mount proc on /newroot/proc` |
| `--device /dev/net/tun` | slirp4netns 为应用创建隔离网络；Miao 的 TUN 模式也需要 |

不需要 `--cap-add SYS_ADMIN`、`--pid=host` 或 `--privileged`。Podman 用 `--security-opt unmask=ALL` 代替 `systempaths=unconfined`。缺少参数时应用启动失败，控制台显示需要补充的参数。

镜像设置了 `IVNC_SECCOMP=container`：iVnc 启动时为自身及其启动的所有进程（Web 终端、主机模式应用、Miao、应用沙箱）安装系统调用过滤，拒绝 io_uring、bpf、userfaultfd、keyctl、perf_event_open、open_by_handle_at、kexec、内核模块、`reboot`、系统时间等接口，与原先的 `docker/seccomp-desktop.json` 相同，但保留沙箱需要的命名空间和挂载；同时设置 `no_new_privs`，set-user-ID 程序不再提升权限。直接在宿主机上运行 iVnc 时不要设置此变量，否则 root 的 Web 终端也会受这些限制。容器内不经 iVnc 启动的进程（例如 `docker exec`）不受过滤。仍可用 `SECCOMP_PROFILE=<文件> ./docker-run.sh` 指定 Docker seccomp 配置，它必须允许命名空间和挂载。

这些参数放宽的是 Docker 对容器本身的限制：容器内以 root 运行、未进入严格隔离的进程（iVnc 主进程、Web 终端、主机模式应用、Miao）不再受 AppArmor 约束，并能写 `/proc/sys` 和 `/proc/sysrq-trigger`，系统调用过滤也不能阻止这一点。未启用 userns-remap 时容器 root 就是宿主机 root，这些进程因此可以影响宿主机内核，部署上应把容器视为可信。多个租户共用一台宿主机时，应启用 Docker userns-remap 或为每个实例使用独立虚拟机。严格隔离的应用不受影响：它们只读访问自己的内核参数，没有 capabilities，且受上述系统调用过滤约束。

Ubuntu 23.10 及以上的宿主机默认限制非特权进程创建 user namespace（`kernel.apparmor_restrict_unprivileged_userns=1`），容器内的应用会启动失败。`docker-run.sh` 会检测并提示；解决方式是 `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0`（写入 `/etc/sysctl.d` 持久化）或为容器加载允许 userns 的 AppArmor 配置。

严格隔离在沙箱内用 nftables 设置防火墙，宿主机内核需提供 `nf_tables`、`nft_ct` 和 `nft_reject_inet` 模块；常见发行版内核自带并会自动加载，否则在宿主机执行 `sudo modprobe -a nft_ct nft_reject_inet`。

A 和 B 都只需一个入口端口，但该端口必须支持 HTTP(S)、WebSocket 和原始 ICE-TCP。支持直接 TCP 连接、Docker 端口发布及能透明转发这些流量的四层入口。只转发 HTTP/WebSocket 的反向代理或 CDN 不适用于当前媒体通道。A 必须能访问 B；没有自动 NAT 穿透或反向隧道。

`--tls` 默认在 `$IVNC_HOME/tls/` 首次生成并持久化证书，重启复用。生产环境可指定可信证书：

```bash
IVNC_TLS_CERT_FILE=/etc/ivnc/cert.pem \
IVNC_TLS_KEY_FILE=/etc/ivnc/key.pem ivnc --tls
```

| 环境变量 | 含义 |
| --- | --- |
| `IVNC_TLS_CERT_FILE` / `IVNC_TLS_KEY_FILE` | PEM 证书链与私钥，必须同时指定；文件更新后 60 秒内自动加载，无需重启，新文件无效时继续使用旧证书 |
| `IVNC_TLS_NAMES` | 首次生成证书时附加的 DNS/IP SAN，以逗号分隔；不会修改已存在的证书 |
| `IVNC_REMOTE_CA_FILE` | A 信任 B 的附加 PEM CA/证书集合，与系统根证书一起用于 HTTPS 和 WSS；仍校验证书名称；文件更新后下次连接时重新加载 |

私有证书的示例：在 B 首次启动时设置 `IVNC_TLS_NAMES=b.example`，将其证书通过可信途径复制给 A，A 设置 `IVNC_REMOTE_CA_FILE=/etc/ivnc/b-ca.pem`。用户浏览器访问 A 的 HTTPS 时还需要独立信任 A 的证书。不要复制 B 的私钥给 A。

## 数据与升级

- `apps.db` 保留应用 ID、启动配置、自启动偏好和原目录。
- `apps.before-isolation.db` 是首次启用新运行环境时的配置快照。升级前仍应备份完整数据目录；该文件不包含应用文件数据。
- `remote.db` 保存节点身份、共享策略、隔离设置、已保存连接的元数据及加密连接信息；`remote.key` 是本机凭据加密密钥。两者权限为 `0600`，需一起备份。旧版本的数据库在启动时自动补充新字段。
- `runtimes/<id>/` 保存应用运行代次和内部端点；不是可对外发布的 API。
- 已有桌面应用默认使用严格隔离，需要访问主机文件或内网时，在应用的隔离设置中添加路径、网段，或改为主机模式。自启动设置和持久化数据不重置；共享仍默认关闭。

连接凭据有效期为 90 天；到期后在 A 编辑应用并重新输入密码。单应用最多保存 128 个凭据，测试连接会清理临时凭据。每实例限制 128 个活动应用连接/数据请求和 128 个打开的应用视图；没有连接的视图空闲 8 小时后失效，使用中的视图不受时长限制。媒体和信令队列有容量限制：短暂拥塞时丢弃媒体包并在恢复后请求关键帧，持续拥塞 10 秒的连接才会关闭。这些是保护上限，不代表该数量的并发性能保证。

## 验证

先构建当前代码，再明确指定待测二进制：

```bash
export IVNC_BIN="$PWD/target/release/ivnc"
bash scripts/remote_app_smoke.sh
REMOTE_SMOKE_TLS=1 bash scripts/remote_app_smoke.sh
bash scripts/remote_hardening_smoke.sh
python3 scripts/remote_isolation_smoke.py
IVNC_BIN="$IVNC_BIN" SKIP_BUILD=1 SKIP_UNIT=1 bash scripts/desktop_smoke.sh
```

测试依赖还包括 `gtk-3-examples`、Chrome、Node.js、Python 3、jq 和前端的 `ws` 包（`npm ci --prefix web/ivnc`）。远程测试创建两个临时实例，验证视频/输入中继、上传下载、观看者权限、所有者接管和撤销；加固测试验证两种隔离级别、允许路径、启动器进程、崩溃退避、连接管理、限流和证书配置；隔离测试验证两个桌面应用、受限服务桥接、独立音频服务与 X11 画面。测试不操作已有 iVnc 实例。

Docker 验证入口（也可在 GitHub Actions 手动运行 “Remote application runtime smoke”）：

```bash
docker build -t ivnc:remote-review .
IVNC_TEST_IMAGE=ivnc:remote-review bash scripts/remote_app_docker_smoke.sh
```

容器测试在一次性容器中安装测试工具并运行双实例和隔离套件。对外发布端口、不同主机网络和负载容量还需在实际部署环境验证。
