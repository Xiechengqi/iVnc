# 设置入口迁移与整体 Review

日期：2026-11-15。

## 实现结果

底部齿轮打开“设置”，保留“概览／应用／连接质量”横向导航。修改密码、重启或升级、语言切换和退出集中在顶部右侧；底部工具区和更多菜单移除改密、退出、升级入口。桌面显示操作文字，窄屏使用带可访问名称的图标，并让导航独立横向滚动。

完整控制台和控制台管理关闭时的精简设置页共用账户、服务操作组件。子弹窗沿用现有弹窗管理与认证机制，支持原生输入、焦点恢复、外部点击和 Escape 逐层关闭；提交期间锁定关闭、重复提交与父弹窗最小化，失败后恢复关闭能力。

操作说明见 [设置](SETTINGS.md)。

## 审查发现与修正

1. **改密后的跨标签通知竞态**：成功后延迟通知可能被媒体会话撤销触发的主页面跳转打断。改为收到成功响应立即广播凭据变更，再走原有登录跳转；真实实例验证主页面及其他标签页均退出。
2. **异步操作与弹窗替换冲突**：给弹窗会话增加锁定状态；添加、编辑和日志入口检查被拒绝的打开请求，避免替换正在执行的账户或服务操作。父级同步禁用最小化，并保留页面离开时的强制清理。
3. **升级断线被误判成功**：只有收到重启阶段且最后日志没有报错时才等待服务恢复。下载阶段断线、最终重启错误和长时间无日志均显示失败或结果无法确认，不显示成功；前期状态保存警告仍允许后续成功完成。
4. **设置入口与更多菜单并存**：设置改为窄屏常显工具后，统一在工具区捕获点击并关闭更多菜单，避免打开设置后菜单仍遮挡界面。
5. **软键盘遮挡**：设置父窗和子弹窗按可视区域调整高度，按钮可滚动访问；缩放手势保留此前可用区域。可视区域通知单独更新设置尺寸。
6. **终端拖动被重复布局通知中断**：原窗口回归稳定复现只移动第一段距离。事件跟踪确认，尺寸控制器重复测量相同工作区时仍发出全局布局通知，终端据此结束指针捕获。改为只有窗口尺寸或工作区边界实际变化时才发送通知。原拖动距离断言保持不变，并增加拖动中重复测量场景；完整终端窗口回归通过。

## 整体审查结论

- 账户操作继续使用原有 Cookie、CSRF、会话撤销和跨标签通知；没有引入密码存储或新的授权机制。
- 打开设置、切换导航、打开或取消确认框均不触发重启或升级；升级 WebSocket 只在确认后建立。日志按纯文本展示并限制数量，关闭页面清理请求、连接和计时器。
- 控制台管理关闭时，精简页不加载管理控制台资源、不调用应用管理接口；原服务端限制仍有效。认证关闭时隐藏改密和退出，保留原有服务操作能力。
- 应用专属视图和内部 worker 不新增实例管理入口。原有终端拖动、缩放、置顶、还原、工具栏排序、改名和应用切换已回归。
- 审查发现的问题已修正，已验证范围内没有遗留阻塞项。

## 验证结果

环境：Linux、Google Chrome、Playwright WebKit。真实运行时测试使用独立数据目录、端口和浏览器配置。

| 检查 | 结果 |
| --- | --- |
| 设置交互，Chromium／WebKit | 每种引擎 15 个场景通过；执行器输出 33 项，含分组 |
| 原工具栏、视口、移动输入浏览器交互 | 执行器输出 135 项通过；工具栏捕获行为调整后另单独复测 65 项通过 |
| 视频、视口、剪贴板状态 | 55 项通过 |
| 手势仲裁 | 18 项通过 |
| 真实认证与设置，HTTP／HTTPS、Chromium／WebKit | 28 项通过，含控制台关闭后的真实改密、跨标签撤销及认证关闭分支 |
| 真实窗口与终端／PTY | 22 项通过，含拖动中重复布局、八方向缩放、置顶、还原、全屏和刷新恢复 |
| 真实工具栏、共享应用与 GTK 窗口 | 33 项通过，含排序、改名、焦点、共享视图保留与持久化 |
| 生产前端构建和入口产物 | 通过，包含完整控制台及精简设置页 |
| 修改脚本语法、工作流 YAML、补丁空白 | 通过 |

主要复测命令：

```sh
npm --prefix web/ivnc run build
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:settings
CHROME_BIN=/usr/bin/google-chrome node --test scripts/toolbar_interaction_test.mjs scripts/viewport_interaction_test.mjs scripts/mobile_input_test.mjs
node --test scripts/video_playback_test.mjs scripts/viewport_test.mjs scripts/clipboard_test.mjs
node scripts/gesture_navigation_smoke.mjs
IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/auth_smoke.mjs
IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/window_ui_smoke.mjs
IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/toolbar_smoke.mjs
git diff --check
```

## 验证边界

- 浏览器测试覆盖移动视口、触摸与模拟软键盘可视区域；未在实体移动设备或 macOS Safari 上验收。
- 重启和升级分支使用隔离 HTTP／WebSocket 服务验证请求、进度、失败、超时和主页面刷新，未执行真实在线程序升级。运行中的原有实例保持运行。
- 本轮只修改前端、测试和文档，未重新编译后端、部署或推送；发布构建需要将新前端重新嵌入二进制。生产构建仍有原有单包超过 500 kB 的提示。
