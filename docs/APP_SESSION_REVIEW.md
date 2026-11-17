# 应用启动会话修复与 Review

日期：2026-11-17。相关说明见 [Shell 启动文件加载](SHELL_PROFILE.md)。

## 问题与根因

远程 `192.168.1.6` 从 tmux/终端前台运行 iVnc 后，内置 Chrome 一直显示“进程运行中 · 等待窗口”。现场没有真正的 Chrome 进程，应用日志为空；reaper 和它启动的 Bash 都处于 `T (stopped)` 状态。

新增的 Shell 配置加载以 `bash --noprofile --norc -i -c` 执行应用启动命令。原启动链只调用 `setpgid(0, 0)`，应用仍继承 manager 的控制终端，却不属于终端前台进程组。Bash 初始化作业控制时调用 `kill(0, SIGTTIN)`，把 reaper 和自己一起暂停。进程没有退出，原退出检测和崩溃重试不会触发。

隔离诊断证实，仅重定向 stdin 或仅添加 Bash `+m` 仍会暂停；创建独立会话后启动包装正常执行。

## 实现

- `native::configure_app_session` 在最外层受管应用进程的 `pre_exec` 中调用 `setsid()`，同时将 stdin 设为 `/dev/null`。桌面应用的 reaper、独立应用会话的 supervisor、后台服务共用此边界。
- `setsid()` 同时保证外层进程 `PID == PGID == SID`，因此已有按负 PGID 发送信号的停止/重启路径继续有效；调用失败作为 spawn 错误返回。
- 独立会话内部的 runtime、reaper 和应用不再次创建会话，继续留在 supervisor 的进程组。移除 supervisor 中冗余的 `setpgid` 调用，避免对会话首进程执行无效操作。
- 原 `.profile` / `.bashrc` 加载顺序、带交互 guard 的 rc、`source`、隔离 HOME 和 Wayland 环境恢复逻辑保留。
- 桌面应用和后台服务读取受管管理进程的实时 `/proc/<pid>/stat`；`T` 状态返回 `app_process_stopped`。不消费 waitpid 事件，不保存过期故障，不自动恢复或崩溃重启暂停进程；`SIGCONT` 后诊断自动清除。调试器的 `t` 状态不作为此故障。
- 控制台优先显示中/英文“进程已暂停”，沿用警告样式并保留停止/重启操作。API 的生命周期 `status` 保持原有语义，暂停进程仍存在。
- Web Terminal 继续通过独立 PTY 启动交互 Shell，不使用应用启动的会话配置函数。

## 验证

所有 UI、浏览器和实例回归均在本地隔离环境执行；没有新增或修改 GitHub Actions 测试步骤。

| 检查 | 结果 |
| --- | --- |
| `cargo test --lib --features tls --offline` | 206 项通过；含独立会话/空 stdin 和暂停/恢复动态诊断测试 |
| `cargo build --features tls --bin ivnc --offline` | 通过 |
| `npm --prefix web/ivnc run build` | 通过，仅既有 500 kB bundle 提示 |
| `python3 scripts/app_session_smoke.py`，root | 5 组检查通过；manager 实际拥有前台控制 PTY，真实 Chrome 映射 Wayland 窗口、重启、后台服务读取 guarded rc、独立 Chrome 会话及暂停/恢复/停止树均成功 |
| 同一脚本，以普通用户 ubuntu 运行 | 5 组检查通过 |
| `scripts/app_sharing_interaction_test.mjs`，Chromium + WebKit | 33 项通过；新增桌面/独立/后台暂停提示、中英切换、操作保留及恢复后的清除 |
| `scripts/launch_compat_smoke.mjs` | 6 组通过；单次兼容重试、子进程回收、worker 与用户停止行为正常 |
| `scripts/launch_smoke.mjs`，真实 Electron、root | 9 组通过；主桌面/worker、窗口就绪、持久数据、启动错误及恢复均正常 |
| `scripts/terminal_smoke.mjs` | 21 项通过；真实 PTY 输入输出、重连、中文输入、复制粘贴和终端窗口操作正常 |
| `cargo fmt --check` / JS、Python 语法 / `git diff --check` | 通过 |

本轮 Rust 验证使用 `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0` 控制本机磁盘占用。真实 Electron 使用现有本地验证安装。所有新实例使用临时 HOME、`IVNC_HOME`、runtime、Chrome profile 和随机端口；测试进程已退出，常驻实例未被替换或停止。

## 整体 Review 结论

未发现阻塞问题。复核了会话创建位置、PGID 与 PID 的约定、Wayland 客户端归属、父进程退出检测、正常与超时强制退出、worker 子树回收、暂停诊断的时效和本地测试清理。新回归使用真实控制 PTY，可覆盖此前无 TTY 验证遗漏的触发条件。

边界：暂停诊断针对受管管理进程，不扫描每个应用内部线程或单独暂停的子进程。Shell 配置中主动执行 `exit`、`exec` 或无限等待仍遵循现有行为。本轮未替换或重启远程 `192.168.1.6` 的二进制；部署新版本后修复才会作用于该实例的新启动链。
