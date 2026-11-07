# Pinglet 会话名称显示为内部 slug 的问题说明

> 实现状态（2026-11-07）：已按方案 A 的收窄版落地——只读 transcript 头尾的 `custom-title` / `ai-title` 标题记录，不读消息正文、不读首条用户消息；无标题时 derived 会话回退目录名。实现细节见文末第 6 节。
>
> 反馈对象：Pinglet 开发
> 环境：Windows 11 / Claude Code 2.1.318 / 模型端点 DeepSeek Anthropic 兼容 API / entrypoint `claude-vscode`
> 关联代码：`src-tauri/src/lib.rs`（`list_sessions`）、`src/session-state.ts`

## 1. 现象

Claude Code 会话内可见的标题（或首条消息兜底）是「这是一个测试」，但 Pinglet 看板该会话显示为 `claude-skill-lab-df`。

- `claude-skill-lab-df` = 目录名 + 2 位随机后缀，是 Claude Code 启动时生成**内部会话名**，不是用户标题。
- 同目录开两个会话会显示为 `claude-skill-lab-df` / `claude-skill-lab-64`，只能靠随机后缀区分，可读性差。

## 2. 根因

Pinglet 与 Claude Code 的**标题数据源不同**：

| 来源             | 位置                                                         | 内容                                                                                         |
| ---------------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| Pinglet 当前读取 | `~/.claude/sessions/<pid>.json` 的 `name` 字段               | 内部会话名。`nameSource: "derived"` 时是目录名 slug；`nameSource: "hook"` 时才是用户可见标题 |
| Claude Code 标题 | `~/.claude/projects/<项目slug>/<sessionId>.jsonl` 的标题记录 | `{"type":"custom-title","customTitle":"…"}` 与 `{"type":"ai-title","aiTitle":"…"}`           |

`list_sessions()` 在 `lib.rs:180-184` 直接取 `session.name`，因此只要 `nameSource` 还是 `derived`，就只能读到 slug。

### 关键机制：`name` 何时才会变成标题

Claude Code 的 `UserPromptSubmit` hook 支持返回 `sessionTitle` 字段。hook 返回该字段后，Claude Code 会把 `sessions/<pid>.json` 的 `name` 改写为标题，并把 `nameSource` 置为 `"hook"`。

也就是说：

1. 新会话启动 → `name` = `derived` slug，Pinglet 只能看到 slug。
2. 直到某个 hook（如 session-keeper）通过 `sessionTitle` 回填，`name` 才变成标题。
3. 回填发生在**下一轮用户提交**时，首轮对话期间 Pinglet 始终显示 slug。

本机实测两个会话的状态文件：

```json
// 20980.json —— 本会话，从未回填
{
  "pid": 20980,
  "sessionId": "9adeca97-5414-480b-89ca-153f956e4daa",
  "cwd": "d:\\MyWorking\\repository\\github\\claude-skill-lab",
  "name": "claude-skill-lab-df",
  "nameSource": "derived",
  "status": "busy"
}

// 30088.json —— 曾通过 hook 回填
{
  "pid": 30088,
  "sessionId": "ea2f8934-55f4-4c30-90a8-e3e6ee4b4731",
  "cwd": "C:\\Users\\Administrator\\.claude",
  "name": "测试 session-keeper 标题更新",
  "nameSource": "hook",
  "status": "idle"
}
```

对本会话还额外确认过：transcript 中 `custom-title` 记录 0 条、`ai-title` 记录 0 条（DeepSeek 兼容端点不生成 `ai-title`），会话内显示的「这是一个测试」实际是**首条用户消息**的兜底展示，并非持久化标题。所以本会话即使 Pinglet 去读 transcript，也只能靠首条消息兜底。

## 3. 建议修复

### 方案 A（推荐）：`list_sessions` 在 `nameSource != "hook"` 时读取 transcript 标题

#### 3.1 扩展反序列化结构

`RawSession` 增加两个字段（serde 的 `rename_all = "camelCase"` 已能自动映射）：

```rust
struct RawSession {
    // ...现有字段
    session_id: Option<String>,
    name_source: Option<String>,
}
```

#### 3.2 标题解析优先级

```
1. nameSource == "hook"  → 直接使用 session.name（无需读文件）
2. transcript 的 custom-title（取最后一条）
3. transcript 的 ai-title（取最后一条）
4. session.name（derived slug 兜底）
5. cwd 的目录名（现有 directory_name 逻辑）
```

第 4 步是否保留由产品决定：slug 比目录名更不可读，建议 derived 情况下跳过 slug 直接用目录名，或前端把两者区分展示。

#### 3.3 定位 transcript

sessionId 全局唯一，**不要硬编码 `projects/<项目slug>` 的转义规则**（Windows 盘符、反斜杠、非 ASCII 字符的替换规则属于内部实现，可能变化）。直接在 `~/.claude/projects/*/` 下查找 `<session_id>.jsonl` 即可，目录数量通常很少。

```rust
fn find_transcript(session_id: &str) -> Option<PathBuf> {
    let projects = home_dir()?.join(".claude").join("projects");
    for entry in fs::read_dir(projects).ok()?.flatten() {
        let candidate = entry.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
```

#### 3.4 只读文件头尾，不要整文件加载

transcript 可达几十 MB，且前端每 2 秒刷新一次（`main.ts:43` 的 `REFRESH_INTERVAL_MS = 2000`），必须避免全量读取与重复解析。

- `ai-title` 在文件开头，`custom-title` 在文件末尾（追加写入）。
- 参考 `suites/session-keeper/dist/hooks/session-keeper.js` 中 `readTitleQuick()` 的实现：
  - 头部窗口从 64 KB 起步，未找到则倍增，上限 4 MB；
  - 尾部窗口从 128 KB 起步，未找到则倍增，上限 8 MB；
  - 逐行匹配 `"type":"custom-title"` / `"type":"ai-title"`，JSON 解析失败的行（窗口截断或正在写入的半行）直接忽略。
- 注意 BOM / UTF-8 边界：按字节窗口读取后转字符串时，窗口边缘可能截断多字节字符，用 `String::from_utf8_lossy` 或按行边界处理。

#### 3.5 缓存

建议在 Rust 侧维护缓存：

```rust
HashMap<String, (SystemTime, String)>  // sessionId -> (transcript mtime, title)
```

transcript 的 mtime 未变化时复用上次结果。这样 2 秒轮询不会反复产生 IO；只有会话有新消息导致 mtime 变化时才重新读头尾。

#### 3.6 前端展示

`session-state.ts` 的 `buildTooltip()`（`session-state.ts:245`）目前把 `row.name` 作为 tooltip 标题。若后端新增了 `title` 字段，建议：

- 列表主行显示 `title`；
- tooltip 同时展示 `name`（内部名）与 `sessionId`，便于排查；
- `nameSource == "derived"` 时可在 UI 上标记「未命名」或在 tooltip 显示 slug，避免用户误以为标题丢了。

### 方案 B（最小改动）

不改 Rust，只在前端把 derived slug 替换为目录名展示（`session-state.ts` 已有 `directoryName()`）。改动小，但无法显示用户实际设置的标题，不推荐作为最终方案。

### 方案 C（不推荐）

让 Pinglet 依赖 session-keeper 每轮回填 `sessionTitle`。不可行：Pinglet 不应假设用户安装了 session-keeper，且回填本身有一轮延迟。

## 4. 验收清单

1. **新会话（无任何标题）**：Pinglet 显示首条用户消息摘要或目录名，不再显示 `xxx-ab` 形式的 slug。
2. **`session-keeper set "测试标题"` 之后**：下一轮提交前，Pinglet 已能从 transcript 读到 `custom-title` 并显示「测试标题」（无需等 `nameSource` 变 `hook`）。
3. **下一轮提交后**：`sessions/<pid>.json` 的 `nameSource` 变为 `hook`，Pinglet 显示一致。
4. **多窗口同目录**：各会话显示各自标题，不再依赖随机后缀区分。
5. **性能**：50 MB 以上 transcript + 2 秒轮询，Pinglet 的 CPU/IO 无明显上升（缓存生效）。
6. **异常文件**：transcript 正在写入（最后一行不完整）、文件已删除、JSON 损坏时，Pinglet 不崩溃、不报错，回退到 `name`/目录名。

## 5. 附：本机复现命令

```bash
# 查看会话内部名与 nameSource
cat ~/.claude/sessions/<pid>.json

# 查看 transcript 标题记录（0 条即表示从未设置过标题）
grep -c '"type":"custom-title"' ~/.claude/projects/*/<sessionId>.jsonl
grep -c '"type":"ai-title"' ~/.claude/projects/*/<sessionId>.jsonl

# 手动设置标题（会写 custom-title + pending，下一轮 hook 回填 name）
node ~/.claude/hooks/session-keeper.js set "测试标题"
```

## 6. 实现说明（2026-11-07）

采用方案 A 的收窄版，实际改动：

- `RawSession` 增加 `sessionId` / `nameSource`；`SessionRow` 增加 `title` / `sessionId`。内部 `name` 保留，用于 `activate_session` 匹配终端窗口标题。
- 标题优先级：`nameSource = "hook"` 的 `name` > transcript 最后一条 `custom-title` > 最后一条 `ai-title` > `nameSource = "derived"` 时的目录名 > 内部名 / 「未命名会话」。
- 只按 `sessionId` 在 `~/.claude/projects/*/` 下查找 `<sessionId>.jsonl`，不硬编码项目 slug 转义规则；`sessionId` 先做文件名白名单校验，避免路径穿越。
- 只读头尾窗口：头部 64 KB 起步、倍增到 4 MB；尾部 128 KB 起步、倍增到 8 MB；逐行匹配 `"type":"custom-title"` / `"type":"ai-title"`，截断、损坏或半行 JSON 直接忽略。
- 缓存按文件大小 + mtime 失效；文件增长时只增量扫描上次 EOF 之后的新区间，2 秒轮询不会反复全量读取几十 MB 的 transcript。
- 文件删除、读取失败、JSON 损坏时静默回退到目录名 / 内部名，不崩溃、不向 UI 报错。
- 前端列表主行显示 `title`；tooltip 标题使用 `title`，标题与内部名不同时补充内部名，并展示 `sessionId`。
- 新增 8 个 Rust 单元测试，覆盖 custom-title 优先级、ai-title 回退、损坏行、增量追加、标题截断、derived 目录名回退、hook 跳过 transcript 与 sessionId 校验。

已知取舍：没有 `custom-title` / `ai-title` 的新会话回退显示目录名，同目录多个未命名会话标题相同，可用 tooltip 的内部名 / 会话 ID 区分；按方案 A 的约定，未实现首条用户消息兜底。
