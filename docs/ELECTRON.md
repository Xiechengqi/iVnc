# Electron 原生 Wayland 应用

iVnc 可以运行使用 Ozone/Wayland 的 Electron 应用。Electron 38 起可以根据 `XDG_SESSION_TYPE=wayland` 自动选择 Wayland；旧版、应用封装的启动器或显式覆盖了后端的程序仍可能需要启动参数。iVnc 不提供 X11/Xwayland。

## 启动

在控制台添加“桌面应用”，通常只需填写程序名或路径：

```sh
amp-desktop
# 或
t3code
```

默认采用自动识别。iVnc 按应用实际的 PATH、HOME 和静态环境赋值找到程序，解析软链接，并检查 Electron 二进制标记、内嵌版本和运行资源布局。检查不执行程序的 `--version`。确认是 Electron 后，按二进制中可识别的能力补齐缺失的 `--ozone-platform=wayland`、`--enable-wayland-ime` 和 `--wayland-text-input-version=3`。实际启动时生成参数，保存的命令不被改写。

自动沙箱策略：**有效 UID 为 0 时，仅对已识别或明确指定为 Electron 的应用补充 `--no-sandbox`，关闭该应用的 Chromium 沙箱；普通用户不自动关闭沙箱。** 显式已有的 `--no-sandbox` 继续保留。普通用户遇到沙箱安装、AppArmor 或 user namespace 错误时不会自动降级；保留沙箱的部署见 [NONROOT.md](NONROOT.md)。

编辑器默认折叠“高级启动设置”。启动兼容可选“自动识别 / 指定为 Electron / 原样执行”，沙箱可选“自动 / 保留 / 对此应用禁用”。“原样执行”不做参数适配，包括不执行沙箱策略；“保留沙箱”与命令中的 `--no-sandbox` 冲突时明确报错。显式 X11 后端不被覆盖，自动适配会提示移除冲突参数。已有长命令无需迁移，也不会重复追加参数。未知程序、Electron Node 模式、复杂 shell 语法保持原样。

高级设置中的“上次启动详情”提供实际 UID、程序、Electron 版本、保存命令、补充参数和执行命令。认证后的 `GET /api/apps/<id>/launch-info` 返回同一记录；记录位于应用数据中的 `launch-info.json`，权限 0600，不包含完整环境。应用新增/编辑 API 的可选配置如下；旧客户端不传该字段时保留已有配置：

```json
{"desktop_launch":{"compatibility":"auto","sandbox":"auto"}}
```

识别支持直接 ELF、软链接、简单静态环境赋值、标准 `env`/`dbus-run-session --`，以及整个有效脚本体仅为 `exec /path/to/app "$@"` 的透明 shell 包装器。引号和用户参数保持原样，补充参数放在目标程序后、用户参数和 `--` 分隔符前。带 shell 展开、重定向、管道、条件执行、身份切换、复杂包装脚本的命令不自动改写；需要时使用程序本身的启动规则或“原样执行”。AppImage 的 FUSE/解包需求、Flatpak/Snap 的会话和封装依赖仍由部署提供。

静态识别不足时，只有精确的 Electron 初始化失败日志，结合目标本身的 Electron 特征或 AppImage 标记，才可能在 10 秒内进行一次兼容修正重试。整个第一轮进程树必须已经退出，且当前启动代次未记录窗口就绪；用户停止、原样执行、普通 X11 错误及不透明启动器不触发这条路径。修正后的错误独立诊断，不被第一轮日志覆盖。后续仍沿用有上限的 watchdog 策略，明确的配置错误不循环重试。

iVnc 对主桌面和独立应用运行环境使用相同适配逻辑，并统一设置 `XDG_SESSION_TYPE=wayland`，分配当前会话的 `WAYLAND_DISPLAY`、`XDG_RUNTIME_DIR`。主机的 `DISPLAY`、D-Bus 地址和预加载库不传入应用。Electron 38 已移除 `ELECTRON_OZONE_PLATFORM_HINT` 环境变量，自动适配不依赖该变量。

不要用 `su - ubuntu -c t3code` 切换身份来加入 root 的桌面：登录切换会清空显示环境，普通用户也不能访问 `/run/user/0` 和 `/root/.ivnc`。仅保留环境变量或改用 `su -m` 不能解决目录、socket、音频与 HOME 权限。需要沙箱时，参见 [普通用户部署](NONROOT.md)，让 iVnc 与应用从启动时就使用同一用户。

每次启动前会检查应用 HOME、Wayland runtime 归属/0700 权限以及 socket。应用工作目录设置为自己的 HOME，不再继承管理进程可能不可访问的目录；不继承父进程的 `WAYLAND_SOCKET` 文件描述符设置。独立应用运行环境在自己的显示会话建立后执行相同检查。

已有的相对路径命令（如 `./my-app`）现在相对于应用 HOME 解析，建议使用绝对路径。`ivnc env` 导出的同用户外部会话环境也包含 `XDG_SESSION_TYPE=wayland`；它不负责跨用户权限授权。

控制台分别展示“进程运行中 · 等待窗口”和“窗口已出现”（主桌面）；独立应用的“运行中”表示其会话已启动，不代表应用已完成加载。已退出进程的日志按本次启动范围诊断，区分 root 沙箱、Wayland 连接、X11 后端及 sandbox 安装问题。确定的沙箱/后端配置错误停止自动重试，修改配置后手动启动即可重新尝试；临时显示连接失败与未知崩溃仍沿用有上限的退避重试。无致命退出的 D-Bus/GPU 警告不触发这些诊断。

## 窗口和输入

| 能力 | 当前行为 |
| --- | --- |
| 多窗口 | 同一进程的独立 toplevel 分别显示在工具栏；不再用 PID 猜测对话框 |
| 最大化 / fullscreen | 默认铺满桌面使用 Maximized；Fullscreen 由应用请求，退出后恢复此前状态 |
| 固定尺寸窗口 | 初次 configure 遵守客户端的相等 min/max 限制，不强制铺满 |
| 对话框 | 按 xdg parent 或 xdg-foreign 关系分组，首次出图居中；后续重绘保留用户拖动的位置 |
| 最小化 | 隐藏整个父子窗口组，工具栏保留入口，点击恢复并激活子窗口 |
| 焦点激活 | 支持 xdg-activation；校验来源、当前焦点和 5 秒有效期，使用后移除；提供 serial 时校验 seat/序号，省略时要求同客户端最近 5 秒内有用户输入 |
| 中文 / Unicode | 跟踪 text-input-v3 的 enable/commit 状态，使用该实例的最新 commit 序号发送 done；不依赖外部输入法进程 |
| 旧客户端输入 | 未启用 text-input-v3 或文本过长时使用有序 Ctrl+V 事务，等待读取结束再处理后续输入；不固定使用 Ctrl+Shift+V |
| 光标 | 支持 cursor-shape 命名光标和常规 SHM 位图光标，转为 PNG 传给浏览器 |
| 缩放 | 提供 viewporter / fractional-scale；输出当前为物理像素、scale=1，浏览器 DPR 不再重复乘到 Wayland 尺寸上 |
| 相对鼠标 | 提供 relative-pointer / pointer-constraints，锁定时继续传送原始增量，并停止发送绝对 motion |

Electron 自身也有平台限制。以实测 Electron 45.3.1 为例，`BrowserWindow.parent` 的 Linux 实现仍只在 X11 分支设置父子关系；Wayland 抓包中没有 `xdg_toplevel.set_parent`。这种窗口保留独立工具栏入口。不能根据同进程关系安全地区分它与第二个主窗口。GTK 等实际发送 parent 或 xdg-foreign 的弹窗按父子窗口处理。

在网页远端应用里使用持续的相对鼠标时，本机浏览器也需要进入 iVnc 的鼠标锁定模式（现有 fullscreen 快捷键）；远端应用的异步请求不能绕过本机浏览器对用户手势的要求。退出锁定、失焦和输入重置会释放约束。指针位置提示不会强行移动本机硬件光标。

位图光标当前覆盖 ARGB8888/XRGB8888、普通变换、逻辑尺寸不超过 128×128 的 SHM 光标。不支持的格式或变换回退到默认光标；尚未实现 GPU cursor buffer 导出。

## 剪贴板和文件

纯文本继续默认启用，沿用带 EOF 送达确认、revision 和焦点校验的 paste v1 事务。在“连接质量”打开 **二进制剪贴板** 后，支持同次粘贴中的 `text/plain`、`text/html`、`image/png`，并支持从应用复制回浏览器。总大小仍限制为 16 MiB。原生应用之间的 Wayland selection 保留各自的 MIME 表示。

图片和 HTML 的本机写入需要浏览器的安全上下文、ClipboardItem 支持及相应权限。纯文本仍可在自动写入受限时使用可见的手动复制面板；纯图片不显示无效的文本复制按钮。HTML/PNG 不通过外部 `wl-copy` 设置，不会把文件 URI 转换为本机文件读取。

上传文件使用既有文件传输通道，保存到配置的上传目录；独立应用运行环境使用其 HOME 下的 `Downloads`。应用的文件选择器访问的是服务端文件系统，不是本机浏览器文件系统。应用内文件选择器、浏览器上传文件和操作系统拖放是不同的操作；未实现 portal 文件授权、跨浏览器文件 URI 粘贴或向原生应用合成文件拖放。

## 按应用需求配置桌面服务

需要独立 D-Bus session 的应用可以使用：

```sh
dbus-run-session -- /path/to/electron-app
```

Docker 运行镜像提供 `dbus-run-session`。它只创建应用自己的 session bus，不自动安装或启动 Secret Service、通知服务或 xdg-desktop-portal。依赖这些服务的应用需要单独部署；iVnc 不借用主机的桌面 bus。

当前合成器使用 Pixman/SHM 软件渲染。Chromium 可回退到软件出图，原生 Electron 的普通桌面界面不需要 GPU；遇到应用强制初始化 GPU 的问题，可在该应用命令中使用 `--disable-gpu` 排查。未宣称支持 DMA-BUF/EGL 零拷贝、硬件 WebGL 或通过 portal 的屏幕共享。

## 验证和排障

应用日志位于 `$IVNC_HOME/apps/<id>/app.log`，控制台“日志”可以查看。启动后无窗口时先检查 sandbox、程序路径、运行库、AppImage/FUSE 和 D-Bus 服务错误。设置该应用的 `WAYLAND_DEBUG=1` 可以在临时诊断中检查协议绑定、configure 和输入事件；协议日志可能包含键盘/剪贴板内容，排障后应关闭。

自动验证使用独立 HOME、runtime、端口和浏览器配置，不连接正在运行的 iVnc：

```sh
npm --prefix web/ivnc run build
CARGO_INCREMENTAL=0 cargo build --features tls --bin ivnc
ELECTRON_BIN=/path/to/unpacked-electron/electron IVNC_BIN=target/debug/ivnc node scripts/launch_smoke.mjs
ELECTRON_BIN=/path/to/unpacked-electron/electron IVNC_BIN=target/debug/ivnc node scripts/electron_smoke.mjs
IVNC_BIN=target/debug/ivnc node scripts/launch_compat_smoke.mjs
```

需要 Chromium、Node、C 编译器、wayland-client 开发文件、wayland-scanner 和 xdg-shell XML。脚本使用实际 Electron 和 WebRTC 浏览器验证窗口、IME、剪贴板、光标及相对输入；配套原生协议客户端验证 Electron 未发送的 parent 请求、拖动重绘和不同客户端复用相同 wire ID 的边界。`XDG_SHELL_XML` 可覆盖 XML 路径，`KEEP_TMP=1` 保留成功测试的临时目录。

本轮完整验证记录、审查修正和已确认的上游限制见 [ELECTRON_REVIEW.md](ELECTRON_REVIEW.md)。

启动身份、沙箱诊断、普通用户部署和远程 t3code 修复记录见 [ELECTRON_LAUNCH_REVIEW.md](ELECTRON_LAUNCH_REVIEW.md)。

自动识别、简化启动设置及本轮验证见 [ELECTRON_AUTO_LAUNCH_REVIEW.md](ELECTRON_AUTO_LAUNCH_REVIEW.md)。
