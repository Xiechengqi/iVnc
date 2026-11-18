# Chrome 桌面应用

控制台添加应用时先选“桌面应用”，再从“应用”下拉框选择“自定义命令”或“Chrome”。Chrome 行显示安装检测结果；没有安装时仍可保存配置，启动时会报告 `google-chrome` 不可用。

检测使用运行 iVnc 的用户、应用环境变量和已启用的 Shell 启动文件，与实际启动环境一致。弹窗打开期间每 5 秒刷新，修改环境变量立即重新检测；关闭弹窗停止检测。检测本身不启动 Chrome，也不创建 profile。没有自动下载或安装浏览器的行为。

## 配置生成

选择 Chrome 后，启动命令默认只读。以下字段修改后，由后端实时生成预览；保存和启动使用同一个生成器。

| 配置 | 新增 Chrome 默认值 | 说明 |
| --- | --- | --- |
| 用户数据目录 | 留空，使用 `$IVNC_HOME/apps/<id>/profile` | 每个应用独立；填写时必须为绝对目录 |
| 崩溃日志目录 | 留空，使用 `$IVNC_HOME/apps/<id>/crash` | 每个应用独立；允许放在自己的 profile 内 |
| 远程调试端口 | 留空 | 留空不传任何远程调试参数；非空为 1–65535 |
| 代理地址 | 留空 | 留空不传代理参数；支持 HTTP、HTTPS、SOCKS4、SOCKS5 与 SOCKS5H |

新增弹窗一开始就分配稳定的应用 ID，预览路径与最终保存路径一致，不随改名变化。目录无需预先创建，启动时创建；应用改名不会迁移目录。

生成器自动添加 Wayland、首次启动、独立窗口身份等参数；以 root 运行时自动添加 `--no-sandbox`，普通用户保留沙箱。Chrome 的 XDG 数据与缓存位于应用自己的 HOME。临时数据使用专属且短的 `/tmp/ivnc-chrome-<uid>-<hash>`，避免 UUID 目录层级使 Chrome 原生 Unix socket 路径超长。

保存运行中应用的新配置不会改变现有进程，弹窗会提示重启。切换目录不会自动复制或删除旧目录里的数据。

## 手动命令

勾选“手动修改启动命令”，当前生成命令成为可编辑文本。手动命令的空白、换行与参数原样保存，不再由配置项或 Electron 自动适配器改写。下面的目录、端口和代理字段变为不可用，保留其草稿值供切回配置模式时使用。

切回配置模式前，如果手动命令有改动，会确认是否替换。手动模式的真实目录由命令自行决定，因此停用 Chrome 的推断清理功能；删除应用时仅删除 iVnc 管理的应用根目录，不清理命令指向的外部数据。

## 内置 Chrome 与升级迁移

内置浏览器使用同一套 Chrome 预设，可调整配置并切换手动模式，其预设类型固定为 Chrome。

- 旧版标准启动模板会迁移为配置生成模式，保留原 profile、`profile/Crash Reports`、调试端口、代理、名称、顺序、环境变量和自启动设置。
- 新注册的内置浏览器同样预填原有目录与调试端口：默认 9222，初次注册可由 `IVNC_CHROME_DEBUG_PORT` 指定。已有应用保存的配置优先，明确清空端口后不会在重启时恢复 9222。
- 如果初次注册时默认端口已分配给另一个 Chrome 应用，内置浏览器关闭调试端口，保留另一个应用的设置。
- 无法确认是标准模板的旧命令保留为手动命令；不尝试任意解析 shell 脚本。旧配置不符合结构化校验时也保留原命令。
- 保存后的外部目录暂时不可用不会阻止 iVnc 启动，可进入控制台修正。该 Chrome 启动时仍会正常检查并报告错误。
- `.desktop` 入口使用稳定应用 ID 和独立脚本，正确执行多行 shell 命令。确认替换成功后仅移除严格识别的旧版内置入口，保留自定义入口。

## 多实例与清理边界

保存配置时拒绝目录重叠（包括已存在路径的符号链接别名）、跨入其他应用根目录以及重复的已配置调试端口。启动时还检查目录权限、原生 Chrome SingletonLock、profile 文件锁及实际占用的端口。不会删除活跃或无法确认归属的 Chrome singleton 锁来强行启动。

运行中的进程持续持有实际目录与端口的资源租约，即使保存了新配置，其他应用仍不能抢占它正在使用的资源。主桌面与独立共享应用会话使用同一套 manager 解析出的绝对目录。

选择性清理支持所有配置生成模式的 Chrome，默认保留扩展和网站本地存储，以保护扩展数据。外部目录需要已由本应用启动并确认归属；只清理已知类别，不支持完全重置。只有 profile 和崩溃目录都位于本应用根目录内才提供完全重置。宿主机 Chrome 的配置和其他应用数据不在清理范围。详见 [选择性数据清理](CLEAR_DATA.md)。

## API

```http
GET /api/apps/desktop-presets
POST /api/apps/desktop-presets
POST /api/apps/chrome-preview
```

`POST desktop-presets` 接受 `id`（编辑）或 `creation_id`（新增草稿），以及 `env_vars`。返回 `presets`；Chrome 的 `installation.status` 为 `installed`、`missing`、`not_executable` 或 `unknown`，检测不能确认时不阻止实际启动。

预览与创建使用以下结构。预览无应用记录和目录创建副作用，返回 `exec_command`、`chrome_config` 与 `chrome_effective`。创建走原有 `POST /api/apps`，编辑走 `PUT /api/apps/{id}`：

```json
{
  "name": "工作浏览器",
  "app_type": "desktop",
  "creation_id": "d91f7970-8ee7-4cd9-a0f1-054197a43a50",
  "desktop_preset": "chrome",
  "chrome_config": {
    "mode": "generated",
    "profile_dir": null,
    "crash_dir": null,
    "debug_port": null
  },
  "proxy_url": null
}
```

新增使用 UUID v4 的 `creation_id`；编辑预览使用已有 `id`。生成模式忽略客户端传入的 `exec_command`；手动模式需传 `mode: "manual"` 和非空 `exec_command`。配置字段支持部分更新；`debug_port: null` 明确关闭调试端口。`proxy_url` 沿用原有字段，不在 `chrome_config` 中重复存储。

应用详情包含 `desktop_preset`、`chrome_config`、`chrome_effective`、`can_clear_data`、`can_full_reset` 和 `clear_categories`，前端按能力显示清理入口。自定义桌面命令使用 `desktop_preset: "custom"`。

## 本地验证

```sh
cargo test --lib --features tls
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:chrome-preset
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:clear-data
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:proxy-url
npm --prefix web/ivnc run build
cargo build --features tls --bin ivnc
IVNC_BIN=target/debug/ivnc python3 scripts/app_session_smoke.py
```

浏览器交互测试使用 Chromium、WebKit；真实 Chrome smoke 使用临时 HOME、数据库、运行目录和端口，验证主桌面、独立共享会话及同时运行的两个 Chrome。所有浏览器测试仅在本地执行，见 [持续集成与本地验证](CI.md)。整体 review 记录见 [Chrome 应用 Review](CHROME_APPS_REVIEW.md)。
