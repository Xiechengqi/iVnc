# Safari 应用 tab 点击修复与 Review

日期：2026-11-13。基于 `db36cd4`，范围为应用 tab 点击、整标签拖拽排序、后台更新与交互生命周期。

## 根因

拖拽排序在鼠标按下时建立候选状态；未发生拖拽的普通点击也会在 `pointerup` 中调用 `cancelDrag()`，随即重绘 tab。重绘通过 `label.textContent` 替换标题文本节点，即使标题没有变化。

WebKit 保存的点击命中目标可为原文本节点，节点被移除后取消后续原生 `click`。因此没有执行应用激活函数，也没有发送窗口聚焦指令。Chromium 在相同情形仍派发点击，使仅使用 Chromium 或 JavaScript `.click()` 的回归遗漏此问题。

修复前，使用仓库 toolbar 逻辑及真实鼠标事件在 Linux WebKit 中确认：点击标题失败；点击 tab 空白边缘、按 Enter 或调用 `.click()` 成功。禁用排序可以避免松开时的重绘，但按住期间收到后台刷新仍会失败。

## 实现

- 普通按下与真正拖拽分开处理。未超过拖拽阈值时清理候选状态，不触发重绘；应用激活仍在原生点击调用栈执行。
- 标题持有独立 Text 节点，只有内容变化才更新 `.data`。标题和状态点不单独参与指针命中；整枚 tab 接收点击和横向拖拽，不显示独立手柄。横向位移达到 5 像素且大于纵向位移才开始排序，触屏横滑继续使用原生滚动。键盘左右方向键直接在 tab 上排序，并保留当前焦点。
- 所有应用 tab（含未托管、不可排序、触摸操作）在按住期间暂缓 DOM 更新；后台模型和版本仍立即更新。点击完成后统一呈现最新状态，避免排序、改名或增删改变正在点击的目标位置。
- 激活时按原窗口 key 和应用 ID 查找最新条目。按住期间消失过的条目标记为取消，即使随后重新出现也不能完成旧点击；键盘激活同样校验有效条目。
- 真正拖拽后仅屏蔽对应指针的收尾点击，新一次按下立即解除屏蔽。替换原先全局 400ms 的屏蔽，拖拽结束后可以立即点击另一个应用；键盘和程序调用的点击不受该屏蔽影响。
- 点击、指针取消、Escape、失焦、后台隐藏和销毁均清理状态。待完成的点击离开 toolbar 时也取消，避免移入 iframe 后收不到松开事件；已经开始的拖拽继续通过指针捕获处理。指针取消只影响匹配的指针；真正拖拽丢失捕获时取消排序。未收到原生点击时，松开后的 500ms 兜底仅释放暂缓的绘制，不重放激活。

iframe 和视图连接仍由 `app-streams.js` 持有；排序和改名只更新 tab。本轮没有新增服务器聚焦协议。

## 验证

```bash
npm --prefix web/ivnc ci
npm --prefix web/ivnc exec -- playwright install --with-deps chromium webkit
npm --prefix web/ivnc run test:toolbar
node --test scripts/video_playback_test.mjs scripts/viewport_test.mjs scripts/clipboard_test.mjs
node scripts/gesture_navigation_smoke.mjs
npm --prefix web/ivnc run build
cargo build --bin ivnc --features tls --offline
IVNC_BIN=target/debug/ivnc node scripts/toolbar_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/clipboard_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/viewport_smoke.mjs
```

浏览器专项从 Vite 加载真实 toolbar、i18n、排序通知和 app-streams 模块，不替换点击处理。隔离的 API 响应用于控制排序结果和视图令牌；嵌入页面模拟视图消息。真实实例 smoke 则验证实际 GTK 窗口、服务端聚焦、持久化、共享应用连接和输入。

| 检查 | 结果 |
| --- | --- |
| 原生浏览器交互 | Chromium、WebKit 各 22 场景，共 44 项通过；含 Retina、标题/空白点击、不可排序、按住时改名/排序/移除再出现、轻微和纵向移动、无手柄的标题与前后空白左右拖动及实时预览、拖后立即点击、键盘排序焦点保持、只读排序限制、Escape、捕获丢失、指针取消、键盘激活、右键改名、触摸、共享/远程 iframe 与本地切换、移入 iframe 后松开、失焦、销毁和点击缺失恢复 |
| 真实 toolbar 实例 | 32 项通过；验证整标签拖动的实时预览、保存与控制台同步，真实标题点击到服务端窗口聚焦，共享应用切换使用真实鼠标点击；触屏点击及横滑滚动均不触发排序 |
| 剪贴板及输入 | 14 项真实实例检查通过 |
| 窗口工作区域 | 14 项真实实例检查通过 |
| 视频、视口、剪贴板状态 | 55 项通过 |
| 手势仲裁 | 10 项通过 |
| 生产前端与嵌入资源的 TLS 服务端构建 | 通过；保留 Vite 既有单文件体积提示 |

以上为 169 项行为检查，重复运行不重复计数。整标签拖拽变更后重新运行其中 86 项（原生浏览器 44、真实 toolbar 32、手势仲裁 10）及前后端构建；其余 83 项为此前 Safari 点击修复的验证结果。Node 测试执行器另计的父级分组不计入行为检查数。

新增 `web-tests.yml`，在主分支推送、相关 PR 和手动触发时，使用 Ubuntu 24.04、macOS 15 分别运行 Chromium/WebKit、状态测试、手势检查和生产构建。CI 工作流已添加，本地尚未执行远端 macOS job。

## 整体 Review

已核对：点击调用栈与用户激活、按下与拖拽状态分离、后台版本更新、有效条目校验、原生与键盘点击、按指针屏蔽收尾点击、捕获转移与取消、暂缓绘制的释放、定时器与监听器销毁，以及真实窗口聚焦、共享 iframe 保留、重命名/排序持久化、网络标识布局、视口和剪贴板输入。

Review 中补齐键盘激活的最新条目校验、无关指针取消隔离，以及捕获丢失和原生点击缺失的回归。另通过真实跨 iframe 松开复现并修复不可排序 tab 的待完成点击不能释放的问题。未发现尚未处理的阻断问题。

整标签拖拽 review 已核对：独立手柄 DOM、样式和事件入口全部移除，标题及空白位置均能起拖；开始拖动前保留点击目标，拖动后不误激活应用；键盘排序迁到标签并保留焦点；排序权限、保存互斥、隐藏应用位置、同应用多窗口分组、冲突恢复、控制台同步和共享连接沿用原有流程。触屏保留原生横向滚动。未发现新增阻断问题。

本地实际环境为 Linux Chrome 与 WebKit 26.5。此结果确认 WebKit 点击缺失的原复现已修复，不能等同于用户 macOS Safari 版本的实机验收。发布后仍需在该 Safari 中确认本地、共享、远程 tab 的高亮、画面与输入焦点同步切换。
