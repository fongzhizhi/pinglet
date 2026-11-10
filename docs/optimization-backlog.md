# Pinglet 优化待办

> 记录 2026-11-07 复查后尚未实施的优化项，供后续排期。完成后勾选并补充日期。
>
> 「优先项」是下一轮建议处理的工程化收尾；「待排期」和「可选/长期」可以按需安排。
>
> 2026-11-10：托盘状态图标、自动主题、告警弹窗及配套改进（单实例保护、免打扰、窗口位置记忆、托盘 tooltip、差量更新）的调研与实施计划见 [plans/pinglet-v1.4.md](plans/pinglet-v1.4.md)。

## 优先项

- [ ] **P1 Clippy 纳入 CI**
  - 现状：`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 有 23 条，主要是 edition 2024 的 `collapsible_if`，另有 2 处 `manual_pattern_char_comparison`、1 处 `unnecessary_sort_by`。
  - 处理：先运行 `cargo clippy --fix` 自动修大部分，手工处理剩余项；在 `.github/workflows/ci.yml` 的 `dtolnay/rust-toolchain` 中加入 `clippy` 组件，并增加 `cargo clippy ... -- -D warnings` 步骤。
  - 验收：本地与 CI 均无 Clippy warning。

- [ ] **P1 补安装包元数据**
  - 现状：`src-tauri/tauri.conf.json` 的 `bundle` 只有 `active` / `targets` / `icon`，安装器与 Windows「应用和功能」中的发布者、版权、描述、语言不完整。
  - 处理：补 `publisher`、`copyright`、`shortDescription`、`longDescription`、`category: "Utility"`；`bundle.windows.nsis` 增加 `languages: ["SimpChinese", "English"]`、`installMode: "currentUser"`。
  - 验收：安装器与「应用和功能」显示正确的发布者、描述和中文界面。

- [ ] **P1 版本号与 Release 标签校验**
  - 现状：版本号分散在 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`；`release.yml` 使用 `tagName: v__VERSION__`，手动触发或标签与版本不一致时可能生成版本号对不上的 Release。
  - 处理：新增 `scripts/check-versions.mjs`，读取三处版本并在 CI 校验；`release.yml` 在 tag 触发时校验 `github.ref_name == "v$version"`，不一致直接失败。
  - 验收：版本漂移时 CI/Release 明确报错。

- [ ] **P1 Dependabot + 固定工具链**
  - 现状：没有 Dependabot；CI 使用 `@stable` 和浮动的 Node 22；GitHub Actions 使用 major tag。
  - 处理：新增 `.github/dependabot.yml`（npm / cargo / github-actions，weekly，分组 minor/patch）；新增 `rust-toolchain.toml`（固定 Rust 1.95.0 + `clippy` / `rustfmt`）和 `.nvmrc`（Node 22），CI 改为读取固定版本。
  - 验收：依赖更新有 PR 可审，Clippy 不会因工具链升级突然多出新 lint。

## 待排期

- [ ] **P2 补测试，优先隐私契约**
  - 前端：把 `src/settings.ts` 的 `parseSettings` / `parseReminders` / `normalizePanelTransparency` 抽成纯函数模块，补损坏 JSON、字段类型错误、范围裁剪、提醒组合等测试。
  - Rust：把 session 文件解析抽成 `parse_session(&str)` / `read_session_file(path)`，补白名单字段、未知字段忽略、非交互 `kind` 跳过、错误信息不含 JSON、transcript 未命中负缓存等测试。
  - 验收：`npm test` 与 `cargo test` 覆盖上述分支；不引入 DOM 测试框架。

- [ ] **P2 发布流程加固**
  - 现状：`release.yml` 没有 `concurrency`，tag 发布不经过 CI，也没有产物校验和。
  - 处理：加 `concurrency`；加 `verify` job（或复用 CI）跑 `npm test` / `cargo test`；生成并上传 `SHA256SUMS`；发布说明可接 CHANGELOG 或自动 notes。
  - 验收：标签推送不会绕过测试；Release 资产带校验和。

- [ ] **P2 CSP 再收紧 + 冒烟**
  - 处理：在 `app.security.csp` / `devCsp` 增加 `base-uri 'none'; form-action 'none'; frame-ancestors 'none'`；评估 `app.security.freezePrototype: true`。
  - 验收：`npm run tauri dev` 打开 DevTools 无 CSP/API 报错；再执行一次 `npm run tauri build -- --bundles nsis` 冒烟。

- [ ] **P2 可访问性小修**
  - 设置面板：`role="dialog" aria-modal="true"`，背景 `inert` 或焦点陷阱；保留现有 Esc 关闭与焦点归还。
  - 汇总区：`aria-live="polite"` 的内容每 2 秒 `replaceChildren`，改为文本变化时才更新。
  - 心跳：`role="status"` + 每秒 `aria-label` 更新，改为仅在等级变化时更新，或改成非 live 的 `role="img"`。

## 可选/长期

- [ ] 拆分大文件：`src/main.ts`（约 1308 行）、`src-tauri/src/lib.rs`（约 1531 行）、`src/settings-panel.ts`（约 512 行）；建议按 `board/reminders/window`、`sessions/titles/tray` 拆开。
- [ ] `TITLE_CACHE` 按当前 `sessionId` 集合 prune，避免长期运行只增不减。
- [ ] 压缩 `docs/images/screenshot.png`（约 806 KB），缩短 README 首屏加载时间。
- [ ] 代码签名：公开分发需要证书与 CI secrets；未签名前保留 SmartScreen 已知限制说明。
- [ ] GitHub 网页设置：仓库描述、topics、首个 Release、分支保护（代码之外，手动完成）。
