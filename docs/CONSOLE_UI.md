# 控制台导航与弹窗

嵌入桌面的控制台和独立 `/console` 共用 `console.html`、`console.css`、`console.js`。顶部横向导航提供概览、应用和连接质量，右侧为语言切换；窄屏仅导航区域横向滚动，正文使用完整宽度。

## 应用类型选择

`select-control.js` 将原生 `#f-app-type` 增强为 select-only combobox，原生节点仍提供值、option 文案、disabled 状态和 change 事件。后台、桌面、CLI、远程四种类型及编辑限制沿用现有规则。JavaScript 完成增强前仍显示原生 select。

组件复用控制台表单的颜色、边框、圆角和焦点样式。支持鼠标、方向键、Home/End、Enter/Space、Esc、Tab 和按名称前缀定位。展开列表位于所属模态遮罩内，按可见空间向上或向下展开；滚动、调整视口时重新定位，触发器滚出表单时收起。中英文、程序设置的值和禁用态同步到自定义控件。

## 子弹窗与输入

`console-dialogs.js` 为每次添加、编辑或日志打开创建独立实例，管理 AbortController、焦点、Tab 循环和关闭回调。点击遮罩关闭当前子弹窗；从内容区拖选到外部松手不会关闭。Esc 先收起下拉列表，再关闭子弹窗。关闭后恢复到打开弹窗的按钮。

子弹窗位于 iframe 中时，同源父子页面立即同步当前实例，并通过带来源校验的消息补齐加载期间的状态。父页面 `overlay-manager.js` 将控制台提升至置顶终端上方，同时安装透明交互遮罩，拦住发往桌面、toolbar 和应用 iframe 的点击。点击外部或控制台标题栏只关闭当前子弹窗并消费整个点击手势；显式最小化按钮仍隐藏整个控制台。重新展开时恢复其中尚未关闭的子弹窗与遮罩。

层级从低到高为：应用与 toolbar、普通浮窗（1800）、置顶终端（1900）、控制台子弹窗的父级交互遮罩（1940）、带子弹窗的控制台（1950）、密码或更新对话框（2000）。iframe 内的菜单属于其所在弹窗，不尝试通过内部 z-index 越过父页面层级。

应用内点击通过 `ivnc-overlay-app-interaction` 通知父页面；父页面验证 origin 和 source 对应当前应用 iframe。这样控制台 iframe 到应用 iframe 的焦点变化不依赖父窗口 blur。置顶终端跳过自动收起。模态子弹窗存在时，远程键盘捕获暂停，底层应用不接收关闭弹窗的点击或按键。

## 异步请求

编辑加载、日志、共享配置和连接测试均绑定当前弹窗实例。关闭时取消读取；即使底层忽略取消，过期结果也不能改动后来打开的弹窗。编辑加载期间显示 busy 状态并禁止提交。

保存提交前固定应用 ID、请求正文、应用状态、名称版本、共享配置与启动配置签名。提交之后关闭弹窗不会撤销操作；保存完成后更新应用列表和通知，仅原实例仍处于打开状态时才能修改或关闭其表单。同一应用的后续编辑等待已提交的应用与共享操作完成后再读取，避免读到中间状态。共享写入失败时明确报告应用已保存及失败部分。重启和复制结果也遵守实例边界。

## 验证

先构建前端和包含资源的服务端，再执行：

```bash
IVNC_BIN=target/debug/ivnc node scripts/window_ui_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/terminal_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/toolbar_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/viewport_smoke.mjs
```

窗口 UI smoke 使用独立目录、端口、数据库、浏览器和真实 PTY，通过真实指针、键盘和受控延迟响应覆盖导航、选择、点击拦截、异步隔离、拖动缩放、置顶、全屏和状态恢复。失败保留诊断与截图；`KEEP_TMP=1` 也保留成功运行的截图。控制台代理面板还使用原有 `miao_smoke.py --browser` 验证。

本次窗口 UI smoke 14 项、终端 smoke 21 项、toolbar smoke 28 项、viewport smoke 14 项和剪贴板端到端 smoke 11 项全部通过；剪贴板与 viewport 单元回归共 16 项通过。前端生产构建、启用 TLS 的服务端构建，以及 HTTP/HTTPS 认证控制台与代理面板集成验证均通过。

整体 review 核对了父子页面的消息来源与实例标识、完整点击手势拦截、焦点和远程输入隔离、保存快照与同应用串行读取、置顶层级、缩放后的按钮命中区、工作区与全屏边界、存储回退和监听器清理。补齐了 iframe 重载后的实例隔离与缩放后的标题栏按钮回归，未发现阻塞问题。
