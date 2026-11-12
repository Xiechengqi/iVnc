# 普通用户部署与 Electron 沙箱

iVnc、Wayland 合成器、PulseAudio 和桌面应用采用同 UID 会话。需要保留 Electron 沙箱时，推荐以专用普通用户运行整套桌面服务，而非在某个应用命令中拼接 `su` 或 `sudo`。

当前内置 Miao 要求 root，因此本部署明确禁用内置代理。需要 Miao/TUN 的现有部署应保留其权限模型；将特权代理分离为独立服务需要后续接口设计，不能仅修改 iVnc 的 User 就假定代理仍可用。按应用切换 UID、跨用户主桌面 socket 授权目前不支持。

## systemd 示例

仓库提供 [ivnc-user.service](../deploy/ivnc-user.service)。在安装好 iVnc 和运行依赖后，由管理员创建专用用户并安装单元：

```sh
useradd --system --user-group --home-dir /var/lib/ivnc --shell /bin/bash ivnc
install -m 0644 deploy/ivnc-user.service /etc/systemd/system/ivnc-user.service
systemctl daemon-reload
systemctl enable --now ivnc-user.service
```

已存在该账户时不要重复创建。先准备 `/etc/ivnc.toml` 的认证、端口等配置，并确保服务用户可读；不要使用默认示例密码。单元通过 systemd 准备 `/var/lib/ivnc` 与 `/run/ivnc`，权限 0700、归属 ivnc。HOME、IVNC_HOME、XDG_RUNTIME_DIR、工作目录一致，不依赖 `/run/user/1000` 是否存在或用户是否登录。

使用现有 ubuntu 用户时，同时调整单元的 User/Group 和数据路径，避免与另一个实例共用 runtime。不要使用 root 会话目录，也不要把 `/run/user/0` 或 `/root` 改为公共可访问。

`server.user` 和 `server.group` 的内部降权尚未实现；设置非空值现在会明确拒绝启动，避免配置宣称降权而进程仍为 root。运行身份应由 systemd 的 User/Group 或容器运行用户指定。

## 已有数据迁移

1. 记录旧实例的启动参数、版本、数据根目录和运行应用；停止旧实例，避免数据库和应用同时写入。
2. 备份整个持久化 IVNC_HOME。复制到新的服务数据目录，只调整这份副本的归属，不递归修改 `/root`、`/run/user/0` 或原始备份。
3. 核查应用二进制、配置文件、上传路径、SSH 密钥及外部项目目录对目标用户的访问权限。原有绝对路径、外部文件归属不会随 IVNC_HOME 自动迁移。
4. 使用新的 runtime 启动，验证应用数据、剪贴板、输入、音频、文件访问和进程停止。不要复制旧 socket、PID、锁文件来复用旧会话。
5. 失败时停止新实例，保留失败副本供排查，再以旧参数和原始数据启动旧实例；两实例不能同时写同一数据目录。

## 沙箱验证

普通用户身份只是前提。Electron 还需要可用的 user namespace 沙箱，或应用发行包正确安装的 sandbox helper；Ubuntu 的 AppArmor 可能对未授权程序的 user namespace 施加限制。应检查该应用的安装说明与宿主机策略，不能全局关闭 AppArmor、添加 SYS_ADMIN 或批量修改 helper 的 setuid 位来掩盖问题。

自动识别在普通用户下不会添加 `--no-sandbox`；高级设置可显式选择“保留沙箱”。验证时不传 `--no-sandbox`，检查进程的实际 UID、应用是否出图以及日志；通过该应用的进程沙箱状态验证隔离。若发行包自身主动关闭沙箱，启动成功也不能作为“沙箱已启用”的证据。

## 启动排障

认证后的 `GET /api/apps/launch-context` 提供实际身份；应用详情的 `launch_error` 包含错误码和说明。日志分类只针对当前启动尝试，旧错误不会覆盖成功启动。主桌面应用的 `window_ready` 区分进程存在与窗口出现；独立应用返回 null，使用其运行环境状态判断会话就绪。

运行身份和上次启动详情位于编辑器默认折叠的“高级启动设置”。启动命令通常只需填程序名，参数按需由后端补齐；root 自动兼容会仅对确认的 Electron 添加 `--no-sandbox`，具体策略见 [ELECTRON.md](ELECTRON.md)。
