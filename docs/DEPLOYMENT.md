# Deployment Guide

本文档描述如何在 Linux 主机上部署 iVnc。

## 系统要求

- Linux（内置 Smithay Wayland 合成器，无需外部 X11/Wayland）
- Rust 1.75+ 工具链
- GStreamer 1.0+
- PulseAudio 或 PipeWire（音频捕获）
- 硬件编码器驱动（可选）

桌面应用是 Wayland 客户端；开启远程访问的应用在独立运行环境中运行，其私有音频使用
`pulseaudio pulseaudio-utils`，不需要额外的内核功能或权限。完整边界与用法见
[远程桌面应用](REMOTE_APPS.md)。

## 浏览器剪贴板

推荐通过浏览器认可的 HTTPS 连接访问。普通 HTTP 域名或 IP 页面通常不提供
`navigator.clipboard.writeText()`；可信 localhost 是例外。这个限制影响远端复制到本机，
用户原生粘贴到 iVnc 使用另一条路径，因此可以出现“粘贴正常、自动复制受限”。

自动写入受限时，iVnc 提供“复制到本机”点击重试及“手动复制”文本面板。
HTTP 页面可在面板中使用 Ctrl/Cmd+C 或右键菜单完成原生复制。修改网站权限不能让
非安全上下文获得异步 Clipboard API；浏览器要求用户手势时，可使用点击重试。

嵌入其他页面时，检查上层页面及反向代理的 `Permissions-Policy`，以及 iframe 的
`allow` 属性。iVnc 自带的应用 iframe 已声明 `clipboard-read; clipboard-write`，
但 iframe 声明不能扩大上层页面禁止的权限。按实际嵌入来源委派写入权限；复制功能
自身不需要额外授予剪贴板读取权限。浏览器不支持权限查询时，客户端会保留未知状态，
不会把查询失败认定为用户拒绝。

复制状态、焦点和兼容边界见 [剪贴板与粘贴事务](CLIPBOARD.md)。

## 依赖安装

### Ubuntu/Debian

#### 编译依赖

```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    pkg-config \
    cmake \
    curl \
    ca-certificates \
    libx11-dev \
    libxcb1-dev \
    libxkbcommon-dev \
    libssl-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev \
    libpulse-dev \
    libopus-dev \
    libwayland-dev \
    libpixman-1-dev \
    libinput-dev \
    libudev-dev \
    libseat-dev
```

#### GStreamer 运行时

```bash
sudo apt-get install -y \
    gstreamer1.0-tools \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly \
    gstreamer1.0-x
```

#### 音频支持

```bash
# PipeWire（推荐）
sudo apt-get install -y pipewire pipewire-pulse pipewire-media-session

# 或 PulseAudio
sudo apt-get install -y pulseaudio
```

#### Miao VPS 部署

Miao 在本机调用 OpenSSH 客户端，源码或独立二进制部署也需安装；Docker 配套镜像已包含此依赖。

```bash
sudo apt-get install -y openssh-client
ssh -V
```

#### 硬件加速（可选）

```bash
# Intel VA-API
sudo apt-get install -y gstreamer1.0-vaapi libva-dev intel-media-va-driver-non-free

# NVIDIA NVENC（需要 NVIDIA 驱动）
sudo apt-get install -y gstreamer1.0-plugins-bad
```

### Fedora/RHEL

```bash
sudo dnf install -y \
    gcc cmake \
    pkg-config \
    libX11-devel \
    libxcb-devel \
    libxkbcommon-devel \
    openssh-clients \
    openssl-devel \
    gstreamer1-devel \
    gstreamer1-plugins-base-devel \
    pulseaudio-libs-devel \
    opus-devel \
    wayland-devel \
    pixman-devel \
    libinput-devel \
    systemd-devel \
    libseat-devel \
    pipewire pipewire-pulseaudio
```

### Arch Linux

```bash
sudo pacman -S --needed \
    base-devel cmake \
    libx11 libxcb xcb-util \
    libxkbcommon openssl \
    openssh \
    gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad \
    libpulse opus \
    wayland pixman libinput \
    pipewire pipewire-pulse
```

## Smithay 依赖

iVnc 依赖本地 smithay 仓库（需放在项目同级目录）：

```bash
git clone https://github.com/Smithay/smithay.git ../smithay
cd ../smithay && git checkout 3d3f9e359352d95cffd1e53287d57df427fcbd34
```

## 编译

```bash
cd /path/to/iVnc

# 使用 build.sh（推荐）
bash build.sh --release

# 或先构建配套 Miao，再直接使用 cargo
bash scripts/build-miao.sh --release
export IVNC_MIAO_BIN="$PWD/target/release/miao-rust"
cargo build --release
```

### 带硬件加速

```bash
# Intel VA-API
cargo build --release --features vaapi

# NVIDIA NVENC
cargo build --release --features nvenc

# Intel Quick Sync
cargo build --release --features qsv
```

构建产物位于 `target/release/ivnc`，`build.sh` 会自动复制到项目根目录。

`build.sh` 默认从相邻的 `../miao` 构建配套面板，也可设置 `MIAO_SOURCE_DIR` 或提供 `IVNC_MIAO_BIN`。Docker / CI 下载的 Miao 发布产物必须支持嵌入协议 v2（`--nopasswd`）；发布顺序和端口、持久化说明见 [Miao 后台应用](MIAO.md)。

## 安装

```bash
# 复制二进制
sudo cp target/release/ivnc /usr/local/bin/

# 复制配置文件
sudo cp config.example.toml /etc/ivnc.toml

# 按需编辑配置
sudo vi /etc/ivnc.toml
```

## 配置

编辑 `/etc/ivnc.toml`：

持久化数据默认写在 `$HOME/.ivnc`（可用环境变量 `IVNC_HOME` 覆盖），包括 apps.db、
Chrome profile、代理面板、上传目录等。Wayland/Pulse 运行时仍使用系统 `XDG_RUNTIME_DIR`。
首次启动会从旧路径（`~/.config/ivnc`、`/tmp/ivnc-apps`）自动迁移。

### 基础配置

```toml
[display]
width = 1920
height = 1080
refresh_rate = 60

[http]
host = "0.0.0.0"
port = 8008
basic_auth_enabled = true
basic_auth_user = "user"
basic_auth_password = "change_me"

[encoding]
target_fps = 30
max_fps = 60
```

### WebRTC 配置

```toml
[webrtc]
enabled = true
tcp_only = true
video_codec = "h264"
video_bitrate = 8000
video_bitrate_max = 16000
video_bitrate_min = 1000
hardware_encoder = "auto"
keyframe_interval = 60
candidate_from_host_header = true
# 公网部署时设置外部地址
# public_candidate = "1.2.3.4:8008"
```

### 音频配置

```toml
[audio]
enabled = true
sample_rate = 48000
channels = 2
bitrate = 128000
```

完整配置示例请参考 `config.example.toml`。

## 启动服务

### 手动启动（前台）

```bash
# 确保 PulseAudio/PipeWire 可用
export XDG_RUNTIME_DIR=/run/user/$(id -u)

# 启动 PipeWire（如未自动启动）
pipewire &
pipewire-media-session &
pipewire-pulse &

# 启动 iVnc
ivnc -c /etc/ivnc.toml --verbose
```

### Systemd Service

创建 `/etc/systemd/system/ivnc.service`：

```ini
[Unit]
Description=iVnc Wayland Desktop Streaming Server
After=network.target pipewire.service

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/ivnc -c /etc/ivnc.toml
Restart=always
RestartSec=5
Environment="XDG_RUNTIME_DIR=/run/user/0"

[Install]
WantedBy=multi-user.target
```

启用并启动：

```bash
sudo systemctl daemon-reload
sudo systemctl enable ivnc
sudo systemctl start ivnc
```

## Docker 部署

正式镜像以 Ubuntu 24.04 LTS 为运行时，内置 Smithay headless 合成器，不需要
外部 X11/Wayland 服务。在仓库目录中运行：

```bash
./docker-run.sh
```

`docker-run.sh` 支持以下环境变量：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `PORT` | `8008` | 宿主机 HTTP/WebRTC 端口 |
| `NAME` | `ivnc` | Docker 容器名 |
| `IMAGE` | `ghcr.io/xiechengqi/ivnc:latest` | 镜像引用 |

```bash
PORT=9000 NAME=ivnc-noble IMAGE=ghcr.io/xiechengqi/ivnc:noble ./docker-run.sh

# 指定 VPS 并启用 Miao 全局代理
./docker-run.sh --proxy-ip 203.0.113.10
```

`docker-run.sh` 的位置参数会原样附加在镜像名后，作为 iVnc CLI 参数。直接执行 Docker 也可使用：

```bash
docker run -d --name ivnc --restart unless-stopped \
  -p 8008:8008 --cap-add NET_ADMIN --device /dev/net/tun \
  -v "$PWD/ivnc-data:/root/.ivnc" -v "$PWD/miao-data:/root/.miao" \
  -v /path/to/ivnc-ssh:/root/.ssh \
  ghcr.io/xiechengqi/ivnc:latest --proxy-ip 203.0.113.10
```

将示例 IP 与 SSH 目录替换为实际值。该目录应包含运行账户可用的密钥，且可写以保存 `known_hosts`。`--proxy-ip` 使 Miao 以 `--nopasswd --mode global --vps-host <IP>` 启动；SSH 默认 root/22、使用现有密钥或 agent。参数支持 IPv4/IPv6；`--proxy-autostart true|false` 控制本次自启动，`--proxy-enabled false` 完全禁用 Miao。启动偏好与重启行为见 [Miao 后台应用](MIAO.md)。

镜像名后不带 iVnc 参数时保持默认启动；自定义前台命令仍可使用 `IMAGE sleep infinity`。同时传 iVnc 参数与前台命令时，使用 `IMAGE --proxy-ip <IP> -- COMMAND ...`。

脚本会创建并持久化仓库目录下的 `ivnc-data` 和 `miao-data`。miao 的全局和进程
代理模式会创建 TUN，因此宿主机必须提供 `/dev/net/tun`，容器也需要脚本中授予的
`NET_ADMIN`。代理池端口如需从容器外连接，需要按实际端口范围扩展脚本的发布参数。

桌面应用及其远程访问不需要额外的 `--security-opt`、`SYS_ADMIN` 或 `--privileged`。
HTTP(S)、信令和原始 ICE-TCP 共用发布端口，因此纯 HTTP 反向代理不能替代透明 TCP 入口。
证书与私有 CA 配置见 [远程应用部署](REMOTE_APPS.md#部署与证书)。

### 镜像版本与回滚

- `latest`：随主分支滚动更新
- `noble`：随主分支滚动更新，明确使用 Ubuntu 24.04 LTS 运行时
- `sha-<提交短哈希>`：不可变构建，用于精确部署和回滚

回滚时可将 `IMAGE` 设置为此前的提交标签，也可以使用 registry 中记录的镜像摘要。
Dockerfile 还提供 `RUNTIME_IMAGE` 和 `EXPECTED_RUNTIME_VERSION` 构建参数，便于在发现
Noble 系统级回归时构建临时的旧运行时镜像。

独立发布二进制仍在 Ubuntu 22.04 容器中构建，以保持较低的 glibc 兼容基线；这不影响
Docker 镜像使用 Ubuntu 24.04。正式镜像和发布二进制保留默认 PulseAudio feature，
并启用 `tls` feature；音频仍需在配置中启用，HTTPS 运行时使用 `--tls`。

GitHub Actions 的独立二进制构建容器使用 `--network host`，但共享网络命名空间并不保证
容器使用与宿主机相同的 `/etc/resolv.conf`。构建前，`scripts/ci_container_dns.sh` 在相同
Ubuntu 22.04 镜像和网络模式中依次测试 runner 的 `/etc/resolv.conf`、systemd-resolved
上游配置；它保留解析器地址，并设置有限的查询超时和重试。只有这些配置失败后，才分别
测试公共 DNS `1.1.1.1`、`8.8.8.8`，不假设 runner 允许访问公共 DNS。

预检覆盖 APT、Node.js、npm、Rust、Cargo 和 GitHub 的依赖域名。首个通过预检的配置会
只读挂载到实际构建容器的 `/etc/resolv.conf`；如果全部失败，则在启动 APT 前终止。
日志会显示候选解析配置、失败域名及最终选择，镜像拉取和容器启动错误也会单独报告。
镜像拉取由 Docker daemon 执行，因此拉取成功不能证明容器内 DNS 可用；如果预检失败，
应根据日志检查 runner 的解析器和 DNS 出站访问。

容器继续使用 Azure Ubuntu 镜像源和 Ubuntu 安全更新源，APT 索引更新启用重试及
`--error-on=any`，索引下载失败时立即终止构建，防止使用不完整索引继续安装。

Docker 镜像发布由另一个 job 中的 BuildKit 执行，不会继承独立二进制构建容器的 DNS
挂载。该 job 单独在 Ubuntu 24.04 容器中预检 Ubuntu、Debian、npm、Cargo、GitHub
及 Chrome 下载域名，将通过的解析器写入 BuildKit 的 `[dns]` 配置。BuildKit 容器与
构建中的 `RUN` 指令均使用 runner 的 host 网络，让本地 DNS stub 可达；首次构建和
两次重试采用同样的配置。这些设置仅作用于 CI 构建网络，不改变发布镜像的运行网络。
需要手动运行 DNS 预检时，可用 `IVNC_DNS_PROBE_HOSTS` 指定一行以空格分隔的域名。

Dockerfile 的 Debian 编译阶段和 Ubuntu 运行时阶段均要求 APT 索引完整更新后再安装，
并对索引和软件包下载启用重试。Ubuntu 运行时将 archive 源切换到 Azure 镜像，兼容
Ubuntu 24.04 的 deb822 `.sources` 和旧版 `.list` 格式，保留发行版、仓库组件、签名
密钥及 security 源。若某个源无法访问，构建会直接报告索引下载错误，避免继续安装后
将索引缺失误报为软件包不存在。

## 验证

```bash
# 检查进程
ps aux | grep ivnc

# 检查端口
ss -tlnp | grep 8008

# 健康检查
curl http://localhost:8008/health

# 检查音频
pactl info
```

浏览器访问 `http://<host>:8008/` 即可使用。

## 故障排除

### WebRTC 连接失败

1. 确认浏览器能访问 HTTP 端口
2. 检查浏览器控制台是否有 ICE/DTLS 错误
3. 如果通过反向代理，确保 WebSocket 和 TCP 连接能正确转发到同一端口
4. 公网部署时需设置 `public_candidate` 或启用 `candidate_from_host_header`

### 无音频

1. 确认 PulseAudio/PipeWire 正在运行：`pactl info`
2. 确认 `XDG_RUNTIME_DIR` 环境变量已设置
3. 确认配置文件中 `[audio] enabled = true`
4. 检查日志中是否有 "PulseAudio capture opened" 消息
5. 可通过 `PULSE_SOURCE` 环境变量指定音频源

### GStreamer 编码器未找到

```bash
gst-inspect-1.0 | grep -E "(x264|openh264|vp8|vaapi|nvenc|qsv)"
sudo apt-get install gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### 硬件编码器不可用

```bash
# VA-API
vainfo

# NVIDIA
nvidia-smi
```

使用软件编码器作为备用：

```toml
[webrtc]
hardware_encoder = "software"
```

### 高延迟或卡顿

- 降低分辨率和比特率
- 减小 keyframe_interval
- 使用硬件加速编码

```toml
[webrtc]
video_bitrate = 4000
keyframe_interval = 30

[display]
width = 1280
height = 720
```

## 端口说明

| 端口 | 协议 | 说明 |
|------|------|------|
| 8008 | HTTP/WS/TCP | Web UI、健康检查、WebRTC 信令、ICE-TCP（同端口复用） |
