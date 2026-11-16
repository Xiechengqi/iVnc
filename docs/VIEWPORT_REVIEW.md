# toolbar 避让与显示模式同步 Review

后续变更：原始大小及查看区域滚动已由铺满屏幕取代，当前行为与验证见 [铺满屏幕与滑动切换常开 Review](DISPLAY_MODES_GESTURES_REVIEW.md)。下文保留本轮历史记录。

日期：2026-11-16。基础提交：`8bce473`。本轮实现与整体 review 已完成，已验证范围内没有剩余阻断问题。

## 问题与修复

此前的缩放高度缓存修复继续保留。本轮处理另两条链路：

- toolbar 的模式按钮只向当前激活的 iframe 发送更新；后台共享/远程应用只在 load、ready 时同步，切回已加载的应用时仍可能使用旧模式。实际复现中，父页面工作区为 956px，子页面仍按原始大小显示 1080px 高的视频；父按钮显示 fit 不能证明子视频已适配。
- 未固定的 toolbar 临时展开时原本采用覆盖方式。现在固定、临时展开和收起动画期间均预留实测高度，完全收起后释放。两种模式使用相同工作区，嵌入应用继承父页面区域，只扣减一次。

`setDisplayMode()` 统一同步所有已创建的应用视图。`app-streams.js` 将纯模式同步与聚焦、音频、激活分开，并在激活、load、ready 时补同步当前值。未打开应用不因广播而启动或创建访问令牌。重复模式同步不清空滚动位置；原始大小下在底部/右侧时，工作区缩小后继续保持边缘可见。

嵌入应用从 fit 初始化并接受父页面当前模式，不读取或写回独立 app-view 的模式偏好。独立访问页仍保存自己的选择。按钮和折叠菜单显示当前模式，tooltip、aria-label 同时说明下一步动作，并跟随中英文切换。

## 整体审查

| 审查点 | 结果 |
| --- | --- |
| 两种模式的工作区 | 视频、输入层和 native 滚动条均限定在 toolbar 上方；fit 完整显示，native 滚动到底后底部可见、可点击 |
| toolbar 显隐 | 展开前同步预留；收起期间保持预留；依据实际 transform 判断完成，覆盖零动画、减少动画、动画取消和快速反转 |
| 模式生命周期 | 当前/后台共享及远程视图均同步；加载期间快速切换以最新值为准；激活、ready 和重载可校准旧状态 |
| 重复同步 | 保留原始大小的有效滚动偏移；纯广播不发送激活、音频解锁或聚焦操作 |
| 偏好归属 | 父页控制嵌入模式，独立 app-view 偏好不被覆盖；原有明确的 native 选择继续保留 |
| 消息兼容 | 校验同源和所属窗口；保留同源父级 dashboard、本页及控制台的合法入口；嵌入页只接受父页；非法模式和旧视图身份被忽略 |
| 输入与连接 | 实际点击底部控件，验证输入映射；原 iframe 和 DataChannel 保留；没有额外视图创建或桌面尺寸请求 |
| 清理 | 销毁时清理显隐计时器、动画帧、边缘/悬停监听和模式语言订阅 |
| 多客户端及已有适配 | 保留键盘遮挡、缩放期间 resize、DPR、全屏、动态 toolbar 高度及客户端独立适配 |
| 构建与缓存 | 生产构建通过；嵌入 JS/CSS 使用已有 no-cache 策略，service worker 不拦截资源请求 |
| CI 范围 | UI 回归仅在本地运行，保留 npm 入口和依赖，未增加 Actions UI 测试 |

审查中补充了父级 dashboard 的兼容分支及回归，避免来源收紧后误拒绝合法父页面控制；同时补齐边缘监听与待执行显隐任务的清理。

## 验证结果

| 验证 | 结果 |
| --- | --- |
| 视口浏览器回归 | Chromium/WebKit 各 14 场景，共 28 项通过；包含两模式逐帧显隐、原始大小底部点击、重复同步、底部锚定、缩放 resize、动画反转和减少动画 |
| 显示模式浏览器回归 | 两引擎各 8 场景，共 16 项通过；包含后台共享/远程、延迟加载/重载、激活校准、旧视图消息、偏好隔离、合法父级控制及提示 |
| 移动输入回归 | 两引擎各 25 场景，共 50 项通过 |
| toolbar 浏览器回归 | 两引擎各 31 场景，共 62 项通过 |
| 坐标与手势检查 | 21 项通过 |
| 真实实例布局 smoke | 22 项通过；真实 Wayland Chrome、多 WebRTC 客户端及共享 GTK 应用，验证实际子视频尺寸和模式、两种模式临时 toolbar 的底部点击、滚动保留、原 iframe/DataChannel 及桌面尺寸稳定 |
| 前端生产构建、JS 语法、补丁空白检查 | 通过 |

浏览器行为检查共 156 项；Node 执行器另计父级分组，视口/显示/移动输入输出为 103 项，toolbar 输出为 65 项。修改前先确认原始大小重复同步及底部锚定用例失败，后台模式同步等 7 个新增显示模式场景也在旧代码上失败；修复后通过。

本地复核命令：

```bash
CHROME_BIN=/usr/bin/google-chrome npm --prefix web/ivnc run test:viewport
CHROME_BIN=/usr/bin/google-chrome node --test scripts/mobile_input_test.mjs scripts/toolbar_interaction_test.mjs
node --test scripts/viewport_test.mjs scripts/gesture_navigation_smoke.mjs
npm --prefix web/ivnc run build
IVNC_BIN="$PWD/target/debug/ivnc" IVNC_WEB_ROOT="$PWD/web/ivnc/dist" CHROME_BIN=/usr/bin/google-chrome node scripts/viewport_smoke.mjs
git diff --check
```

真实实例测试使用隔离的数据目录、端口和浏览器配置。本轮未替换本机或 `192.168.1.8` 的生产部署。Linux WebKit 覆盖本次显示与交互逻辑，macOS Safari 和移动端系统键盘仍需在相应设备验收；远端部署新构建后应重走“应用 native → 主桌面 fit → 返回应用”路径，并分别检查两种模式下的底部控件。
