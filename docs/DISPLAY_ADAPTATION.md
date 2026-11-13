# 客户端独立显示适配

每个主桌面或本地共享应用运行时拥有固定的逻辑桌面。客户端完整等比居中显示，空白区域不向远端发送鼠标或滚轮输入；“原始大小”按视频像素显示并允许滚动。各客户端可以独立切换，嵌入应用只预留一次父页面的工具栏高度，不重建 iframe 或 WebRTC 连接。

终端、控制台等本地弹窗使用浏览器 CSS 像素和工作区布局，不跟随远程视频缩放。这样能保持文字清晰、控件可读，并避免 CSS transform 使终端文字选区偏离光标。

## 尺寸与配置

- **桌面尺寸**：合成器输出、应用布局、输入坐标的唯一依据。
- **视频尺寸**：共同编码流的实际尺寸。H.264 接收级别限制触发 `videoscale` 缩小视频，不配置或缩小应用窗口。视频输入始终为桌面尺寸，输出保持 Constrained Baseline、8-bit 4:2:0。
- **客户端显示区域**：根据当前浏览器工作区、本地显示模式、工具栏和全屏状态计算。DPR 不叠加到远程输入坐标；视频插值依据实际物理像素缩放比例选择。

优先级依次为：`[display].locked = true` 的配置宽高、旧版 `IVNC_MANUAL_WIDTH/HEIGHT` 或启用的 `IVNC_IS_MANUAL_RESOLUTION_MODE`、控制台保存宽高、服务器 `[display]` 宽高。锁定配置拒绝在线修改。

主桌面保存在 `$IVNC_HOME/console.json` 的 `display` 字段；共享应用保存在对应应用 `worker-state/console.json`，互不覆盖。编码降级不持久化。旧版浏览器 `manual_width/height` 仅迁移本地适应/原始偏好，宽高被清除，不会在重连时重放。

允许偶数宽高，宽 320–7680、高 200–4320，总像素不超过 8,294,400，限制合成器缓冲与编码资源。

## 管理与协议

通过现有控制台认证和访问策略保护的 `GET/PUT /api/console/display` 管理主桌面。`?app_id=<id>` 仅可管理本实例正在运行的共享桌面应用，父实例通过内部凭据转发给该应用运行时。远程应用由来源实例控制，app-view 及 remote 网关的 HTTP 允许列表不包含管理接口。

PUT 为 `{width,height,revision}`，revision 从 GET 获取。不匹配或另一个修改待处理时返回 409，锁定返回 423，超出范围返回 400。返回 200 之前，已完成新编码管线启动、合成器缓冲分配、输出应用和配置原子保存；失败保留原版本，并还原旧缓冲与编码管线。只有实际桌面改变才配置应用窗口。独立的运行参数保存与尺寸保存串行合并，避免相互覆盖。

客户端发送 `_display,1` 协商，服务端私发 `display,{...}`，之后广播变化。状态包含 `protocol`、`revision`、`desktop`、`encoded`、`locked`、`pending`、`last_error`。`revision` 只随桌面坐标改变递增，重启后重新协商。`last_error` 记录上一次改尺寸失败的原因，查询状态仍正常成功，不会妨碍再次提交。

现代指针消息为 `i,<revision>,<m/m2/b/w 消息>`，坐标使用桌面像素；服务器入队、消费时校验版本。桌面切换释放按键，前端等待新的显示帧再恢复输入。鼠标、触摸、笔统一映射；相对移动按缩放比例计算并保留小数余量；拖拽在图像外释放仍清理按钮。

`_viewport,{width,height,dpr,mode}` 仅报告当前连接的工作区。旧 `r,WxH` 不再调整共享桌面。旧指针协议由服务端从当前编码像素转换到桌面像素。新客户端连接旧服务器时不发送 `r`，缺少 display 能力时使用视频像素作为旧输入坐标。其他旧客户端仍可能修改旧服务器，只有升级服务器才能提供完整隔离。

编码管线重建清空旧 RTP 帧及关键帧缓存，请求新的 SPS/PPS+IDR，沿用既有每会话 RTP 时钟转换。接收级别下降仍影响所有客户端的视频清晰度与 FPS；每客户端独立编码和原生应用布局不在本次范围。

## 验证

构建后执行：

```sh
npm --prefix web/ivnc run build
cargo test --features tls --offline -- --test-threads=1
cargo build --features tls --bin ivnc --offline
npm --prefix web/ivnc run test:viewport
npm --prefix web/ivnc run test:toolbar
IVNC_BIN=target/debug/ivnc node scripts/viewport_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/video_playback_smoke.mjs
```

`viewport_interaction_test.mjs` 在 Chromium/WebKit 运行生产适配和输入模块，覆盖比例、空白区、滚动、触摸/笔、拖拽释放、几何版本、相对移动余量与旧服务端回退。

`viewport_smoke.mjs` 使用真实服务器、原生 Wayland Chrome 及多个 WebRTC 客户端，校验服务器输出和应用实际窗口尺寸，覆盖横竖屏/DPR、刷新顺序、低 H.264 级别、管理修改/冲突/保存失败回滚、共享应用范围和重启恢复。视频、工具栏、剪贴板、手势、终端浮窗和两实例远程应用另有回归脚本。

Linux WebKit 自动化不能代替 macOS Safari 真机验收；发布前应在 Safari 补充横竖屏设备同时连接、刷新、缩放、全屏和共享应用切换验证。

本轮实现审查、修正项与验证记录见 [整体 Review](DISPLAY_ADAPTATION_REVIEW.md)。
