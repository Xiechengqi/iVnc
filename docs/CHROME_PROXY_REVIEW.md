# 内置浏览器代理配置实现与 Review

日期：2026-11-16。使用说明见 [内置浏览器代理配置](CHROME_PROXY.md)。

## 背景与实现

此前内置 Chrome 的启动命令（`builtin_chrome_command`）保存的是一段 shell 脚本：运行时读取 `$IVNC_HOME/miao/chrome-proxy` 文件，在 `IVNC_PROXY_PANEL_ENABLED != 0` 时拼接 `--proxy-server=<socks5://...>`。代理地址由 Miao 的 watchdog 与启动前准备逻辑写入该文件，属于“动态监测”。

本轮改为显式配置：

- **数据模型**：`ManagedApp` 新增 `proxy_url: Option<String>`（默认 `None`）；SQLite 通过 `ensure_column("proxy_url", "TEXT")` 幂等迁移，追加到 SELECT 末尾避免列下标错位。
- **命令生成**：`builtin_chrome_command(proxy_url)` 删除动态读文件脚本段，非空时以 `--proxy-server=<shell_quote(url)>` 静态注入；shell 引号转义阻止注入。
- **生命周期**：`ensure_builtin_chrome` 刷新命令时从已存 `proxy_url` 生成命令，不清除用户的代理选择；`update_app` 提交 `proxy_url` 时为 Chrome 重新生成命令；`app_json` 对 Chrome 下发 `proxy_url` 与 `proxy_configurable`。
- **清理死代码**：移除 `chrome-proxy` 文件写入与 `prepare_chrome_proxy`、`IVNC_PROXY_PANEL_ENABLED` 的设置与透传；watchdog 保留纯健康检查用于 Miao 崩溃自愈。
- **前端**：编辑弹窗在 desktop 分组新增 `proxy-url` 字段（仅 `proxy_configurable` 时显示），默认空；提交 body 含 `proxy_url`（空转 `null`）；含协议校验与中英文案。
- **文档**：更新 `docs/MIAO.md` 移除旧文件说明，新增本文与使用文档。

## 取舍

- 代理从“动态跟随 Miao 端口”变为“显式静态配置”，默认不再使用代理。Miao 端口变化需用户手动更新字段，这是去掉动态监测的必然结果，已在使用文档与 UI 帮助中说明。
- `proxy_url` 仅对内置 Chrome 生效，避免为普通桌面应用自动改写其自定义启动命令。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| Rust 库测试（含命令注入/转义、proxy_url 迁移、刷新保留、更新重建命令、JSON 元数据） | 全部通过 |
| 浏览器交互（Chromium / WebKit，各 6 场景） | 双引擎全部通过 |
| `npm --prefix web/ivnc run build` | 通过（仅既存 500 kB chunk 提示） |
| `cargo fmt --check` / `git diff --check` / `node --check` | 通过 |

浏览器回归覆盖：Chrome 编辑弹窗显示空代理字段、非配置应用隐藏该字段、保存代理提交 PUT、清空提交 `null`、非法 URL 被拦截且不发请求、服务端已配置值正确回填。

## 验证边界

- 浏览器测试使用隔离的 Vite 服务与拦截的 API，不修改真实应用或重启浏览器。
- 未在真实 Chrome 进程上验证 `--proxy-server` 的实际联网效果；仅验证命令文本与配置链路。代理连通性取决于用户填写的地址与网络环境。
- 未做实体设备或 TLS/代理服务器的端到端验收。
