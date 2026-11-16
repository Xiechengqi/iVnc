# 选择性清理数据实现与 Review

日期：2026-11-16。基础提交：`bfe18de` 之后的未提交改动。实现、本地回归与整体 review 已完成，已验证范围内没有剩余阻断问题。使用说明见 [内置 Chrome 的选择性数据清理](CLEAR_DATA.md)。

## 第一轮修正

实现主体完成后，独立 review 发现并修正了以下问题：

- **缓存类别误删宿主机 Chrome 配置**：`chrome_cache_paths` 原先把 `chrome_sidecar_config_dirs()` 整目录列入缓存清单，勾选“缓存数据”会 `remove_dir_all` 删除真实用户的 `~/.config/google-chrome`（书签、密码、历史、扩展）。现已从所有选择性类别中移除，sidecar 仅由完全重置处理，并新增回归测试断言全选类别后 sidecar 仍存在。
- **测试未注册**：`src/apps/api.rs` 中两个新增 `#[test]` 被错误嵌套在另一个 `async fn` 内，编译器报 `cannot test inner items`，实际从未运行。已移到模块顶层，确认可单独按名称运行通过。
- **注释与实现不一致**：选择性清理的文档注释曾声称保留 “Local State”，但实现从不删除它。已修正注释，明确 Local State 属于浏览器级元数据，始终保留。
- **样式缺失**：新对话框的类名在 `console.css` 中无定义。已补齐面板、类别列表、勾选行、危险按钮与窄屏样式。
- **HTML 杂物**：`console.html` 中误插入日志模态框内部的注释和错位缩进已清理。

## 第二轮修正：扩展数据可恢复性

进一步分析 Chrome 存储机制后发现：原 `cookies` 类别删除 `Local Storage`/`Session Storage`/`WebStorage` 和整个 `Service Worker`，会清除扩展的 `localStorage` 并破坏 SW 注册，与“默认保留扩展数据”的承诺冲突。按成本与收益选择了最优组合：

- **新增 `site_storage`（网站本地存储）类别**：把 `Local Storage`/`Session Storage`/`WebStorage` 从 `cookies` 中拆出，**默认不勾选**。显式勾选才会清理，并在中英文案中明确警示“会同时删除扩展的 localStorage，无法恢复”。
- **cookies 类别不再删除整个 `Service Worker` 目录**：仅缓存类别清理其中的 `CacheStorage`/`ScriptCache`，使扩展/网站的 SW 注册在选择性清理中稳定保留，与缓存类别原有注释意图对齐。
- **修正默认集合**：`default_chrome()` 现在同时排除 `extensions` 与 `site_storage`，因此默认清理后扩展的代码、安装状态、`chrome.storage.*`、扩展 IndexedDB、SW 注册和 localStorage **全部保留**。
- **更新 API 与前端**：类别数由 6 增至 7；`clear_selection` 测试、下发的 `clear_categories`、前端 I18N 与 `CLEAR_DEFAULT_UNCHECKED` 同步更新。
- **文案对齐**：`cookies` 描述改为“Cookie 与网站标识”，不再声称清理本地存储；新增 `site_storage` 说明。
- 未采用 LevelDB 级按 origin 精确过滤（高成本、跨版本格式不稳定、并发写入风险），因此显式清理 `site_storage` 时仍无法区分网站与扩展的 localStorage，已通过文档与 UI 明确披露这一固有限制。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| Rust 库测试（含新增 cookies/site_storage 保留矩阵与 sidecar 断言） | 187 项通过 |
| 新增 API 测试按名称可运行、无 `cannot test inner items` 告警 | 通过 |
| 浏览器交互（Chromium / WebKit，各 11 场景） | 双引擎全部通过 |
| `npm --prefix web/ivnc run build` | 通过 |
| `cargo fmt --check` / `git diff --check` / `node --check` | 通过 |

浏览器回归覆盖：默认勾选（扩展与网站本地存储不选）、全部取消后确认按钮禁用、取消/关闭/遮罩/Escape 不发送请求、仅提交勾选项并在等待期间锁定、默认提交不含扩展与网站本地存储、显式勾选网站本地存储可提交、完全重置二次确认、请求失败后保持打开并允许重试、服务端错误文本不解析为 HTML、非 Chrome 应用走旧确认流程、类别文案随语言切换。

## 验证边界

- 测试使用隔离的 Vite 服务与拦截的 API，不清理真实应用数据，也不重启服务。
- sidecar 与存储保留测试在子进程中使用临时 HOME，确保断言不受其他并行测试影响；它们验证“选择性清理不触碰 sidecar 与默认保留项”，不验证完全重置删除宿主机目录的真实行为（该行为沿用旧逻辑）。
- 未做实体设备或真实 Chrome profile 的手工清理验收；类别清单依据已知 profile 条目，Chrome 未来版本的目录变化可能需要更新清单。
- 显式 `site_storage` 清理会同时删除扩展 localStorage 的限制由 Chrome 混合存储决定，未尝试 LevelDB 级精确过滤。
