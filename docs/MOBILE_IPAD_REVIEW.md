# 移动端手动键盘与 iPad 铺满窗口修复 Review

日期：2026-11-16。基础提交：`025d7c9`。本轮实现与整体 review 已完成，已验证范围内没有剩余阻断问题。当前使用行为见 [移动端键盘](MOBILE_INPUT.md) 与 [窗口工作区](VIEWPORT.md)。

## 问题与实现

移动端自动弹键盘来自两条明确的前端路径：点击已知编辑区域时直接聚焦输入代理，以及点击后收到编辑元数据时再次聚焦。现在点击画面只更新位置、操作远端应用，元数据只更新输入用途与光标；移动端仅在用户点击 toolbar 的“键盘输入”按钮时同步聚焦。代理失焦、手动关闭、切换应用和断线后结束本次移动输入，后续点击或元数据不会重新打开。桌面中文输入偏好与点击恢复焦点继续保留。

原铺满实现仅用 `object-fit: fill` 对视频原生层做非等比缩放，同时工作区只处理 Visual Viewport 的高度。前者在部分 Safari 媒体呈现路径存在兼容风险，后者遗漏可视宽度与横向偏移；未取得用户 iPad 的系统版本和现场渲染数据，不能确认其唯一根因。WebKit 历史问题 [263036](https://bugs.webkit.org/show_bug.cgi?id=263036) 记录过相关呈现差异，但该问题已有修复，不能据此认定当前所有 Safari 均受影响。

现在铺满模式先让视频布局框保持源比例并放在工作区内，再从左上角用 CSS transform 独立拉伸横纵轴，使最终画面覆盖整个工作区。输入层直接使用最终尺寸，鼠标、触摸与候选位置继续按最终边界换算；切回适应窗口时清除变换。工作区与 toolbar 共用可视宽度和横向偏移，底部仍按实际 toolbar 高度预留，不产生查看区域滚动。

## 整体审查

| 审查点 | 结果 |
| --- | --- |
| 手动键盘入口 | 普通点击、延迟元数据、刷新、应用切换、重连均不自动打开；按钮保留原始用户点击栈内的同步聚焦 |
| 焦点与关闭 | 移动端失焦后结束输入状态；已手动打开时仍可操作远端光标；iPad 桌面 UA 与外接鼠标走相同行为 |
| 文字与桌面 IME | 原生文字、兼容通道、组合提交去重与取消保留；桌面偏好、光标位置和临时恢复焦点回归通过 |
| 铺满与适应 | 源比例布局框和变换后画面均在工作区内；模式切换、视频元数据与尺寸变化后重新计算 |
| 输入映射与底部 | 变换后的画面与输入层边界一致；真实远端底部按钮可直接点击，固定与临时显示 toolbar 均预留空间 |
| 可视区域与嵌入 | 横竖屏、可视宽度缩窄及横向偏移同步 toolbar；嵌入视图继承父级区域，避免重复扣减；pinch 不作为新的键盘遮挡 |
| 生命周期 | 新增 CSS 变量在销毁时清理；原 ResizeObserver、视口、元数据及 DPR 监听清理保留 |
| 实例与 CI | UI 测试仅本地执行；使用隔离实例，未重启生产进程、部署或提交推送 |

## 本地验证

| 验证 | 结果 |
| --- | --- |
| 移动输入、窗口、显示模式、IME、文字提交、toolbar 与手势回归 | Node 输出 248 项通过，包含父分组；覆盖 Chromium 与 WebKit |
| 剪贴板与视频状态回归 | 53 项通过 |
| 真实 Electron 移动输入，主桌面与共享应用 | Chromium / WebKit 共 26 项通过 |
| iPad Pro 11 模拟配置下的真实 Electron 输入 | Chromium 主桌面与共享应用共 14 项通过 |
| 真实窗口与多客户端布局 | 22 项通过，含底部点击、fit/fill、toolbar 显隐、pin、刷新、DPR、全屏与共享视图生命周期 |
| 前端生产构建、JS 语法及补丁空白检查 | 通过；构建保留既有大于 500 KB 的 chunk 提示 |

主要复测命令（Electron 路径按本地安装设置）：

```sh
CHROME_BIN=/usr/bin/google-chrome node --test scripts/mobile_input_test.mjs scripts/viewport_interaction_test.mjs scripts/display_mode_interaction_test.mjs scripts/ime_input_test.mjs scripts/text_commit_test.mjs scripts/toolbar_interaction_test.mjs scripts/viewport_test.mjs scripts/gesture_navigation_smoke.mjs
node --test scripts/clipboard_test.mjs scripts/video_playback_test.mjs
npm --prefix web/ivnc run build
IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome ELECTRON_BIN=/path/to/electron node scripts/mobile_input_smoke.mjs
MOBILE_DEVICE='iPad Pro 11' BROWSERS=chromium IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome ELECTRON_BIN=/path/to/electron node scripts/mobile_input_smoke.mjs
IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/viewport_smoke.mjs
git diff --check
```

另用真实 canvas MediaStream 绘制四条彩色边，分别在 Chromium / WebKit 的 iPad 横竖屏模拟中观察新旧实现。强制媒体使用 cover 模拟“原生层保留比例导致裁切”的路径时，旧实现丢失左右边，新实现完整显示四边；截图像素抽查和目视核对通过。这验证了兼容策略，属于人工模拟，不是用户 iPad 故障的真机复现。

## 验证边界

- 没有实体 iPad / iPhone，尚未完成系统键盘及 iPad Safari 视频合成的真机验收；Playwright 的设备参数与 WebKit 不能替代这些检查。
- Linux 无头 WebKit 的真实 WebRTC 输入与 DataChannel 验证通过，但该环境的视频未确认展示，不计为 Safari 视频验收。上述 canvas MediaStream 验证实际展示了视频，属于不同媒体路径。
- 铺满模式允许横纵比例不同导致画面变形；需要保持比例时使用默认的“适应窗口”。本次未修改服务端桌面分辨率。
