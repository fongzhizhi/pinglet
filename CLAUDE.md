# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目定位

Pinglet 是 Tauri 2 + 原生 TypeScript（无 React/Vue、无 CSS 框架）的 Windows 桌面看板，只读聚合 Claude Code 的本地交互式会话：窗口可见时每 2 秒轮询 `%USERPROFILE%\.claude\sessions\*.json`，以四种状态（待处理 / 已中断 / 进行中 / 已完成）展示，并提供置顶小窗、三态托盘图标、自绘通知弹窗、自动主题、全局快捷键、提醒与双击激活 VSCode / Git Bash；窗口隐藏时由 Rust 后台线程每 5 秒采集以驱动托盘与通知弹窗。不联网、不调用 Claude API、不写 Claude 配置或会话数据。

## 常用命令

| 目的 | 命令 |
| --- | --- |
| 安装依赖 | `npm install` |
| 浏览器预览（仅 Vite，端口 1420） | `npm run dev` |
| 桌面开发（前端热更新 + Tauri 窗口） | `npm run tauri dev` |
| 类型检查 + 前端构建 | `npm run build`（`tsc` + `vite build`） |
| 前端单元测试 | `npm test`（Vitest，单次运行） |
| 前端测试监听 | `npm run test:watch` |
| ESLint 检查 | `npm run lint` |
| Prettier 格式化 / 校验 | `npm run format` / `npm run format:check` |
| 全部 Rust 单元测试 | `cargo test --manifest-path src-tauri/Cargo.toml` |
| Rust 格式检查 | `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` |
| 运行单个 Rust 测试 | `cargo test --manifest-path src-tauri/Cargo.toml prefers_last_custom_title_over_ai_title` |
| 打包 NSIS 安装包 | `npm run tauri build -- --bundles nsis` |
| 由唯一源文件重生成图标 | `npx tauri icon assets/app-icon.svg` |

- 打包前先退出正在运行的 Pinglet，否则 release 下的 `pinglet.exe` 被占用会报 `os error 5`。
- 产物：安装包 `src-tauri/target/release/bundle/nsis/Pinglet_<version>_x64-setup.exe`，便携版 `src-tauri/target/release/pinglet.exe`。
- 前置依赖：Rust MSVC 工具链、Visual Studio Build Tools 的 C++ 工作负载 + Windows SDK、WebView2 Runtime。
- 浏览器预览在非 Tauri 环境自动加载 `src/mock-sessions.ts`；`http://localhost:1420/?settings=1` 可直接打开设置面板。
- 前端工具链：`eslint.config.js` + `.prettierrc.json` + `vitest.config.ts`；纯逻辑测试在 `src/session-state.test.ts`、`src/settings-schema.test.ts`、`src/auto-theme.test.ts`（`environment: "node"`；auto-theme 用 `vi.mock` 隔离 `settings` / `system` 的 DOM 副作用）。`npm run build` 里的 `tsc` 仍是唯一的前端类型检查。
- Rust 测试位于 `src-tauri/src/lib.rs` 末尾的 `#[cfg(test)] mod tests`（覆盖标题解析、边缘吸附、状态分类、托盘 tooltip / 通知弹窗内容与定位、亮度中位数与托盘图标合成）。
- CI 配置在 `.github/workflows/ci.yml`：Windows runner 上依次执行 lint、format check、Vitest、前端构建、`cargo fmt --check` 与 `cargo test`；release 流程见 `docs/build.md`。

## 架构总览

### 两进程数据流

```
~/.claude/sessions/*.json ──► Rust collect_sessions ──┬─ IPC list_sessions ──► src/system.ts ──► src/main.ts（可见时每 2s）
                                   │                  │                         │
                                   ├─ sysinfo 判断 pid 存活                    ├─ session-state.ts 纯函数
                                   ├─ transcript 标题解析 + 增量缓存           └─ 差量更新 DOM / 提醒 / 窗口高度
                                   └─ 后台线程每 5s：托盘图标 / tooltip / 通知弹窗
```

- Rust 独占文件、进程与 Win32 API 访问；前端不直接读文件。
- `list_sessions` 是 `#[tauri::command(async)]`（同步函数在线程池执行），返回 `{ rows, errors }`；`errors` 只含文件名与失败原因，绝不包含 JSON 内容。
- `src/session-state.ts` 必须保持无 DOM 依赖的纯函数（四态判定、排序、时长文案、目录名、入口标签、tooltip、双击动作），前后端与测试共用这些映射。
- Rust 后台监控线程每 5 秒调用一次 `collect_sessions`，只更新托盘状态 / tooltip / 通知弹窗，不受 WebView2 后台定时器节流影响；前端隐藏时暂停 2 秒轮询与心跳，恢复可见后立即刷新一次。
- 行身份 `getRowKey()` 固定为 `pid > cwd > name`；DOM 复用与状态突变提醒都按此 key 对比，改动会影响提醒去重。Rust 侧 `session_key()` 必须保持同一规则。

### 前端模块

- `src/main.ts` — 入口与所有交互：轮询、行渲染、汇总胶囊（计数签名差量更新）、logo 状态、心跳（tooltip 可见时才写文本）、提醒（呼吸灯 / 音效 / 抖动 / 免打扰门控）、tooltip、错误条、窗口自动高度、缩放热区、设置面板开关、自动主题与窗口显隐事件接线。文件较大，改动尽量局部化。
- `src/settings-schema.ts` — 无 DOM / localStorage 依赖的设置契约与纯解析：`parseSettings`、`normalizeShortcut`、`normalizePanelTransparency`、默认值，供 `settings.ts` 与 Vitest 共用。
- `src/settings.ts` — 设置单例 + 订阅；持久化键 `pinglet.settings.v1`；区分离散的用户主题模式与生效主题（`getEffectiveTheme` / `setEffectiveTheme`）；`applyTheme` / `applyFontSize` / `applyPanelTransparency` / `applyPanelEffects` 只改根节点 dataset 与 CSS 变量，不经过 Rust。`panelEffects`（异常行呼吸灯 / 进行中行流光）默认开启，写入根节点 `data-alert-pulse` / `data-working-flow` 供 CSS 开关动效。
- `src/auto-theme.ts` — 自动主题控制器：`resolveAutoTheme` 纯判定（≥0.65 暗色 / ≤0.35 亮色 / 灰区保持）+ 开启自动 / 移动结束 / 重新可见时 400ms 防抖后立即采样应用；周期采样每 10 秒、连续 2 次一致、10 秒最小切换间隔。
- `src/settings-panel.ts` — 命令式构建设置面板；系统项（自启 / 关闭行为 / 穿透 / 快捷键 / 通知消息偏好）经 `src/system.ts` 同步到 Rust，失败时回滚 UI 状态。设置面板里的「免打扰」只控制 Pinglet 自己的提醒链路；通知消息不经过 Windows 通知通道，不受系统「请勿打扰」影响。设置面板打开时 main.ts 调用 `handle.syncSystemState()` 重读系统状态。
- `src/system.ts` — 所有 Tauri API 的唯一封装；`IS_TAURI` / `requireTauri()` 保证浏览器预览不抛错；监听 `pinglet://click-through-changed`、`pinglet://open-settings`、`pinglet://window-visibility`（Rust 侧权威显隐）、`pinglet://window-moved`（自动主题重采样）。
- `src/styles.css` — 主题走 `:root[data-theme]`，字号走 `[data-font-size]` 与 `--font-size-base`；图标、圆点、按钮尺寸都用 `calc(var(--font-size-base) * …)`，不要写死 px。会话行持续动效由状态类驱动：待处理 / 已中断行背景呼吸 + 状态灯光环，进行中行底部流光；根节点 `data-alert-pulse="off"` / `data-working-flow="off"` 关闭，`is-window-paused` 暂停，`prefers-reduced-motion` 下只保留静态状态色。
- `src/mock-sessions.ts` — 仅 dev + 非 Tauri 的模拟数据（四态、未知状态兜底、死进程残留）。
- `notification.html` / `src/notification.ts` / `src/notification.css` — 右下角自绘通知弹窗的独立 Vite 入口；每次显示前调用 `reloadSettingsFromStorage()` 重读设置（自动主题读取 `settings.ts` 持久化的生效结果 `pinglet.effective-theme.v1`，本页不运行采样），启动时 `invoke("get_notification_payload")` 拉取内容，按内容实测高度调用 `resize_notification_window`；3 秒后淡出并调用 `hide_notification_window`，点击卡片调用 `open_main_window`，× 立即隐藏。`vite.config.ts` 的 `build.rollupOptions.input` 同时声明 `index.html` 与 `notification.html`。

### Rust 后端（`src-tauri/src/lib.rs`）

- 命令：`list_sessions`、`activate_session`、`quit_app`、`set_close_to_tray`、`set_click_through`、`is_autostart_enabled`、`set_autostart`、`set_global_shortcut`、`set_notification_preferences`、`show_test_notification`、`get_notification_payload`、`hide_notification_window`、`open_main_window`、`resize_notification_window`、`reset_window_position`、`sample_backdrop_luminance`。`src-tauri/src/main.rs` 只是 `pinglet_lib::run()` 的薄入口。
- `AppState` 持有 `close_to_tray` / `click_through` / `tray_ready` / 托盘勾选项 / 当前快捷键 / 吸附边缘状态，以及通知偏好（`system_notifications` / `do_not_disturb`）、待渲染的通知弹窗 payload 与自动隐藏线程的 generation 计数、托盘未读标志、窗口活跃标志、`TrayRuntime`、预生成的 `TrayIcons` 与监控线程停止标志。
- 插件注册顺序：`tauri-plugin-single-instance` 必须第一个；随后 autostart、global-shortcut。window-state 只启用 `StateFlags::POSITION`，存档为 `app_config_dir()/.window-state.json`，并用 `with_denylist` 排除通知弹窗；setup 中无存档才定位主屏右上角，有存档则 `clamp_window_into_work_area`。
- 标题解析：`nameSource = "hook"` 直接用 `name`；否则按 `sessionId` 在 `~/.claude/projects/*/<sessionId>.jsonl` 找 transcript，取最后一条 `custom-title`，其次 `ai-title`，`derived` 回退目录名，最后内部名。头部窗口 64 KB→4 MB、尾部 128 KB→8 MB，按 size + mtime 缓存并只增量扫描新增区间；transcript 未命中负缓存 30 秒，避免为新会话反复扫描目录；`sessionId` 经文件名字符白名单校验，防路径穿越。
- 状态时间：`statusUpdatedAt ?? updatedAt ?? 会话文件 mtime`。
- 托盘：三态图标在启动时由 `build_tray_icons` 内存合成（保持默认窗口图标完整尺寸，Idle / Working 绿 / Alert 红，状态色同时画外圈圆环与右下角实心角标；环宽 `max(3, 短边/10)`，角标半径 `短边 × 0.24`，保证 16px 托盘下仍醒目）。图标底图来自 exe 内嵌的图标资源（`default_window_icon()`），`build.rs` 已声明 `cargo:rerun-if-changed=icons`，重新生成 `icons/` 后必须重编 Windows 资源才会生效。后台线程 200ms tick、600ms 闪烁相位：窗口隐藏 + 新告警 + 未读 + 非免打扰时交替 Alert/Idle；窗口回到前台清除未读。tooltip 只在文本变化时更新。右键菜单「显示 / 鼠标穿透 / 设置 / 退出」；左键切换显隐；穿透状态下左键与全局快捷键先关闭穿透再显示。关闭按钮在 `close_to_tray && tray_ready` 时隐藏到托盘，否则 `app.exit(0)`。
- 后台会话监控：`spawn_session_monitor` 每 5 秒采集，首次扫描只播种；窗口隐藏或最小化 + 新增告警 + 通知消息开启 + 非免打扰时调用 `show_notification_popup`（同一轮聚合成一张、最多 3 条、标题截断 24 字）。告警去重键 `alert_event_key`：待处理带上 `statusUpdatedAt`，同一次等待不重复弹、再次进入待处理会重新弹（即使中间的 busy 短于 5 秒轮询周期）；已中断只用会话身份，进程已死后文件重写不再唤醒弹窗。不设全局冷却。
- 通知弹窗：`notification` 窗口懒创建并复用，`WebviewWindowBuilder` 设为透明、无边框、始终置顶、跳过任务栏、不抢焦点、初始隐藏，并调用 `set_focusable(false)` 避免显示时抢走当前应用的键盘焦点；显示与隐藏必须走 `show_notification_window` / `hide_notification_window_raw`（裸 `ShowWindow(SW_SHOWNOACTIVATE)` / `SW_HIDE`）——`window.show()` 会走 `SW_SHOW` 抢焦点，而裸显示后 tao 的 `VISIBLE` 标志仍为 false，`window.hide()` 会因 diff 为空变成 no-op，两者都不能换回 Tauri 的 `show()` / `hide()`；位置固定为主屏工作区右下角（16 逻辑像素边距，按 scale 换算物理坐标），工作区小于窗口时退回左上角。前端按卡片实测高度调用 `resize_notification_window` 收缩窗口并重新贴边，避免透明空白区域拦截桌面点击。Rust 将 payload 存入 `AppState` 后 emit `pinglet://notification-show`，页面启动时用 `get_notification_payload` 拉取；前端 3 秒淡出，Rust `schedule_notification_auto_hide` 5 秒兜底，generation 计数使旧计时失效。该窗口使用独立 capability `src-tauri/capabilities/notification.json`；`on_window_event` 必须以 `window.label() == "main"` 守卫，避免通知窗口事件触发全局处理或应用退出。弹窗不经过 Windows 通知通道，不受系统通知开关与「请勿打扰」影响。裸 `ShowWindow` 只改原生窗口，WebView2 仍认为页面可见，隐藏期间渲染器可能被遮挡检测挂起、复显后不出帧；显示 / 隐藏时必须同时调用 `with_webview` 的 `controller().SetIsVisible(true/false)` 同步渲染可见性，否则第二次及之后的弹窗会显示空白卡片。
- 自动主题采样：`sample_backdrop_luminance` 用 GDI `GetDC(NULL)` / `GetPixel` 在窗口下方 6 逻辑像素取 12 点，取亮度中位数；采样点夹在窗口所在显示器工作区内。
- `WindowEvent::Moved` 触发边缘吸附：10 逻辑像素内吸附，拖出 24 像素才解除（迟滞），实现见 `resolve_snap` / `snap_window_to_edges`；吸附后发出 `pinglet://window-moved` 供自动主题重采样。窗口显隐 / 最小化变化通过 `pinglet://window-visibility` 通知前端。
- `activate_session` 仅 Windows 实现：VSCode 先 `code <dir>`（移除 `ELECTRON_RUN_AS_NODE`）再轮询聚焦同名窗口；CLI 先沿进程父链找 VSCode 集成终端，否则按 Git Bash 的 `--cd` / 终端标题匹配 mintty；只激活已有窗口，不新开终端。其他平台返回错误。
- `windows-sys` 需启用 `Win32_Graphics_Gdi`；单实例、window-state 依赖见 `Cargo.toml`（不再依赖 `tauri-plugin-notification`）。
- 窗口 API 权限在 `src-tauri/capabilities/default.json` 最小化声明；新增 `core:window:*` 调用时必须同步补权限，否则运行时报错。

## 状态模型

以下规则是 v1.2 / v1.3 的设计定稿，`docs/usage.md` 的「看板状态」一节有完整文案：

- `waiting` / `blocked` → 待处理；`failed` / `crashed` / `stopped` 或 `alive = false` → 已中断；`busy` / `shell` / `working` / `starting` / `running` / `resuming` / `adopted` 或未知状态且进程存活 → 进行中；`idle` / `done` → 已完成。
- 排序：待处理 > 已中断 > 进行中 > 已完成；待处理 / 进行中按 `statusUpdatedAt` 升序（等得久的在前），已中断 / 已完成按降序。
- 展示标题优先级：`nameSource = "hook"` 的 `name` > transcript 最后一条 `custom-title` > 最后一条 `ai-title` > `nameSource = "derived"` 的目录名 > 内部名 / 「未命名会话」。列表主行必须用 `title`；内部 `name` 用于终端窗口标题匹配，并在 tooltip 中作为补充信息。
- 提醒只在状态突变时触发：启动首轮已有会话不补闪；异常（待处理 / 已中断）优先于「进行中 → 已完成」；同一次轮询多个异常取优先级更高者。

## 窗口行为

- `tauri.conf.json`：320×320 默认，宽 280–600、高 200–800，透明、无边框、始终置顶、跳过任务栏；Rust `setup` 显示窗口并定位到主屏右上角（有 window-state 存档时恢复位置并 clamp）。`app.security.csp` / `devCsp` 已启用，新增外部资源或 `connect-src` 时必须同步放行。
- 高度随内容自适应：`resizeWindowToContent()` 按内容高度调用 `setSize`；用户拖拽过的高度是上限而非固定值；用户拖拽后 500 ms 内暂停自动调整。尺寸持久化在 `pinglet.window.v2`；位置由 window-state 插件单独记录，不与尺寸偏好混用。
- 8 个透明热区在 `index.html` 的 `#resize-layer`，按下调用 `startResizeDragging`；顶栏拖动靠 `data-tauri-drag-region`，图钉只切换顶栏是否可拖动（窗口始终置顶）。
- 鼠标穿透：设置面板内开启时先挂起，面板关闭后才调用 Rust；关闭立即生效。托盘菜单 / 全局快捷键是穿透后的恢复入口。
- 单实例：第二次启动只唤起已有窗口（关穿透 → 还原 → 显示 → 聚焦），不创建新托盘或新监控线程。
- 「恢复默认」会清空全部 `localStorage` 并刷新页面，同时把关闭行为、穿透、自启、快捷键、通知消息开关恢复默认，并调用 `reset_window_position` 回到主屏右上角。

## 版本与资源

- 版本号三处必须同步：`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`。Vite 用 `package.json` 的版本注入 `__APP_VERSION__`，设置面板「关于」区展示它。
- 图标唯一源文件 `assets/app-icon.svg`；`index.html` 顶栏内联 SVG 必须与其几何一致；`npx tauri icon` 生成后只保留 `bundle.icon` 引用的 5 个文件，生成物不要手改。
- 用户文档：`docs/usage.md`（交互与窗口行为）、`docs/build.md`（构建与打包）；设计定稿：`docs/plans/pinglet-v1.2.md`、`docs/plans/pinglet-v1.3.md`、`docs/plans/pinglet-v1.4.md`（含实现状态与偏差）；标题数据源与实现说明：`docs/session-title-source.md`；背景与实现过程：`docs/plans/pinglet.md`。

## 修改时的硬约束

1. **只读、离线**：不写 `~/.claude` 下任何文件、不联网、不调用 Claude API、不修改 Claude 配置。
2. **隐私**：`sessions/*.json` 只解析白名单字段（`pid` / `cwd` / `name` / `nameSource` / `sessionId` / `kind` / `entrypoint` / `status` / `statusUpdatedAt` / `updatedAt`）；错误信息只含文件名与原因；transcript 只读头尾窗口中的 `custom-title` / `ai-title`，不读消息正文、不展示 Prompt、不写回。
3. **只展示交互式会话**：显式声明为其他 `kind` 的会话直接跳过。
4. **UI 文案**：中文；四态文案固定三字（待处理 / 已中断 / 进行中 / 已完成）；未知状态在列表兜底为「进行中」，不新增「未知」态。
5. **样式**：不引入前端 / CSS 框架；主题、字号、透明度全部走 CSS 变量；不使用 `backdrop-filter`（设计明确禁用）。
6. **代码风格**：文件名 kebab-case、变量 camelCase、常量 UPPER_SNAKE_CASE；魔法数字提取为文件顶部命名常量；注释沿用 `NOTE:` / `SECURITY:` 标签；AI 生成的模块/文件保留 `@ai-generated` 头注释。
7. 默认设置：自动主题、标准字号、透明度 0%、提醒全开、异常行呼吸灯与进行中行流光开启、通知消息开启、免打扰关闭、开启点击关闭时最小化到托盘、无鼠标穿透、无开机自启、快捷键 `Alt+Shift+P`、拖动模式。
