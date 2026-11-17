# Shell 启动文件加载

iVnc 默认在启动应用命令和 Web Terminal 时自动加载用户 HOME 下的 `.profile` 与 `.bashrc`。

## 行为

- **应用启动**（桌面应用与后台应用均适用）：在原有命令执行前，按顺序 source 两个位置的 `.profile` 和 `.bashrc`：
  1. iVnc 运行用户的**真实 HOME**（manager 进程继承的 `HOME`）；
  2. 应用的**隔离 HOME**（`$IVNC_HOME/apps/<id>/home`）。
  后加载的文件在冲突时覆盖先加载的文件；应用隔离 HOME 通常没有 rc 文件，仅作为补充。
- **Web Terminal**：每次新建终端会话（首次打开以及之后每次新建）都加载真实 HOME 的这两个文件。终端没有应用隔离 HOME，因此只读真实 HOME。

由于 `.bashrc` 的 guard（`case $- in *i*) ;; *) return;; esac`）在非交互 shell 中会直接返回，应用启动使用一个**交互式 bash** 来 source 文件（`--noprofile --norc -i`），保证带 guard 的 `.bashrc` 真正执行。

桌面应用、后台应用和独立应用会话的最外层管理进程通过 `setsid()` 创建独立会话，并将标准输入设为 `/dev/null`。因此从 SSH、tmux 或本机终端前台启动 iVnc 时，应用不会继承该终端的控制权，交互式 Bash 也不会因处于后台进程组而用 `SIGTTIN` 暂停整组进程。会话创建仍保持管理进程 `PID == PGID`，停止、重启和后代进程回收沿用原逻辑。独立会话内部的 runtime、reaper 和应用继续共享 supervisor 的进程组。

source 每个文件时会**临时把 `HOME` 设为该文件所属的 HOME**，因此 rc 中类似 `export NVM_DIR="$HOME/.nvm"` 的写法会正确展开为真实 HOME（而非应用的隔离 HOME）；source 完成后 `HOME` 恢复为隔离 HOME。

原始启动命令最终在**同一个 bash 中以 `eval` 执行**，而不是交给 `/bin/sh`。这带来两个好处：rc 加载后的 `PATH` 对命令生效；命令本身可以使用 `source` 等 bash 专有内置命令（dash 没有 `source`，这正是此前 `source: not found` 的原因）。命令文本作为单个 `$0` 参数传入，不会被包装层提前解析。

终端在 pty 内使用 `bash --rcfile <bootstrap> -i`（或 `sh` 的 `ENV` 机制），rc 文件在**同一个交互式 shell** 中执行，因此其中定义的函数和别名会保留。

控制台会将受管桌面应用或后台服务的管理进程暂停状态显示为“进程已暂停”，保留停止、重启操作；进程恢复后该提示自动消失。该诊断读取管理进程的实时状态，不会自动恢复用户暂停的进程，也不将调试器的跟踪暂停误判为启动故障。

## 隔离红线

source 完成后，iVnc 会重新导出它负责的变量，防止用户 rc 意外破坏会话：

- `HOME` 恢复为应用的隔离 HOME；
- `XDG_SESSION_TYPE`、`GDK_BACKEND`、`QT_QPA_PLATFORM` 固定为 `wayland`；
- `XDG_RUNTIME_DIR`、`WAYLAND_DISPLAY`、`PULSE_SERVER` 恢复为 source 前的值；
- `DISPLAY`、`WAYLAND_SOCKET`、`LD_PRELOAD` 被清除。

PATH 等普通变量保留 rc 修改后的值——这正是加载 rc 的目的（如 nvm、conda）。`NVM_DIR` 等基于 `$HOME` 的变量按真实 HOME 展开，不指向隔离目录。

## 关闭

全局配置（默认均为 `true`）：

```toml
[shell_profile]
apps = false      # 应用启动不加载 rc
terminal = false  # Web Terminal 不加载 rc
```

关闭后恢复为此前的确定性隔离行为：应用只继承经过过滤的环境变量，完全不执行任何启动文件。

## 注意事项

- 用户 rc 中的 `echo`/`printf` 会写入应用日志（`app.log`）；iVnc 的启动诊断仅匹配特定错误文本，普通输出不会造成误判。
- rc 中若执行 `exit` / `exec`、长时间等待或交互命令，可能导致应用启动中断或阻塞。请保持 rc 文件非交互且快速返回。
- rc 中重设 `DISPLAY` / `XDG_SESSION_TYPE` 不会生效，见上文隔离红线。
- worker 与 A 在同一主机、以同一用户运行；若两者 HOME 内容不同，则加载结果不同。
- 无 TTY 的交互式 bash 会在 `app.log` 留下两行无害的 job control 警告（`cannot set terminal process group` / `no job control`）。

实现与验证见 [实现与 Review](SHELL_PROFILE_REVIEW.md)，控制终端暂停修复及本地回归见 [应用启动会话 Review](APP_SESSION_REVIEW.md)。
