# 剪贴板与粘贴事务

本机复制后第一次在远端粘贴必须使用本次内容。原实现先转发 V 按键，随后在浏览器 `paste` 事件里发送 `cw`；服务端又把两者分入不同队列。因此远端可能先粘贴旧内容，下一次才使用新内容。固定延时、1 秒节流和“每轮先处理剪贴板”都无法提供顺序保证。

## 当前流程

1. `input2.js` 识别 Ctrl/Cmd+V、Ctrl+Shift+V 和 Shift+Insert，交给 `clipboard.js` 捕获用户原生 `paste` 事件，不提前发送 V。本地 toolbar、表单和 Web 终端保持原生输入。
2. 一次手势保存文本、快捷键类型和目标窗口。后续键盘/鼠标输入按顺序排在这次操作后；使用 `event.repeat` 抑制长按，不按时间吞掉独立粘贴。
3. 同一有序输入 DataChannel 上协商能力，发送 `paste` 的 start/chunk/commit。文本按 UTF-8 字节分块，空文本合法；图片/富文本不转换为旧文本粘贴。
4. start 在合成器输入队列中绑定实际 `WlSurface`，commit 只在完整长度、UTF-8、期限和目标校验通过后执行。应用关闭、焦点改变、取消或连接结束后的操作不迁移到别的窗口。
5. 合成器先建立独立 `Arc<[u8]>` selection，再派发对应快捷键，并恢复之前持有的修饰键。每份 Wayland offer 保留自己的内容，后来的粘贴不能改变旧 offer 的读取结果。
6. 应用请求读取 offer 后，内容异步写入管道并关闭写端（EOF），才向请求连接返回 `paste_ack`。`applied` 确认 selection、按键派发和管道交付，**不表示 Chrome 的异步文本插入已经完成**。写入失败或超过 10 秒返回 `delivery_failed`；应用始终不读取时由 30 秒操作期限结束。客户端不自动重试。

真实 Chrome 验证还发现，仅在按键派发后确认会使快速 A/B 粘贴复用尚未完成的第一次读取，得到 AA；将确认放在管道关闭之后，连续粘贴得到预期 AB。同一 seat 的其他连接在前一次交付结束前发起事务时返回 `busy`，避免覆盖尚未读取的 selection。

每条连接使用单调递增操作 ID。相同 ID 不会再次执行；相同文本的新手势仍会执行。浏览器端最多保留 16 MiB 待粘贴文本，单条请求最多 16 MiB、单块 12 KiB，传输/确认超时 30 秒。服务端限制待执行请求数；Wayland 输出采用独立单线程异步执行器、最多 64 个并发写入及 10 秒写入期限，避免慢应用阻塞合成器。

## 双向同步和生命周期

- 每次 selection 变化增加 revision。异步读取管道绑定启动时的 revision；发生新选择、超过 16 MiB 或 10 秒未读完时丢弃旧读取。
- 浏览器收到的内容带 `revision` 和 `origin`。`origin=browser` 不再写回本机；远端复制只由当前、可见、有焦点的视图写本机剪贴板。异步写入串行执行，并用连接和操作代次丢弃尚未执行的旧写入。
- 不再依赖焦点事件中的 `readText()` 预同步，不保留 500 毫秒 blanket suppression，也不使用文本相等代替远端应用确认。
- Ctrl+C/X 后紧接 Ctrl+V 时，粘贴等待该远端复制的 selection 回传，避免使用尚未更新的本机剪贴板。若本机写入权限失败，在当前远端上下文仍可使用已确认的复制内容；离开该上下文即取消该状态。
- 失焦、本地编辑器获得焦点、页面隐藏、切换视图、连接关闭、页面销毁均取消未完成操作，释放按键并清理监听器。重连丢弃旧连接回调和残余剪贴板分块。已经派发给应用的操作不回滚。
- 浏览器不支持/拒绝本机剪贴板写入时给出提示，不通过隐藏 textarea 抢走本地编辑器焦点。

## 兼容边界

旧 `cw/c/cws/cwd/cwe` 文本协议保留，统一走输入队列，在真正应用后广播。旧前端原先“按键先发”的竞态仍需升级前端才能解决。

新前端只对支持 paste v1 的服务器启用跨端粘贴；旧服务器会得到升级提示，避免悄悄粘贴旧内容。服务端显式关闭剪贴板同步时，保留远端原生快捷键语义。

文本回传支持原来的裸 base64 以及带元数据的 JSON。分块使用 4095 字节，使中间 base64 块没有 padding，兼容旧客户端先拼接再解码的方式。

原生远端应用的菜单操作继续使用应用自身的剪贴板；本文的跨端内容绑定适用于 Web 捕获到的粘贴手势。图片、文件和富文本保留原有独立协议，不在本次文本事务范围内。

## 验证

```bash
node --test scripts/clipboard_test.mjs
cargo test --features tls --lib
npm --prefix web/ivnc run build
# 在前端构建后构建服务端，确保嵌入最新前端
cargo build --features tls --bin ivnc
IVNC_BIN="$PWD/target/debug/ivnc" node scripts/clipboard_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" node scripts/toolbar_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" SKIP_BUILD=1 SKIP_UNIT=1 bash scripts/desktop_smoke.sh
IVNC_BIN="$PWD/target/debug/ivnc" bash scripts/remote_app_smoke.sh
```

clipboard smoke 启动独立 iVnc、独立 Wayland Chrome 和独立 headless Web 查看器，通过真实系统剪贴板 API 和按键验证实际插入结果，不接触正在运行的用户实例。失败时保留临时目录；`KEEP_TMP=1` 可保留成功运行的日志。诊断只使用测试生成的文本。

本次验证：Rust 111 项、前端事件/生命周期 13 项、真实 Chrome 剪贴板 11 项、toolbar 28 项、桌面管理 56 项、远程应用共享 64 项全部通过；前端构建和启用 TLS 的服务端构建通过。

整体 review 已修正提前确认造成的连续粘贴串值、合成器线程缺少 Tokio 运行时导致的管道异常、旧连接回调干扰重连、分块 base64 padding、本地输入框误拦截和同一 seat 并发事务覆盖。确认与取消的边界遵循上文说明，不依赖固定延时重试。
