# 应用可用区域与固定 toolbar

固定 toolbar 以前只改变浮层的显示状态，主视频与输入层仍占满整个浏览器；自动分辨率也读取整个窗口高度，导致应用底部被挡住。共享视图另有一份扣减逻辑，与主视图不一致。

`web/ivnc/viewport.js` 现在统一管理可用区域和视频/输入层的布局：

- 固定且可见的 toolbar 按实际测量高度预留空间。高度变化由 `ResizeObserver` 跟踪，不写死 36px。
- `#app` 使用动态视口高度减去预留高度，视频与输入层使用同一组位置和尺寸。
- 未固定时不预留；鼠标移到边缘临时展开 toolbar 不触发桌面重排。
- 嵌入应用由父页面分配可用区域，不再次扣减；单独打开的应用视图使用自己的 toolbar。
- 视频单独全屏时，其外部 toolbar 不可见，不预留；退出后恢复。
- 缩放期间保留已确认的键盘遮挡量，仍按当前浏览器窗口高度计算可用区域。窗口缩放、旋转或调整大小后，应用底边继续与固定 toolbar 顶边对齐。

例如可见页面高 800 CSS px、固定 toolbar 高 36 CSS px，可用高度为 764 CSS px。窗口缩小到 650 CSS px 后，可用高度为 614 CSS px；取消固定后恢复为 650 CSS px。DPR 不参与 toolbar 高度扣减。

## 分辨率与输入

适应窗口模式将完整远端画面等比例缩放到当前客户端的可用区域，原始大小模式提供横纵滚动条。Pin、窗口大小、DPR 和浏览器缩放只影响当前客户端，不再通过 `r,宽x高` 改变共享桌面分辨率。服务端分辨率由管理员在设置中明确修改，详见 [不同客户端的显示适配](DISPLAY_ADAPTATION.md)。

适应窗口／原始大小偏好保存在本机，刷新后恢复；切回适应窗口时清除滚动位置。客户端仅用 `_viewport` 报告可用区域，不据此请求调整远端应用窗口。

鼠标和触摸均根据视频实际显示位置映射到桌面坐标，使用相同映射；触摸不再次乘 DPR。滚动和布局变化刷新坐标，画面外的留白不接收远端点击。

共享/远程应用的外层由工作区域定位，不再跟随主视频的手动缩放留白。重新布局不替换 iframe、访问令牌或 DataChannel。

## 验证

```bash
CHROME_BIN=/usr/bin/google-chrome node --test scripts/viewport_interaction_test.mjs scripts/mobile_input_test.mjs
npm --prefix web/ivnc run build
IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/viewport_smoke.mjs
```

布局 smoke 使用独立 iVnc、真实 Wayland Chrome、独立 headless 查看器和共享 GTK 应用。实际点击远端页面底部按钮，覆盖固定切换、动态高度、窄屏、奇数尺寸、DPR 切换、原始大小与滚动、刷新、全屏、浏览器实际缩放后的窗口变化及嵌入应用。所有运行目录、端口和浏览器配置隔离。

2026-11-15 回归定位：`d063545` 增加移动端键盘适配后，在 `visualViewport.scale != 1` 时保留了完整的旧可用区域。浏览器从 800px 缩到 650px，应用底边仍为 764px，固定 toolbar 顶边则已变为 614px，导致遮挡。修复只保留已确认的键盘遮挡量，每次布局重新读取窗口高度，并测量当前固定 toolbar 的高度。普通未缩放页面的 Pin 切换在修复前已正常。

视口浏览器回归改为读取实际桌面 CSS，防止测试使用另一套布局而漏报；新增的窗口缩小用例在修复前于 Chromium/WebKit 均失败（764 ≠ 614）。修复后视口每个引擎 9 场景、移动输入每个引擎 25 场景通过，共 68 场景；Node 执行器含父级分组为 74 项。真实实例布局 smoke 19 项、前端生产构建、JavaScript 语法及补丁空白检查通过。

整体 review 核对了未缩放、缩放中窗口大小变化、软键盘、Pin 切换、实测 toolbar 高度、全屏和嵌入视图。共享应用只扣减父页面 toolbar 一次，原 iframe 与 DataChannel 保留，服务端桌面及原生应用尺寸保持稳定。已验证范围内没有剩余阻塞项。移动设备软键盘的系统弹出行为仍需实机验证，本轮新增验证针对其视口几何状态。
