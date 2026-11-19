# 持续集成与本地验证

GitHub Actions 只保留 `Build and Release`。UI、浏览器交互以及包含页面操作的端到端测试在本地执行；后续功能实施和整体 review 不再添加自动或手动的 Actions UI 测试入口，除非用户明确要求。

## 工作流范围

| 工作流 | 处理 |
| --- | --- |
| `Build and Release` | 保留二进制构建、Docker 构建发布和 release，以及 Docker 参数转发、容器 DNS、APT 失败处理检查 |
| `Web client regression` | 移除工作流定义，并在 GitHub 禁用工作流；不再通过 push、PR 或手动触发浏览器回归 |
| `Remote application runtime smoke` | 移除工作流定义，并在 GitHub 禁用工作流；它通过远程应用脚本间接启动 Chrome，相关集成测试改为本地执行 |

前端生产构建仍执行 `npm ci` 和 `npm run build`，这两个命令没有配置执行 UI 测试的生命周期钩子。本地测试脚本、npm 测试命令、Playwright 和 `ws` 开发依赖保留。发布镜像中的浏览器用于产品运行。

GitHub 的工作流禁用立即阻止后续触发；仓库定义的删除在提交推送后同步。既有运行记录保留供追溯，不代表仍有可执行入口。

## 本地验证

按改动范围选用现有命令：

```sh
npm --prefix web/ivnc ci
npm --prefix web/ivnc exec -- playwright install chromium webkit
npm --prefix web/ivnc run test:toolbar
npm --prefix web/ivnc run test:viewport
npm --prefix web/ivnc run test:mobile-input
npm --prefix web/ivnc run test:ime
npm --prefix web/ivnc run test:settings
npm --prefix web/ivnc run test:scroll
npm --prefix web/ivnc run test:app-sharing
npm --prefix web/ivnc run test:chrome-preset
npm --prefix web/ivnc run test:clear-data
npm --prefix web/ivnc run test:proxy-url
node --test scripts/video_playback_test.mjs scripts/viewport_test.mjs scripts/clipboard_test.mjs
node scripts/gesture_navigation_smoke.mjs
npm --prefix web/ivnc run build
# 本地构建后：带控制终端的真实 Chrome、后台服务、独立会话回归
python3 scripts/app_session_smoke.py
# 本地生产前端、真实 Chrome / GTK，以及 A → B 远程滚动
CHROME_BIN=/usr/bin/google-chrome node scripts/scroll_input_smoke.mjs
# 2–4 个真实 Chrome 视图、输入隔离、连接回收和远程撤权
IVNC_BIN=target/debug/ivnc node scripts/multi_app_view_smoke.mjs
# 可设置 LEGACY_IVNC_BIN 验证旧来源的单窗口回退；旧二进制需支持应用共享
IVNC_BIN=target/debug/ivnc LEGACY_IVNC_BIN=/path/to/old/ivnc node scripts/multi_app_view_smoke.mjs
# 同样可验证向旧来源下发滚轮事件
CHROME_BIN=/usr/bin/google-chrome LEGACY_IVNC_BIN=/path/to/old/ivnc node scripts/scroll_input_smoke.mjs
```

可设置 `CHROME_BIN` 使用本地 Chrome。实际服务、终端、认证和应用窗口的验证继续使用已有 smoke 脚本，运行时要求见各功能文档。远程应用的本地 Docker 验证见 [远程应用](REMOTE_APPS.md)。

本轮移除的审查和验证记录见 [CI 调整 Review](CI_REVIEW.md)。
