# Shell 启动文件加载实现与 Review

日期：2026-11-17。使用说明见 [Shell 启动文件加载](SHELL_PROFILE.md)。

## 背景

此前 iVnc 的应用环境由 `app_environment` 从 manager 环境过滤重建，`application_command` 用 `env_clear()` 丢弃继承环境，最终由 `Plan::command()` 以 `/bin/sh -c` 执行——非交互、非登录 shell 不读取 `.profile` / `.bashrc`；`HOME` 也已被替换为应用隔离目录。Web Terminal 则显式使用 `bash --noprofile --norc -i`。本轮按用户选择实现**全局默认开启**、**真实 HOME 优先、隔离 HOME 补充**的 rc 加载。

## 实现

- `src/shell_profile.rs`（新）：进程级开关（默认 `true`，由 `Config::shell_profile` 初始化）、`app_profile_homes`（真实 HOME 在前、隔离 HOME 在后、去重）、`app_command`（bash bootstrap 命令）、`terminal_profile_script` / `write_terminal_profile`。
- `Config` 新增 `ShellProfileConfig { apps, terminal }`，两者 `default = true`；`main` 与 `worker::set_parent_config` 调用 `shell_profile::configure`。
- `compat::Plan` 新增 `profile_homes`（`#[serde(default)]`，旧报告兼容）。`Plan::command()` 在列表为空时保持原 `/bin/sh -c`，非空时改用 bash bootstrap。
- `application_command` 增加 `profile_homes` 参数并写入 Plan。主桌面在 `build_desktop_command` 捕获 manager 的真实 HOME；worker 把它写入 `Launch.profile_home`（`#[serde(default)]`），由 supervisor 传给 `application_command`——两条路径得到相同的 HOME 列表。
- `terminal/ws.rs`：每个新会话写 bootstrap rc 文件，bash 用 `--rcfile`，sh/dash 用 `ENV`，均在 pty 内的同一 shell 加载 rc；失败时回退原行为。
- 文档：新增本文与使用文档，更新 README 与 `docs/REMOTE_APPS.md`。

## 关键取舍

- **用交互式 bash 而非 plain `sh -c`**：Debian 默认 `.bashrc` 的 `case $- in *i*)` guard 会让非交互 source 立即返回。实测 `bash --noprofile --norc -i -c` 的 `$-` 含 `i`，带 guard 的 `.bashrc` 能正常导出变量。代价是无 TTY 时有两行 job control 警告（写入 `app.log`，不影响诊断）。
- **source 后重钉隔离变量**：用户 rc 可能导出 `DISPLAY=:0` / `XDG_SESSION_TYPE=x11`，若不放回会破坏 Wayland 会话。脚本在 source 后恢复 iVnc 负责的变量并清除 `DISPLAY` / `LD_PRELOAD`，同时 `cd` 回隔离 HOME。
- **命令文本零重解析**：原命令作为 `$0` 单个参数传递，最终 `exec /bin/sh -c "$0"`，与旧行为一致；单引号、换行、`;` 等都不会被包装层解释。
- **终端用 `--rcfile` 而非 `exec` 第二个 shell**：如果用 `bash -c 'source rc; exec bash'`，rc 中定义的函数/别名不会跨 `exec` 保留。`--rcfile` 在同一个 shell 加载，保留了这些定义。
- **全局默认开启但可关闭**：保留 `shell_profile.apps` / `shell_profile.terminal` 两个开关作为确定性隔离的逃生口。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `cargo test --lib` | 198 项通过（新增 7 项） |
| 端到端 bootstrap 测试 | 通过：带 guard 的 `.bashrc` 生效、真实 HOME 先于隔离 HOME、`XDG_SESSION_TYPE` 恢复为 `wayland`、`DISPLAY` 被清除 |
| 终端 rc 脚本实测（bash + 函数定义） | 通过：两个文件均加载，函数在交互 shell 中保留 |
| `npm --prefix web/ivnc run build` | 通过（仅既存 500 kB 提示） |
| `cargo fmt --check` / `git diff --check` | 通过 |

新增测试覆盖：HOME 顺序与去重、命令作为 `$0` 不被重解析、bootstrap 端到端行为、终端脚本顺序、Plan 在有无 profile_homes 时选择正确命令、Plan JSON 携带 HOME 列表。

## 验证边界

- 终端 rc 通过手工 bash 会话验证；未用真实浏览器 WebSocket 会话做端到端回归（pty 内）。
- 未覆盖用户在 rc 中执行 `exit` / 交互命令 / 长时间阻塞的场景；文档已说明。
- 未在 Docker / 非 root 用户环境实测；`HOME` 取自 manager 进程环境，特权切换场景需用户自行确认 HOME 正确。
