# iVnc

基于 Rust 的高性能 Wayland 桌面流媒体服务，内置 Smithay 合成器，支持 WebRTC + GStreamer 低延迟流媒体传输。

## 功能特性

- **Wayland 合成器** - 内置 Smithay headless 合成器，无需外部 X11/Wayland 服务
- **WebRTC 流媒体** - 低延迟视频流，支持硬件加速编码
- **多编码器支持** - H.264, VP8, VP9, AV1
- **硬件加速** - Intel VA-API, NVIDIA NVENC, Intel Quick Sync Video
- **自适应比特率** - 基于网络状况动态调整比特率
- **输入转发** - 通过 WebRTC DataChannel 支持键盘/鼠标/剪贴板
- **音频流媒体** - PulseAudio 捕获 + Opus 编码（默认启用）
- **文件传输** - 支持上传/下载文件
- **Web UI** - 内置 Web 界面，便于访问
- **HTTP API** - 健康检查和 Prometheus 指标端点
- **Basic Auth** - 内置 HTTP 基础认证

## 技术架构

### 流媒体管道

```
Smithay Compositor → GStreamer pipeline → H.264/VP8 Encoder → RTP → WebRTC → Browser
Browser Input → RTCDataChannel → SCTP/DTLS → Parse → Smithay Input → Compositor
PulseAudio → Opus Encoder → WebRTC Audio Track → Browser
```

### 模块结构

| 模块 | 功能 |
|------|------|
| `compositor/` | Smithay Wayland 合成器（headless backend） |
| `gstreamer/` | GStreamer 管道、编码器选择 |
| `webrtc/` | WebRTC 会话管理、信令、DataChannel |
| `transport/` | WebRTC 信令服务器 |
| `input.rs` | 键盘/鼠标事件处理 |
| `audio/` | PulseAudio 捕获和 Opus 编码 |
| `web/` | Axum HTTP 服务器、嵌入式前端资源 |
| `config/` | TOML 配置管理、UI 配置 |
| `clipboard.rs` | 剪贴板同步 |
| `file_upload.rs` | 文件上传处理 |

## 系统依赖

### 编译依赖

```bash
apt-get install build-essential pkg-config curl ca-certificates \
  libx11-dev libxcb1-dev libxkbcommon-dev libssl-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libpulse-dev \
  libwayland-dev libpixman-1-dev libinput-dev libudev-dev libseat-dev
```

### Smithay 依赖

iVnc 依赖本地 smithay 仓库（需放在项目同级目录）：

```bash
git clone https://github.com/Smithay/smithay.git ../smithay
cd ../smithay && git checkout 3d3f9e359352d95cffd1e53287d57df427fcbd34
```

### GStreamer 运行时

```bash
apt-get install \
  gstreamer1.0-tools \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly \
  gstreamer1.0-x
```

### 硬件加速（可选）

```bash
# Intel VA-API
apt-get install gstreamer1.0-vaapi libva-dev

# NVIDIA NVENC（需要 NVIDIA 驱动）
apt-get install gstreamer1.0-plugins-bad

# Intel Quick Sync Video
apt-get install intel-media-va-driver-non-free
```

## 编译

使用 `build.sh` 脚本（推荐）：

```bash
# Release 构建（默认包含 PulseAudio 音频支持）
bash build.sh --release

# Debug 构建
bash build.sh --debug
```

构建完成后二进制文件位于项目根目录：`./ivnc`

也可以直接使用 cargo：

```bash
cargo build --release
# 输出：target/release/ivnc
```

### Cargo Features

| Feature | 说明 | 默认 |
|---------|------|------|
| `pulseaudio` | PulseAudio 音频捕获 + Opus 编码 | ✅ |
| `audio` | cpal 音频捕获 + Opus 编码 | |
| `vaapi` | Intel VA-API 硬件编码 | |
| `nvenc` | NVIDIA NVENC 硬件编码 | |
| `qsv` | Intel Quick Sync Video | |

## 运行

```bash
# 使用默认配置
./ivnc

# 指定配置文件
./ivnc --config config.toml

# 覆盖端口和分辨率
./ivnc --http-port 8000 --width 1920 --height 1080

# 调试模式
RUST_LOG=debug ./ivnc --verbose
```

### 命令行参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `-c, --config` | `/etc/ivnc.toml` | 配置文件路径 |
| `--width` | `1920` | 显示宽度 |
| `--height` | `1080` | 显示高度 |
| `--http-port` | 配置文件值 | HTTP 端口 |
| `--basic-auth-enabled` | `true` | 启用基础认证 |
| `--basic-auth-user` | | 认证用户名 |
| `--basic-auth-password` | | 认证密码 |
| `--webrtc-stun-host` | | STUN 服务器 |
| `--webrtc-turn-host` | | TURN 服务器 |
| `--webrtc-turn-shared-secret` | | TURN HMAC 密钥 |
| `--webrtc-udp-mux-port` | | UDP 复用端口 |
| `--webrtc-tcp-mux-port` | | TCP 复用端口 |
| `-v, --verbose` | | 详细日志 |
| `--foreground` | | 前台运行 |

完整参数列表：`./ivnc --help`

### 配置文件

复制示例配置：

```bash
cp config.toml config.toml
# 或
cp config/ivnc.example.toml config.toml
```

主要配置段：

```toml
[display]
width = 1920
height = 1080
refresh_rate = 60

[http]
host = "0.0.0.0"
port = 8000
basic_auth_enabled = true
basic_auth_user = "user"
basic_auth_password = "mypasswd"

[encoding]
target_fps = 30
max_fps = 60

[audio]
enabled = true
sample_rate = 48000
channels = 2
bitrate = 128000

[webrtc]
enabled = true
video_codec = "h264"
video_bitrate = 8000
hardware_encoder = "auto"
adaptive_bitrate = true
keyframe_interval = 60

[[webrtc.ice_servers]]
urls = ["stun:stun.l.google.com:19302"]
```

完整配置示例见 `config.toml`。

### 环境变量

UI 相关配置可通过环境变量覆盖（值后加 `|locked` 可锁定前端不可修改）：

| 环境变量 | 说明 |
|----------|------|
| `IVNC_ENCODER` | 编码器选项（逗号分隔） |
| `IVNC_FRAMERATE` | 帧率或帧率范围（如 `30` 或 `15-60`） |
| `IVNC_AUDIO_ENABLED` | 启用音频 (`true`/`false`) |
| `IVNC_AUDIO_BITRATE` | 音频比特率或范围 |
| `IVNC_MOUSE_ENABLED` | 启用鼠标 |
| `IVNC_KEYBOARD_ENABLED` | 启用键盘 |
| `IVNC_CLIPBOARD_ENABLED` | 启用剪贴板 |
| `IVNC_MANUAL_WIDTH` | 手动分辨率宽度 |
| `IVNC_MANUAL_HEIGHT` | 手动分辨率高度 |
| `IVNC_UI_SHOW_SIDEBAR` | 显示侧边栏 |

## Web 界面

内置前端通过 HTTP 端口提供（默认 `8000`）：

```
http://localhost:8000/
```

WebRTC 信令通过 WebSocket：

```
ws://localhost:8000/webrtc
```

## HTTP 端点

| 端点 | 说明 |
|------|------|
| `GET /` | Web 界面 |
| `GET /health` | 健康检查（JSON） |
| `GET /metrics` | Prometheus 指标 |
| `GET /clients` | 活跃连接列表 |
| `GET /ui-config` | UI 配置 |
| `GET /webrtc` | WebRTC 信令 WebSocket |

## DataChannel 输入协议

输入事件通过 WebRTC DataChannel 传输：

| 格式 | 说明 |
|------|------|
| `m,{x},{y}` | 鼠标移动 |
| `b,{button},{pressed}` | 鼠标按键 |
| `w,{dx},{dy}` | 鼠标滚轮 |
| `k,{keysym},{pressed}` | 键盘事件 |
| `t,{text}` | 文本输入 |
| `c,{base64}` | 剪贴板数据 |

## Docker 部署

```dockerfile
FROM rust:1.70 AS builder

RUN apt-get update && apt-get install -y \
    pkg-config libx11-dev libxcb1-dev libxkbcommon-dev libssl-dev \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libpulse-dev libwayland-dev libpixman-1-dev \
    libinput-dev libudev-dev libseat-dev

WORKDIR /build
COPY . .
# smithay 需要在 ../smithay
RUN cargo build --release

FROM ubuntu:22.04

RUN apt-get update && apt-get install -y \
    libx11-6 libxcb1 libpulse0 \
    gstreamer1.0-tools gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly gstreamer1.0-x \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ivnc /usr/local/bin/
COPY config.toml /etc/ivnc.toml

EXPOSE 8000

CMD ["ivnc", "--config", "/etc/ivnc.toml"]
```

## 故障排除

### GStreamer 编码器未找到

```bash
gst-inspect-1.0 | grep -E "(x264|vp8|vaapi|nvenc|qsv)"
apt-get install gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### WebRTC 连接失败

1. 检查防火墙设置（UDP 端口）
2. 配置 TURN 服务器用于 NAT 穿透
3. 检查浏览器控制台错误信息

### Chrome DTLS 握手失败

项目包含 `patches/webrtc-dtls-0.10.0` 补丁，修复 Chrome 发送 X25519Kyber768 曲线导致的 DTLS 握手失败问题。此补丁通过 `Cargo.toml` 的 `[patch.crates-io]` 自动应用。

### 高延迟或卡顿

```toml
[webrtc]
video_bitrate = 2000
keyframe_interval = 30

[display]
width = 1280
height = 720
```

## 许可证

详见 LICENSE 文件。
