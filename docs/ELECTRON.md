# Electron 原生 Wayland 应用

iVnc 可以运行使用 Ozone/Wayland 的 Electron 应用。Electron 38 起可以根据 `XDG_SESSION_TYPE=wayland` 自动选择 Wayland；旧版、应用封装的启动器或显式覆盖了后端的程序仍可能需要启动参数。iVnc 不提供 X11/Xwayland。

## 启动

在控制台添加“桌面应用”。启动命令为空时可使用 **Electron Wayland 模板**，替换其中的程序路径：

```sh
/path/to/electron-app --ozone-platform=wayland --enable-wayland-ime --wayland-text-input-version=3
```

参数需要传到 Electron 进程；有些发行版启动脚本使用 `--` 分隔参数，应按该应用的规则填写。模板不覆盖现有命令或环境变量。iVnc 对主桌面和独立应用运行环境统一设置 `XDG_SESSION_TYPE=wayland`，并分配当前会话的 `WAYLAND_DISPLAY`、`XDG_RUNTIME_DIR`。主机的 `DISPLAY`、D-Bus 地址和预加载库不传入应用。

默认保留应用自己的 sandbox 策略。root 运行的 Chromium/Electron 通常会以 `Running as root without --no-sandbox is not supported` 退出；这种部署可以显式在该应用的命令中添加 `--no-sandbox`。iVnc 不会给所有桌面应用自动关闭 sandbox。AppImage 的 FUSE/解包需求、应用二进制及依赖仍由部署者提供。

应用编辑器显示 iVnc 实际的运行用户和 UID。Electron 模板默认保留沙箱；勾选“模板中禁用此应用的沙箱”后生成的命令才包含 `--no-sandbox`，已有命令不会被自动修改。该选项只控制模板，不是系统或所有应用的全局开关。

以 root 部署的 t3code 原生 Linux 二进制可使用：

```sh
t3code --no-sandbox --ozone-platform=wayland --enable-wayland-ime --wayland-text-input-version=3
```

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
dbus-run-session -- /path/to/electron-app --ozone-platform=wayland --enable-wayland-ime --wayland-text-input-version=3
```

Docker 运行镜像提供 `dbus-run-session`。它只创建应用自己的 session bus，不自动安装或启动 Secret Service、通知服务或 xdg-desktop-portal。依赖这些服务的应用需要单独部署；iVnc 不借用主机的桌面 bus。

当前合成器使用 Pixman/SHM 软件渲染。Chromium 可回退到软件出图，原生 Electron 的普通桌面界面不需要 GPU；遇到应用强制初始化 GPU 的问题，可在该应用命令中使用 `--disable-gpu` 排查。未宣称支持 DMA-BUF/EGL 零拷贝、硬件 WebGL 或通过 portal 的屏幕共享。

## 验证和排障

应用日志位于 `$IVNC_HOME/apps/<id>/app.log`，控制台“日志”可以查看。启动后无窗口时先检查 sandbox、程序路径、运行库、AppImage/FUSE 和 D-Bus 服务错误。设置该应用的 `WAYLAND_DEBUG=1` 可以在临时诊断中检查协议绑定、configure 和输入事件；协议日志可能包含键盘/剪贴板内容，排障后应关闭。

自动验证使用独立 HOME、runtime、端口和浏览器配置，不连接正在运行的 iVnc：

```sh
npm --prefix web/ivnc run build
CARGO_INCREMENTAL=0 cargo build --features tls --bin ivnc
ELECTRON_BIN=/path/to/unpacked-electron/electron IVNC_BIN=target/debug/ivnc node scripts/electron_smoke.mjs
```

需要 Chromium、Node、C 编译器、wayland-client 开发文件、wayland-scanner 和 xdg-shell XML。脚本使用实际 Electron 和 WebRTC 浏览器验证窗口、IME、剪贴板、光标及相对输入；配套原生协议客户端验证 Electron 未发送的 parent 请求、拖动重绘和不同客户端复用相同 wire ID 的边界。`XDG_SHELL_XML` 可覆盖 XML 路径，`KEEP_TMP=1` 保留成功测试的临时目录。

本轮完整验证记录、审查修正和已确认的上游限制见 [ELECTRON_REVIEW.md](ELECTRON_REVIEW.md)。

启动身份、沙箱诊断、普通用户部署和远程 t3code 修复记录见 [ELECTRON_LAUNCH_REVIEW.md](ELECTRON_LAUNCH_REVIEW.md)。
