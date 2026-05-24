# iVnc

基于 Rust 的高性能 Wayland 桌面流媒体服务，内置 Smithay 合成器，使用 str0m Sans-I/O WebRTC 库 + GStreamer 实现低延迟流媒体传输。

## 功能特性

- **Wayland 合成器** - 内置 Smithay headless 合成器，无需外部 X11/Wayland 服务
- **str0m Sans-I/O WebRTC** - 基于 str0m 的纯 Rust WebRTC 实现，ICE-lite 模式，TCP 传输
- **同端口复用** - HTTP、WebSocket 信令、ICE-TCP 共享同一端口
- **多编码器支持** - H.264, VP8, VP9, AV1
- **硬件加速** - Intel VA-API, NVIDIA NVENC, Intel Quick Sync Video
- **输入转发** - 通过 WebRTC DataChannel 支持键盘/鼠标/文本输入（IME）
- **双向剪贴板** - 浏览器 ↔ 远程应用剪贴板同步，500ms 回声抑制
- **任务栏** - 窗口列表广播，支持从浏览器切换焦点/关闭窗口
- **光标同步** - 远程光标样式实时同步到浏览器
- **音频流媒体** - PulseAudio/PipeWire 捕获 + Opus 编码
- **文件传输** - 支持上传/下载文件
- **Web UI** - 内置 Web 界面，支持 PWA 安装
- **HTTP API** - 健康检查和 Prometheus 指标端点
- **Basic Auth** - 内置 HTTP 基础认证
- **TLS** - 可选自签名 HTTPS（`--tls`）
- **MCP 服务器** - 可选 [Model Context Protocol](https://modelcontextprotocol.io) 支持，AI 代理可通过 13 个工具控制远程桌面（截图、鼠标、键盘、剪贴板、窗口管理）
- **VLM 原生 Agent** - 可选内置 AI 自动化闭环：给定一个任务目标和一个视觉大模型（VLM）API，iVnc 在进程内自主完成"看屏幕 → 决策 → 操作 → 再看"循环，支持 OpenAI / Anthropic / Gemini / Holo3 / 本地 VLM

## 快速开始

```bash
# 1. 安装编译依赖（见"从源码编译"章节）
# 2. 编译
bash build.sh --release

# 3. 运行
./ivnc -c config.toml --http-port 8008
```

浏览器访问 `http://<server-ip>:8008/` 即可使用。

## 从源码编译

### 编译依赖

```bash
apt-get install build-essential pkg-config curl ca-certificates cmake \
  libxcb1-dev libxkbcommon-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libpulse-dev libopus-dev \
  libwayland-dev libpixman-1-dev libinput-dev libudev-dev libseat-dev
```

> smithay 和 str0m 通过 Git URL 引用，cargo 构建时自动拉取，无需手动 clone。

### 编译

使用 `build.sh` 脚本（推荐）：

```bash
# Release 构建（默认包含 PulseAudio 音频 + MCP 支持）
bash build.sh --release

# Debug 构建
bash build.sh --debug

# 追加额外 feature（如 TLS），mcp 始终包含
bash build.sh --release --features tls
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
| `tls` | 自签名 HTTPS（`--tls` 启用，PWA 支持） | |
| `mcp` | MCP 服务器（AI 代理远程桌面控制） | |
| `agent` | VLM 原生 Agent 框架（含 `replay` provider，隐含 `mcp`） | |
| `agent-openai` / `agent-anthropic` / `agent-gemini` / `agent-holo3` / `agent-local` | 启用对应云/本地 VLM provider（各自隐含 `agent`） | |
| `agent-all` | 启用全部 VLM provider | |
| `vaapi` | Intel VA-API 硬件编码 | |
| `nvenc` | NVIDIA NVENC 硬件编码 | |
| `qsv` | Intel Quick Sync Video | |

## 部署

### 运行时依赖

从预编译二进制直接运行时，需要安装以下运行时库：

```bash
apt-get install \
  libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 \
  libpixman-1-0 libxkbcommon0 \
  gstreamer1.0-tools gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly gstreamer1.0-x \
  libpulse0 libopus0 pulseaudio pulseaudio-utils
```

> `libglib-2.0`、`libgobject-2.0` 等由 GStreamer 自动依赖，无需单独安装。

### 音频配置

音频捕获需要 PulseAudio。推荐使用原生 PulseAudio（PipeWire-Pulse 的 null-sink 在无音频播放时处于 SUSPENDED 状态，会导致捕获超时）。

启动 PulseAudio 并配置虚拟音频设备（无物理声卡的服务器环境必需）：

```bash
export XDG_RUNTIME_DIR=/run/user/$(id -u)
mkdir -p "$XDG_RUNTIME_DIR"

# 启动 PulseAudio（--exit-idle-time=-1 防止空闲退出）
pulseaudio --start --exit-idle-time=-1

# 加载虚拟 sink（远程应用的音频输出目标）
pactl load-module module-null-sink sink_name=ivnc_sink \
  sink_properties=device.description=iVnc_Output \
  rate=48000 channels=2 format=s16le
```

iVnc 会自动检测默认 sink 的 monitor source（`ivnc_sink.monitor`）来捕获桌面音频输出。也可通过 `PULSE_SOURCE` 环境变量指定音频源。

> **注意**：PipeWire-Pulse 的 `module-null-sink` 在 SUSPENDED 状态下不产生数据，PulseAudio Simple API 连接会超时。如果必须使用 PipeWire，需要确保有真实音频设备或始终有客户端连接到 sink。

### 硬件加速（可选）

```bash
# Intel VA-API
apt-get install gstreamer1.0-vaapi libva-dev

# NVIDIA NVENC（需要 NVIDIA 驱动）
apt-get install gstreamer1.0-plugins-bad

# Intel Quick Sync Video
apt-get install intel-media-va-driver-non-free
```

### Docker 部署

```dockerfile
FROM rust:1.75 AS builder

RUN apt-get update && apt-get install -y \
    pkg-config cmake libxcb1-dev libxkbcommon-dev \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libpulse-dev libopus-dev libwayland-dev libpixman-1-dev \
    libinput-dev libudev-dev libseat-dev

WORKDIR /build
COPY . .
RUN cargo build --release

FROM ubuntu:22.04

RUN apt-get update && apt-get install -y \
    libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 \
    libpixman-1-0 libxkbcommon0 libpulse0 libopus0 \
    pulseaudio pulseaudio-utils \
    gstreamer1.0-tools gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly gstreamer1.0-x \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ivnc /usr/local/bin/
COPY config.example.toml /etc/ivnc.toml

EXPOSE 8008

ENV XDG_RUNTIME_DIR=/run/user/0

# Start PulseAudio with virtual sink, then iVnc
CMD mkdir -p $XDG_RUNTIME_DIR && \
    pulseaudio --start --exit-idle-time=-1 && \
    pactl load-module module-null-sink sink_name=ivnc_sink \
      sink_properties=device.description=iVnc_Output \
      rate=48000 channels=2 format=s16le && \
    ivnc --config /etc/ivnc.toml
```

## 配置

### 命令行参数

```bash
# 使用默认配置（/etc/ivnc.toml，不存在则使用内置默认值）
./ivnc

# 指定配置文件
./ivnc -c config.toml

# 覆盖端口和分辨率
./ivnc -c config.toml --http-port 8008 --width 1920 --height 1080

# 启用自签名 HTTPS（需要 tls feature 编译）
./ivnc -c config.toml --tls

# 调试模式
./ivnc -c config.toml --verbose
```

音频捕获需要 `XDG_RUNTIME_DIR` 环境变量指向 PulseAudio socket 所在目录：

```bash
XDG_RUNTIME_DIR=/run/user/$(id -u) ./ivnc -c config.toml --http-port 8008
```

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `-c, --config` | `/etc/ivnc.toml` | 配置文件路径 |
| `--width` | `1920` | 显示宽度 |
| `--height` | `1080` | 显示高度 |
| `--http-port` | 配置文件值 | HTTP 端口（同时用于 ICE-TCP） |
| `--tls` | | 启用自签名 HTTPS |
| `--basic-auth-enabled` | `true` | 启用基础认证 |
| `--basic-auth-user` | | 认证用户名 |
| `--basic-auth-password` | | 认证密码 |
| `-v, --verbose` | | 详细日志 |
| `--foreground` | | 前台运行 |
| `--mcp-stdio` | | 同时启用 MCP stdio 和 Web VNC（需 `mcp` feature） |

完整参数列表：`./ivnc --help`

### 配置文件

复制示例配置：

```bash
cp config.example.toml config.toml
```

主要配置段：

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
tcp_only = true
video_codec = "h264"
video_bitrate = 8000
video_bitrate_max = 16000
video_bitrate_min = 1000
hardware_encoder = "auto"
keyframe_interval = 60
candidate_from_host_header = true
# public_candidate = "1.2.3.4:8008"
```

完整配置示例见 `config.example.toml`。

### 环境变量

| 环境变量 | 说明 |
|----------|------|
| `XDG_RUNTIME_DIR` | PulseAudio/PipeWire socket 目录（音频捕获必需） |
| `PULSE_SOURCE` | 指定 PulseAudio 音频源（默认自动检测 monitor source） |
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

UI 相关环境变量值后加 `|locked` 可锁定前端不可修改。

## API 参考

### Web 界面

内置前端通过 HTTP 端口提供：

```
http://localhost:8008/
```

WebRTC 信令通过 WebSocket（同端口）：

```
ws://localhost:8008/webrtc
```

ICE-TCP 连接也复用同一端口，通过首字节分类自动区分。

### HTTP 端点

| 端点 | 说明 |
|------|------|
| `GET /` | Web 界面 |
| `GET /health` | 健康检查（JSON） |
| `GET /metrics` | Prometheus 指标 |
| `GET /clients` | 活跃连接列表 |
| `GET /ui-config` | UI 配置 |
| `GET /ws-config` | WebSocket 端口配置 |
| `GET /webrtc` | WebRTC 信令 WebSocket |
| `POST /mcp` | MCP Streamable HTTP 端点（需 `mcp` feature） |

### DataChannel 协议

输入事件和控制消息通过 WebRTC DataChannel 传输。

**客户端 → 服务端：**

| 格式 | 说明 |
|------|------|
| `m,{x},{y},{buttonMask},{0}` | 鼠标移动（buttonMask 变化时合成按键事件） |
| `b,{button},{pressed}` | 鼠标按键 |
| `w,{dx},{dy}` | 鼠标滚轮 |
| `k,{keysym},{pressed}` | 键盘事件（X11 KeySym） |
| `t,{text}` | IME 文本输入（zwp_text_input_v3） |
| `cw,{base64}` | 剪贴板内容 |
| `r,{width}x{height}` | 分辨率调整 |
| `focus,{id}` | 切换窗口焦点 |
| `close,{id}` | 关闭窗口 |
| `kr` | 键盘重置（释放所有修饰键） |
| `pong` | 心跳响应 |

**服务端 → 客户端：**

| 格式 | 说明 |
|------|------|
| `cursor,{json}` | 光标样式变化 |
| `clipboard,{base64}` | 剪贴板内容 |
| `taskbar,{json}` | 窗口列表更新 |
| `stats,{json}` | 性能统计（每秒） |
| `ping` | 心跳请求 |

完整协议规范见 [docs/PROTOCOL.md](docs/PROTOCOL.md)。

## 技术架构

### 流媒体管道

```
视频流 (Server → Browser):
┌──────────────┐    ┌─────────────────────────────────────────────┐    ┌──────────────┐
│   Smithay    │    │              GStreamer Pipeline              │    │    str0m      │
│  Compositor  │───▶│ appsrc → videoconvert → encoder → rtppay    │───▶│  write_rtp()  │
│  (headless)  │    │                         H.264/VP8/VP9/AV1   │    │  Sans-I/O     │
└──────────────┘    └─────────────────────────────────────────────┘    └──────┬───────┘
   RGBA 帧                                                                    │
                                                                    SRTP 加密 │ NullPacer
                                                                              ▼
                    ┌─────────────┐    ┌──────────────────┐    ┌──────────────────┐
                    │   Browser   │◀───│  RFC 4571 TCP    │◀───│  poll_output()   │
                    │  (WebRTC)   │    │  帧封装 (同端口)  │    │  drain_outputs() │
                    └─────────────┘    └──────────────────┘    └──────────────────┘

音频流 (Server → Browser):
┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐
│  PulseAudio  │───▶│    Opus      │───▶│    str0m     │───▶│  SRTP → TCP 帧   │───▶ Browser
│  /PipeWire   │    │   Encoder    │    │  write_rtp() │    │  (RFC 4571)      │
└──────────────┘    └──────────────┘    └──────────────┘    └──────────────────┘

输入流 (Browser → Server):
┌──────────────┐    ┌──────────────────┐    ┌──────────────┐    ┌──────────────┐
│   Browser    │───▶│  RTCDataChannel  │───▶│    str0m     │───▶│   Smithay    │
│  键盘/鼠标   │    │  SCTP/DTLS/TCP   │    │  ChannelData │    │  Seat 注入   │
└──────────────┘    └──────────────────┘    └──────────────┘    └──────────────┘
```

### WebRTC 传输层

iVnc 使用 str0m Sans-I/O WebRTC 库，所有 I/O 由调用方驱动：

- **ICE-lite 模式** - 服务端仅提供 TCP passive candidate，不主动探测
- **RTP 模式** - GStreamer 产出的 RTP 包通过 `write_rtp()` 直接传入 str0m，str0m 负责 SRTP 加密、SSRC 分配、RTP header extension 注入
- **NullPacer** - BWE 默认关闭，使用 NullPacer，每次 `handle_timeout()` → `poll_output()` 循环发射一个包
- **同端口复用** - 通过首字节分类区分 HTTP 请求和 ICE-TCP 数据包

### 双向剪贴板同步

浏览器 → 远程应用：
- DataChannel `cw,{base64}` → `set_data_device_selection()` → Wayland 客户端读取
- 500ms `clipboard_suppress_until` 窗口防止回声循环（Wayland 客户端重新断言 `wl_data_source`）

远程应用 → 浏览器：
- Wayland 客户端复制 → `new_selection()` 保存 mime type（延迟模式）
- 主循环 `event_loop.dispatch()` 后调用 `request_data_device_client_selection()` + `flush_clients()`
- 非阻塞 pipe 读取 → base64 编码 → DataChannel `clipboard,{base64}` 广播

延迟读取原因：smithay 在 `new_selection()` 返回后才更新 `seat_data.clipboard_selection`，回调内直接读取会失败。

### 任务栏窗口管理

- `window_registry` 维护窗口列表（稳定顺序）
- 窗口创建/销毁时通过 DataChannel 广播 `taskbar,{json}`（包含 id, title, app_id, display_name, focused）
- 浏览器可发送 `focus,{id}` / `close,{id}` 控制窗口
- `display_name` 从 `.desktop` 文件解析
- 新 DataChannel 打开时（`datachannel_open_count` 变化）自动重发窗口列表

### 模块结构

| 模块 | 功能 |
|------|------|
| `compositor/` | Smithay Wayland 合成器（headless backend） |
| `gstreamer/` | GStreamer 管道、编码器选择、RTP 打包 |
| `webrtc/rtc_session.rs` | str0m Sans-I/O 会话驱动（事件循环、RTP 转发、DataChannel） |
| `webrtc/session.rs` | 会话管理、ICE-TCP 连接匹配 |
| `webrtc/tcp_framing.rs` | RFC 4571 TCP 帧编解码 |
| `transport/` | WebRTC 信令服务器（WebSocket） |
| `input.rs` | 键盘/鼠标事件处理 |
| `audio/` | PulseAudio/PipeWire 捕获和 Opus 编码 |
| `web/` | Axum HTTP 服务器、同端口复用、嵌入式前端资源 |
| `config/` | TOML 配置管理、UI 配置 |
| `clipboard.rs` | 剪贴板同步 |
| `file_upload.rs` | 文件上传处理 |
| `mcp/` | MCP 服务器（截图、输入、剪贴板、窗口管理工具） |

## MCP 服务器（AI 代理控制）

iVnc 支持 [Model Context Protocol (MCP)](https://modelcontextprotocol.io)，允许 AI 代理（如 Claude）通过标准化协议控制远程桌面。

### 编译

`build.sh` 默认包含 `mcp` feature，无需额外指定：

```bash
bash build.sh --release
```

直接用 cargo 则需手动指定：

```bash
cargo build --release --features mcp
```

### 传输方式

**Stdio 模式** — 适用于本地 MCP 客户端（如 Claude Desktop），Web VNC 同时可用：

```bash
./ivnc -c config.toml --mcp-stdio
# MCP 通过 stdin/stdout 通信，HTTP/Web VNC 照常启动
```

**Streamable HTTP 模式** — 正常启动 iVnc 即可，MCP 端点自动挂载在 `/mcp`：

```bash
./ivnc -c config.toml --http-port 8008
# MCP 端点：http://localhost:8008/mcp
```

> `/mcp` 端点受 Basic Auth 保护（如已启用）。

### MCP 工具列表

| 工具 | 说明 |
|------|------|
| `screenshot` | 截取桌面 JPEG 图像，支持延迟捕获 |
| `mouse_move` | 移动鼠标光标 |
| `mouse_click` | 鼠标点击（左/右/中键，支持双击） |
| `mouse_scroll` | 鼠标滚轮 |
| `keyboard_type` | 键入文本（自动处理 Shift） |
| `keyboard_type_multiline` | 键入多行文本 |
| `keyboard_key` | 按键/组合键（如 `Ctrl+c`、`Alt+F4`） |
| `clipboard_read` | 读取剪贴板 |
| `clipboard_write` | 写入剪贴板 |
| `get_screen_info` | 获取屏幕尺寸、FPS、带宽等统计 |
| `list_windows` | 列出所有窗口 |
| `window_focus` | 聚焦窗口 |
| `window_close` | 关闭窗口 |

### AI Agent 接入

#### Claude Code

在项目目录的 `.mcp.json` 中添加：

```json
{
  "mcpServers": {
    "ivnc": {
      "type": "streamable-http",
      "url": "http://<server-ip>:8008/mcp",
      "headers": {
        "Authorization": "Basic <base64(user:password)>"
      }
    }
  }
}
```

如果未启用 Basic Auth，去掉 `headers` 即可。

也可以用 CLI 快速添加：

```bash
claude mcp add ivnc --transport http http://<server-ip>:8008/mcp
```

#### Claude Desktop

```json
{
  "mcpServers": {
    "ivnc": {
      "command": "/path/to/ivnc",
      "args": ["-c", "/path/to/config.toml", "--mcp-stdio"]
    }
  }
}
```

> Claude Desktop 通过 stdio 通信，会自动启动 ivnc 进程。适合本地使用。

#### 其他 MCP 客户端

任何支持 MCP Streamable HTTP 的客户端都可以直接连接：

```
POST http://<server-ip>:8008/mcp
Content-Type: application/json
Authorization: Basic <base64(user:password)>
```

## VLM 原生 Agent（内置 AI 自动化）

除了把桌面"暴露"给外部 AI 的 MCP 工具之外，iVnc 还内置了一个 **VLM 原生 Agent**：你只需给它一个自然语言任务（例如"打开浏览器，查一下香港 VPS 价格并整理成表格"）和一个视觉大模型（VLM）的 API，它就会在 iVnc 进程内自己跑完整个"看 → 想 → 做"的循环，直到任务完成。

### 与 MCP 工具的区别

| | MCP 工具 | 内置 VLM Agent |
|---|---|---|
| 谁在决策 | 外部 AI（如 Claude Desktop） | iVnc 进程内的 Agent 循环 |
| 调用粒度 | 外部每次调用一个工具（截图/点击…） | 给一个任务目标，内部自动多步执行 |
| 网络往返 | 外部 ↔ iVnc 每个动作一次 | 只有 Agent ↔ VLM 的模型调用 |
| 典型场景 | 把 iVnc 当作 Claude 的"手和眼" | 让 iVnc 独立完成一个端到端任务 |

### 设计理念：纯像素、不靠辅助信息

Agent 是 **VLM-native / pixel-native** 的：模型只看一张桌面截图，直接输出"点屏幕哪个坐标、敲什么键"。它**不依赖** DOM、无障碍树（accessibility tree）、OCR 或目标检测——这套机制对任意 GUI（浏览器、原生应用、游戏）都通用，也不需要在被控端安装额外探针。

### 核心闭环

每一步（step）都重复同一个循环，直到模型主动 `done`/`ask`、预算耗尽或被用户中断：

```
                ┌──────────────────────────────────────────────────────┐
                │                  Agent Loop (loop.rs)                 │
                │                                                       │
  ┌─────────────▼─────────────┐    ┌──────────────────────────────┐    │
  │  1. 截图观测               │    │  2. BrainProvider.next_action │    │
  │  capture_observation()    │───▶│  把 任务+截图+历史 翻译成      │    │
  │  截图 + 尺寸/缩放 + 窗口   │    │  模型 API 调用，拿回 Action   │    │
  │  列表 + 剪贴板 + sha256    │    │  (OpenAI/Anthropic/Gemini…)   │    │
  └───────────────────────────┘    └───────────────┬──────────────┘    │
                ▲                                   │                   │
                │                                   ▼                   │
  ┌─────────────┴─────────────┐    ┌──────────────────────────────┐    │
  │  4. 等待画面沉降           │    │  3. 执行动作 exec::execute()  │    │
  │  action_settle_ms (250ms) │◀───│  图像坐标→屏幕坐标，注入       │    │
  │  然后回到第 1 步           │    │  Wayland Seat（鼠标/键盘…）   │    │
  └───────────────────────────┘    └──────────────────────────────┘    │
                │                                                       │
                └──────────── done / ask / 预算到顶 / 中断 ────────────┘
                                          │
                                          ▼
                              RunReport（成败 + 交付物 output + 轨迹）
```

### 关键数据契约（`src/agent/types.rs`）

- **Observation（观测）** —— 一帧"现在屏幕长什么样"：JPEG 截图字节、屏幕真实尺寸、缩放后的图像尺寸、`image_to_screen_scale`、窗口列表、剪贴板预览，以及一个 `sha256`（用于判断画面是否变化）。
- **Action（动作）** —— 一个枚举，覆盖 `mouse_move/click/down/up/drag`、`scroll`、`zoom`、`type_text`、`key_chord`、`key_hold`、`clipboard_write/read`、`window_focus/close`、`wait`、`screenshot`、`launch_app`（按 id/名称启动内置应用，可附带 URL）、`done`、`ask`。
- **坐标空间（CoordinateSpace）** —— 模型看到的是**缩放后的图像像素**，executor 用 `image_to_screen_scale` 把它换算回真实屏幕像素再注入。不同模型用不同坐标约定（`ImagePixels` / `ScreenPixels` / 归一化 `NormalizedUnit` 千分比），由各 provider 的 `capabilities` 声明，Agent 自动适配。模型永远不需要关心真实分辨率。
- **History / Step** —— 每一步记录"观测摘要 + 动作 + 结果 + 模型 token 用量"，作为下一步的上下文。截图按滑动窗口只保留最近 `max_history_images`（默认 3）帧，避免 token 爆炸。
- **Budget / RunOptions** —— 步数、输入/输出 token、墙钟时长、截图数、可选费用（micro-USD）上限，任一触顶就自动停止。还可配置动作沉降时间、是否记录轨迹、`dry_run`（只记录不真正注入）等。
- **RunReport** —— 一次运行的最终产物：是否成功、`finish_reason`、**`output`（任务真正的交付物，而不仅是状态句）**、`warnings`、token 统计、轨迹文件路径。

### 可插拔的"大脑"：BrainProvider

`BrainProvider` trait 负责把"任务 + 观测 + 历史"翻译成某家模型的 API 请求，再把模型回复翻译回 `Action`。新增一个模型后端只需实现这个 trait。每个 provider 通过 `ProviderCapabilities` 声明自己的坐标空间、动作语法、能吃几帧历史图、所需 API key 环境变量等。

内置 provider：

| Provider | 默认模型 | 默认 endpoint | API Key 环境变量 | 动作语法 |
|---|---|---|---|---|
| `replay` | — | —（离线） | 无 | 回放固定轨迹（测试/演示） |
| `local` | `local-vlm` | `http://localhost:8000/v1` | `LOCAL_VLM_API_KEY`（可选） | OpenAI 兼容 tool call |
| `openai` | `gpt-4o-mini` | `https://api.openai.com/v1` | `OPENAI_API_KEY` | OpenAI tool call / Responses |
| `anthropic` | `claude-haiku-4-5` | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` | Anthropic computer use |
| `gemini` | `gemini-2.0-flash` | `https://generativelanguage.googleapis.com/v1beta` | `GEMINI_API_KEY` | Gemini computer use |
| `holo3` | `holo3-35b-a3b` | `https://api.hcompany.ai/v1` | `HAI_API_KEY` | OpenAI 兼容 tool call |

> OpenAI 兼容 provider（`openai`/`local`/`holo3`）支持 `api_format = chat_completions`（默认）或 `responses` 两种请求格式。所有 provider 的模型回复最终都会经过统一的 `text_action_parser` 兜底解析——它同时能识别 **JSON 工具调用**与**自然语言文本动作**（如 `Action: click(point='(840,210)')`），即使模型不严格遵守函数调用格式也能稳健落地。

### 可靠性与"诚实"机制

纯视觉 Agent 容易陷入两类问题：**反复点同一个没反应的地方**，以及**没干活却谎报成功**。iVnc 内置了多重护栏：

- **卡死检测** —— 连续两帧截图 `sha256` 完全相同（画面没变），会在下一轮提示中警告模型"你上一步没产生任何效果，换个策略"。
- **循环阻断** —— 如果模型在画面未变的情况下重复完全相同的动作，executor 直接拒绝执行并回一条错误提示，强制它改变方法或收尾。
- **预算临近提示** —— 用掉 >60% 步数时提示尽快 `done`；最后一步会强制要求返回 `done`，避免把额度浪费在无意义探索上。
- **假成功护栏** —— 如果模型声称 `success=true`，却既没做任何实质性动作、也没提供 `output` 交付物，运行报告会附带一条 warning，提示该结论很可能未经验证。
- **越界保护** —— 动作坐标超出屏幕范围会返回 `OutOfBounds`；连续 3 次越界则中止运行。
- **交付物要求** —— `done.output` 用于承载"用户真正想要的答案"（如整理好的表格、查到的结论），与"是否成功"分离。

### 安全与操作隔离

- **破坏性动作守卫（`safety.rs`）** —— `window_close`、危险快捷键（`Alt+F4`、`Ctrl+Alt+Del`、`Cmd/Super+Q`）、剪贴板覆盖等动作默认被拦截，需显式 `allow_destructive=true` 才执行；也可通过 `require_confirmation_for` 指定需要人工确认的类别。
- **随时中断（kill-switch）** —— `agent_stop` 工具或 Web DataChannel 控制消息会触发 `CancellationToken`，正在执行的运行会立即停止。
- **独占与时间线广播** —— Agent 运行期间设为独占模式；每执行一步都通过 DataChannel 广播 `mcp_action,{json}`（动作 + 结果），运行状态变化广播 `mcp_control,{json}`，前端据此显示横幅与中断按钮。
- **dry-run 演练** —— `dry_run=true` 时模型照常被调用，但动作只记录不真正注入桌面，便于安全地预演一条轨迹。
- **轨迹持久化** —— 每步以 JSONL 追加写入 `~/.local/share/ivnc/trajectories/<run_id>.jsonl`，并伴随 `<run_id>.report.json` 旁车文件，进程重启后运行列表与轨迹不丢失。

### MCP Agent 工具

启用 `agent`（或 `agent-all`）feature 后，MCP 端点会额外暴露以下工具：

| 工具 | 说明 |
|------|------|
| `agent_start` | 启动一次 Agent 运行并立即返回 run_id（适合长任务，异步） |
| `agent_run` | 启动并**等待**运行结束后返回最终报告（适合短任务） |
| `agent_status` | 查询某次运行的实时状态/报告 |
| `agent_stop` | 中断正在进行的运行 |
| `agent_step` | 对实时桌面执行**单个** Action（同样受破坏性守卫约束），返回结果与新观测摘要 |
| `agent_history_get` | 获取一次运行的完整轨迹（步骤序列） |
| `agent_history_replay` | 用 `replay` provider 回放一条已有轨迹 |
| `provider_list` | 列出已编译且已配置（有 key）的 provider |
| `provider_health` | 探测某个 provider 的可用性 |

### 编译与配置

```bash
# build.sh 默认即包含 mcp + agent-all（全部 provider）
bash build.sh --release

# 仅用 cargo 时按需指定
cargo build --release --features agent-openai            # 只要 OpenAI
cargo build --release --features mcp,agent-all           # 全部 provider
```

provider 的 endpoint / model / api_format / api_key / system_prompt 可写入 `~/.config/ivnc/console.json`（通过内置管理控制台编辑，文件权限 0600），或用上表中的环境变量提供。Agent 默认 provider 为 `local`。

```jsonc
// ~/.config/ivnc/console.json（节选）
{
  "providers": {
    "openai": {
      "endpoint": "https://api.openai.com/v1",
      "model": "gpt-4o-mini",
      "api_format": "responses",
      "api_key": "sk-..."        // 仅存于本地，不要提交到 git
    }
  },
  "agent": { "default_provider": "openai" }
}
```

跑一个任务（通过任意 MCP 客户端调用 `agent_run`）：

```jsonc
{
  "name": "agent_run",
  "arguments": {
    "provider": "openai",
    "task": "打开浏览器，搜索香港 VPS 并整理一张价格对比表",
    "budget": { "max_steps": 30, "max_wall_seconds": 240 }
  }
}
```

### 模块结构（`src/agent/`）

| 模块 | 功能 |
|------|------|
| `types.rs` | 数据契约：Observation / Action / History / Budget / RunOptions / RunReport 等 |
| `provider.rs` | `BrainProvider` trait、`ProviderCapabilities`、`ProviderTurn`、`ProviderSession` |
| `registry.rs` | provider 预设注册表与按 key 配置的构建/可用性探测 |
| `loop.rs` | 主循环：观测→决策→执行→沉降，含卡死/循环/预算/假成功护栏 |
| `exec.rs` | 把 Action 转成 Wayland 输入注入，含坐标换算与破坏性守卫 |
| `safety.rs` | 破坏性动作分类 |
| `budget.rs` | 预算计量 |
| `trajectory.rs` | JSONL 轨迹与 report.json 旁车的读写 |
| `run_store.rs` | 运行注册表（内存 + 磁盘恢复） |
| `providers/` | 各模型后端：`openai_compat` / `anthropic` / `gemini` / `holo3` / `local_vlm` / `replay`，以及 `text_action_parser` 兜底解析器 |

## 故障排除

### GStreamer 编码器未找到

```bash
gst-inspect-1.0 | grep -E "(x264|openh264|vp8|vaapi|nvenc|qsv)"
apt-get install gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### WebRTC 连接失败

1. 确认浏览器能访问 HTTP 端口
2. 检查浏览器控制台是否有 ICE/DTLS 错误
3. 如果通过反向代理，确保 WebSocket 和 TCP 连接能正确转发

### 无音频

1. 确认 PulseAudio 正在运行：`pactl info`
2. 确认虚拟 sink 已加载：`pactl list sinks short`（应看到 `ivnc_sink`）
3. 确认 `XDG_RUNTIME_DIR` 环境变量已设置
4. 确认配置文件中 `[audio] enabled = true`
5. 检查日志中是否有 `PulseAudio capture opened` 消息
6. 如果日志显示 `PulseAudio connect failed: Timeout`，说明 PulseAudio 环境异常（PipeWire-Pulse 的 null-sink 不支持，需换用原生 PulseAudio）
7. 浏览器自动播放策略要求用户交互（点击/按键）后才能播放音频

### 高延迟或卡顿

```toml
[webrtc]
video_bitrate = 4000
keyframe_interval = 30

[display]
width = 1280
height = 720
```

## 许可证

详见 LICENSE 文件。
