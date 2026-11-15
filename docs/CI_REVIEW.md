# 移除 Actions UI 测试与整体 Review

日期：2026-11-15。

## 实施结果

- 删除 `.github/workflows/web-tests.yml`，移除 Ubuntu／macOS 上的 Chromium／WebKit 回归、状态测试和对应触发入口。
- 删除 `.github/workflows/remote-smoke.yml`。该工作流虽然只支持手动触发，但调用链 `remote_app_docker_smoke.sh → remote_app_smoke.sh → remote_view_smoke.mjs` 会启动 Chrome 进行视频、输入、上传和内嵌页面验证，因此一并移除 Actions 入口。
- 已通过 GitHub API 禁用上述两个工作流，并核验它们均为 `disabled_manually`。当前唯一处于 `active` 状态的工作流是 `Build and Release`；两个已禁用工作流均没有正在运行或排队的任务。
- 保留所有本地回归脚本、npm 测试入口与开发依赖。新增根目录 `AGENTS.md`，明确除非用户要求，后续不再添加或恢复任何自动、手动及间接调用的 Actions UI 测试。
- 更新开发说明及移动输入、远程应用文档；旧 review 中的 CI 描述注明当时状态和本次调整，保留原验证记录。

## 整体审查

1. **间接入口**：检查了工作流实际调用的脚本，补齐手动远程冒烟中的浏览器入口，避免只删除名称中带前端测试的工作流。
2. **构建发布链**：`build-release.yml`、Dockerfile 和 `build.rs` 与实施前逐字节一致。构建仍保留前端打包、Rust 二进制、Docker 镜像和 release；依赖关系仍为 `build → docker → release`，release 同时依赖 build 和 docker。
3. **本地验证能力**：测试脚本及 `package.json`、`package-lock.json` 未修改，所有已有 npm 测试入口仍指向存在的脚本。`npm ci` 和 `npm run build` 没有执行 UI 测试的生命周期钩子。
4. **GitHub 配置**：已核对 main 无分支保护和适用规则集，不存在待完成的 UI 必需检查，无需调整合并规则。构建发布工作流保持启用。
5. **执行与历史**：远端禁用立即生效，仓库中的文件删除在提交推送后同步。保留历史运行记录；它们不会因禁用或删除定义而消失，不代表工作流仍处于启用状态。

已验证范围内没有剩余阻塞项。自动浏览器回归转为本地验证，后续功能实施与整体 review 仍按改动范围执行本地测试。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| 本地工作流与 YAML | 仅剩 `build-release.yml`；语法、触发器和三个 job 的依赖检查通过 |
| GitHub 工作流状态 | 两个浏览器相关工作流已禁用；仅构建发布启用 |
| GitHub 分支规则及任务 | main 无保护或规则集；两个禁用工作流没有运行中或排队任务 |
| Docker 入口参数、信号和退出行为 | 11 项通过 |
| 构建容器 DNS 选择和交接 | 14 项通过 |
| Docker APT 失败处理 | 5 项通过 |
| 发布配置、构建文件、本地测试及依赖保留 | 与实施前逐字节比对通过 |
| 文档链接与补丁空白 | 通过 |

复测命令：

```sh
python3 scripts/docker_entrypoint_test.py -v
python3 scripts/ci_container_dns_test.py -v
python3 scripts/docker_apt_test.py -v
git diff --check
```

本轮只调整工作流入口、开发约定和文档。未重新执行 UI 套件、完整发布构建、部署或运行实例重启；相关产品代码和构建配置没有变更。GitHub 禁用操作已生效，仓库中的定义删除随本次提交同步。
