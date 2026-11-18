# 滚动输入与浏览器速度设置 Review

## 行为

“设置 → 高级”新增独立的“滚动速度”卡片，支持 0.25–4.0×，默认 1.0×，步长 0.25，提供恢复默认。调整立即应用，刷新和切换应用后保留。本机浏览器使用同一站点的 localStorage 保存，不写服务器配置；其他设备不会被改动。现有与新建的应用 iframe、设置页和同源标签页同步。存储不可用时，当前页面及其应用 iframe 仍可在内存中同步。

滚动设置与 IP 白名单的加载、草稿和保存互不影响，完整控制台和精简设置页共用控件。中文和英文、键盘调节、移动端窄屏均覆盖。

## 根因与修复

旧实现用历史最小 delta 除当前 delta，再截成整数步。触控板还有 100 ms 丢事件节流；deltaMode 未转换，触摸滚动丢弃每次小于等于 2 px 的位移。竖向小幅输入也可能被应用切换手势提前吞掉。

现在先归一化像素、行（24 逻辑像素）和页（当前视图宽高）三种单位，再乘浏览器速度。8 ms 短批次累加双轴位移，不按设备幅度猜测鼠标或触控板，不再采用历史最小值和旧节流。单次异常输入及累积批次限制为每轴 32768 逻辑像素，普通大幅滚动不受旧的 10 步上限影响。

新连接发送 `_input,1`，服务通过 `input_capabilities` 回应 `scroll_pixels: 1` 后启用 `w2,dx,dy`。新协议使用浮点逻辑像素，包含有限值、范围、字段数量与几何版本校验。服务转换为 Wayland 连续轴；传统的 10 轴单位相当于 Chromium 的 120 滚动像素。没有声明物理来源、离散刻度或伪造手指惯性。

旧 `w` 协议保持原义。新前端连接旧版远程服务时，以每单位 12 像素累积整数及余量，不发送新协议。连接断开后清空能力和待发量，重新协商。应用共享协议本身没有升级要求。

竖向输入从首个明确样本放行；方向尚不明确的样本先累积，转为内容滚动时补发。仍未确定方向时最多等待 60 ms，触摸正常结束时也释放未确定的样本。应用切换的距离判断使用乘速度前的值，已切换的惯性继续由原有共享控制器消费。

切换视图、改变几何、失焦、页面隐藏/离开、弹窗交互、断开连接和修改速度都会清除待发输入。普通输入改变指针或修饰键前先发送完整滚动批次；按键/按钮状态改变时丢弃不足传输单位的旧余量，避免串到下一次操作。发送继续经过剪贴板的有序队列。

## Review 中修正的问题

- Wayland 24.8 定点数会再次截断小数。前端按 12/256 像素量化并保留余量，避免重复小幅输入持续缩水，极小输入也能累积生效。
- 跨 iframe 的偏好订阅在页面离开时注销，避免父页面长期保留已关闭的应用视图。
- 触摸结束释放未确定的滚动；触摸取消、几何变化和视图失活则丢弃，防止晚到输入作用于其他窗口。
- 保留按键与滚动的发送顺序，避免短批次在修饰键释放后才到达应用。
- 对已进入粘贴有序队列的滚动，在真正发送时再校验视图/几何代次；即使切换的是同一主桌面中的另一个应用，旧滚动也不能在粘贴结束后晚到。

审查未发现遗留阻断项。GitHub Actions 没有增加或启用任何浏览器测试。

## 验证

环境：Linux、本地 Google Chrome 146.0.7680.153、Playwright WebKit、GTK 3.24.33。生产前端、真实 iVnc 服务和应用均使用临时目录、独立配置及端口，未操作部署实例。

| 场景 | 实测结果 |
| --- | --- |
| Chrome 单次 100 px，1.0× | 原实现约滚动 12 px；修复后滚动 100 px |
| Chrome 单次 100 px，0.5× / 2.0× | 滚动 50 / 200 px |
| 60 次 10 px 连续输入，2.0× | 本地、独立与 A→B 均收到并滚动 1200 px；应用可合并相邻事件，距离保持 |
| GTK 单次 120 px，1.0× / 2.0× | 平滑 delta 为 1 / 2，滚动约 77.56 / 155.11 px |
| 新 A → 旧 B（237be89） | 自动发送旧 `w`；100 px 分别约为 96 / 48 / 192 px，连续输入合计 1200 px |
| 确定性回放 | 60/120 Hz 全量保留；小数、双轴、倍率、余量、方向变化、极小输入、异常值与旧协议覆盖 |
| 128 次 0.0078125 px 极小样本，2.0× | 本地、独立与 A→B 均能累积并滚动 2 px，余量误差小于一个 Wayland 传输单位 |

本地执行：

```sh
cargo fmt --check
cargo test --lib --features tls --offline
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:scroll
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:settings
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:toolbar
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:mobile-input
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:viewport
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:ime
node scripts/gesture_navigation_smoke.mjs
npm --prefix web/ivnc run build
cargo build --features tls --bin ivnc --offline
CHROME_BIN=/usr/bin/google-chrome LEGACY_IVNC_BIN=/path/to/old/ivnc node scripts/scroll_input_smoke.mjs
git diff --check
```

Rust 单测 245 项通过；滚动专项 33 项（11 项纯逻辑、22 个浏览器用例）、设置交互 62 个浏览器用例、工具栏 66 个浏览器用例通过。剪贴板与文字提交的 60 项检查、移动输入、视口、IME 与手势回归通过。浏览器与真实服务测试均为本地执行。

旧版服务只能使用整数精度；GTK 等应用自行决定一行/一步的内容距离，因此不同工具包不承诺相同的内容像素数，速度倍率仍成比例。物理 macOS Safari、iPad 和独立 Electron 运行时本轮未做设备实测，WebKit 与输入/缩放/移动端模块回归不能替代设备验证。
