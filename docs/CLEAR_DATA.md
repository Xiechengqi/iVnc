# Chrome 的选择性数据清理

控制台“应用”页的“清理数据”操作对配置生成模式的 Chrome 预设打开选择性清理对话框，包含内置 Chrome（`builtin-chrome`）和用户新增的 Chrome。手动命令模式停用目录清理；其他应用保持原有的一次性确认流程。本文说明类别、扩展数据边界、接口与安全边界，预设配置见 [Chrome 桌面应用](CHROME_APPS.md)。

## 清理类别

接口在应用的 `clear_categories` 字段中下发类别列表，前端按该列表渲染，不硬编码内置应用 ID。**默认勾选：缓存、Cookie 与网站标识、历史、密码、书签；默认不勾选：扩展、网站本地存储。**

| 类别 | 主要删除内容 | 保留内容 |
| --- | --- | --- |
| `cache` 缓存数据 | profile 与各 Default 子目录下的 Cache、Code Cache、GPUCache、ShaderCache、Service Worker 的 CacheStorage/ScriptCache、应用 `.cache` 等 | 登录状态、网站数据、Service Worker 注册、扩展；完整 sidecar 配置目录不受影响 |
| `cookies` Cookie 与网站标识 | Cookies、Safe Browsing Cookies、Trust Tokens、DIPS、SharedStorage、BrowsingTopics、Network Persistent State、TransportSecurity、Reporting and NEL 及非扩展 IndexedDB | **localStorage / sessionStorage / WebStorage、Service Worker 注册**、缓存文件、历史、密码、书签、扩展 |
| `history` 浏览历史 | History、Favicons、Top Sites、Sessions、Download Service、购物/优惠相关数据库 | 其他类别 |
| `passwords` 密码与自动填充 | Login Data、Web Data、Account Web Data、Affiliation Database、passkey 状态等 | 其他类别 |
| `bookmarks` 书签 | Bookmarks、Bookmarks.bak、BookmarkMergedSurfaceOrdering | 其他类别 |
| `extensions` 扩展及其数据（默认不选） | Extensions、Local Extension Settings、扩展 IndexedDB；并从 Preferences 删除 `extensions` 记录、删除 Secure Preferences 以便 Chrome 重建 | 其他类别；Preferences 无法解析为 JSON 时整体删除重建 |
| `site_storage` 网站本地存储（默认不选） | Local Storage、Session Storage、WebStorage | Cookie、缓存、历史、密码、书签、扩展、Service Worker 注册 |

类别清单只列出已知的 Chrome profile 条目。Chrome 更新后新增的未知文件会被保留，因此选择性清理可能留下少量数据。宿主机用户的 `~/.config/google-chrome`、`$IVNC_HOME/.config/google-chrome` 及全局 `/tmp/com.google.Chrome.*` 不属于本应用数据，选择性清理和完全重置都不会删除它们。

## 扩展数据能否恢复

Chrome 把网站数据和扩展数据混合存放在同一批存储里，因此“保留扩展数据”只能按存储类型分别成立：

| 扩展数据 | 默认清理后 | 原因 |
| --- | --- | --- |
| 扩展代码、安装/启用状态、Secure Preferences MAC | ✅ 完整保留 | 位于 `Extensions/` 与 `Preferences`，非扩展类别不触及 |
| `chrome.storage.local` / `chrome.storage.sync` | ✅ 完整保留 | 位于 `Local Extension Settings/<id>`、`Sync Extension Settings/<id>` |
| 扩展 IndexedDB | ✅ 完整保留 | `clear_indexeddb` 显式保留 `chrome-extension_` 前缀 |
| Service Worker 注册 | ✅ 保留 | 默认清理不删除整个 `Service Worker` 目录；仅缓存类别删除其中的 CacheStorage/ScriptCache |
| 扩展的 `localStorage` | ✅ 默认保留 | 位于 `Local Storage`/`WebStorage`，现已拆到 `site_storage` 且默认不选 |

**注意**：若用户显式勾选 `site_storage`，Chrome 无法按 origin 区分网站与扩展的 localStorage（两者共用一个 LevelDB），文件系统级删除会一并清除扩展的 localStorage，这部分数据**无法恢复**。UI 已为此提供明确警示。这是 Chrome 混合存储的固有限制，不是可以通过文件清单规避的问题。

## 完全重置

“完全重置”需要二次点击确认，仅在 profile 与崩溃目录均位于该应用根目录内时提供：删除该应用的数据目录、隔离 HOME、profile、崩溃目录及其专属临时目录，并重建应用布局。它不会删除其他应用或宿主机浏览器的数据。

外部目录仅支持按类别清理，且必须已由此应用启动并建立归属标记；不会递归删除整个外部目录。手动模式没有可信的有效目录，返回 `400 chrome_manual_cleanup`。外部目录请求完全重置返回 `400 chrome_external_full_reset`。删除应用记录也只删除该应用自己的根目录，保留外部目录。

清理前会先确认该应用停止，并在进程管理锁和资源锁内执行，避免 watchdog 重启或其他 Chrome 启动与删除竞争。活动 profile、目录重叠以及通过符号链接越界的删除会被拒绝。单个条目删除失败会汇总返回。

## 接口

```
POST /api/apps/{id}/clear-data
Content-Type: application/json

{"categories": ["cache", "cookies"]}     # 选择性清理
{"full_reset": true}                      # 完全重置
```

- 省略请求体时按默认类别（不含 `extensions` 与 `site_storage`）清理；`site_storage` 需显式提交才会执行。
- 未知类别返回 `400 invalid_clear_category`；空列表返回 `400 no_clear_category`。
- Miao 返回 `403`。浏览器会话的 CSRF 由现有请求层统一注入。
- 成功返回 `200`，含 `ok`、`categories`、`full_reset`、`freed_bytes` 与 `freed_human`（人类可读的释放大小）。

浏览器交互回归见 [清理数据与 Review](CLEAR_DATA_REVIEW.md)。
