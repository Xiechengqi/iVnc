# Safari 画面兼容修复与 Review

日期：2026-11-13。基于 `70d8da2`，范围为 WebRTC 视频协商、H.264 编码输出、首次播放及应用视图恢复。

## 根因与修复

Safari 支持 WebRTC 和 H.264。此次源码中确认的是视频协商与实际输出不一致，以及播放失败后缺少有效反馈的兼容性 bug；不是需要另建一套 Safari 视频协议。

| 问题 | 修复后的行为 |
| --- | --- |
| 从 str0m 全局能力列表选择 H.264 PT，其中可能包含没有被本次 offer 接受的格式 | 只启用实际配置的视频编码器及 Opus；从对应 media 的 `remote_pts()` 与 codec 参数交集中取得 PT；没有结果时不猜测 96/111 |
| `videoconvert` 未限定格式，x264 可选择 Y444 并输出 High 4:4:4，尽管 SDP 声明 Constrained Baseline | 原始输入限定 8-bit 4:2:0（I420/NV12），H.264 输出限定 Constrained Baseline；不具备对应输出 caps 的硬件编码器回退软件编码 |
| SDP Level 3.1 与实际 1080p/高帧率码流不一致 | 按 H.264 Annex A 同时限制宏块数、长边、宏块率和码率；编码器按实际参数设置 level，x264 使用受码率限制的模式 |
| 新接收者加入时可能收到尚未调整好的较高 level 码流 | pending 会话也注册接收限制；H.264 转发等待可兼容 SPS；会话结束或超时销毁时释放限制 |
| 分辨率或编码参数变化后 GStreamer 的随机 RTP 起点跳变 | 每条 str0m 会话保留连续视频时间轴；重建时清空旧缓存并请求 IDR，每个 IDR 携带 SPS/PPS |
| ICE/DataChannel 成功即隐藏连接提示，自动播放被拒绝时按钮未及时显示 | 视频状态独立于连接状态，收到可播放画面后才显示成功；自动播放被拒绝立即出现播放按钮，点击在用户操作调用栈中执行 `play()` |
| 隐藏 iframe 被 Safari 暂停后无法恢复，父层提示还可能挡住子视图按钮 | 文档可见或视图激活时恢复视频并请求关键帧；子视图负责播放提示，父层保留访问撤销、过期和重开提示 |

本机旧管线 1920×1080@60 的实验输出为 `f4002a`（High 4:4:4 Level 4.2）。修复后实际输出为 `42c02a`（Constrained Baseline Level 4.2，I420 输入）。SDP 的 `42e0` 与码流的 `42c0` 是 RFC 6184 中同一 Constrained Baseline profile 的等价表示；实际发送 level 可以低于接收上限。

## 接收能力和画质边界

浏览器原生 offer 常只声明 Level 3.1。客户端通过 WebRTC 类型的 `MediaCapabilities.decodingInfo` 探测更高接收能力，仅在返回支持后提高接收 level（先探测 5.2，再探测 4.2）。探测缺失、失败、超时或浏览器拒绝修改 SDP 时保留原生 offer，不按 UA 猜测能力。

同一桌面/应用 worker 目前只有一个公共编码流，输出必须适合所有当前接收者。先尽量保留分辨率和比例，再限制帧率；超过单帧限制才缩小画面，码率也受 level 限制。低能力接收者会影响同一公共流的其他查看者；它离开后恢复最新请求的尺寸和帧率。不同应用 worker 独立适配。此实现没有增加按客户端独立编码。

4:2:0 会降低细小彩色文字的色度分辨率，这是标准浏览器 H.264 解码兼容性的取舍；不会改变应用真实像素坐标与输入映射。需要更高文字质量时可增加可用码率，但不能通过重新发送未协商的 High 4:4:4 来处理。

## 播放诊断和生命周期

前台活动视图连接后等待首帧，4 秒尚未出图时请求关键帧，8 秒后结合接收字节、解码帧数及 video 状态显示无画面数据、解码失败或播放失败。缺少 Safari 帧统计字段时不据此断言解码器失败。静止画面在首帧显示后不会触发上述首帧超时。

视频保持 `muted`、`autoplay`、`playsInline`，音频使用独立元素。按 track 构造 MediaStream，支持合法的 stream-less `ontrack`。播放拒绝不无限自动重试；暂停后显示播放按钮，网络/解码故障可重新连接。不兼容编码或浏览器缺少 WebRTC API 会显示明确提示。

重连/替换流隔离旧的播放 Promise、统计回调和 peer 事件；销毁时取消首帧观察器、可见性监听、播放检测、重连计时器及信令重试。替换的信令 socket 不能再触发新会话协商。已结束的应用视图不能被晚到的播放成功消息恢复为连接成功。

## 验证

```bash
node --test scripts/video_playback_test.mjs scripts/viewport_test.mjs scripts/clipboard_test.mjs
npm --prefix web/ivnc run build
cargo test --lib --features tls --offline
cargo build --bin ivnc --features tls --offline
IVNC_BIN=target/debug/ivnc node scripts/video_playback_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/viewport_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/toolbar_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/clipboard_smoke.mjs
IVNC_BIN=target/debug/ivnc bash scripts/remote_app_smoke.sh
```

浏览器回归使用独立实例、配置目录、端口及真实 Wayland 应用。新的视频 smoke 使用浏览器默认自动播放策略，CDP evaluate 不伪造用户激活；受限播放通过确定性的 play 拒绝模拟，再以真实鼠标点击恢复。低能力场景使用原生 SDP、关闭能力提升探测，验证多个查看者在公共流调整前后均能解码。

| 检查 | 结果 |
| --- | --- |
| Rust 单元与实际编码 | 140 项通过；含非默认 PT、不同 level、格式拒绝、接收限制生命周期、RTP 连续性、真实 x264 SPS/PPS 及 I420/NV12 检查 |
| 前端状态、能力探测、剪贴板与输入 | 55 项通过 |
| 视频专项浏览器回归 | 9 项通过；检查解码画面的像素不全黑、默认自动播放、stream-less track、点击恢复、多接收者、连续 resize、不兼容格式及嵌入视图恢复 |
| 窗口工作区域 | 14 项通过；含固定 toolbar、Retina 尺寸、手动缩放、滚动及真实鼠标坐标 |
| toolbar | 31 项通过 |
| 完整剪贴板 | 14 项通过；含 HTTP 本机复制与立即粘贴 |
| 远程应用及中继 | 64 项通过；含并行视图、真实输入/上传和访问撤销 |
| 生产前端、TLS 服务端构建 | 通过；Vite 保留已有的单文件体积提示 |
| Rust 格式、变更 JavaScript 语法及 diff 空白检查 | 通过 |

整体 review 核对了实际编码与 SDP 的一致性、公共输出限制的注册/释放、管线切换的时间轴与缓存、首帧判定、播放拒绝、旧异步结果隔离、iframe 激活与结束、重连/销毁及输入/剪贴板回归。Review 中补齐了最新请求尺寸的恢复、失败重建的重试间隔、错误状态不被晚到回调覆盖，以及已结束视图不再接收播放成功消息；仅修改帧率或码率时不会重新分配桌面缓冲或重排应用窗口。未发现尚未处理的阻断问题。

## 实机范围

本轮实际执行环境为 Linux Chrome 和 x264 软件编码器，没有 macOS Safari 或可用硬件编码设备。以上结果验证了协议约束和真实浏览器链路，不能等同于 Safari 或硬件编码的实机验收。

macOS Safari 的验收应覆盖首次进入主桌面/共享/远程视图、自动播放被禁止后的点击播放、后台切回、应用 tab 切换、Retina 与分辨率变化、多客户端、断线重连及输入；记录 Safari/macOS 版本、连接 origin、协商 PT/fmtp 与实际接收/解码统计。浏览器未开放 WebRTC 的 HTTP 上下文需要改用 HTTPS；本修复不能绕过浏览器对 API 的限制。
