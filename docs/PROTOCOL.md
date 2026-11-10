# iVnc 协议规范

## 1. 概述

本文档定义了 iVnc 服务端与浏览器客户端之间的通信协议。iVnc 使用 WebRTC 进行低延迟视频/音频流传输，通过 DataChannel 传输输入事件和控制消息。

传输层基于 str0m Sans-I/O WebRTC 库，所有媒体和数据通道均通过 ICE-TCP 传输（RFC 4571 帧格式），HTTP/WebSocket 信令/ICE-TCP 共享同一端口。

## 2. 连接建立

### 2.1 信令端点

```
ws://{host}:{port}/webrtc
wss://{host}:{port}/webrtc  (TLS)
```

默认端口：8008

### 2.2 连接流程

```
Client                                          Server
   │                                               │
   │─────────── HTTP GET /webrtc ──────────────────▶│
   │◀────────── WebSocket Upgrade ─────────────────│
   │                                               │
   │─────────── SDP Offer (JSON) ──────────────────▶│
   │                                               │
   │◀────────── SDP Answer (JSON) ─────────────────│
   │                                               │
   │═══════════ ICE-TCP 连接 (同端口) ═════════════│
   │                                               │
   │◀────────── DTLS/SRTP 握手 ────────────────────│
   │                                               │
   │◀══════════ Video RTP (H.264/VP8) ═════════════│
   │◀══════════ Audio RTP (Opus) ══════════════════│
   │◄─────────► DataChannel (SCTP/DTLS) ──────────►│
   │                                               │
```

1. 客户端通过 WebSocket 连接到 `/webrtc` 信令端点
2. 客户端发送 SDP Offer（包含 ICE-TCP candidate）
3. 服务端返回 SDP Answer（包含 ICE-lite TCP passive candidate）
4. 浏览器通过同端口建立 ICE-TCP 连接
5. DTLS/SRTP 握手完成后，开始接收视频和音频 RTP 流
6. DataChannel 打开后，用于双向输入事件和控制消息

### 2.3 同端口复用

HTTP、WebSocket 信令和 ICE-TCP 共享同一端口（默认 8008）。服务端通过 TCP 连接的首字节自动区分协议类型：

- HTTP 方法首字母（`G`/`P`/`H`/`D`/`O`/`C`）→ Axum HTTP 路由
- 其他字节 → ICE-TCP 数据包 → SessionManager

## 3. WebRTC 信令协议

### 3.1 信令消息格式

WebRTC 信令使用 JSON 格式通过 WebSocket 传输。

#### 3.1.1 Offer 消息

客户端发送 SDP Offer：

```json
{
  "type": "offer",
  "sdp": "v=0\r\no=- ..."
}
```

#### 3.1.2 Answer 消息

服务端返回 SDP Answer：

```json
{
  "type": "answer",
  "sdp": "v=0\r\no=- ...",
  "session_id": "session-abc123"
}
```

SDP Answer 中包含 ICE-lite TCP passive candidate，指向同一端口。

## 4. DataChannel 消息格式

### 4.1 通用格式

所有 DataChannel 文本消息均为 UTF-8，使用逗号 `,` 分隔字段：

```
{message_type},{field1},{field2},...
```

二进制消息用于文件上传。

## 5. 服务端消息 (Server → Client via DataChannel)

### 5.1 光标消息 `cursor`

传输鼠标光标状态变化。

**格式:**
```
cursor,{json_data}
```

**JSON 结构:**
```json
{
    "override": "default"
}
```

`override` 值为 CSS cursor 名称：`default`, `pointer`, `text`, `move`, `none` 等。

**示例:**
```
cursor,{"override":"text"}
cursor,{"override":"none"}
```

### 5.2 剪贴板消息 `clipboard`

传输远程应用的剪贴板内容到浏览器。

**格式:**
```
clipboard,{base64_text_data}
```

**示例:**
```
clipboard,SGVsbG8gV29ybGQh
```

### 5.3 任务栏消息 `taskbar`

传输窗口列表，以及以应用标签显示在工具栏中的应用。

**格式:**
```
taskbar,{json_data}
```

**JSON 结构:**
```json
{
    "windows": [
        {
            "id": 0,
            "title": "Terminal",
            "app_id": "org.gnome.Terminal",
            "display_name": "Terminal",
            "focused": true,
            "managed_app_id": "terminal-app"
        },
        {
            "id": 1,
            "title": "Files",
            "app_id": "org.gnome.Nautilus",
            "display_name": "Files",
            "focused": false,
            "managed_app_id": null
        }
    ],
    "apps": [
        {
            "id": "a1b2c3",
            "name": "Chrome",
            "kind": "shared",
            "status": "running"
        },
        {
            "id": "d4e5f6",
            "name": "remote-demo",
            "kind": "remote",
            "status": "connected"
        }
    ],
    "ordered_ids": ["terminal-app", "a1b2c3", "d4e5f6"],
    "order_revision": 12
}
```

`windows` 是主桌面合成器中的窗口。`apps` 是不在主合成器中绘制、需要通过应用视图打开的应用：

| 字段 | 说明 |
|------|------|
| `id` | 应用 ID，与 `/api/apps` 相同 |
| `name` | 显示名称 |
| `kind` | `shared`：本实例开启远程访问、在独立运行环境中运行的桌面应用；`remote`：本实例已启动的远程应用 |
| `status` | `shared` 为 `starting` 或 `running`；`remote` 为 `/api/apps` 的 `remote_state`（`connecting`、`connected`、`starting`、`stopped`、`not_ready`、`auth_required`、`incompatible`、`unreachable`） |

任务栏消息另含 `ordered_ids`（本实例完整应用展示顺序）和 `order_revision`（持久化排序版本）。`windows` 中的 `managed_app_id` 为关联的应用 ID，无法关联时为 `null`；它与 Wayland `app_id` 的桌面类名不同。客户端将普通窗口与 `apps` 按同一应用顺序合并，保留窗口 ID 用于聚焦。同一应用的多个窗口保留组内顺序。排序 API 和合并规则见 [工具栏与排序](TOOLBAR.md)。

排序或窗口关联变化、`apps` 变化时以及每个新 DataChannel 打开时重发整条消息；应用运行环境（worker）内的 `apps` 始终为空数组。客户端在 `status` 为 `running`（`shared`）或 `connected`（`remote`）时才打开视图。

### 5.4 统计消息 `stats`

传输性能统计信息（每秒发送一次）。

**格式:**
```
stats,{json_data}
```

**JSON 结构:**
```json
{
    "fps": 30.0,
    "bandwidth": 2500000,
    "total_frames": 1800,
    "total_bytes": 45000000
}
```

### 5.5 Ping 消息

服务端定期发送 keepalive。

**格式:**
```
ping
```

### 5.6 Pong 消息

响应客户端的 ping。

**格式:**
```
pong
```

## 6. 客户端消息 (Client → Server via DataChannel)

### 6.1 鼠标移动 `m`

**格式:**
```
m,{x},{y},{buttonMask},{unused}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| x | i32 | X 坐标 (像素) |
| y | i32 | Y 坐标 (像素) |
| buttonMask | u32 | 按键位掩码 (bit0=左, bit1=中, bit2=右) |
| unused | i32 | 保留字段 |

buttonMask 变化时自动合成按键按下/释放事件。

**示例:**
```
m,512,384,0,0
m,512,384,1,0    # 左键按下（buttonMask bit0 = 1）
m,600,400,1,0    # 拖拽中
m,600,400,0,0    # 左键释放
```

### 6.2 鼠标按键 `b`

**格式:**
```
b,{button},{pressed}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| button | u8 | 按键编号 (0=左, 1=中, 2=右) |
| pressed | u8 | 状态 (0=释放, 1=按下) |

**示例:**
```
b,0,1    # 左键按下
b,0,0    # 左键释放
```

### 6.3 鼠标滚轮 `w`

**格式:**
```
w,{dx},{dy}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| dx | i32 | 水平滚动量 |
| dy | i32 | 垂直滚动量 (正=向下, 负=向上) |

**示例:**
```
w,0,-120   # 向上滚动
w,0,120    # 向下滚动
```

### 6.4 键盘事件 `k`

**格式:**
```
k,{keysym},{pressed}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| keysym | u32 | X11 KeySym 值 |
| pressed | u8 | 状态 (0=释放, 1=按下) |

服务端将 X11 keysym 转换为 xkb keycode（evdev + 8）注入 Wayland 合成器。

**常用 KeySym 值:**
| 键 | KeySym (十六进制) |
|----|-------------------|
| a-z | 0x61-0x7A |
| A-Z | 0x41-0x5A |
| 0-9 | 0x30-0x39 |
| Space | 0x20 |
| Enter | 0xFF0D |
| Backspace | 0xFF08 |
| Tab | 0xFF09 |
| Escape | 0xFF1B |
| Shift_L/R | 0xFFE1/0xFFE2 |
| Control_L/R | 0xFFE3/0xFFE4 |
| Alt_L/R | 0xFFE9/0xFFEA |
| Super_L/R | 0xFFEB/0xFFEC |
| F1-F12 | 0xFFBE-0xFFC9 |
| Arrows | 0xFF51-0xFF54 |

**示例:**
```
k,97,1     # 'a' 按下
k,97,0     # 'a' 释放
k,65507,1  # Ctrl 按下
k,97,1     # 'a' 按下 (Ctrl+A)
k,97,0     # 'a' 释放
k,65507,0  # Ctrl 释放
```

### 6.5 文本输入 `t`

通过 IME 提交的文本，使用 zwp_text_input_v3 协议注入。

**格式:**
```
t,{text}
```

**示例:**
```
t,你好世界
```

### 6.6 剪贴板 `cw`

浏览器剪贴板内容发送到远程应用。

**格式:**
```
cw,{base64_text_data}
```

**示例:**
```
cw,SGVsbG8gV29ybGQh
```

### 6.7 分辨率调整 `r`

**格式:**
```
r,{width}x{height}
```

**示例:**
```
r,1920x1080
r,1280x720
```

### 6.8 键盘重置 `kr`

释放所有修饰键（Shift/Ctrl/Alt/Super），清除粘滞状态。

**格式:**
```
kr
```

### 6.9 窗口焦点 `focus`

切换焦点到指定窗口。

**格式:**
```
focus,{window_id}
```

### 6.10 窗口关闭 `close`

关闭指定窗口。

**格式:**
```
close,{window_id}
```

### 6.11 Pong 响应 `pong`

响应服务端的 ping 消息。

**格式:**
```
pong
```

### 6.12 运行时设置 `SETTINGS`

动态调整服务端参数。

**格式:**
```
SETTINGS,{json_data}
```

### 6.13 客户端统计

**格式:**
```
_f,{fps}                    # 客户端渲染帧率
_l,{latency_ms}             # 客户端延迟
_arg_fps,{fps}              # 请求目标帧率
_stats_video,{json}         # WebRTC 视频统计
_stats_audio,{json}         # WebRTC 音频统计
_client_activity,{json}     # 页面可见性、焦点与输入活动
```

`_client_activity` 报告页面是否可见；所有连接都不可见（或长时间无焦点、无输入）时服务端让合成器进入空闲。页面被隐藏、内嵌视图的标签未被选中，以及桌面页面中应用标签覆盖主桌面（见[内嵌视图](#内嵌视图postmessage)）时都报告 `false`；它只影响空闲判断，不暂停输入和声音。

## 7. 消息序列图

### 7.1 正常会话流程

```
Client                                          Server
   │                                               │
   │─────────── WS: SDP Offer ─────────────────────▶│
   │◀────────── WS: SDP Answer ────────────────────│
   │                                               │
   │═══════════ ICE-TCP + DTLS 握手 ═══════════════│
   │                                               │
   │◀══════════ Video RTP (H.264) ═════════════════│
   │◀══════════ Audio RTP (Opus) ══════════════════│
   │                                               │
   │◄──────────► DataChannel Open ─────────────────│
   │                                               │
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │◀────────── cursor,{"override":"default"} ─────│
   │                                               │
   │                 [用户输入]                      │
   │─────────── m,512,384,0,0 ─────────────────────▶│
   │─────────── k,97,1 ────────────────────────────▶│
   │─────────── k,97,0 ────────────────────────────▶│
   │                                               │
   │                 [心跳保活]                      │
   │◀────────── ping ──────────────────────────────│
   │─────────── pong ──────────────────────────────▶│
   │                                               │
   │◀────────── stats,{...} ───────────────────────│
   │                                               │
```

### 7.2 剪贴板同步流程

```
Client                                          Server
   │                                               │
   │           [远程应用复制]                        │
   │                                               │
   │           Wayland client → new_selection()     │
   │           → clipboard_pending_mime (延迟)      │
   │           → request_data_device_client_selection│
   │           → 非阻塞 pipe 读取                   │
   │                                               │
   │◀────────── clipboard,SGVsbG8... ──────────────│
   │                                               │
   │           [浏览器粘贴到远程]                    │
   │                                               │
   │─────────── cw,V29ybGQh... ───────────────────▶│
   │                                               │
   │           → set_data_device_selection()        │
   │           → clipboard_suppress_until (500ms)   │
   │           → Wayland client reads via           │
   │             send_selection()                   │
   │                                               │
```

### 7.3 窗口管理流程

```
Client                                          Server
   │                                               │
   │           [新窗口打开]                         │
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │                                               │
   │           [切换窗口焦点]                       │
   │─────────── focus,1 ───────────────────────────▶│
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │                                               │
   │           [关闭窗口]                           │
   │─────────── close,0 ───────────────────────────▶│
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │                                               │
```

## 8. 错误处理

### 8.1 协议错误

当收到无效 DataChannel 消息时，服务端记录日志但不断开连接：

```
debug!("Session {} DC parse error: {}", session.id, e);
```

### 8.2 连接断开

- 服务端通过 ping/pong 机制检测连接活性（15 秒间隔，45 秒超时）
- TCP 连接关闭时自动清理会话
- 客户端应实现自动重连机制

## 9. HTTP 端点

| 端点 | 方法 | 说明 |
|------|------|------|
| `/` | GET | Web 界面（嵌入式前端资源） |
| `/webrtc` | GET (WS) | WebRTC 信令 WebSocket |
| `/health` | GET | 健康检查（JSON） |
| `/metrics` | GET | Prometheus 指标 |
| `/clients` | GET | 活跃连接列表 |
| `/ui-config` | GET | UI 配置 |
| `/ws-config` | GET | WebSocket 端口配置 |

所有 HTTP 端点支持 Basic Auth（可配置）。

---

*文档版本: 2.0*
*最后更新: 2026-02-16*

## 应用级远程共享

使用方式与部署边界见 [远程桌面应用](REMOTE_APPS.md)。`/api/` 管理接口使用所在实例的管理认证，`/remote/v1/` 接口使用访问密码或 B 签发的会话令牌。

### 管理接口

`GET /api/apps` 的每一项在原有字段之外包含：

| 字段 | 适用 | 说明 |
| --- | --- | --- |
| `status` | 全部 | `running`、`starting`（共享应用的运行环境尚未就绪）、`stopped`、`crashed`；CLI 应用另有 `missing`、`installed`、`not_file`、`not_executable`、`invalid_path` |
| `sharing` | 桌面应用 | `{"enabled": bool, "restart_required": bool}`；`restart_required` 表示应用当前的运行方式（是否在独立运行环境中）与 `enabled` 不一致，只在运行中有意义，停止时为 `false` |
| `started` | 远程应用 | 是否已启动（连接）；与本机应用的运行状态一起保存，短时间内重启（5 分钟内）后恢复 |
| `autostart` | 后台、桌面和远程应用 | 原有的随 iVnc 启动偏好，远程应用同样支持：每次 iVnc 启动时自动启动（连接）；通过创建和更新应用接口设置 |
| `remote_state` | 远程应用 | `disconnected`（未启动）、`connecting`、`connected`、`starting`（B 的应用已开启共享，运行环境仍在启动；不是错误，就绪后自动连接）、`stopped`（B 的应用未运行）、`not_ready`（B 需重启应用）、`auth_required`（密码错误或旧连接）、`incompatible`（协议版本不一致）、`unreachable` |
| `remote_detail` | 远程应用 | 状态说明的英文文本 |
| `remote_detail_code` | 远程应用 | 状态说明的错误码，取值同错误响应的 `code`（如 `remote_refused`、`remote_auth_failed`、`remote_app_stopped`）；没有说明时为 `null` |
| `remote_detail_params` | 远程应用 | 错误码的参数，同错误响应的 `params`；没有参数时为 `null` |

| 接口 | 说明 |
| --- | --- |
| `POST /api/apps/{id}/start` \| `stop` \| `restart` | 远程应用：`start` 标记为已启动并连接，`stop` 取消启动并关闭本实例中该应用的全部视图；不影响 B 的进程 |
| `GET /api/apps/{id}/sharing` | 桌面应用的远程访问设置，见下文 |
| `PUT /api/apps/{id}/sharing` | 修改远程访问设置，见下文 |
| `POST /api/apps/{id}/view` | 为共享应用或已启动的远程应用创建视图，返回 `{"url": "/app-view/{token}/", "name": "..."}`；每个页面为每个应用申请自己的令牌，一个令牌只允许一个连接 |
| `POST /api/remote/test` | 请求体 `{"url", "password"}`，用连接地址和访问密码验证连接，返回 B 的 `info`（名称、`status` 为 `running`/`starting`/`stopped` 等、`ready`），或错误响应 |
| `POST /app-view/{token}/close` | 关闭视图及其信令和媒体 |

`/api/apps`、`/api/remote/`、`/remote/v1/` 和应用视图接口的 JSON 错误响应为 `{"error": "...", "code": "...", "params"?: {...}}`。`error` 是原有的英文说明，iVnc CLI 和脚本读取它；`code` 是稳定的 snake_case 错误码，如 `app_name_exists`、`share_password_length`、`remote_url_not_share`、`remote_not_started`，无法归类的内部错误（数据库、文件读写等）为 `internal`；`params` 是说明中的动态部分，如 `name`、`status`，其中 `detail` 是无法翻译的底层说明，如网络错误的完整原因。控制台按 `code` 以界面语言显示，把 `detail` 附在后面；不认识的错误码显示通用文本并附上英文原文。

`GET /api/apps/{id}/sharing` 返回：

```json
{
    "enabled": true,
    "password_set": true,
    "password": "解密后的访问密码",
    "password_viewable": true,
    "share_path": "/remote/v1/apps/{id}",
    "restart_required": false
}
```

`password` 只在 `password_viewable` 为 `true` 时返回明文，否则为 `null`；从旧版本升级的密码只有校验值，此时 `password_set` 为 `true`、`password_viewable` 为 `false`。

`PUT /api/apps/{id}/sharing` 请求体 `{"enabled": bool, "password"?: string}`，省略或留空 `password` 时保留原密码。成功返回 `{"ok": true, "revoked": bool, "restart_required": bool}`：`revoked` 表示因关闭共享或修改密码而断开了远程会话。没有设置过密码时开启共享（`share_password_required`）、密码长度不在 8–256 字节（`share_password_length`）等错误返回 4xx 和错误响应。

### B 的远程接口

| 接口 | 说明 |
| --- | --- |
| `POST /remote/v1/apps/{id}/authenticate` | 请求体 `{"password"}`，换取仅绑定此应用和当前策略版本的短期会话令牌；按来源地址限流 |
| `GET /remote/v1/apps/{id}/info` | 持 Bearer 令牌读取名称、状态、`ready`/`detail`（是否需重启应用）、节点身份和协议版本 |
| `GET /remote/v1/apps/{id}/socket` | 持 Bearer 令牌建立中继信令 WebSocket |
| `GET /remote/v1/apps/{id}/data/{path}` | 仅转发 UI 配置和渲染状态；不代理管理 API，也不提供文件下载 |
| `DELETE /remote/v1/apps/{id}/session` | 持 Bearer 令牌结束该会话令牌；A 的“测试连接”用完即调用，不占用每个应用的会话名额 |

响应为 `{"protocol", "node_id", "app_id", "name", "token"}`。令牌只保存在 B 的内存中，空闲 12 小时后失效；每个应用的令牌数有上限，超出时只淘汰该应用最旧的令牌。B 关闭共享或修改密码时策略版本递增，已签发的令牌立即失效，使用它们的信令和媒体同时结束；A 收到 401 后用保存的密码重新认证。协议版本不一致时 A 显示 `incompatible`。

以下旧接口已删除：`/api/desktop/services`、`DELETE /api/apps/{id}/sessions/{session}`、`DELETE /api/apps/{id}/grants/{grant}`、`DELETE /remote/v1/apps/{id}/credential`，以及运行环境的 `/app/files*`。

### 应用视图信令

应用视图沿用 SDP 信令协议，所有连接都可以发送输入、剪贴板、上传和分辨率调整，没有控制权消息。结束消息 `{"ivnc_revoked":true,"reason":"revoked|stopped|closed"}` 要求客户端停止自动重连：`revoked` 为共享关闭或密码修改，`stopped` 为应用停止（包括 A 停止远程应用），`closed` 为视图被关闭或连接配置被替换。

ICE-TCP 通过已授权 SDP 中的 ICE ufrag 路由，仍需完成运行环境的 ICE/DTLS 握手。信令与媒体均绑定同一应用和运行代次，应用重启不复用旧媒体会话。

### 内嵌视图（postMessage）

桌面页面把应用标签以同源 iframe 打开：`/app-view/{token}/?embed=1`。内嵌视图不显示自己的工具栏、底边触发区、连接指示、状态栏和“无窗口”提示，由父页面负责。双方只处理 `event.origin === location.origin` 且 `event.source` 为对方窗口的消息，发送时也指定同源。

父页面 → iframe：

| 消息 | 说明 |
| --- | --- |
| `{type: "ivnc-view-active", active: bool}` | 标签被选中或切走；切走时释放按住的键，选中时获得焦点 |
| `{type: "ivnc-unmute"}` | 页面已获得用户手势，可以播放声音 |
| `{type: "ivnc-upload", files: File[]}` | 把文件上传到该应用 |
| `{type: "ivnc-ime"}` | 切换输入法模式 |

iframe → 父页面（每条消息另带 `view: <iframe 的 pathname>`，父页面据此忽略已被替换的视图）：

| 消息 | 说明 |
| --- | --- |
| `{type: "ivnc-view-ready"}` | iframe 已安装消息监听；父页面随即重发当前的 `ivnc-view-active`，用户已选中过该标签时再发 `ivnc-unmute`。iframe `load` 时发出的同类消息可能早于监听而丢失 |
| `{type: "ivnc-view-state", state: "connecting" \| "connected" \| "ended", reason?: string}` | 连接状态；`ended` 的 `reason` 来自 `ivnc_revoked` |
| `{type: "ivnc-ime-state", active: bool}` | 输入法模式变化 |
| `{type: "ivnc-edge-hover"}` | 指针到达底边，用于显示未固定的工具栏 |
| `{type: "ivnc-reopen"}` | 运行代次变化，请父页面申请新令牌并重新打开 |

应用标签被选中期间，桌面页面在自身的 `_client_activity` 中报告 `visible: false`，主桌面合成器因此可以空闲；切回窗口标签或该标签消失后恢复 `visible: true`。主桌面的输入和声音不受影响。

控制台页面（桌面页面中的 iframe）在用户启动应用后向父页面发送 `{type: "ivnc-app-started", id}`，桌面页面在该应用的标签出现时切换过去。
