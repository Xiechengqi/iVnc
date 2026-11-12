# Electron 启动身份与会话 Review

本文记录启动会话修复时的行为；后续 Electron 自动识别与沙箱默认策略见 [ELECTRON_AUTO_LAUNCH_REVIEW.md](ELECTRON_AUTO_LAUNCH_REVIEW.md)。

日期：2026-11-12。针对 root 直接启动 t3code 被 Electron 沙箱拒绝、改用 `su - ubuntu` 后无法连接 root Wayland 会话的问题。此前已实现的原生 Wayland 协议支持保持不变。

## 最终行为

- 认证后的启动上下文接口与应用编辑器展示实际有效 UID/GID、用户名。Electron 模板默认保留沙箱，只有显式勾选后才生成 `--no-sandbox`；不修改已有命令、不猜测任意 shell 命令中的应用类型。
- 主桌面与独立 worker 在启动应用前校验 HOME、runtime 所有者与 0700 权限、socket 类型与可访问性。初始服务启动也检查 runtime，不再忽略目录创建失败或修改已有目录的权限。
- Docker 入口按 0700 创建新 runtime，并在启动 PulseAudio 前校验现有目录；避免原先默认 mkdir 产生的 0755 目录与新校验冲突，不擅自修改挂载目录或符号链接目标。
- 应用工作目录统一为自己的 HOME，并过滤宿主机 `WAYLAND_SOCKET`；同用户外部启动使用的 `ivnc env` 同时导出 Wayland 会话类型。
- 按本次启动的日志偏移与文件身份诊断退出原因，读取上限 64 KiB，不跟随日志符号链接、不阻塞读取 FIFO。区分 root 沙箱、Wayland 连接、X11 后端和沙箱安装问题；非致命 D-Bus/GPU 警告不构成失败。
- 确定的沙箱/后端配置错误停止自动重试，用户修正后可再次启动。临时显示错误与未知崩溃保留有上限的退避策略。预检失败保留在应用详情，停止或重新启动会清除上一尝试的错误。
- 主桌面窗口状态绑定当前受管理进程，避免旧窗口快照在快速重启后把新进程误报为窗口已出现；新尝试开始前清理上一代已退出进程，避免旧 watchdog 记录覆盖本次预检错误。独立 worker 的运行状态仍表示会话就绪，其 `window_ready` 为 null。
- 非空 `server.user/group` 明确报错，避免静默忽略降权配置。提供同 UID 普通用户 systemd 单元和备份、迁移、回滚说明。

## 验证

| 验证 | 结果 |
| --- | --- |
| `cargo test --features tls --bin ivnc -- --test-threads=4` | 120 项通过 |
| `scripts/launch_smoke.mjs`，root、Electron 45.3.1 | 6 项通过：真实沙箱拒绝、停止重试、显式兼容恢复、窗口、持久化、独立 worker、显示预检 |
| 同一启动验证，以 UID/GID 65534 运行 | 5 项通过；未传 `--no-sandbox`，renderer sandbox 配置开启，Linux NoNewPrivs=1、Seccomp=2 |
| `scripts/window_ui_smoke.mjs` | 21 项通过，含运行身份、模板默认保留沙箱及显式选择、已有命令/环境保护 |
| `scripts/electron_smoke.mjs` | 27 项通过，真实 Electron → Wayland → WebRTC，含输入、剪贴板、窗口和文件 |
| `scripts/docker_entrypoint_test.py` | 11 项通过；隔离测试覆盖新 runtime 权限、拒绝公共目录/符号链接、参数转发和停止清理 |
| 前端生产构建、TLS 服务端构建、补丁空白检查 | 通过 |
| `systemd-analyze verify deploy/ivnc-user.service` | 单元通过；系统现有 snapd 单元提示一个不相关的 RestartMode 兼容警告 |

默认并发的完整 Rust 测试出现过一次既有剪贴板管道测试的 3 秒超时；该测试单独复测以及限制为 4 线程的完整测试均通过。未修改该测试的期限或断言。编译期间磁盘曾耗尽，清理本项目可重建产物并恢复受影响的测试 fixture 后重新构建、验证。

## 远程 t3code 验收

实例 `192.168.1.8` 的 iVnc 进程继续使用原有 `0c34484` 二进制，没有重启整套服务。通过管理 API 备份并修改 T3code 应用命令为：

```sh
t3code --no-sandbox --ozone-platform=wayland --enable-wayland-ime --wayland-text-input-version=3
```

备份：远程 `/root/.ivnc/backups/t3code-launch-20261112T081325Z.json`。保留原应用 ID、数据目录、环境与自启动设置。应用持续运行；通过远程主机上的独立 headless 浏览器实际连接 iVnc，确认 WebSocket 101、WebRTC 视频解码、T3code 工具栏入口，并截图确认 T3 Code 欢迎页。验证浏览器已关闭，t3code 保持运行。

从开发机直接观看该远程实例以及经 HTTP SSH 转发观看时，WebRTC 未建立，故改用远程主机上的浏览器完成出图验收；未修改生产网络配置。远程新增的 UI 和诊断代码仍需按正常发布流程部署。

## 边界与兼容性

- 当前远程 root 兼容命令明确关闭该应用的 Chromium 沙箱。普通用户部署保留沙箱的路径已验证，但未自动迁移现有实例。
- 内置 Miao 依然要求 root。普通用户单元明确禁用该功能；特权代理分离、按应用切换 UID 和跨用户会话授权属于后续独立设计，不在本次实现中。
- 既有相对路径启动命令现在以应用 HOME 为工作目录，应改为绝对路径或把文件放在该 HOME。原有非 0700/其他用户所有的 runtime 会明确拒绝启动，需要部署配置修正。
- 沙箱诊断覆盖明确的 Electron/Chromium 启动签名；不承诺识别所有第三方启动器。普通用户环境仍取决于应用发行包及宿主机 AppArmor/user namespace 策略。
- 不同 Electron 应用的 Secret Service、通知、portal、GPU 和账户配置仍按需提供；此次 t3code 验收未配置其 agent 账户或执行项目任务。

整体 review 未发现本轮修改中未处理的阻断问题。使用说明见 [ELECTRON.md](ELECTRON.md) 与 [NONROOT.md](NONROOT.md)。
