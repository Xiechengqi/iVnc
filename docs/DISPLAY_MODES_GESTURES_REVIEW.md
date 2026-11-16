# 铺满屏幕与滑动切换常开：实现和整体 Review

日期：2026-11-16。基础提交：`d224cd3`。实现与整体 review 已完成，已验证范围内没有剩余阻断问题。

## 最终行为

- “原始大小”替换为“铺满屏幕”（`fill`）：完整远端画面的宽高分别拉伸到本机工作区，允许变形，不裁切。默认仍为“适应窗口”（`fit`），按比例居中。
- 两种模式均在固定或临时展开的 toolbar 上方显示完整画面；显隐动画、窗口缩放及软件键盘改变工作区时同步调整，无需拖动查看区域。应用内部的内容滚动保留。
- 本地旧 `native` 偏好和显示控制消息迁移为 `fill`，旧手动模式布尔配置及 `setScaleLocally(false)` 同样归一。主页面同步已创建的后台应用，嵌入页继承父页面模式，独立访问页保存自己的偏好。
- 删除底部工具区和“更多工具”中的滑动开关；滑动切换始终开启，历史关闭偏好和 URL 参数失效。
- 单个已选中应用或向首尾外滑动时，新手势直接交给应用。修饰键横滑、鼠标锁定、组合输入等保护路径继续让行。手势归属在一次滑动中保持稳定，已接管的滑动取消或完成后消费剩余惯性；保留连续同向、反向切换的提前识别。

## 整体审查

| 审查点 | 结果 |
| --- | --- |
| 工作区与完整画面 | 视频和输入层共用实测 toolbar 工作区；`fit` 等比，`fill` 横纵独立缩放；两模式查看区域滚动偏移归零 |
| 底部输入 | 拉伸后四角、中心和底部点击按横纵轴独立映射；真实远端底部按钮可直接点击 |
| 动态适配 | 保留显隐动画预留、动态 toolbar 高度、浏览器缩放、DPR、全屏、横竖屏及键盘遮挡适配 |
| 多视图与多客户端 | 父页面同步后台/活动应用，激活和 ready 时校准；原 iframe、DataChannel、独立页面偏好及共享桌面尺寸保持稳定 |
| 兼容与来源校验 | 旧 `native`、布尔手动模式及旧控制消息归一为 `fill`；非法模式和无关窗口消息被忽略，合法父级 dashboard 入口保留 |
| 横滑目标 | `canNavigate()` 和 `navigate()` 共用目标计算，按排序、待确认目标、可用性及交互状态判断；查询不触发激活 |
| 手势归属 | 首尾/单应用从首事件让行；修饰键释放或目标新增不在滑动中途抢占；取消已接管手势时排空剩余输入 |
| 惯性与快速切换 | 成功切换后的惯性跨 iframe 消费；只有新一轮加速、短暂停顿或明确反向才重判，保留约 160 ms 间隔的连续切换覆盖 |
| 鼠标锁定 | Ctrl+Shift+左键手动锁定仍可使用；锁定时横向滚轮送到远端应用，滑动切换保持开启 |
| 生命周期 | 页面隐藏和断线重置手势；销毁时清理新增可见性监听，删除旧开关状态与样式绑定 |
| 渲染 | 非等比缩放使用平滑插值；仅横纵一致的整数物理像素放大使用像素化，状态栏尺寸计入边框和内边距 |
| CI | UI/浏览器回归仅本地执行；保留生产构建，遵循仓库不新增 Actions UI 测试的约定 |

真实 smoke 的输入必须等首帧解码与输入几何就绪，避免把数据通道重连成功误当成画面已可交互。双指切换测试显式选择有相邻目标的窗口，覆盖新的首尾让行规则。

## 验证结果

以下按独立行为场景计数，不重复计入 Node 父级分组。

| 验证 | 结果 |
| --- | --- |
| 手势序列与坐标单元检查 | 29 项通过：手势 26、视口 3 |
| 视口浏览器交互 | Chromium/WebKit 各 15，共 30 项通过 |
| 显示模式与父子页面同步 | 两引擎各 9，共 18 项通过 |
| toolbar 浏览器交互 | 两引擎各 33，共 66 项通过 |
| 移动输入与键盘工作区 | 两引擎各 25，共 50 项通过 |
| 真实布局 smoke | 22 项通过：Wayland Chrome、多客户端、共享 GTK 视图、底部实际点击、全屏与尺寸管理 |
| 真实 toolbar smoke | 33 项通过：双指切换、旧关闭配置失效、排序/改名、真实 GTK 聚焦及共享 iframe 保留 |
| 真实 Electron Wayland smoke | Electron 45.3.1，28 项通过，包含手动鼠标锁定、相对输入、横向滚轮及重连 |
| 前端生产构建、JS 语法与补丁空白检查 | 通过；构建保留已有大于 500 KB 的 chunk 提示 |

本地浏览器场景合计 164 项，真实实例检查 83 项；另有 29 项单元检查。浏览器批次的 Node 输出计入父分组为 176/176；手势、视口与移动输入批次计入父分组为 82/82，两个批次的移动输入场景重叠。

复核命令：

```bash
node --test scripts/gesture_navigation_smoke.mjs scripts/viewport_test.mjs
CHROME_BIN=/usr/bin/google-chrome node --test scripts/viewport_interaction_test.mjs scripts/display_mode_interaction_test.mjs scripts/toolbar_interaction_test.mjs scripts/mobile_input_test.mjs
npm --prefix web/ivnc run build
IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/viewport_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/toolbar_smoke.mjs
ELECTRON_BIN=/path/to/electron IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/electron_smoke.mjs
git diff --check
```

测试使用隔离的数据目录、端口和浏览器配置，加载本次前端生产构建。本轮未替换本机或 `192.168.1.8` 的生产部署。

“铺满屏幕”在比例不同时会使画面变形，这是所选模式的预期行为；需要保持比例时使用“适应窗口”。Linux WebKit 已覆盖本次布局与交互逻辑，Mac 外接显示器、实体触控板及实际 macOS Safari 仍需设备验收。
