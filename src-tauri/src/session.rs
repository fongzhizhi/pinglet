//! 会话文件采集与看板四态聚合。

use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::title::{prune_title_cache, resolve_session_title};

const SESSION_STATUS_UNKNOWN: &str = "unknown";
const SESSION_KIND_INTERACTIVE: &str = "interactive";
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionRow {
    pid: Option<u32>,
    /// 内部会话名，用于激活终端窗口时匹配标题；不直接展示给用户。
    name: String,
    /// 展示标题：优先用户标题，其次 transcript 标题，最后目录名 / 内部名。
    title: String,
    session_id: Option<String>,
    cwd: Option<String>,
    status: String,
    status_updated_at: Option<i64>,
    alive: Option<bool>,
    entrypoint: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListResult {
    pub(crate) rows: Vec<SessionRow>,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSession {
    pid: Option<u32>,
    cwd: Option<String>,
    name: Option<String>,
    name_source: Option<String>,
    session_id: Option<String>,
    kind: Option<String>,
    entrypoint: Option<String>,
    status: Option<String>,
    status_updated_at: Option<i64>,
    updated_at: Option<i64>,
}
/// 与前端 `classify()` 一致的看板四态；托盘与通知只需要异常 / 进行中的聚合计数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoardKind {
    Blocked,
    Interrupted,
    Working,
    Done,
}

impl BoardKind {
    pub(crate) fn classify(status: &str, alive: Option<bool>) -> Self {
        // NOTE: 进程退出时无论最后状态是什么都归为已中断，与前端保持一致。
        if alive == Some(false) {
            return Self::Interrupted;
        }
        match status.to_ascii_lowercase().as_str() {
            "waiting" | "blocked" => Self::Blocked,
            "failed" | "crashed" | "stopped" => Self::Interrupted,
            "idle" | "done" => Self::Done,
            _ => Self::Working,
        }
    }

    pub(crate) fn is_alert(self) -> bool {
        matches!(self, Self::Blocked | Self::Interrupted)
    }
}

pub(crate) struct AlertItem {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) kind: BoardKind,
}

#[derive(Default)]
pub(crate) struct SessionSnapshot {
    pub(crate) blocked: usize,
    pub(crate) interrupted: usize,
    pub(crate) working: usize,
    pub(crate) alert_keys: HashSet<String>,
    pub(crate) alert_items: Vec<AlertItem>,
}

/// 与前端 `getRowKey()` 相同的身份规则，用于判断哪些警告是新增的。
fn session_key(row: &SessionRow) -> String {
    if let Some(pid) = row.pid {
        return format!("pid:{pid}");
    }
    if let Some(cwd) = row.cwd.as_deref() {
        return format!("cwd:{cwd}");
    }
    format!("name:{}", row.name)
}

/// 告警事件身份：会话身份 + 待处理状态时间戳。
/// NOTE: 只用 session_key 无法区分「同一会话再次进入待处理」——当 waiting → busy → waiting
/// 的 busy 间隔短于 5 秒轮询周期时，轮询看不到离开告警态的那一刻，alert_keys 永不释放，
/// 第二次请求会被当成旧告警静默丢弃。待处理停留期间 statusUpdatedAt 不变，带上时间戳既能
/// 识别新一次请求，也不会在同一次等待中重复弹窗。
/// 已中断仍只用会话身份：进程已死，文件被重写不应再次唤醒弹窗。
fn alert_event_key(row: &SessionRow, kind: BoardKind) -> String {
    let key = session_key(row);
    if kind == BoardKind::Blocked
        && let Some(timestamp) = row.status_updated_at
    {
        return format!("{key}:{timestamp}");
    }
    key
}

pub(crate) fn build_snapshot(rows: &[SessionRow]) -> SessionSnapshot {
    let mut snapshot = SessionSnapshot::default();
    for row in rows {
        let kind = BoardKind::classify(&row.status, row.alive);
        match kind {
            BoardKind::Blocked => snapshot.blocked += 1,
            BoardKind::Interrupted => snapshot.interrupted += 1,
            BoardKind::Working => snapshot.working += 1,
            BoardKind::Done => {}
        }
        if kind.is_alert() {
            let key = alert_event_key(row, kind);
            snapshot.alert_keys.insert(key.clone());
            snapshot.alert_items.push(AlertItem {
                key,
                title: row.title.clone(),
                kind,
            });
        }
    }
    snapshot
}
/// 解析 Claude Code 的内部会话文件；字段随版本变化时只需改这里。
/// NOTE: 异步命令会在线程池执行，避免文件扫描阻塞主线程。
#[tauri::command(async)]
pub fn list_sessions() -> ListResult {
    collect_sessions()
}

/// 前端轮询与后台托盘监控共用的扫描入口。
pub(crate) fn collect_sessions() -> ListResult {
    let mut rows = Vec::new();
    let mut errors = Vec::new();

    let Some(dir) = sessions_dir() else {
        return ListResult { rows, errors };
    };

    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return ListResult { rows, errors };
        }
        Err(err) => {
            errors.push(format!("读取会话目录失败: {err}"));
            return ListResult { rows, errors };
        }
    };

    let mut sessions: Vec<(RawSession, Option<i64>)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_json_file(&path) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>")
            .to_string();

        // SECURITY: 只解析白名单字段；错误信息里不输出 JSON 内容（可能含 IPC 路径等敏感数据）。
        match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<RawSession>(&content) {
                Ok(session) => {
                    if let Some(kind) = session.kind.as_deref()
                        && !kind.eq_ignore_ascii_case(SESSION_KIND_INTERACTIVE)
                    {
                        continue;
                    }
                    sessions.push((session, modified_ms(&path)));
                }
                Err(err) => errors.push(format!("{file_name}: JSON 解析失败 ({err})")),
            },
            Err(err) => errors.push(format!("{file_name}: 读取失败 ({err})")),
        }
    }

    // 上一轮存在、本轮已消失的会话不再占用标题缓存，避免长期运行只增不减。
    prune_title_cache(
        sessions
            .iter()
            .filter_map(|(session, _)| session.session_id.as_deref()),
    );

    let pids: Vec<Pid> = sessions
        .iter()
        .filter_map(|(session, _)| session.pid)
        .map(Pid::from_u32)
        .collect();
    let mut system = match System::new() {
        Ok(system) => Some(system),
        Err(err) => {
            errors.push(format!("初始化进程信息失败: {err}"));
            None
        }
    };
    if let Some(system) = system.as_mut()
        && !pids.is_empty()
    {
        system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
    }

    for (session, file_modified_at) in sessions {
        let pid = session.pid;
        let alive = match (system.as_ref(), pid) {
            (Some(system), Some(value)) => Some(system.process(Pid::from_u32(value)).is_some()),
            _ => None,
        };
        let status_updated_at = session
            .status_updated_at
            .or(session.updated_at)
            .or(file_modified_at);
        let cwd = session.cwd;
        let name = session
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| cwd.as_deref().and_then(directory_name))
            .unwrap_or_else(|| "未命名会话".to_string());
        let title = resolve_session_title(
            &name,
            session.name_source.as_deref(),
            session.session_id.as_deref(),
            cwd.as_deref(),
        );

        rows.push(SessionRow {
            pid,
            name,
            title,
            session_id: session.session_id,
            cwd,
            status: session
                .status
                .unwrap_or_else(|| SESSION_STATUS_UNKNOWN.to_string()),
            status_updated_at,
            alive,
            entrypoint: session.entrypoint,
        });
    }

    ListResult { rows, errors }
}

fn sessions_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".claude").join("sessions"))
}

fn is_json_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

pub(crate) fn directory_name(cwd: &str) -> Option<String> {
    let trimmed = cwd.trim_end_matches(['\\', '/']);
    Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
}

fn modified_ms(path: &Path) -> Option<i64> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(system_time_ms)
}

pub(crate) fn system_time_ms(time: std::time::SystemTime) -> Option<i64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_row(
        pid: Option<u32>,
        cwd: Option<&str>,
        status: &str,
        alive: Option<bool>,
    ) -> SessionRow {
        SessionRow {
            pid,
            name: "demo".to_string(),
            title: "演示会话".to_string(),
            session_id: None,
            cwd: cwd.map(str::to_string),
            status: status.to_string(),
            status_updated_at: None,
            alive,
            entrypoint: None,
        }
    }

    #[test]
    fn classifies_like_frontend() {
        // 进程退出优先于状态字段，与前端 classify() 一致
        assert_eq!(
            BoardKind::classify("done", Some(false)),
            BoardKind::Interrupted
        );
        assert_eq!(
            BoardKind::classify("waiting", Some(true)),
            BoardKind::Blocked
        );
        assert_eq!(BoardKind::classify("BLOCKED", None), BoardKind::Blocked);
        for status in ["failed", "crashed", "stopped"] {
            assert_eq!(
                BoardKind::classify(status, Some(true)),
                BoardKind::Interrupted
            );
        }
        for status in ["idle", "done"] {
            assert_eq!(BoardKind::classify(status, Some(true)), BoardKind::Done);
        }
        for status in [
            "busy", "shell", "working", "starting", "running", "resuming", "adopted", "unknown",
        ] {
            assert_eq!(BoardKind::classify(status, Some(true)), BoardKind::Working);
        }
    }

    #[test]
    fn session_key_prefers_pid_then_cwd() {
        assert_eq!(
            session_key(&session_row(Some(42), Some("C:\\demo"), "busy", Some(true))),
            "pid:42"
        );
        assert_eq!(
            session_key(&session_row(None, Some("C:\\demo"), "busy", Some(true))),
            "cwd:C:\\demo"
        );
        assert_eq!(
            session_key(&session_row(None, None, "busy", Some(true))),
            "name:demo"
        );
    }

    fn session_row_at(
        pid: Option<u32>,
        status: &str,
        status_updated_at: Option<i64>,
    ) -> SessionRow {
        SessionRow {
            status_updated_at,
            ..session_row(pid, None, status, Some(true))
        }
    }

    #[test]
    fn alert_event_key_distinguishes_repeated_blocked_requests() {
        let first_wait = session_row_at(Some(7), "waiting", Some(1000));
        let same_wait = session_row_at(Some(7), "waiting", Some(1000));
        let next_wait = session_row_at(Some(7), "waiting", Some(2000));

        // 同一次等待期间 statusUpdatedAt 不变，轮询重复采集不能重复触发弹窗
        assert_eq!(
            alert_event_key(&first_wait, BoardKind::Blocked),
            "pid:7:1000"
        );
        assert_eq!(
            alert_event_key(&same_wait, BoardKind::Blocked),
            "pid:7:1000"
        );
        // 再次进入待处理会刷新 statusUpdatedAt，应视为新的告警事件
        assert_ne!(
            alert_event_key(&next_wait, BoardKind::Blocked),
            alert_event_key(&first_wait, BoardKind::Blocked)
        );
    }

    #[test]
    fn alert_event_key_falls_back_without_blocked_timestamp() {
        let row = session_row_at(Some(3), "waiting", None);

        assert_eq!(alert_event_key(&row, BoardKind::Blocked), "pid:3");
    }

    #[test]
    fn interrupted_alert_key_ignores_status_timestamp() {
        let row = session_row_at(Some(9), "failed", Some(1000));
        let touched = session_row_at(Some(9), "failed", Some(2000));

        // 进程已死：会话文件被重写不应再次唤醒弹窗
        assert_eq!(alert_event_key(&row, BoardKind::Interrupted), "pid:9");
        assert_eq!(alert_event_key(&touched, BoardKind::Interrupted), "pid:9");
    }

    #[test]
    fn snapshot_counts_states_and_alert_keys() {
        let rows = [
            session_row(Some(1), Some("C:\\a"), "waiting", Some(true)),
            session_row(Some(2), Some("C:\\b"), "failed", Some(true)),
            session_row(Some(3), Some("C:\\c"), "busy", Some(true)),
            session_row(Some(4), Some("C:\\d"), "done", Some(false)),
            session_row(Some(5), Some("C:\\e"), "idle", Some(true)),
        ];

        let snapshot = build_snapshot(&rows);
        assert_eq!(snapshot.blocked, 1);
        assert_eq!(snapshot.interrupted, 2);
        assert_eq!(snapshot.working, 1);
        assert_eq!(snapshot.alert_items.len(), 3);
        assert!(snapshot.alert_keys.contains("pid:1"));
        // 进程已退出时即使状态是 done 也算已中断
        assert!(snapshot.alert_keys.contains("pid:4"));
        assert!(!snapshot.alert_keys.contains("pid:3"));
    }
}
