# Changelog

本文件记录 Pinglet 的版本变更，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added

- CI 纳入 `cargo clippy -- -D warnings`，并新增版本号一致性校验（`npm run check:versions`）；Release 的 `verify` job 在构建前运行全部检查。
- Release 上传 `SHA256SUMS.txt`（安装包与便携版可执行文件的 SHA-256），发布说明从本文件对应版本小节自动生成。
- Dependabot 每周检查 npm / cargo / GitHub Actions 依赖；新增 `rust-toolchain.toml`（Rust 1.99.1 + clippy / rustfmt）与 `.nvmrc`（Node 22）。
- NSIS 安装包补充 publisher / copyright / category / 中英双语等元数据。
- 设置面板新增「托盘动效」开关，可关闭托盘图标的呼吸 / 闪烁动画并退回静态三态。

### Changed

- 会话行布局调整：目录标签与入口图标移到第一行（状态之后、时间之前），标题独占第二行以获得更完整的展示宽度；目录作为稳定的会话识别锚点优先于 AI 生成的标题。
- Rust 后端按职责拆分 `lib.rs` 为 `activation` / `monitor` / `notify` / `session` / `settings` / `title` / `tray` / `window` 模块，测试随代码迁移。
- 前端从 `main.ts` 抽取目录配色、提醒判定、心跳等级、窗口尺寸四组纯逻辑模块，并补齐 Vitest 覆盖。
- 设置面板按使用场景重组分组，减少同类选项分散。

### Fixed

- 标题缓存与 transcript 未命中缓存按当前活跃 `sessionId` 裁剪，修复长期运行内存只增不减。
- 窗口尺寸存档拒绝 `NaN` / `Infinity`，避免异常存档导致窗口尺寸失效。
- 修正 NSIS 打包脚本参数。

## [1.4.1] - 2026-11-11

### Added

- 已完成会话支持右键「临时隐藏」，仅驻留内存，状态变化或会话消失后自动恢复；设置面板可一键恢复全部隐藏项。
- 主窗口显示 / 隐藏动画（缩放 + 淡入淡出），与行动效互不干扰。
- 主题切换时的颜色过渡，仅在主题真正变化且未开启 reduce-motion 时播放。
- 托盘状态动画：进行中 15 帧呼吸、告警 10 帧闪烁，80ms tick，保留降级开关。
- CLI 会话激活新增逐级兜底：VSCode 集成终端 → mintty `--cd` → mintty 标题 → VSCode 标题 → 唯一 VSCode 窗口。

### Changed

- 通知弹窗停留时间由 3 秒延长到 5 秒（Rust 兜底同步延长到 8 秒）。
- 顶栏 logo 改为常亮，不再随状态置灰 / 高亮。
- 应用图标改为透明背景并放大图形；托盘图标改为深绿空心圆角描边底图，不再叠加状态色圆环 / 角标。

### Fixed

- 修复双击会话时 VSCode / Git Bash 窗口匹配失败的多个兜底场景。

## [1.4.0] - 2026-11-11

### Added

- 托盘三态图标与 tooltip：空闲 / 进行中 / 告警分别以静态、呼吸、闪烁表达；tooltip 汇总各状态数量。
- 自动主题：按窗口下方屏幕亮度中位数自动切换暗 / 亮主题，带防抖、连续采样与最小切换间隔。
- 自绘通知弹窗：窗口隐藏或最小化时，新告警以右下角卡片聚合展示，不经过 Windows 通知通道。
- 单实例保护、窗口位置记忆、应用内免打扰。
- 面板行持续动效：异常行背景呼吸 + 状态灯光环，进行中行底部流光，均可单独关闭。
- 汇总区与心跳改为差量更新，降低每秒 DOM 写入。

### Changed

- 窗口隐藏时由 Rust 后台线程每 5 秒采集会话并驱动托盘与通知，不受 WebView2 后台定时器节流影响；前端 2 秒轮询仅在窗口可见时运行。

### Fixed

- 修复通知弹窗显示时抢占当前应用焦点、复用后显示空白、同一次等待重复弹窗等问题。

## [1.3.1] - 2026-11-07

### Changed

- 完善工程化配置与发布流程。

### Docs

- 补充 VS Code 扩展会话不出现在看板上的排查说明。

## [1.3.0] - 2026-11-07

### Added

- 首个公开版本：四态看板（待处理 / 已中断 / 进行中 / 已完成）、状态突变提醒、托盘与全局快捷键、设置面板、暗 / 亮 / 自动主题、窗口边缘吸附与高度自适应。
- 双击会话行激活对应的 VSCode 窗口或 Git Bash / VSCode 集成终端。
- 只读、离线的数据访问：仅解析会话 JSON 白名单字段与 transcript 标题记录。
