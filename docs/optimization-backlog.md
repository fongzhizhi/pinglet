# Pinglet 优化待办

> 记录 2026-11-07 复查后尚未实施的优化项，供后续排期。完成后勾选并补充日期。
>
> 「优先项」是下一轮建议处理的工程化收尾；「待排期」和「可选/长期」可以按需安排。
>
> 2026-11-10：托盘状态图标、自动主题、告警弹窗及配套改进（单实例保护、免打扰、窗口位置记忆、托盘 tooltip、差量更新）的调研与实施计划见 [plans/pinglet-v1.4.md](plans/pinglet-v1.4.md)。
>
> 2026-11-12：完成工程护栏与发布质量收尾（Clippy、版本校验、安装包元数据、Dependabot、固定工具链、Release 校验与校验和）。
>
> 2026-11-12：完成模块化第一阶段（Rust `lib.rs` 拆分、前端纯逻辑抽取）、标题缓存裁剪、截图压缩，并接入 CHANGELOG 发布说明。

## 优先项

- [x] **P1 Clippy 纳入 CI**（2026-11-12）
  - 完成：`cargo clippy --fix` 自动修复 31 处（edition 2024 `collapsible_if`、`manual_pattern_char_comparison`、`as_chunks_mut` 等），手工修复 `unnecessary_sort_by`；当前 `--all-targets -- -D warnings` 零警告。CI 与 Release 的 `verify` job 均加入 Clippy 步骤。

- [x] **P1 补安装包元数据**（2026-11-12）
  - 完成：`bundle` 增加 `publisher`、`copyright`、`category: "Utility"`、`shortDescription`、`longDescription`；`bundle.windows.nsis` 增加 `installMode: "currentUser"`、`languages: ["SimpChinese", "English"]`。安装包界面效果需下次打包时冒烟确认。

- [x] **P1 版本号与 Release 标签校验**（2026-11-12）
  - 完成：新增 `scripts/check-versions.mjs`（`npm run check:versions`），校验三处版本一致与 semver 格式；CI 每次运行校验，标签触发 Release 时额外用 `--tag` 校验标签与版本一致。

- [x] **P1 Dependabot + 固定工具链**（2026-11-12）
  - 完成：新增 `.github/dependabot.yml`（npm / cargo / github-actions，每周一，minor/patch 分组）；`rust-toolchain.toml` 固定 Rust 1.99.1 + `clippy` / `rustfmt`；`.nvmrc` 固定 Node 22。CI 与 Release 从这两个文件读取版本。

## 待排期

- [ ] **P2 补测试，优先隐私契约**
  - 前端：把 `src/settings.ts` 的 `parseSettings` / `parseReminders` / `normalizePanelTransparency` 抽成纯函数模块，补损坏 JSON、字段类型错误、范围裁剪、提醒组合等测试。
    - v1.4 已完成：纯逻辑迁至 `src/settings-schema.ts`，`src/settings-schema.test.ts` 覆盖空存储、自动主题、通知字段、类型回退、损坏 JSON 与快捷键截断；其余分支继续补。
    - 2026-11-12 已完成：目录配色、提醒判定、心跳等级、窗口尺寸迁至独立模块并由 Vitest 覆盖；`main.ts` 剩余部分仍以 DOM 与窗口行为为主。
  - Rust：把 session 文件解析抽成 `parse_session(&str)` / `read_session_file(path)`，补白名单字段、未知字段忽略、非交互 `kind` 跳过、错误信息不含 JSON、transcript 未命中负缓存等测试。
  - 验收：`npm test` 与 `cargo test` 覆盖上述分支；不引入 DOM 测试框架。

- [x] **P2 发布流程加固**（2026-11-12）
  - 完成：`release.yml` 增加 `concurrency`（同 ref 不并发）；新增 `verify` job，标签触发时校验版本、再跑 lint / format / Vitest / 前端构建 / `cargo fmt` / Clippy / `cargo test`，通过后才构建；发布后生成并上传 `SHA256SUMS.txt`（安装包 + 便携版 exe）。发布说明接入 CHANGELOG 仍待后续处理。

- [ ] **P2 CSP 再收紧 + 冒烟**
  - 处理：在 `app.security.csp` / `devCsp` 增加 `base-uri 'none'; form-action 'none'; frame-ancestors 'none'`；评估 `app.security.freezePrototype: true`。
  - 验收：`npm run tauri dev` 打开 DevTools 无 CSP/API 报错；再执行一次 `npm run tauri build -- --bundles nsis` 冒烟。

- [ ] **P2 可访问性小修**
  - 设置面板：`role="dialog" aria-modal="true"`，背景 `inert` 或焦点陷阱；保留现有 Esc 关闭与焦点归还。
  - 汇总区：`aria-live="polite"` 的内容每 2 秒 `replaceChildren`，改为文本变化时才更新。
    - v1.4 已完成：按计数签名比较，未变化不重建 DOM。
  - 心跳：`role="status"` + 每秒 `aria-label` 更新，改为仅在等级变化时更新，或改成非 live 的 `role="img"`。
    - v1.4 已完成：改为 `role="img"`；等级变化才写 `data-level`，文本仅在 tooltip 可见时更新。

## 可选/长期

- [x] **拆分 Rust `lib.rs`**（2026-11-12）
  - 完成：拆为 `activation` / `monitor` / `notify` / `session` / `settings` / `title` / `tray` / `window` 模块，`lib.rs` 仅保留 crate 文档、模块声明、共享常量、`AppState` 与 `run()`；测试随代码迁移。
- [ ] 继续拆分前端大文件：`src/main.ts`（约 1739 行，已抽出纯逻辑，剩余以 DOM / 窗口行为为主）、`src/settings-panel.ts`（约 512 行）；建议按 `board/reminders/window`、`system/general` 继续拆开。
- [x] **`TITLE_CACHE` 按当前 `sessionId` 集合 prune**（2026-11-12）
  - 完成：新增 `prune_title_cache`，每轮采集在成功读取会话目录后调用，同时裁剪标题缓存与 transcript 未命中缓存。
- [x] **压缩 `docs/images/screenshot.png`**（2026-11-12）
  - 完成：无损重编码，806 KB → 35 KB。
- [ ] 代码签名：公开分发需要证书与 CI secrets；未签名前保留 SmartScreen 已知限制说明。
- [ ] GitHub 网页设置：仓库描述、topics、首个 Release、分支保护（代码之外，手动完成）。
