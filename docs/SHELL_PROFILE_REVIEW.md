# Shell 启动文件加载实现与 Review

日期：2026-11-17。使用说明见 [Shell 启动文件加载](SHELL_PROFILE.md)。

后续带控制终端的启动暂停修复及补充验证见 [应用启动会话 Review](APP_SESSION_REVIEW.md)。

## 背景

此前 iVnc 的应用环境由 `app_environment` 从 manager 环境过滤重建，`application_command` 用 `env_clear()` 丢弃继承环境，最终由 `Plan::command()` 以 `/bin/sh -c` 执行——非交互、非登录 shell 不读取 `.profile` / `.bashrc`；`HOME` 也已被替换为应用隔离目录。Web Terminal 则显式使用 `bash --noprofile --norc -i`。随后按用户选择实现**全局默认开启**、**真实 HOME 优先、隔离 HOME 补充**的 rc 加载（第一版）。

第一版只覆盖桌面应用与终端。后台应用走独立的 `ServiceProcessManager::build_service_command()`，仍以 `Command::new("sh")` 执行，因此用户为该类型应用配置的 `source /root/.bashrc` 报 `sh: 1: source: not found`，且 nvm 未初始化导致 `which node` 失败。本轮修复后台应用接入 bootstrap，并修正两个共性问题：source 时 `$HOME` 的展开目标、原命令的执行 shell。

## 实现

- `src/shell_profile.rs`（新）：进程级开关（默认 `true`，由 `Config::shell_profile` 初始化）、`app_profile_homes`（真实 HOME 在前、隔离 HOME 在后、去重）、`app_command`（bash bootstrap 命令）、`terminal_profile_script` / `write_terminal_profile`。
- `Config` 新增 `ShellProfileConfig { apps, terminal }`，两者 `default = true`；`main` 与 `worker::set_parent_config` 调用 `shell_profile::configure`。
- `compat::Plan` 新增 `profile_homes`（`#[serde(default)]`，旧报告兼容）。`Plan::command()` 在列表为空时保持原 `/bin/sh -c`，非空时改用 bash bootstrap。
- `application_command` 增加 `profile_homes` 参数并写入 Plan。主桌面在 `build_desktop_command` 捕获 manager 的真实 HOME；worker 把它写入 `Launch.profile_home`（`#[serde(default)]`），由 supervisor 传给 `application_command`——两条路径得到相同的 HOME 列表。
- `terminal/ws.rs`：每个新会话写 bootstrap rc 文件，bash 用 `--rcfile`，sh/dash 用 `ENV`，均在 pty 内的同一 shell 加载 rc；失败时回退原行为。
- **本轮**：`apps/service_process.rs` 的 `build_service_command` 在 manager 端捕获真实 HOME、解析应用隔离 HOME，经 `service_command()` 复用 `app_command` bootstrap——后台应用现在与桌面应用加载同一组 rc 文件；抽出 `service_command()` 便于测试，并新增该模块的测试。
- **本轮**：bootstrap 脚本 source 每个文件前临时 `HOME="$ivnc_h"`，使 rc 中 `$HOME/.nvm` 等按文件所属 HOME 展开（nvm 等工具需要），source 后恢复隔离 HOME。
- **本轮**：bootstrap 末行由 `exec /bin/sh -c "$0"` 改为 `eval "$0"`，原命令在已加载 rc 的同一 bash 中执行——`source` 等 bash 内置命令在启动命令中可用，且 PATH 生效。
- 文档：新增本文与使用文档，更新 README 与 `docs/REMOTE_APPS.md`。

## 关键取舍

- **用交互式 bash 而非 plain `sh -c`**：Debian 默认 `.bashrc` 的 `case $- in *i*)` guard 会让非交互 source 立即返回。实测 `bash --noprofile --norc -i -c` 的 `$-` 含 `i`，带 guard 的 `.bashrc` 能正常导出变量。代价是无 TTY 时有两行 job control 警告（写入 `app.log`，不影响诊断）。
- **source 后重钉隔离变量**：用户 rc 可能导出 `DISPLAY=:0` / `XDG_SESSION_TYPE=x11`，若不放回会破坏 Wayland 会话。脚本在 source 后恢复 iVnc 负责的变量并清除 `DISPLAY` / `LD_PRELOAD`，同时 `cd` 回隔离 HOME。
- **原命令在 bash 中 `eval`，仍在 `$0` 单参数内**：原命令作为 `$0` 单个参数传入，单引号、换行、`;` 不会被包装层提前解析。末行 `eval "$0"` 让它在同一 bash 中执行（而非 `/bin/sh`），这样 `source` 可用、rc 带来的 PATH 生效；这是修复 `source: not found` 与 `which node` 失败的关键。代价是命令的 shell 语义从 sh 变为 bash（bash 兼容 sh，且用户显式需要 `source`）。
- **`$HOME` 按文件所属 HOME 展开**：source 前临时切 `HOME`，使 `NVM_DIR="$HOME/.nvm"` 指向真实 HOME；实测远端 `FINAL_NVM_DIR=/root/.nvm`、`FINAL_HOME=<隔离 HOME>`，两者均正确。
- **终端用 `--rcfile` 而非 `exec` 第二个 shell**：如果用 `bash -c 'source rc; exec bash'`，rc 中定义的函数/别名不会跨 `exec` 保留。`--rcfile` 在同一个 shell 加载，保留了这些定义。
- **全局默认开启但可关闭**：保留 `shell_profile.apps` / `shell_profile.terminal` 两个开关作为确定性隔离的逃生口。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `cargo test --lib` | 202 项通过（本轮新增 4 项） |
| 端到端 bootstrap 测试 | 通过：带 guard 的 `.bashrc` 生效、真实 HOME 先于隔离 HOME、`XDG_SESSION_TYPE` 恢复为 `wayland`、`DISPLAY` 被清除 |
| `$HOME` 展开与 `source` 测试 | 通过：rc 中 `$HOME` 按文件所属 HOME 展开、启动命令内 `source` 可用 |
| 远端实测（ssh root@192.168.1.8，用新 bootstrap 跑用户命令） | 通过：`source` 不再报错、`which node` 找到 `/root/.nvm/.../node`、`FINAL_NVM_DIR=/root/.nvm`、`FINAL_HOME=<隔离 HOME>` |
| 终端 rc 脚本实测（bash + 函数定义） | 通过：两个文件均加载，函数在交互 shell 中保留 |
| `npm --prefix web/ivnc run build` | 通过（仅既存 500 kB 提示） |
| `cargo fmt --check` / `git diff --check` | 通过 |

新增测试覆盖：后台应用无 profile 时用 `sh -c`、有 profile 时用 bash bootstrap（`source` 在 `$0` 内、真实 HOME 在前）、`$HOME` 按文件展开与命令内 `source`、原端到端行为、HOME 顺序与去重、Plan 命令选择。

## 验证边界

- 终端 rc 通过手工 bash 会话验证；未用真实浏览器 WebSocket 会话做端到端回归（pty 内）。
- 未覆盖用户在 rc 中执行 `exit` / 交互命令 / 长时间阻塞的场景；文档已说明。
- 未在 Docker / 非 root 用户环境实测；`HOME` 取自 manager 进程环境，特权切换场景需用户自行确认 HOME 正确。
