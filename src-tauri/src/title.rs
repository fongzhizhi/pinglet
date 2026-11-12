//! 从 Claude Code transcript 增量读取会话标题。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::session::{directory_name, system_time_ms};

/// transcript 标题扫描：头部找 ai-title，尾部找 custom-title；窗口按需倍增并设上限。
const TITLE_HEAD_START_BYTES: u64 = 64 * 1024;
const TITLE_HEAD_MAX_BYTES: u64 = 4 * 1024 * 1024;
const TITLE_TAIL_START_BYTES: u64 = 128 * 1024;
const TITLE_TAIL_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// 增量扫描重叠区，避免标题记录恰好跨在上次 EOF 边界被截断。
const TITLE_SCAN_OVERLAP_BYTES: u64 = 8 * 1024;
/// 标题展示上限，防止异常 transcript 写入超长字符串。
const MAX_TITLE_CHARS: usize = 200;
/// transcript 未找到时的负缓存有效期；新会话的 transcript 可能稍后才落盘。
const TRANSCRIPT_MISS_CACHE_TTL: Duration = Duration::from_secs(30);
/// transcript 标题缓存：文件未变化时直接复用，增长时只扫描新增部分。
struct CachedTitle {
    path: PathBuf,
    file_size: u64,
    modified_ms: Option<i64>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    head_scanned_size: u64,
}

impl CachedTitle {
    fn title(&self) -> Option<String> {
        self.custom_title.clone().or_else(|| self.ai_title.clone())
    }
}

static TITLE_CACHE: OnceLock<Mutex<HashMap<String, CachedTitle>>> = OnceLock::new();
static TRANSCRIPT_MISS_CACHE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
fn projects_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".claude").join("projects"))
}

/// sessionId 来自本地 JSON，仍按文件名白名单校验，避免路径穿越。
fn is_safe_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

/// 按 sessionId 在 ~/.claude/projects/*/ 下查找 transcript，不硬编码项目 slug 转义规则。
/// NOTE: 未命中结果缓存一段时间，避免为尚未生成 transcript 的新会话重复扫描目录。
fn find_transcript(session_id: &str) -> Option<PathBuf> {
    if let Ok(cache) = transcript_miss_cache().lock()
        && let Some(checked_at) = cache.get(session_id)
        && checked_at.elapsed() < TRANSCRIPT_MISS_CACHE_TTL
    {
        return None;
    }

    let projects = projects_dir()?;
    for entry in fs::read_dir(projects).ok()?.flatten() {
        let candidate = entry.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            if let Ok(mut cache) = transcript_miss_cache().lock() {
                cache.remove(session_id);
            }
            return Some(candidate);
        }
    }

    if let Ok(mut cache) = transcript_miss_cache().lock() {
        cache.retain(|_, checked_at| checked_at.elapsed() < TRANSCRIPT_MISS_CACHE_TTL);
        cache.insert(session_id.to_string(), Instant::now());
    }
    None
}

/// 解析展示标题：hook 名称优先；否则读 transcript 标题记录；derived 会话最后回退目录名。
pub(crate) fn resolve_session_title(
    internal_name: &str,
    name_source: Option<&str>,
    session_id: Option<&str>,
    cwd: Option<&str>,
) -> String {
    if name_source.is_some_and(|source| source.eq_ignore_ascii_case("hook")) {
        return internal_name.to_string();
    }

    if let Some(session_id) = session_id.filter(|value| is_safe_session_id(value))
        && let Some(title) = transcript_title(session_id)
    {
        return title;
    }

    if name_source.is_some_and(|source| source.eq_ignore_ascii_case("derived"))
        && let Some(directory) = cwd.and_then(directory_name)
    {
        return directory;
    }

    internal_name.to_string()
}

fn title_cache() -> &'static Mutex<HashMap<String, CachedTitle>> {
    TITLE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn transcript_miss_cache() -> &'static Mutex<HashMap<String, Instant>> {
    TRANSCRIPT_MISS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 清理已消失会话的缓存，避免长期运行时标题缓存只增不减。
/// NOTE: 由每轮会话采集在成功读取目录后调用，传入本轮仍在看板上的 sessionId。
pub(crate) fn prune_title_cache<'a>(active_ids: impl IntoIterator<Item = &'a str>) {
    let active: HashSet<&str> = active_ids.into_iter().collect();
    if let Ok(mut cache) = title_cache().lock() {
        cache.retain(|session_id, _| active.contains(session_id.as_str()));
    }
    if let Ok(mut cache) = transcript_miss_cache().lock() {
        cache.retain(|session_id, _| active.contains(session_id.as_str()));
    }
}

/// 读取 transcript 标题；缓存按文件大小与 mtime 失效，增长时只扫描新增区间。
fn transcript_title(session_id: &str) -> Option<String> {
    let mut cache = title_cache().lock().ok()?;

    if let Some(cached) = cache.get_mut(session_id) {
        match fs::metadata(&cached.path) {
            Ok(metadata) => {
                let size = metadata.len();
                let modified_ms = metadata.modified().ok().and_then(system_time_ms);
                if size == cached.file_size && modified_ms == cached.modified_ms {
                    return cached.title();
                }
                if size > cached.file_size {
                    update_cached_title(cached, size);
                    cached.modified_ms = modified_ms;
                    return cached.title();
                }
                // 文件被截断或重写：删除缓存后重新扫描。
            }
            Err(_) => {
                // 文件已删除：删除缓存，按 sessionId 重新查找。
            }
        }
        cache.remove(session_id);
    }

    let path = find_transcript(session_id)?;
    let metadata = fs::metadata(&path).ok()?;
    let size = metadata.len();
    let modified_ms = metadata.modified().ok().and_then(system_time_ms);
    let entry = scan_title_file(&path, size, modified_ms)?;
    let title = entry.title();
    cache.insert(session_id.to_string(), entry);
    title
}

fn scan_title_file(path: &Path, size: u64, modified_ms: Option<i64>) -> Option<CachedTitle> {
    if size == 0 {
        return Some(CachedTitle {
            path: path.to_path_buf(),
            file_size: 0,
            modified_ms,
            custom_title: None,
            ai_title: None,
            head_scanned_size: 0,
        });
    }

    // 头部：ai-title 通常紧跟首条消息，窗口从 64 KB 倍增到 4 MB。
    let mut head_scanned_size = size.min(TITLE_HEAD_START_BYTES);
    let mut head_text = read_file_slice(path, 0, head_scanned_size)?;
    let mut ai_title = last_title_record(&head_text, "ai-title", "aiTitle");
    while ai_title.is_none() && head_scanned_size < TITLE_HEAD_MAX_BYTES && head_scanned_size < size
    {
        head_scanned_size = head_scanned_size
            .saturating_mul(2)
            .min(TITLE_HEAD_MAX_BYTES)
            .min(size);
        head_text = read_file_slice(path, 0, head_scanned_size)?;
        ai_title = last_title_record(&head_text, "ai-title", "aiTitle");
    }

    // 尾部：custom-title 追加写入，窗口从 128 KB 倍增到 8 MB。
    let custom_title = if head_scanned_size >= size {
        last_title_record(&head_text, "custom-title", "customTitle")
    } else {
        let mut tail_size = size.min(TITLE_TAIL_START_BYTES);
        loop {
            let tail_text = read_file_slice(path, size - tail_size, tail_size)?;
            if let Some(title) = last_title_record(&tail_text, "custom-title", "customTitle") {
                break Some(title);
            }
            if tail_size >= TITLE_TAIL_MAX_BYTES || tail_size >= size {
                break None;
            }
            tail_size = tail_size
                .saturating_mul(2)
                .min(TITLE_TAIL_MAX_BYTES)
                .min(size);
        }
    };

    Some(CachedTitle {
        path: path.to_path_buf(),
        file_size: size,
        modified_ms,
        custom_title,
        ai_title,
        head_scanned_size,
    })
}

/// 文件追加后只读取新增区间；头部扫满上限后不再重复读取。
fn update_cached_title(cached: &mut CachedTitle, size: u64) {
    let path = cached.path.clone();

    if cached.ai_title.is_none() && cached.head_scanned_size < TITLE_HEAD_MAX_BYTES {
        let head_end = size.min(TITLE_HEAD_MAX_BYTES);
        let head_start = cached
            .head_scanned_size
            .saturating_sub(TITLE_SCAN_OVERLAP_BYTES);
        if head_end > head_start {
            if let Some(text) = read_file_slice(&path, head_start, head_end - head_start)
                && let Some(title) = last_title_record(&text, "ai-title", "aiTitle")
            {
                cached.ai_title = Some(title);
            }
            cached.head_scanned_size = head_end;
        }
    }

    let tail_start = cached.file_size.saturating_sub(TITLE_SCAN_OVERLAP_BYTES);
    let start = tail_start.max(size.saturating_sub(TITLE_TAIL_MAX_BYTES));
    if size > start
        && let Some(text) = read_file_slice(&path, start, size - start)
        && let Some(title) = last_title_record(&text, "custom-title", "customTitle")
    {
        cached.custom_title = Some(title);
    }

    cached.file_size = size;
}

fn read_file_slice(path: &Path, start: u64, length: u64) -> Option<String> {
    let length = usize::try_from(length).ok()?;
    let mut file = fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buffer = vec![0_u8; length];
    let read = file.read(&mut buffer).ok()?;
    buffer.truncate(read);
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// 取窗口内最后一条指定类型的标题记录；截断、损坏或半行 JSON 直接忽略。
fn last_title_record(text: &str, record_type: &str, title_key: &str) -> Option<String> {
    let type_marker = format!("\"{record_type}\"");
    let key_marker = format!("\"{title_key}\"");
    let mut found = None;

    for line in text.lines() {
        // 首行可能带 BOM，先剥掉再交给 serde
        let line = line.trim_start_matches('\u{feff}');
        if !line.contains(&type_marker) || !line.contains(&key_marker) {
            continue;
        }
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if record.get("type").and_then(|value| value.as_str()) != Some(record_type) {
            continue;
        }
        let Some(title) = record.get(title_key).and_then(|value| value.as_str()) else {
            continue;
        };
        let title = normalize_title(title);
        if !title.is_empty() {
            found = Some(title);
        }
    }

    found
}

fn normalize_title(title: &str) -> String {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_title(&normalized)
}

fn truncate_title(title: &str) -> String {
    match title.char_indices().nth(MAX_TITLE_CHARS) {
        Some((index, _)) => format!("{}…", &title[..index]),
        None => title.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pinglet-title-{name}-{}-{unique}.jsonl",
            std::process::id()
        ))
    }

    fn write_transcript(path: &Path, content: &str) {
        fs::write(path, content).expect("write temp transcript");
    }

    fn append_transcript(path: &Path, content: &str) {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open temp transcript");
        file.write_all(content.as_bytes())
            .expect("append temp transcript");
    }

    fn file_size(path: &Path) -> u64 {
        fs::metadata(path).expect("temp transcript metadata").len()
    }

    #[test]
    fn prefers_last_custom_title_over_ai_title() {
        let path = temp_path("prefer-custom");
        write_transcript(
            &path,
            concat!(
                "{\"type\":\"ai-title\",\"aiTitle\":\"AI 标题\"}\n",
                "{\"type\":\"custom-title\",\"customTitle\":\"旧标题\"}\n",
                "{\"type\":\"custom-title\",\"customTitle\":\"新标题\"}\n",
            ),
        );

        let cached = scan_title_file(&path, file_size(&path), None).expect("scan transcript");
        assert_eq!(cached.title().as_deref(), Some("新标题"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn falls_back_to_ai_title() {
        let path = temp_path("ai-fallback");
        write_transcript(&path, "{\"type\":\"ai-title\",\"aiTitle\":\"自动标题\"}\n");

        let cached = scan_title_file(&path, file_size(&path), None).expect("scan transcript");
        assert_eq!(cached.title().as_deref(), Some("自动标题"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn ignores_truncated_and_invalid_lines() {
        let path = temp_path("broken-lines");
        write_transcript(
            &path,
            concat!(
                "{\"type\":\"custom-title\",\"customTitle\":\"有效标题\"}\n",
                "{\"type\":\"custom-title\",\"customTitle\":\"未闭合\n",
                "not-json\n",
            ),
        );

        let cached = scan_title_file(&path, file_size(&path), None).expect("scan transcript");
        assert_eq!(cached.title().as_deref(), Some("有效标题"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn incrementally_reads_appended_custom_title() {
        let path = temp_path("incremental");
        write_transcript(
            &path,
            "{\"type\":\"ai-title\",\"aiTitle\":\"初始标题\"}\n{\"type\":\"user\"}\n",
        );

        let mut cached = scan_title_file(&path, file_size(&path), None).expect("scan transcript");
        assert_eq!(cached.title().as_deref(), Some("初始标题"));

        append_transcript(
            &path,
            "{\"type\":\"custom-title\",\"customTitle\":\"追加标题\"}\n",
        );
        update_cached_title(&mut cached, file_size(&path));
        assert_eq!(cached.title().as_deref(), Some("追加标题"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn truncates_overlong_titles() {
        let long_title = "标".repeat(MAX_TITLE_CHARS + 1);
        let truncated = truncate_title(&long_title);
        assert_eq!(truncated.chars().count(), MAX_TITLE_CHARS + 1);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn derived_session_falls_back_to_directory_name() {
        let title = resolve_session_title(
            "demo-alpha-3f",
            Some("derived"),
            Some("00000000-0000-0000-0000-000000000000"),
            Some("C:\\Projects\\demo-alpha"),
        );
        assert_eq!(title, "demo-alpha");
    }

    #[test]
    fn hook_name_skips_transcript_lookup() {
        let title = resolve_session_title(
            "用户标题",
            Some("hook"),
            Some("../unsafe"),
            Some("C:\\Projects\\demo-alpha"),
        );
        assert_eq!(title, "用户标题");
    }

    #[test]
    fn prune_title_cache_drops_vanished_sessions() {
        fn entry(path: &str) -> CachedTitle {
            CachedTitle {
                path: PathBuf::from(path),
                file_size: 0,
                modified_ms: None,
                custom_title: None,
                ai_title: None,
                head_scanned_size: 0,
            }
        }

        title_cache()
            .lock()
            .expect("title cache")
            .insert("keep".to_string(), entry("keep.jsonl"));
        title_cache()
            .lock()
            .expect("title cache")
            .insert("drop".to_string(), entry("drop.jsonl"));
        transcript_miss_cache()
            .lock()
            .expect("miss cache")
            .insert("keep".to_string(), Instant::now());
        transcript_miss_cache()
            .lock()
            .expect("miss cache")
            .insert("drop".to_string(), Instant::now());

        prune_title_cache(["keep"]);

        let cache = title_cache().lock().expect("title cache");
        assert!(cache.contains_key("keep"));
        assert!(!cache.contains_key("drop"));
        drop(cache);
        let misses = transcript_miss_cache().lock().expect("miss cache");
        assert!(misses.contains_key("keep"));
        assert!(!misses.contains_key("drop"));
    }

    #[test]
    fn rejects_unsafe_session_ids() {
        assert!(is_safe_session_id("9adeca97-5414-480b-89ca-153f956e4daa"));
        assert!(!is_safe_session_id("../escape"));
        assert!(!is_safe_session_id("dir/file"));
        assert!(!is_safe_session_id(""));
    }
}
