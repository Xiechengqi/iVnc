# 内置浏览器代理配置

内置 Chrome 的代理由应用自身的“代理地址”（`proxy_url`）配置项显式指定，不再在启动时动态读取内置代理面板（Miao）的实时 SOCKS 地址。

## 配置方式

在控制台“应用”页对 Chrome 点击“编辑”，在“代理地址”字段填写代理 URL，例如：

```text
socks5://127.0.0.1:1080
```

- 留空（默认）表示不使用代理。
- 支持的协议：`socks5://`、`socks4://`、`socks5h://`、`http://`、`https://`。
- 该值作为 `--proxy-server=<url>` 直接写入 Chrome 的启动命令，并做 shell 引号转义，防止注入。
- 保存后需重启 Chrome 才会应用新代理；运行中的浏览器不会动态跟随字段变化。
- Miao 的 SOCKS 端口若因重启或配置变化而改变，需要相应更新此字段。

只有内置 Chrome 暴露此字段（由应用元数据 `proxy_configurable` 标识）；其他桌面应用的启动命令由用户自行管理，不自动注入代理参数。

## 与 Miao 的关系

内置代理面板（Miao）继续由 `ui_features.proxy_enabled` 与 `--proxy-enabled` 控制启停、健康检查与崩溃自愈，与 Chrome 的代理配置相互独立。iVnc 不再生成 `$IVNC_HOME/miao/chrome-proxy` 文件，也不再向应用环境透传 `IVNC_PROXY_PANEL_ENABLED`。

配置字段的存储、迁移、API 契约及验证见 [实现与 Review](CHROME_PROXY_REVIEW.md)。
