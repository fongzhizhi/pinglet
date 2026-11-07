# Pinglet 独立仓库迁移计划

> 状态：已执行（2026-11-07）。本文档随 Pinglet 一起迁入新仓库，作为迁移历史记录保留。
>
> 目标：把 `apps/pinglet` 从 `claude-skill-lab` 迁出，建立独立 GitHub 仓库，用于对外分享、版本发布和后续维护；两个仓库互相推荐与引用。

## 一、背景与目标

Pinglet 已经具备 v1.3 可用版本，准备对外分享。当前它位于 `claude-skill-lab/apps/pinglet/`，而 `claude-skill-lab` 的定位是 Claude Code 扩展能力（skill / agent / command / hook / workflow）的个人研发工作台。

拆分的原因：

- Pinglet 是自包含的 Tauri 应用，有独立的 `package.json`、`src-tauri/`、构建链和版本号，不依赖 lab 的代码。
- 对外分享需要清晰的仓库地址、README、Release 和 issue 入口。
- 独立仓库可以单独打 tag、发布安装包、配置 CI，不与 lab 的工具链混在一起。
- lab 可以继续保持“工作台”定位，不被应用构建产物和用户反馈干扰。

本次迁移不做：

- 不修改 Pinglet 的功能与 UI。
- 不引入自动更新、代码签名、GitHub Actions（列为后续项）。
- 不迁移 `clis/`、`suites/` 等其他目录。

## 二、目标结构

假定 GitHub 用户名为 `fongzhizhi`（按实际账号替换）：

| 仓库 | 地址 | 迁移后内容 |
| --- | --- | --- |
| Pinglet | `https://github.com/fongzhizhi/pinglet` | 应用源码、README、LICENSE、设计文档、Release |
| claude-skill-lab | `https://github.com/fongzhizhi/claude-skill-lab` | 保留 lab 模块；移除 `apps/pinglet/`，仅保留外链与索引 |

新仓库根目录建议：

```text
pinglet/
├── README.md
├── LICENSE
├── .gitignore
├── package.json
├── package-lock.json
├── index.html
├── vite.config.ts
├── tsconfig.json
├── assets/
├── src/
├── src-tauri/
└── docs/
    └── plans/
        ├── pinglet.md
        ├── pinglet-v1.2.md
        └── pinglet-v1.3.md
```

## 三、迁移前检查

- [ ] 确认新仓库名与可见性。建议先建 private 仓库，验证通过后再 public。
- [ ] 确认 `apps/pinglet/` 没有未提交改动，记录当前提交：`git log -1 --oneline -- apps/pinglet`。
- [ ] 全局搜索 lab 内对 Pinglet 的引用：`grep -rn "apps/pinglet\|Pinglet" --exclude-dir=target --exclude-dir=node_modules .`。
- [ ] 检查 README 中的相对链接：`apps/pinglet/README.md` 当前引用 `../../project-docs/plans/pinglet*.md`，迁移后要改成 `docs/plans/pinglet*.md`。
- [ ] 确认根 `LICENSE` 可复用（MIT，`Copyright (c) 2026 Victor Hayes`），新仓库需要带同一份。
- [ ] 确认当前版本号为 `1.3.0`，安装包为 `Pinglet_1.3.0_x64-setup.exe`。
- [ ] 决定是否保留 git 历史。当前 `apps/pinglet` 只有一次提交（`52f2652`），直接复制即可；如需保留，使用 `git subtree split`。

## 四、迁移步骤

### 1. 创建新仓库

在 GitHub 新建空仓库 `pinglet`，**不要**勾选 README、LICENSE、.gitignore，避免与本地文件冲突。

```bash
git clone https://github.com/fongzhizhi/pinglet.git
cd pinglet
```

### 2. 导出应用源码

在 `claude-skill-lab` 仓库根执行：

```bash
git archive HEAD:apps/pinglet | tar -x -C ../pinglet
```

`git archive` 只导出已跟踪文件，不会带 `node_modules/`、`src-tauri/target/` 等构建产物。

如果需要保留提交历史：

```bash
git subtree split --prefix=apps/pinglet -b pinglet-export
cd ../pinglet
git pull ../claude-skill-lab pinglet-export
```

当前 Pinglet 只有一个提交，推荐直接导出，历史价值不大。

### 3. 迁移设计文档

```bash
mkdir -p ../pinglet/docs/plans
cp project-docs/plans/pinglet.md \
   project-docs/plans/pinglet-v1.2.md \
   project-docs/plans/pinglet-v1.3.md \
   ../pinglet/docs/plans/
```

迁移后在 lab 删除这三份文档，避免双份维护；本迁移计划可以保留在 lab，作为历史记录。

`pinglet.md` 内仍有 3 处 `apps/pinglet/` 路径描述（第 11、262、646 行附近），迁移后改为“仓库根目录”或 `docs/plans/`，避免新仓库里出现不存在的路径。

### 4. 新仓库补齐文件

**LICENSE**：从 lab 根目录复制 `LICENSE` 到新仓库根目录，保持 MIT 与版权署名不变。

**.gitignore**：`apps/pinglet/.gitignore` 目前**不含** `src-tauri/target/`，该规则由 lab 根 `.gitignore` 的 `apps/*/src-tauri/target/` 兜底。新仓库必须把两处规则合并，并去掉 `apps/*/` 前缀：

```gitignore
# 构建产物
node_modules
dist
dist-ssr
src-tauri/target/
src-tauri/gen/
*.local

# 日志
logs
*.log
npm-debug.log*
yarn-debug.log*
yarn-error.log*
pnpm-debug.log*
lerna-debug.log*

# 本地 UI 截图
.shots/

# 生成的图标（只保留 bundle.icon 引用的 5 个文件）
src-tauri/icons/android/
src-tauri/icons/ios/
src-tauri/icons/64x64.png
src-tauri/icons/Square*.png
src-tauri/icons/StoreLogo.png
src-tauri/icons/icon.png

# 编辑器
.vscode/*
!.vscode/extensions.json
.idea
.DS_Store
*.suo
*.ntvs*
*.njsproj
*.sln
*.sw?
```

**README.md**：以 `apps/pinglet/README.md` 为基础修改：

- 删除 `apps/pinglet/` 路径前缀，所有命令改为在新仓库根目录执行。
- 把 `../../project-docs/plans/...` 改为 `docs/plans/...`。
- 在“运行与构建”之后增加“相关项目”段落，链接回 lab。
- 补充 Release 安装包说明与 SmartScreen 提示。

**配置**：`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 中的 `name` / `identifier` / `version` 保持不变：

- `name`: `pinglet`
- `identifier`: `com.fongzhizhi.pinglet`
- `version`: `1.3.0`

### 5. 首次提交与发布

```bash
cd ../pinglet
git add -A
git commit -m "feat: 初始化 Pinglet v1.3.0"
git tag v1.3.0
git push -u origin main
git push origin v1.3.0
```

如果本地分支不是 `main`，按实际分支名推送。

在 GitHub 创建 Release：

- Tag：`v1.3.0`
- 标题：`Pinglet v1.3.0`
- 附件：`Pinglet_1.3.0_x64-setup.exe`，并附 SHA256
- 说明：Windows 10/11、WebView2、未签名安装包的 SmartScreen 绕过方式

### 6. 清理 lab 仓库

```bash
git rm -r apps/pinglet
git rm project-docs/plans/pinglet.md \
       project-docs/plans/pinglet-v1.2.md \
       project-docs/plans/pinglet-v1.3.md
```

然后更新 lab 内引用：

- 根 `README.md`：第 35 行「独立桌面应用」的示例说明原指 Pinglet，改为通用描述或注明“当前无本地应用”；第 130–134 行「独立应用（apps/）」表中的 Pinglet 改为外链，状态写 `v1.3 可用（独立仓库）`。
- `apps/README.md`：保留 `apps/` 作为自研应用的约定与索引；Pinglet 一行改为外链，注明“已迁移至独立仓库”。
- `CLAUDE.md`：`clis/ 与 apps/` 小节可保留通用约定；如提到示例，注明 Pinglet 已迁出。
- `project-docs/design.md`：目录结构中的 `apps/<name>/` 约定保留，供未来应用使用。
- `.gitignore`：`apps/*/target/` 等规则可以保留，不影响其他目录。

### 7. 提交 lab 迁移

```bash
git add -A
git commit -m "refactor: 将 Pinglet 迁移至独立仓库"
git push
```

## 五、两个仓库互相引用

### 5.1 新仓库 README 增加

建议放在“运行与构建”之后或文末，措辞可按实际调整：

```markdown
## 相关项目

- [claude-skill-lab](https://github.com/fongzhizhi/claude-skill-lab)：Pinglet 的孵化仓库，Claude Code 扩展能力（skill / agent / command / hook / workflow）的个人研发工作台。
```

### 5.2 lab 的 `apps/README.md`

在表格前加一段说明：

```markdown
> Pinglet 已迁移至 [fongzhizhi/pinglet](https://github.com/fongzhizhi/pinglet) 独立维护；本目录保留自研应用的约定与索引，不再存放 Pinglet 源码。
```

表格行改为：

```markdown
| [Pinglet](https://github.com/fongzhizhi/pinglet) | v1.3 可用 | 常驻置顶、可切换拖动/固定的小窗……已迁移至独立仓库维护。 |
```

### 5.3 lab 根 `README.md`

独立应用表改为：

```markdown
| **[Pinglet](https://github.com/fongzhizhi/pinglet)** | v1.3 可用 | 置顶常驻小窗，聚合所有 Claude Code 会话状态；已迁移至独立仓库维护。 |
```

### 5.4 GitHub 仓库信息（可选）

- Pinglet 的 About 描述：`轻量常驻的 Claude Code 会话状态看板（Windows / Tauri）`
- Pinglet 的 Topics：`claude-code`、`tauri`、`windows`、`dashboard`、`session-monitor`
- `claude-skill-lab` 的 About 或 README 首屏可补一行指向 Pinglet，方便从 lab 进入应用仓库。

### 5.5 链接检查

- [ ] 新仓库 README 中的 lab 链接可打开。
- [ ] lab 根 README 与 `apps/README.md` 中的 Pinglet 链接可打开。
- [ ] 新仓库 `docs/plans/` 内部链接使用相对路径。
- [ ] lab 中不再有指向 `apps/pinglet/` 的相对链接。
- [ ] 新仓库 README 中不再有 `../../project-docs/` 路径。

## 六、分享前准备

- [ ] 在干净 Windows 账户或机器上安装测试：空会话、无 VSCode、无 Claude 数据时的表现。
- [ ] 准备 README 截图或短动图（主面板、设置面板、悬停提示）。
- [ ] Release Notes 说明数据来源与隐私边界：只读 `~/.claude/sessions/*.json` 白名单字段与 transcript 标题记录，不读消息正文、不联网、不调用 Claude API。
- [ ] 说明未签名安装包的 SmartScreen 提示：“更多信息 → 仍要运行”。
- [ ] 说明系统要求：Windows 10/11、WebView2 Runtime。
- [ ] 可选：issue 模板、贡献说明、更新日志。

## 七、验收清单

- [ ] 新仓库可执行 `npm install` 与 `npm run tauri dev`。
- [ ] 新仓库可执行 `npm run tauri build -- --bundles nsis` 生成安装包。
- [ ] Release `v1.3.0` 已上传安装包与 SHA256。
- [ ] lab 执行 `git ls-files apps/pinglet` 无输出。
- [ ] lab 的 `git status` 干净，所有 Pinglet 链接指向新仓库。
- [ ] `claude-lab list` / `status` / `deploy --all` 不受影响（`apps/` 本就不参与 lab 部署）。
- [ ] 两个仓库互相链接，且新仓库不依赖 lab 内部的相对路径。

## 八、后续（不在本次范围）

- GitHub Actions：Windows + Rust + Node 构建 NSIS，tag 触发 Release。
- `bundle` 元数据：`publisher`、`copyright`、`shortDescription`、`category`。
- 代码签名，消除 SmartScreen 警告。
- Tauri 自动更新。
- 版本策略：`v1.3.x` 修补，`v1.4+` 功能。
- 公开后的 issue 模板、贡献指南与隐私说明。

## 九、风险与决策

| 风险 / 决策 | 建议 |
| --- | --- |
| 新仓库名被占用 | 优先 `pinglet`；不可用则 `pinglet-app` |
| 历史丢失 | 当前仅一次提交，直接导出可接受；需要时用 `git subtree split` |
| 两个仓库双份维护 | 迁移后 lab 只保留外链，不保留源码 |
| 文档链接失效 | 迁移后全局搜索 `apps/pinglet` 与 `../../project-docs` |
| 安装包未签名 | 在 Release Notes 说明 SmartScreen 绕过方式 |
| 新仓库过早公开 | 先 private 验证构建与安装，再转 public |
| GitHub 用户名 / 组织 | 按实际账号替换 `fongzhizhi`；若放组织下，调整所有 URL |
