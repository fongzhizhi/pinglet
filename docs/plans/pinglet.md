# pinglet：Claude 会话状态看板实现计划

> 名称 **pinglet** 取自网络与游戏中的延迟指标：它表示连接是否通畅、对方是否在响应，正好对应看板要展示的会话状态（等待 / 进行中 / 已完成）。同时 pinglet 作动词是「呼叫、提醒」，对应看板「重点提示待处理会话」的核心职责。统一使用小写 `pinglet`（目录、包名、窗口标题、文档一致），避免与常见产品写法产生歧义。
>
> 目标：在 Windows 上做一个置顶、常驻的轻量看板，展示所有 Claude Code 会话的名称、目录和状态，重点提示“等待处理”的会话，避免多会话并行时卡在无人处理的状态。
>
> 本文档已按本机实测数据校准：v0.1 采用 **Tauri 2 + 原生 TypeScript 前端**；如果 Rust / MSVC 工具链成为阻塞，退回 PowerShell + WPF 做最小验证。布局和颜色是首版默认值，后期按实际使用调整。
>
> **设计定稿**：UI 与功能以 [pinglet-v1.2.md](pinglet-v1.2.md) 为准（四态、暗/亮主题、设置面板、托盘 / 快捷键 / 边缘吸附）。本文保留背景、目标与实现计划，与 v1.2 冲突的部分以 v1.2 为准。
>
> **实现状态（2026-11-07）**：v1.2 已落地本仓库根目录，`npm run tauri dev` 可运行；系统集成与 NSIS 打包待在真实桌面环境验收。

## 一、背景与目标

### 问题

多个 Claude Code 会话在后台并行时，某个会话可能停在权限询问、等待输入，但用户不知道，导致任务卡住。VSCode 插件本身有状态筛选，但不主动提醒，也不常驻可见。

典型场景：三个 VSCode 窗口分别打开 `pro-ui`、`pro-pcb`、`chameleon`，各有一个 Claude 会话。某个窗口弹出权限询问后，用户正在另一个窗口里看代码，几分钟后才发现第一个会话一直在等。

### 目标

| 目标           | 说明                                                   |
| -------------- | ------------------------------------------------------ |
| **常驻可见**   | 置顶窗口，贴在屏幕角落，不占任务栏和 Alt-Tab           |
| **状态聚合**   | 一个窗口看到所有活跃会话                               |
| **重点突出**   | 等待处理的会话排在最前、红色高亮                       |
| **低开销**     | 不调用 Claude API，只读本地 JSON，轮询开销可忽略       |
| **简单易用**   | 最终形态是一个双击即用的桌面小工具，不需要命令行和配置 |
| **灵活可扩展** | UI 改布局/颜色/主题要容易；后续可加托盘、通知、快捷键  |
| **可丢弃**     | 官方出 UI 后可随时退休，不投入过多沉没成本             |

### 非目标（v0.1 不做）

- 跨机器聚合
- 会话内容预览（读 transcript）
- 远程控制会话（批准 / 拒绝权限）
- 托盘、通知音、Windows 通知
- 历史记录、统计图表
- 开机自启、自动更新
- 修改 `~/.claude/settings.json` 或注册 hooks
- 引入 React / Svelte / 状态管理 / UI 组件库
- 引入 `tauri-plugin-shell` 等额外插件

### 设计原则

1. **只读**：只读会话状态文件，不写 Claude 的任何配置或会话数据。
2. **本地**：不联网，不调用 Claude API。
3. **不碰敏感文件**：忽略 `sessions/*.key`，不读 transcript。
4. **未知不崩**：字段缺失、JSON 解析失败、状态值不认识时，显示“未知”而不是报错退出。
5. **先能用，再好看**：布局、颜色、字体都是可替换的默认值，不追求第一版完美。
6. **非稳定接口隔离**：会话文件是 Claude Code 的内部实现，解析逻辑集中在 Rust 的一个函数里，版本变化时只改这一处。
7. **Tauri 复杂度最小化**：Rust 只暴露两个 command，前端只用原生 TS + CSS，不引入不必要的框架和插件。

## 二、状态来源（已实测）

### 会话状态文件

Windows 上的路径：

```
%USERPROFILE%\.claude\sessions\<pid>.json
```

每个活跃的交互式会话对应一个文件；同一目录下还有 `<pid>.<hash>.key` 等侧车文件，**只匹配 `*.json`，不读取 `.key`**。

实测样本（已脱敏）：

```json
{
  "pid": 25416,
  "sessionId": "b0cc8af4-...",
  "cwd": "d:\\MyWorking\\github\\claude-skill-lab",
  "startedAt": 1793879602413,
  "procStart": "134383531899773629",
  "version": "2.1.314",
  "kind": "interactive",
  "entrypoint": "claude-vscode",
  "name": "claude-skill-lab-a0",
  "nameSource": "derived",
  "updatedAt": 1793938443363,
  "status": "busy",
  "statusUpdatedAt": 1793938443363
}
```

看板实际使用的字段：

| 字段              | 用途                                   |
| ----------------- | -------------------------------------- |
| `pid`             | 进程存活检测                           |
| `procStart`       | 可选：PID 复用校验（v0.2）             |
| `cwd`             | 显示目录；点击行时用 `code <cwd>` 打开 |
| `name`            | 会话名（如 `claude-skill-lab-a0`）     |
| `kind`            | 过滤：默认只显示 `interactive`         |
| `entrypoint`      | 可选：显示 `VSCode` / `CLI` 标签       |
| `status`          | 状态判定的核心字段                     |
| `statusUpdatedAt` | 当前状态已持续多久（epoch 毫秒）       |
| `updatedAt`       | `statusUpdatedAt` 缺失时的兜底         |

### 状态枚举

从 Claude Code 2.1.311 的会话解析器确认，会话文件的 `status` 只有四种值（解析器枚举：`["busy","shell","idle","waiting"]`）：

```
busy / shell / idle / waiting
```

| 文件状态  | 含义                   | 看板处理                 |
| --------- | ---------------------- | ------------------------ |
| `waiting` | 等待用户输入或权限确认 | 待处理（最高优先级）     |
| `busy`    | 模型正在工作           | 进行中                   |
| `shell`   | 正在执行命令           | 进行中（按“在工作”处理） |
| `idle`    | 空闲 / 本轮结束        | 已完成                   |

> 注意：扩展 UI 的一个映射函数把 `shell` 归到了 idle 分支，但从用户视角看，执行命令时 Claude 仍在工作。v0.1 先把 `shell` 当作“进行中”；如果实测发现它长时间停留造成误报，再单独调整颜色或文案。

> **与 agent view / 后台会话 5 态的关系**：`claude agents` 等后台会话使用一套 5 态模型（working / blocked / done / failed / stopped），其中 `failed` 可能来自 API 断开、工具报错等，`stopped` 表示手动或异常停止。pinglet 的看板状态采用同一套 5 态命名，但数据仍来自 `sessions/*.json` 的 4 值 `status` + 进程存活情况；`failed` / `stopped` 是推断结果，不是文件里的原始值。若以后要看后台会话，需要单独接入 agent view 的数据源。

### 为什么不用其他来源

| 来源                                            | 结论                                                              |
| ----------------------------------------------- | ----------------------------------------------------------------- |
| `~/.claude/ide/*.lock`                          | 存在过期文件，且含 `authToken` 等敏感字段，不适合作为活跃会话依据 |
| `~/.claude/projects/**/*.jsonl` transcript      | 文件大、更新频繁，含会话内容；MVP 不需要                          |
| hooks（`Notification` / `Stop` / `SessionEnd`） | 需要改 `settings.json`，多一套事件链路；状态文件不够用时再考虑    |
| Claude API                                      | 需要网络和鉴权，完全没必要                                        |

## 三、状态定义与颜色

### 映射表

看板与官方 agent view 对齐，保留 5 种可见状态；`sessions/*.json` 的 `status` 只有 4 种，`failed` / `stopped` 由文件状态 + 进程存活情况推断：

| 看板状态   | 判定依据                                                                    | 优先级 | 默认颜色                 | 文案              |
| ---------- | --------------------------------------------------------------------------- | ------ | ------------------------ | ----------------- |
| **待处理** | `status = waiting` 或 `blocked`                                             | 1      | 红 `#E5484D`             | 等待 2m           |
| **异常**   | `status = failed/crashed`，或进程已退出且最后状态不属于 `idle/done/stopped` | 2      | 深红 `#C93C37`           | 异常 18m          |
| **已停止** | `status = stopped`，或进程已退出且最后状态是 `idle/done`                    | 3      | 黄 `#D4A72C`             | 停止 1h           |
| **进行中** | `status = busy/shell`，或 `working/starting/running/resuming/adopted`       | 4      | 绿 `#30A46C`             | 运行 5m / 命令 1m |
| **已完成** | `status = idle` 或 `done`                                                   | 5      | 灰 `#8B949E`             | 空闲 3m           |
| **未知**   | `status` 不在上述已知值，且进程仍存活                                       | 6      | 弱文字色（不可单独配置） | 未知              |

**为什么 `failed` / `stopped` 要推断**：正常退出会删除会话文件，`alive = false` 的残留记录只可能来自异常退出。最后状态是 `idle/done` 时归为“已停止”（会话已结束，文件未清理）；仍在 `busy/shell/waiting` 等状态时归为“异常”（工作中断，可能是 API 断开、工具报错或进程被杀）。`sessions/*.json` 不记录具体原因，因此无法进一步区分 API 断开与手动停止。

**排序**：待处理 > 异常 > 已停止 > 进行中 > 已完成 > 未知。同状态内，待处理 / 进行中按 `statusUpdatedAt` 升序（等得最久的在前），其余按最近更新在前。

### 颜色说明

颜色按交通灯习惯分配：**红色 = 待处理，深红 = 异常，黄色 = 已停止，绿色 = 进行中，灰色 = 已完成**。颜色不是稳定契约，前端用 CSS 变量集中定义：

```css
:root {
  --red: #e5484d;
  --deep-red: #c93c37;
  --yellow: #d4a72c;
  --green: #30a46c;
  --gray: #8b949e;
  --light-gray: #6e7681;
  --bg: rgba(30, 30, 30, 0.9);
  --text: #e6edf3;
}
```

后期想换成“绿色 = 结束”或亮色主题，只改这里。布局同理，都是 CSS，改起来比 WPF 快。

> 主题系统落地后，实际颜色由 `src/theme.ts` 在运行时写入 CSS 变量（`--text`、`--border`、`--muted`、`--subtle`、5 个 `--state-*` 等），上面是「深夜」内置主题的默认值；CSS 中的 `:root` 仅作首屏兜底。`未知` 状态固定使用 `--subtle`。

### 边界情况

- **PID 复用**：长时间运行时，旧 PID 可能被其他进程复用，导致残留记录被误判为存活。正常退出会删除文件，残留罕见；v0.2 用 `procStart` 与 `sysinfo` 的进程启动时间比对。
- **正常关闭后文件残留**：`alive = false` 的记录保留展示，最后状态是 `idle/done` 时归为“已停止”，其余归为“异常”；无法区分 API 断开、工具报错与手动停止。
- **`waiting` 是否覆盖所有询问**：需要实测权限弹窗和 `AskUserQuestion` 是否都写入 `waiting`；如果漏报，再考虑 hooks。
- **`idle` 的语义**：可能是“本轮结束”，也可能是“会话空闲”；看板统一显示“已完成/空闲”，不区分。
- **多会话同目录**：`name` 的后缀（如 `-a0`）用于区分；如果 `name` 缺失，回退显示 `cwd` 的目录名。
- **无会话**：显示“暂无活跃会话”，不报错。

## 四、技术选型

### 推荐：Tauri 2 + 原生 TypeScript

| 项       | 选择                                                                 |
| -------- | -------------------------------------------------------------------- |
| 桌面框架 | Tauri 2                                                              |
| 前端     | 原生 TypeScript + Vite（不引入 React/Svelte）                        |
| 样式     | 原生 CSS + CSS 变量                                                  |
| 后端     | Rust，只暴露两个 command                                             |
| 状态读取 | Rust 读取 `%USERPROFILE%\.claude\sessions\*.json`                    |
| 进程检测 | `sysinfo` crate                                                      |
| 定时刷新 | 前端 `setInterval` 2 秒 + `invoke`                                   |
| 打包     | Tauri 的 MSI / NSIS 安装包                                           |
| 窗口形态 | `alwaysOnTop` / `decorations: false` / `transparent` / `skipTaskbar` |

理由：

- **最终用户体验更好**：打包后是正常桌面应用，双击即用；后续加托盘、通知、快捷键、主题都比 WPF 容易。
- **UI 灵活**：HTML/CSS 改布局、颜色、动画最快；你熟悉前端，迭代成本低。
- **跨平台潜力**：现在只做 Windows，但未来想跑 macOS/Linux 不用重写。
- **体积可控**：Tauri 应用二进制小，WebView2 是系统共享运行时，不需要像 Electron 那样打包整个 Chromium。
- **AI 写代码的成本低**：Rust 后端只需要两个 command，复杂度远低于完整的 Tauri 应用。

### 本机环境检查

| 项                             | 状态                                                                     |
| ------------------------------ | ------------------------------------------------------------------------ |
| Node.js                        | v22.13.1 ✅                                                              |
| npm                            | 10.9.2 ✅                                                                |
| WebView2 Runtime               | 143.0.3650.139 ✅                                                        |
| Rust / cargo                   | 1.99.1 ✅                                                                |
| MSVC Build Tools + Windows SDK | VS Build Tools 17.14.44 / MSVC 14.44.35207 / Windows SDK 10.0.26100.0 ✅ |

结论：工具链已就绪，`npm run tauri dev` 可运行；首次编译约 1 分钟，后续增量编译约 4~10 秒。

### 为什么不再首选 WPF

WPF 单文件方案仍然有价值：

- 零安装、零构建，改完直接跑；
- 单文件，维护和排查最简单；
- 不依赖 Rust/MSVC。

但按你现在更看重的三项——**易用性、灵活性、软件是否简单易用**——Tauri 2 更合适：

- 最终用户拿到的是正常安装包，不是 `.ps1` + `.cmd`；
- UI 用 CSS 迭代，布局/颜色/主题调整快；
- 托盘、通知、窗口状态、快捷键都有现成插件；
- 长期看更像一个“产品”，而不是一次性脚本。

因此：**Tauri 2 为主，WPF 作为工具链受阻时的 fallback**。两者的数据层设计（读 JSON、PID 检测、状态映射）完全一致，迁移成本低。

### 方案对比

| 维度         | Tauri 2                                  | PowerShell + WPF                     |
| ------------ | ---------------------------------------- | ------------------------------------ |
| 最终用户体验 | 打包 exe，托盘/通知/主题/动画容易        | `.cmd`/快捷方式；托盘/通知需额外代码 |
| UI 灵活性    | Web/CSS，改布局最快                      | XAML/PowerShell，样式迭代笨重        |
| 维护简单     | 项目结构多，Rust/MSVC/构建链             | 单文件，零依赖                       |
| 首次环境     | 需 Rust + MSVC + Windows SDK             | 系统自带                             |
| 运行依赖     | WebView2（本机已有）                     | .NET/WPF（系统自带）                 |
| 跨平台       | 可以                                     | 仅 Windows                           |
| 长期扩展     | 插件生态、tray、updater、global shortcut | 需自己写/引 WinForms                 |
| 调试         | Rust 编译/IPC/ACL 有学习成本             | 直接改直接跑                         |

### Tauri 复杂度最小化

为了避免“为了简单反而复杂”，v0.1 做以下约束：

- Rust 只做两件事：
  - `list_sessions`：读目录、解析 JSON、PID 存活检测；
  - `open_in_vscode`：调用 `cmd /C code <cwd>`。
- 不用 `tauri-plugin-shell`；用 Rust 标准库 `std::process::Command`，减少 ACL/权限配置面。
- 不用前端框架、路由、状态管理；一个 `main.ts` + 一个 `styles.css`。
- 不用数据库；窗口位置 v0.2 用一个小 JSON 或 `tauri-plugin-window-state`。
- 不引入 updater、telemetry、i18n；这些以后再说。
- 只加必要 crate：`serde`、`serde_json`、`sysinfo`；可选 `dirs`。

这样 Tauri 的复杂度主要集中在“首次安装工具链”和“窗口配置”，代码本身很少。

### 项目结构

```
仓库根目录
  package.json
  index.html
  src/
    main.ts
    styles.css
  src-tauri/
    Cargo.toml
    tauri.conf.json
    src/
      main.rs
      lib.rs
    capabilities/
      default.json
```

### 开发命令

```bash
npm install
npm run tauri dev      # 开发，前端热更新
npm run tauri build    # 打包 MSI / NSIS
```

前置条件：Rust（rustup）、Visual Studio Build Tools（C++ 工作负载 + Windows SDK）、WebView2 Runtime。可以用 `npm create tauri-app@latest` 生成骨架，也可以手写最小项目。

## 五、架构与采集逻辑

### 架构

```
┌─────────────────────────────────────────────┐
│ Tauri 2 窗口（WebView2 + 原生 TS 前端）       │
│  ┌───────────────────────────────────────┐  │
│  │ 列表：状态点 + 会话名 + 目录 + 时长     │  │
│  │ 2 秒 setInterval → invoke(list_sessions)│  │
│  │ 点击行 → invoke(open_in_vscode)         │  │
│  └───────────────────────────────────────┘  │
└──────────────────┬──────────────────────────┘
                   │ Tauri IPC
┌──────────────────▼──────────────────────────┐
│ Rust 后端                                    │
│  - list_sessions：读 JSON + sysinfo 查 PID   │
│  - open_in_vscode：cmd /C code <cwd>        │
└──────────────────┬──────────────────────────┘
                   │ 只读
┌──────────────────▼──────────────────────────┐
│ %USERPROFILE%\.claude\sessions\*.json       │
│ 每个活跃会话一个文件；忽略 *.key              │
└─────────────────────────────────────────────┘
```

没有服务端、没有 sidecar、没有数据库、没有网络请求。

### Rust 数据模型

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionRow {
    pid: Option<u32>,
    name: String,
    cwd: Option<String>,
    status: String,
    status_updated_at: Option<i64>,
    alive: Option<bool>,
    entrypoint: Option<String>,
}

#[derive(serde::Serialize)]
struct ListResult {
    rows: Vec<SessionRow>,
    errors: Vec<String>,
}
```

### `list_sessions` 逻辑

1. 读取 `%USERPROFILE%\.claude\sessions` 目录；目录不存在时返回空列表。
2. 只处理 `*.json`，跳过 `.key` 和其他文件。
3. 逐个解析 JSON；字段用 `Option` 兼容缺失。
4. `kind` 存在且不等于 `interactive` 时跳过；`kind` 缺失时保留，按 `status` 正常映射。
5. 用 `sysinfo` 查询 `pid` 是否存在；查询失败时返回 `None`，不把整批会话误判为死亡。
6. 组装 `SessionRow`；单个文件失败时记录到 `errors`，不影响其他会话。
7. 返回 `ListResult`；前端按“最后状态 + `alive`”把残留记录归为“异常/已停止”，不做过滤。

### `open_in_vscode`

```rust
#[tauri::command]
fn open_in_vscode(cwd: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "code", &cwd])
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
```

说明：

- Windows 下 `code` 通常是 `code.cmd`，直接 `Command::new("code")` 可能找不到；用 `cmd /C` 最稳。
- 如果 `code` 不在 PATH，后续可以按常见安装路径探测，或在设置里配置 `code` 路径。
- 如果不需要“打开 VSCode”，而是想打开终端/文件夹，也可以用 `explorer`，但 v0.1 先做 `code`。

### 前端轮询

```ts
import { invoke } from "@tauri-apps/api/core";

type SessionRow = {
  pid: number | null;
  name: string;
  cwd: string | null;
  status: string;
  statusUpdatedAt: number | null;
  alive: boolean | null;
  entrypoint: string | null;
};

type ListResult = {
  rows: SessionRow[];
  errors: string[];
};

async function refresh() {
  const result = await invoke<ListResult>("list_sessions");
  render(result);
}

setInterval(refresh, 2000);
refresh();
```

状态映射、死进程归类、排序、颜色都在前端做；Rust 只返回原始数据，保持后端简单。

### 容错原则

- 单个 JSON 解析失败 → 记录到 `errors`，该会话不显示或显示“未知”，其他行正常。
- `status` 不认识 → 显示“未知 + 原始值”，不崩溃。
- `statusUpdatedAt` 缺失 → 回退 `updatedAt`，再回退文件修改时间。
- `name` 缺失 → 回退 `cwd` 的目录名。
- `cwd` 缺失 → 显示“未知目录”，点击跳转禁用。
- `sessions` 目录不存在 → 显示“未检测到会话”，不报错。
- `sysinfo` 查询失败 → `alive = null`，按 `status` 正常映射，不武断归为“异常/已停止”。

## 六、UI 设计（v0.1 默认）

> 以下布局、颜色、字号都是首版默认值，不是稳定契约；用起来不顺就改。Tauri 用 CSS，调整成本很低。

### 窗口形态

| 属性     | v0.1 设置                              |
| -------- | -------------------------------------- |
| 尺寸     | 约 320×400 px，固定                    |
| 位置     | 主屏右侧靠上，留 20px 边距             |
| 置顶     | `alwaysOnTop: true`                    |
| 任务栏   | `skipTaskbar: true`                    |
| 边框     | `decorations: false`                   |
| 透明     | `transparent: true` + CSS 半透明背景   |
| 关闭     | `getCurrentWindow().close()`，直接退出 |
| 拖动     | 标题栏加 `data-tauri-drag-region`      |
| 刷新     | 前端 `setInterval` 2 秒                |
| 记忆位置 | v0.2 再做                              |
| 托盘     | v0.3 再做                              |

`tauri.conf.json` 关键配置：

```json
{
  "app": {
    "windows": [
      {
        "title": "pinglet",
        "width": 320,
        "height": 400,
        "resizable": false,
        "alwaysOnTop": true,
        "decorations": false,
        "transparent": true,
        "skipTaskbar": true
      }
    ]
  }
}
```

CSS 关键点：

```css
html,
body {
  margin: 0;
  background: transparent;
}

.panel {
  background: var(--bg);
  border: 1px solid #30363d;
  border-radius: 12px;
  color: var(--text);
  font-family: "Microsoft YaHei UI", sans-serif;
}
```

### 列表项

每行两行文本：

```
🔴 待处理   claude-skill-lab-a0
   d:\MyWorking\github\claude-skill-lab        等待 2m

🟢 进行中   pro-pcb-a0
   d:\MyWorking\github\pro-pcb                  运行 5m

⚪ 已完成   chameleon-a0
   d:\MyWorking\github\chameleon                空闲 3m
```

字段：

- 第一行：状态色点 + 状态文字 + 会话名（`name`）。
- 第二行：目录（`cwd`，过长省略）+ 右侧时长。
- 可选：小标签显示 `VSCode` / `CLI`（来自 `entrypoint`），第一版可以不放。
- 悬停：Tooltip 显示完整路径、PID、最后更新时间。
- 行数超过窗口高度时加滚动区域；3~6 个会话通常不需要滚动。

### 颜色与字体

| 用途     | 默认值                                                       |
| -------- | ------------------------------------------------------------ |
| 窗口背景 | `rgba(30, 30, 30, 0.9)`                                      |
| 边框     | `#30363D`                                                    |
| 主文字   | `#E6EDF3`                                                    |
| 次要文字 | `#8B949E`                                                    |
| 待处理   | `#E5484D`                                                    |
| 进行中   | `#30A46C`                                                    |
| 已完成   | `#8B949E`                                                    |
| 未知     | 跟随主题弱文字色 `--subtle`                                  |
| 字体     | `Microsoft YaHei UI`，正文 12px，会话名 13px 加粗，目录 11px |

可选：待处理行左侧加 3px 红色竖条，或整行淡红底色；第一版先用色点 + 文字，够用。

### 交互

| 操作          | v0.1 行为                                    |
| ------------- | -------------------------------------------- |
| 左键点击行    | `invoke('open_in_vscode', { cwd })`          |
| 拖动标题/空白 | `data-tauri-drag-region`                     |
| 点击关闭      | `getCurrentWindow().close()`                 |
| 悬停          | 显示完整路径、PID、最后更新                  |
| 右键          | v0.1 不做；v0.2 加“隐藏已完成 / 刷新 / 退出” |
| 双击标题      | v0.2 可做“置顶开关”或“收起”                  |

### 空状态

没有活跃会话时，窗口中间显示“暂无活跃会话”，整体降低亮度。不要自动隐藏窗口，否则用户会以为程序没启动。

## 七、开发计划

### 阶段 0：环境与骨架（约 0.5~1 小时）

- [x] 确认会话文件路径：`%USERPROFILE%\.claude\sessions\<pid>.json`
- [x] 确认字段：`pid` / `cwd` / `name` / `status` / `statusUpdatedAt` / `kind` / `entrypoint`
- [x] 确认状态枚举：`busy` / `shell` / `idle` / `waiting`
- [x] 确认 `code <cwd>` 可用
- [x] 确认 Node.js / npm / WebView2 已就绪
- [x] 安装 Rust（rustup）与 Visual Studio Build Tools（C++ 工作负载 + Windows SDK）
- [x] 用 `npm create tauri-app@latest` 生成最小项目（vanilla TS），确认 `npm run tauri dev` 能打开空白窗口

### 阶段 1：技术验证 Spike（约 0.5~1 小时）

**目标**：先验证 Tauri 2 能做出这个窗口，不急着做完整功能。

- [x] 配置透明、无边框、置顶、跳过任务栏的窗口
- [x] 实现 `list_sessions`，返回真实会话数据
- [x] 前端每 2 秒 `invoke` 并渲染会话名/目录/状态
- [x] 点击行调用 `open_in_vscode`
- [x] 用 2 个真实会话验证（`waiting` 红色置顶、`idle` 灰色已完成）

**决策点**：如果 Rust/MSVC 安装或透明置顶窗口在 1~2 小时内无法跑通，退回 PowerShell + WPF 做 v0.1；数据层和状态设计不变，后续仍可迁回 Tauri。

### 阶段 2：MVP（约 0.5~1 天）

- [x] 完整状态映射、排序、颜色、时长
- [x] `未知` 兜底与死进程残留归类（异常/已停止）
- [x] 空状态、解析失败容错（错误条不影响其他会话）
- [x] 窗口拖动、位置/尺寸（主屏右上角，实测可拖动）
- [ ] 关闭按钮实测（实现与 `core:window:allow-close` 权限已就位）
- [x] Tooltip、最后刷新时间
- [ ] 打包 MSI / NSIS，安装后能运行

**验收标准**：

1. 打开 2~3 个 VSCode + Claude 会话，看板在 2 秒内列出全部会话。
2. 某个会话进入 `waiting`，该行在 2 秒内变红并排到最上面。
3. 某个会话 `busy`，显示绿色“进行中”。
4. 手动结束一个会话进程，看板在 2 秒内隐藏该行（残留状态文件不显示为独立状态）。
5. 点击某行，对应的 VSCode 窗口被打开/聚焦。
6. 窗口始终置顶，不占任务栏，不干扰正常输入。
7. 故意放一个坏 JSON 到 `sessions/`，看板显示错误提示，其他会话不受影响。
8. 空闲时 CPU 占用接近 0，内存可接受。

### 阶段 3：打磨（约 0.5~1 天）

- [ ] 记住窗口位置和大小（`tauri-plugin-window-state` 或自定义 JSON）
- [ ] `procStart` 校验，避免 PID 复用误判
- [ ] “隐藏已完成”开关
- [x] 主题系统：4 套内置主题 + 自定义主题（背景、不透明度、文字、边框、状态文案/颜色），localStorage 持久化
- [ ] Tooltip：完整路径、PID、最后更新时间、原始 status
- [ ] `busy`/`shell` 超过 30 分钟未变化时提示“可能卡住”
- [ ] 圆角、阴影、字体微调
- [ ] 异常日志写到本地文件，方便排查

### 阶段 4：可选增强

- [ ] 托盘图标，关闭窗口不退出
- [ ] `waiting` 时弹 Windows 通知或提示音
- [ ] 行闪烁 / 任务栏闪烁
- [ ] 全局快捷键显示/隐藏
- [ ] 配置文件：轮询间隔、颜色、过滤规则
- [ ] 开机自启
- [ ] hooks 增强（仅在状态文件不够准时）
- [ ] 自动更新 / 代码签名（如果要分发）

## 八、风险与应对

| 风险                                      | 影响                  | 应对                                                                         |
| ----------------------------------------- | --------------------- | ---------------------------------------------------------------------------- |
| Rust / MSVC 工具链安装失败                | Tauri 无法启动        | 一次性成本；卡住则退回 WPF 做 v0.1                                           |
| Tauri 2 的 capabilities / ACL 配置        | 自定义 command 调不通 | 只暴露两个自定义 command，不引入 shell 插件；按最小权限配置                  |
| WebView2 在别的机器缺失                   | 应用无法启动          | 本机已有；分发时用 Tauri 的 WebView2 bootstrapper 或要求装 Evergreen Runtime |
| 透明 + 置顶窗口在 Windows 10 上有兼容问题 | 视觉异常              | 先做 spike；不行就退化为不透明圆角面板，功能不受影响                         |
| 首次 Rust 编译慢                          | 开发迭代慢            | 用 `tauri dev`，前端热更新；Rust 代码改动少                                  |
| 会话文件结构随 Claude Code 版本变化       | 解析失败              | 解析集中在 `list_sessions`，未知状态显示“未知”，不崩溃                       |
| `waiting` 未覆盖所有询问类型              | 漏报                  | 实测权限弹窗 / `AskUserQuestion`；不够用再加 hooks                           |
| `shell` 语义不明确                        | 误显示为进行中        | 先按“进行中”，实测长时间停留再调整映射                                       |
| PID 复用                                  | 残留记录被误判为存活  | v0.2 用 `procStart` 与 `sysinfo` 的启动时间比对                              |
| 正常关闭后状态文件残留                    | 旧会话重新出现在看板  | `alive = false` 一律隐藏，不区分最后状态                                     |
| `code` 不在 PATH                          | 点击跳转失败          | 按常见安装路径探测，或提供 `code` 路径配置                                   |
| 多显示器 / DPI 缩放                       | 窗口位置异常          | v0.1 固定在主屏；v0.2 记住位置并处理 DPI                                     |
| 置顶窗口遮挡内容                          | 体验差                | 小尺寸 + 半透明；后续加收起/全局快捷键                                       |
| 未签名安装包触发 SmartScreen              | 安装提示              | 个人使用可忽略；分发时再考虑签名                                             |
| 官方推出会话面板                          | 工具失去价值          | 保持项目小、可随时删除，不投入过多                                           |

## 九、迭代路径

```
v0.1  Tauri 2 只读看板：置顶透明窗口 + 2 秒刷新 + 点击跳转
  ↓
v0.2  位置记忆、PID 复用校验、卡住提示、隐藏已完成
  ↓
v0.3  托盘、通知、主题、全局快捷键、配置
  ↓
v0.4  hooks 增强（仅在状态文件不够用时）
  ↓
v1.0  可选：自动更新、代码签名、跨平台
```

**Fallback**：如果 Tauri 工具链或透明窗口在 spike 阶段受阻，v0.1 改用 PowerShell + WPF；状态映射、数据模型、验收标准不变，后续仍可迁回 Tauri。

## 十、待确认事项

| 项                                             | 状态                                                                       |
| ---------------------------------------------- | -------------------------------------------------------------------------- |
| Rust + MSVC Build Tools 安装是否可接受         | 已完成（Rust 1.99.1 / VS Build Tools 17.14.44 / Windows SDK 10.0.26100.0） |
| Tauri 2 具体版本 / CLI 版本                    | tauri 2.12.5；`@tauri-apps/api` / `cli` 2.x                                |
| 前端是否坚持原生 TS                            | 是；已拆为 `main.ts` + `session-state.ts`                                  |
| 打包格式 MSI 还是 NSIS                         | 配置为 NSIS，尚未打包验证                                                  |
| `waiting` 是否覆盖权限确认和 `AskUserQuestion` | 已实测到 1 个 `waiting` 会话并红色置顶；具体触发来源未细分                 |
| `idle` 是“本轮结束”还是“会话空闲”              | 已实测：进程存活 + `idle` → 已完成为灰色                                   |
| `shell` 的持续时间和转换时机                   | 未实测；先按“进行中”                                                       |
| 正常关闭后 `sessions/*.json` 是否删除          | 未实测；`alive = false` 的记录保留并归为“异常/已停止”                      |
| 是否显示 CLI 会话（`entrypoint=claude`）       | 默认显示所有 `kind=interactive`，用标签区分                                |
| 轮询间隔 2 秒是否合适                          | 默认 2 秒，实测刷新正常                                                    |
| 是否第一版就要托盘/通知                        | 建议 v0.3                                                                  |
| 是否纳入 `claude-lab` 部署体系                 | 不纳入；Pinglet 始终为 lab 体系之外的独立工具，现已迁至独立仓库          |

## 十一、实现记录（2026-11-07）

- 代码位置：本仓库根目录；运行 `npm run tauri dev`，构建 `npm run tauri build`（NSIS，未验证）。
- 已完成：透明无边框置顶窗口、主屏右上角定位、跳过任务栏、2 秒轮询、状态映射与排序、错误条、tooltip、行点击调用 `open_in_vscode`、拖动、关闭按钮实现。
- 已实测：真实会话状态（`waiting` 待处理红色置顶 / `busy` 进行中绿色 / `idle` 已完成灰色）、2 秒刷新、窗口拖动、主屏右上角定位、`code --version` 可用。
- 未实测：点击行实际打开 VSCode、关闭按钮、真实进程退出后的“异常/已停止”归类、NSIS 打包。
- 与计划的差异：
  1. `SessionRow.alive` 使用 `Option<bool>`：sysinfo 初始化失败时返回 `null`，前端保留该行并按 `status` 正常映射，避免把整批会话误判为死亡。
  2. 前端拆为 `main.ts`（DOM/轮询/交互）与 `session-state.ts`（纯状态逻辑），避免单文件超过 300 行。
  3. 未引入 `tauri-plugin-opener`；`open_in_vscode` 用 `std::process::Command` 调用 `cmd /C code <cwd>`，减少插件与 ACL 配置面。
  4. 增加浏览器预览层：非 Tauri 环境（`npm run dev` + 浏览器）用 `src/mock-sessions.ts` 的模拟数据覆盖 5 种可见状态、未知兜底与死进程残留（残留按最后状态归为异常/已停止），按 320×400 居中显示并标注“预览”；仅 Tauri 可用的关闭/打开 VSCode 以提示代替。UI 在浏览器迭代，窗口级效果回 Tauri 验收。
  5. 主题系统拆为 `src/theme.ts`（主题模型、内置主题、localStorage 持久化、CSS 变量应用与变更订阅）与 `src/theme-panel.ts`（设置面板、实时预览）。内置「深夜 / 浅色 / 北境 / 透明」4 套；内置主题只读，可复制为自定义后编辑名称、背景色、不透明度、主/次/弱文字、边框，以及 5 种可见状态的文字与颜色。主题只写前端 localStorage（`pinglet.themes.v1` / `pinglet.active-theme.v1`），不增加 Rust command，不触碰 Claude 配置。
  6. 状态模型与官方 agent view 对齐：保留 `working` / `blocked` / `done` / `failed` / `stopped` 五种可见状态 + “未知”兜底。`sessions/*.json` 只有 4 值 `status`，`failed` / `stopped` 由“最后状态 + 进程存活”推断：死进程 + `idle/done` → 已停止，死进程 + 其他最后状态 → 异常。
  7. 旧版三态自定义主题读取时自动迁移：`waiting` → `blocked`、`running` → `working`，缺失的 `failed` / `stopped` 用默认值补齐，避免升级后主题丢失。
- 主题与状态已通过浏览器交互回归：主题复制/重命名/改色/改文案/切换内置主题/刷新持久化/删除回退，以及 5 状态归并排序、死进程残留归类、未知状态兜底、主题页 5 个状态编辑项均正常；`?theme=1` 可在开发环境直接打开主题页。
- 后续：关闭按钮/行点击实测、NSIS 打包（等 UI 与功能稳定）、`procStart` 校验、窗口位置记忆、托盘/通知等阶段 3/4 项。

---

_本文档根据本机实测数据更新；技术选型为 Tauri 2 优先、PowerShell + WPF 兜底。布局和颜色的默认值可在 CSS 中随时调整。_
