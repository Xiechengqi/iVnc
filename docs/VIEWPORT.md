# 应用可用区域与固定 toolbar

固定 toolbar 以前只改变浮层的显示状态，主视频与输入层仍占满整个浏览器；自动分辨率也读取整个窗口高度，导致应用底部被挡住。共享视图另有一份扣减逻辑，与主视图不一致。

`web/ivnc/viewport.js` 现在统一管理可用区域和视频/输入层的布局：

- 固定且可见的 toolbar 按实际测量高度预留空间。高度变化由 `ResizeObserver` 跟踪，不写死 36px。
- `#app` 使用动态视口高度减去预留高度，视频与输入层使用同一组位置和尺寸。
- 未固定时不预留；鼠标移到边缘临时展开 toolbar 不触发桌面重排。
- 嵌入应用由父页面分配可用区域，不再次扣减；单独打开的应用视图使用自己的 toolbar。
- 视频单独全屏时，其外部 toolbar 不可见，不预留；退出后恢复。

例如可见页面高 800 CSS px、固定 toolbar 高 36 CSS px，可用高度为 764 CSS px。自动分辨率根据容器尺寸计算，仅在发送时进行 DPR 换算和偶数对齐。物理像素模式下 DPR=2 时，对应高度为 1528 px。DPR 单独变化也会更新布局与分辨率。

## 分辨率与输入

自动模式在可用区域尺寸变化后使用现有 `r,宽x高` 协议更新服务端，沿用 500ms 去抖并对相同分辨率去重。初次连接和重连主动同步当前尺寸。切换到手动模式、断线和销毁会取消待执行计时器；零尺寸视图不提交无效分辨率。

手动模式保留用户指定的远端分辨率：适应窗口时在可用区域内等比例缩放；原始大小时容器提供横纵滚动条。窗口变化只重排显示，不覆盖手动分辨率。退出手动模式恢复自动布局并清除滚动位置。手动尺寸与模式使用一致的存储字段，刷新后恢复并主动发送分辨率。

鼠标和触摸均根据视频实际显示位置、原生帧尺寸和 `object-fit: contain` 留白计算坐标，使用相同映射；触摸不再次乘 DPR。滚动和布局变化刷新坐标。坐标被限制在有效帧范围内。

共享/远程应用的外层由工作区域定位，不再跟随主视频的手动缩放留白。重新布局不替换 iframe、访问令牌或 DataChannel。

## 验证

```bash
node --test scripts/viewport_test.mjs scripts/clipboard_test.mjs
npm --prefix web/ivnc run build
CARGO_INCREMENTAL=0 cargo build --features tls --bin ivnc
IVNC_BIN="$PWD/target/debug/ivnc" node scripts/viewport_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" node scripts/toolbar_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" node scripts/clipboard_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" bash scripts/remote_app_smoke.sh
```

布局 smoke 使用独立 iVnc、真实 Wayland Chrome、独立 headless 查看器和共享 GTK 应用。实际点击远端页面底部按钮，覆盖固定切换、动态高度、窄屏、奇数尺寸、DPR 切换、手动缩放与滚动、刷新、全屏、嵌入与单独应用视图。所有运行目录、端口和浏览器配置隔离，不接触现有用户实例。

本次验证通过 133 项检查：输入与剪贴板单元测试 16 项、布局 smoke 14 项、toolbar smoke 28 项、剪贴板 smoke 11 项、远程应用 smoke 64 项。前端与启用 TLS 的服务端构建均通过；Vite 仍有已有的单文件体积提示。

整体 review 核对了尺寸计算、DPR 与偶数对齐、手动模式恢复、输入坐标、全屏和嵌入视图边界，以及连接切换时的计时器与观察器清理，未发现阻塞问题。主桌面手动缩放的留白不会缩小共享应用区域；固定状态切换与重新布局保持原有 iframe、访问令牌、连接和应用选择。
