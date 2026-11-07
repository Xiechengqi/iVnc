# 远程桌面应用

iVnc 可以把 B 上正在运行的一个桌面应用添加到 A 的应用列表。用户浏览器连接 A，A 中继 B 的信令、ICE-TCP 媒体和文件传输；B 的应用运行环境编码画面，A 不重新编码。

## 使用

1. 在 B 设置非默认的实例管理密码，启动要共享的桌面应用。
2. 在应用的“远程访问”中启用共享，设置独立的应用密码（8–256 字节），复制连接地址。
3. 在 A 新增“远程应用”，填写连接地址和应用密码，可选填本地名称；“测试连接”会显示目标应用状态。
4. 打开应用即可连接原有进程。本地桌面应用也使用同一套应用视图。

共享默认关闭。密码输入可以明文查看，B 保存 Argon2 校验值，保存后不回显原密码。A 保存加密的连接凭据，应用列表不返回凭据。公网地址必须使用 HTTPS；明文 HTTP 仅接受回环地址和私有 IP 地址，用于可信内网或本地测试。

| 操作 | 行为 |
| --- | --- |
| A 关闭视图或返回列表 | 关闭视图会断开连接；返回列表保留视图并停止其键鼠、剪贴板和声音输出 |
| A 删除远程应用 | 删除连接配置并尝试撤销对应凭据，B 的应用继续运行 |
| A 重启 | 保留连接配置，打开时重新连接，不改变 B 的进程 |
| B 关闭共享或修改应用密码 | 撤销远程凭据，断开远程信令、媒体和下载；B 本地访问保留 |
| B 停止或重启应用 | 旧运行代次的会话失效；应用恢复后用“重新连接”打开新视图 |
| B 的应用未运行 | 返回明确错误，A 不提供远端启停功能 |

同一应用的多个视图共享同一个进程。首个连接取得控制权，其余连接只能观看；控制者可释放控制，B 所有者可接管。服务端限制观看者的输入、剪贴板写入、上传和分辨率修改。切换活动视图会释放按键和鼠标按钮。下载面向所有已授权观看者，文件仅来自应用 HOME 的 `Downloads` 目录。

共享 Chrome 包括它已有的标签页、登录状态和允许访问的后台服务。远程用户在应用内部关闭窗口、退出程序等操作，会实际作用于 B 上的应用。

## 运行与服务隔离

每个桌面应用有自己的 compositor、进程/PID/挂载/网络命名空间、HOME、D-Bus 和 PulseAudio。使用 bubblewrap、slirp4netns 和 nftables；依赖或权限不足时启动失败，不回退到主机桌面。

应用可以读取挂载的系统程序和运行库，可写自己的应用数据。其他应用的数据、主机显示/音频 socket、iVnc 配置与连接凭据不挂载进应用环境。网络阻止私有地址、主机接口地址及未授权回环服务；IPv6 出站当前禁用。若 B 还有不在主机网卡上的 NAT/公网别名，应通过 `IVNC_DESKTOP_DENY_IPS=IP1,IP2` 列出；配置的公网 ICE 地址也会加入禁止列表。网络地址变更后需重启应用以更新规则。内置 Chrome 经 Miao 的 SOCKS5 桥接也检查目的地址，并在桥接端解析为允许的 IPv4 地址，避免代理重新解析到内网。

在“远程访问与应用隔离”中勾选允许访问的后台服务。应用重启后，可以用该服务在 B 的回环地址和端口访问它，例如 `http://127.0.0.1:PORT/`。只有已注册、具有 HTTP(S) URL、监听 B 回环地址的后台服务可授权。iVnc 管理端口和运行环境保留端口不能作为后台服务开放。Chrome 的代理设置会绕过回环地址，允许的服务走专用桥接。

授权后台服务意味着同时授予该服务本身的能力。例如免密码的 Miao 面板仍可更改 B 的代理设置。后台 Web 应用和 CLI 应用不单独提供远程共享。

默认运行原生 Wayland 应用。需要 X11 的应用可勾选“X11 兼容环境”，重启后使用本应用的 Xwayland 和 Openbox。特殊 GPU、宿主机设备或桌面协议依赖仍需逐项适配；当前运行环境不挂载宿主机 GPU、摄像头、麦克风或 USB 设备。

文件上传和下载拒绝目录穿越、符号链接和硬链接。下载为附件响应。关闭视图、共享撤销或凭据失效会取消仍在等待响应或传输中的文件请求。

## 部署与证书

主机需要 Linux 5.6+（文件限制使用 `openat2`）及 user、mount、PID、network namespace 支持。Ubuntu/Debian 额外依赖：

```bash
sudo apt-get install bubblewrap slirp4netns nftables iproute2 dbus-daemon \
  xwayland openbox x11-xkb-utils pulseaudio pulseaudio-utils
bash build.sh --release --features tls
```

Docker 镜像包含这些运行依赖、音频捕获及 TLS。`docker-run.sh` 使用 `seccomp=unconfined` 和 `apparmor=unconfined` 允许应用创建嵌套命名空间；应用内部仍会安装防火墙、丢弃 capabilities 并设置 `no_new_privs`。部署环境若禁止 user namespace，需先解决宿主机限制。容器级安全策略的放宽应纳入部署配置，不能把应用隔离视为虚拟机级隔离。

A 和 B 都只需一个入口端口，但该端口必须支持 HTTP(S)、WebSocket 和原始 ICE-TCP。支持直接 TCP 连接、Docker 端口发布及能透明转发这些流量的四层入口。只转发 HTTP/WebSocket 的反向代理或 CDN 不适用于当前媒体通道。A 必须能访问 B；没有自动 NAT 穿透或反向隧道。

`--tls` 默认在 `$IVNC_HOME/tls/` 首次生成并持久化证书，重启复用。生产环境可指定可信证书：

```bash
IVNC_TLS_CERT_FILE=/etc/ivnc/cert.pem \
IVNC_TLS_KEY_FILE=/etc/ivnc/key.pem ivnc --tls
```

| 环境变量 | 含义 |
| --- | --- |
| `IVNC_TLS_CERT_FILE` / `IVNC_TLS_KEY_FILE` | PEM 证书链与私钥，建议同时指定 |
| `IVNC_TLS_NAMES` | 首次生成证书时附加的 DNS/IP SAN，以逗号分隔；不会修改已存在的证书 |
| `IVNC_REMOTE_CA_FILE` | A 信任 B 的附加 PEM CA/证书集合，同时用于 HTTPS 和 WSS；仍校验证书名称 |

私有证书的示例：在 B 首次启动时设置 `IVNC_TLS_NAMES=b.example`，将其证书通过可信途径复制给 A，A 设置 `IVNC_REMOTE_CA_FILE=/etc/ivnc/b-ca.pem`。用户浏览器访问 A 的 HTTPS 时还需要独立信任 A 的证书。不要复制 B 的私钥给 A。

## 数据与升级

- `apps.db` 保留应用 ID、启动配置、自启动偏好和原目录。
- `apps.before-isolation.db` 是首次启用新运行环境时的配置快照。升级前仍应备份完整数据目录；该文件不包含应用文件数据。
- `remote.db` 保存节点身份、共享策略及加密连接信息；`remote.key` 是本机凭据加密密钥。两者权限为 `0600`，需一起备份。
- `runtimes/<id>/` 保存应用运行代次和内部端点；不是可对外发布的 API。
- 已有桌面应用需重启进入独立环境，未保存的窗口状态不保证保留。自启动设置和持久化数据不重置；共享仍默认关闭。

连接凭据有效期为 90 天；到期后在 A 编辑应用并重新输入密码。单应用最多保存 128 个凭据，测试连接会清理临时凭据。每实例限制 128 个活动应用连接/数据请求和 128 个打开的应用视图；视图最长 8 小时。媒体和信令队列有容量限制，慢连接会关闭，避免无界积压。这些是保护上限，不代表该数量的并发性能保证。

## 验证

先构建当前代码，再明确指定待测二进制：

```bash
export IVNC_BIN="$PWD/target/release/ivnc"
bash scripts/remote_app_smoke.sh
REMOTE_SMOKE_TLS=1 bash scripts/remote_app_smoke.sh
python3 scripts/remote_isolation_smoke.py
IVNC_BIN="$IVNC_BIN" SKIP_BUILD=1 SKIP_UNIT=1 bash scripts/desktop_smoke.sh
```

测试依赖还包括 `gtk-3-examples`、Chrome、Node.js、Python 3、jq 和前端的 `ws` 包（`npm ci --prefix web/ivnc`）。远程测试创建两个临时实例，验证视频/输入中继、上传下载、观看者权限、所有者接管和撤销；隔离测试验证两个桌面应用、受限服务桥接、独立音频服务与 X11 画面。测试不操作已有 iVnc 实例。

Docker 验证入口：

```bash
docker build -t ivnc:remote-review .
IVNC_TEST_IMAGE=ivnc:remote-review bash scripts/remote_app_docker_smoke.sh
```

容器测试在一次性容器中安装测试工具并运行双实例和隔离套件。对外发布端口、不同主机网络和负载容量还需在实际部署环境验证。此次开发环境的 Docker daemon 不可用，未完成真实容器运行验收。
