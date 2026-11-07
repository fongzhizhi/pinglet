use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use sysinfo::{Pid, ProcessesToUpdate, System};
#[cfg(target_os = "windows")]
use sysinfo::{Process, ProcessRefreshKind, UpdateKind};
use tauri::menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow,
};
#[cfg(target_os = "windows")]
use windows_sys::core::BOOL;

const SESSION_STATUS_UNKNOWN: &str = "unknown";
const SESSION_KIND_INTERACTIVE: &str = "interactive";
const TRAY_ID: &str = "pinglet-tray";
const TRAY_MENU_SHOW: &str = "show";
const TRAY_MENU_SETTINGS: &str = "settings";
const TRAY_MENU_CLICK_THROUGH: &str = "click-through";
const TRAY_MENU_QUIT: &str = "quit";
const SNAP_DISTANCE_LOGICAL: f64 = 10.0;
/// 已吸附后需要拖出的距离；大于吸附距离，避免小步移动被反复拉回边缘。
const SNAP_RELEASE_DISTANCE_LOGICAL: f64 = 24.0;
/// transcript 标题扫描：头部找 ai-title，尾部找 custom-title；窗口按需倍增并设上限。
const TITLE_HEAD_START_BYTES: u64 = 64 * 1024;
const TITLE_HEAD_MAX_BYTES: u64 = 4 * 1024 * 1024;
const TITLE_TAIL_START_BYTES: u64 = 128 * 1024;
const TITLE_TAIL_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// 增量扫描重叠区，避免标题记录恰好跨在上次 EOF 边界被截断。
const TITLE_SCAN_OVERLAP_BYTES: u64 = 8 * 1024;
/// 标题展示上限，防止异常 transcript 写入超长字符串。
const MAX_TITLE_CHARS: usize = 200;
/// 默认窗口宽度（逻辑像素），与 tauri.conf.json / 前端默认值保持一致。
const DEFAULT_WINDOW_WIDTH_LOGICAL: f64 = 280.0;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 进程父链遍历上限，避免异常数据导致死循环。
#[cfg(target_os = "windows")]
const MAX_ANCESTOR_DEPTH: usize = 32;

/// 用目录名匹配 VSCode 窗口标题时，忽略过短路径段（如盘符、单字符目录）以免误命中。
#[cfg(target_os = "windows")]
const MIN_VSCODE_TITLE_SEGMENT_LEN: usize = 3;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionRow {
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
struct ListResult {
    rows: Vec<SessionRow>,
    errors: Vec<String>,
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

/// 当前吸附在哪些边缘；用于拖出时保持迟滞，避免每帧都被吸回。
#[derive(Clone, Copy, Default)]
struct SnapEdges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

/// 应用级状态：关闭行为、鼠标穿透、托盘可用标记与当前全局快捷键。
struct AppState {
    close_to_tray: AtomicBool,
    click_through: AtomicBool,
    tray_ready: AtomicBool,
    show_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    click_through_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    current_shortcut: Mutex<Option<Shortcut>>,
    snapped_edges: Mutex<SnapEdges>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            close_to_tray: AtomicBool::new(true),
            click_through: AtomicBool::new(false),
            tray_ready: AtomicBool::new(false),
            show_item: Mutex::new(None),
            click_through_item: Mutex::new(None),
            current_shortcut: Mutex::new(None),
            snapped_edges: Mutex::new(SnapEdges::default()),
        }
    }
}

/// 解析 Claude Code 的内部会话文件；字段随版本变化时只需改这里。
#[tauri::command]
fn list_sessions() -> ListResult {
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
                    if let Some(kind) = session.kind.as_deref() {
                        if !kind.eq_ignore_ascii_case(SESSION_KIND_INTERACTIVE) {
                            continue;
                        }
                    }
                    sessions.push((session, modified_ms(&path)));
                }
                Err(err) => errors.push(format!("{file_name}: JSON 解析失败 ({err})")),
            },
            Err(err) => errors.push(format!("{file_name}: 读取失败 ({err})")),
        }
    }

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
    if let Some(system) = system.as_mut() {
        if !pids.is_empty() {
            system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
        }
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

/// 双击会话时只处理 VSCode 与 CLI 两种入口，其他入口直接拒绝。
#[tauri::command]
fn activate_session(
    cwd: String,
    entrypoint: Option<String>,
    name: Option<String>,
    pid: Option<u32>,
) -> Result<(), String> {
    let directory = PathBuf::from(&cwd);
    if !directory.is_dir() {
        return Err(format!("目录不存在: {cwd}"));
    }

    if is_vscode_entrypoint(entrypoint.as_deref()) {
        open_vscode(&directory)
    } else if is_cli_entrypoint(entrypoint.as_deref()) {
        open_terminal(&directory, name.as_deref(), pid)
    } else {
        Err("该会话不支持双击打开".to_string())
    }
}

fn is_vscode_entrypoint(entrypoint: Option<&str>) -> bool {
    entrypoint.is_some_and(|value| {
        let lowered = value.to_ascii_lowercase();
        lowered.contains("vscode") || lowered == "code" || lowered.contains("visual studio code")
    })
}

fn is_cli_entrypoint(entrypoint: Option<&str>) -> bool {
    entrypoint.is_some_and(|value| {
        let lowered = value.to_ascii_lowercase();
        lowered == "claude" || lowered.contains("cli")
    })
}

fn open_vscode(directory: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let Some(code_exe) = find_vscode_executable() else {
            return Err("未找到 VSCode".to_string());
        };

        Command::new(code_exe)
            // 从 VSCode / Electron 派生的环境可能带有该变量，会让 Code.exe 以 Node 模式运行
            .env_remove("ELECTRON_RUN_AS_NODE")
            .arg(directory)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|err| format!("打开 VSCode 失败: {err}"))?;

        // VSCode 已有实例时，系统焦点抢占策略可能让窗口留在后台，由 Pinglet 显式激活
        reveal_vscode_window(directory);
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        Command::new("code")
            .arg(directory)
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("打开 VSCode 失败: {err}"))
    }
}

#[cfg(target_os = "windows")]
fn find_vscode_executable() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new("where");
    command.arg("code").creation_flags(CREATE_NO_WINDOW);
    if let Ok(output) = command.output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let path = PathBuf::from(line.trim());
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                if extension.eq_ignore_ascii_case("exe") {
                    return Some(path);
                }
                if extension.eq_ignore_ascii_case("cmd") {
                    if let Some(install_dir) = path.parent().and_then(Path::parent) {
                        let candidate = install_dir.join("Code.exe");
                        if candidate.is_file() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }
    } else {
        eprintln!("pinglet: where code 执行失败");
    }

    let candidates = [
        std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("Programs\\Microsoft VS Code\\Code.exe")),
        std::env::var_os("ProgramFiles")
            .map(|base| PathBuf::from(base).join("Microsoft VS Code\\Code.exe")),
        std::env::var_os("ProgramFiles(x86)")
            .map(|base| PathBuf::from(base).join("Microsoft VS Code\\Code.exe")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|candidate| candidate.is_file())
}

/// VSCode 已存在实例时，Code.exe 的窗口可能因焦点抢占策略留在后台。
/// 在独立线程里轮询，等窗口出现后显式激活。
#[cfg(target_os = "windows")]
fn reveal_vscode_window(directory: &Path) {
    let folder = directory
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string());

    std::thread::spawn(move || {
        const ATTEMPTS: u32 = 25;
        const INTERVAL_MS: u64 = 100;
        for _ in 0..ATTEMPTS {
            if focus_vscode_window(folder.as_deref()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(INTERVAL_MS));
        }
    });
}

#[cfg(target_os = "windows")]
struct WindowCandidate {
    hwnd: HWND,
    pid: u32,
    title: String,
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return TRUE;
        }

        let title_length = GetWindowTextLengthW(hwnd);
        if title_length <= 0 {
            return TRUE;
        }

        let mut buffer = vec![0u16; title_length as usize + 1];
        let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        if copied <= 0 {
            return TRUE;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return TRUE;
        }

        let candidates = &mut *(lparam as *mut Vec<WindowCandidate>);
        candidates.push(WindowCandidate {
            hwnd,
            pid,
            title: String::from_utf16_lossy(&buffer[..copied as usize]),
        });
        TRUE
    }
}

#[cfg(target_os = "windows")]
fn collect_visible_windows() -> Vec<WindowCandidate> {
    let mut candidates: Vec<WindowCandidate> = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_window),
            &mut candidates as *mut Vec<WindowCandidate> as LPARAM,
        );
    }
    candidates
}

#[cfg(target_os = "windows")]
fn is_vscode_process_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "code.exe" || name == "code - insiders.exe"
}

#[cfg(target_os = "windows")]
fn focus_vscode_window(folder: Option<&str>) -> bool {
    let candidates = collect_visible_windows();
    if candidates.is_empty() {
        return false;
    }

    // 同一进程可能有多个顶层窗口；重复 PID 会让 sysinfo 漏掉该进程
    let mut pids: Vec<Pid> = candidates
        .iter()
        .map(|candidate| Pid::from_u32(candidate.pid))
        .collect();
    pids.sort_unstable();
    pids.dedup();
    let Ok(mut system) = System::new() else {
        return false;
    };
    system.refresh_processes(ProcessesToUpdate::Some(&pids), true);

    let folder_pattern = folder.map(str::to_lowercase);
    for candidate in &candidates {
        let Some(process) = system.process(Pid::from_u32(candidate.pid)) else {
            continue;
        };
        if !is_vscode_process_name(&process.name().to_string_lossy()) {
            continue;
        }
        if let Some(pattern) = folder_pattern.as_deref() {
            if !candidate.title.to_lowercase().contains(pattern) {
                continue;
            }
        }

        if activate_window(candidate.hwnd) {
            return true;
        }
    }

    false
}

#[cfg(target_os = "windows")]
fn activate_window(hwnd: HWND) -> bool {
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd) != 0 {
            return true;
        }
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd) != 0 || GetForegroundWindow() == hwnd
    }
}

/// 沿会话进程的父链向上找 VSCode 主进程。
/// VSCode 集成终端里的进程链是 `claude.exe → powershell.exe → Code.exe --type=utility → Code.exe`，
/// 链尾的 Code.exe 才拥有窗口，因此取最深（最后遇到）的那个。
#[cfg(target_os = "windows")]
fn find_vscode_ancestor(system: &System, session_pid: u32) -> Option<u32> {
    let mut current = Pid::from_u32(session_pid);
    let mut vscode_pid: Option<u32> = None;

    for _ in 0..MAX_ANCESTOR_DEPTH {
        let Some(process) = system.process(current) else {
            break;
        };
        let Some(parent) = process.parent() else {
            break;
        };
        if system.process(parent).is_some_and(|process: &Process| {
            is_vscode_process_name(&process.name().to_string_lossy())
        }) {
            vscode_pid = Some(parent.as_u32());
        }
        current = parent;
    }

    vscode_pid
}

/// 用会话目录的各级目录名匹配 VSCode 窗口标题，返回最长命中段的长度。
///
/// VSCode 标题通常是 `<文件名> - <工作区目录> - Visual Studio Code`，
/// 而会话 cwd 可能位于工作区子目录，因此从完整路径的各级目录名里找命中项。
#[cfg(target_os = "windows")]
fn vscode_title_score(title: &str, directory: &Path) -> Option<usize> {
    let title = title.to_lowercase();
    directory
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .map(str::to_lowercase)
        .filter(|segment| segment.len() >= MIN_VSCODE_TITLE_SEGMENT_LEN && title.contains(segment))
        .map(|segment| segment.len())
        .max()
}

/// 激活会话所属的 VSCode 窗口；标题无法定位时仅当该进程只有一个窗口才兜底激活。
#[cfg(target_os = "windows")]
fn focus_vscode_session_window(vscode_pid: u32, directory: &Path) -> Result<(), String> {
    let candidates: Vec<WindowCandidate> = collect_visible_windows()
        .into_iter()
        .filter(|candidate| candidate.pid == vscode_pid)
        .collect();
    if candidates.is_empty() {
        return Err("找到 VSCode 进程，但未找到可激活的窗口".to_string());
    }

    let mut scored: Vec<(usize, &WindowCandidate)> = candidates
        .iter()
        .filter_map(|candidate| {
            vscode_title_score(&candidate.title, directory).map(|score| (score, candidate))
        })
        .collect();
    scored.sort_by(|left, right| right.0.cmp(&left.0));

    let target = scored
        .first()
        .map(|(_, candidate)| *candidate)
        .or_else(|| (candidates.len() == 1).then(|| &candidates[0]))
        .ok_or_else(|| "找到 VSCode，但无法定位该会话所在的窗口".to_string())?;

    if activate_window(target.hwnd) {
        Ok(())
    } else {
        Err("找到 VSCode 窗口，但无法切换到前台".to_string())
    }
}

fn open_terminal(directory: &Path, name: Option<&str>, pid: Option<u32>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // 只激活已有窗口；不新开窗口，避免出现用户没预期的重复终端
        focus_existing_terminal(directory, name, pid)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (directory, name, pid);
        Err("当前平台暂不支持激活终端窗口".to_string())
    }
}

#[cfg(target_os = "windows")]
fn focus_existing_terminal(
    directory: &Path,
    name: Option<&str>,
    pid: Option<u32>,
) -> Result<(), String> {
    let target = normalize_path_key(&directory.to_string_lossy());

    let Ok(mut system) = System::new() else {
        return Err("无法读取进程列表".to_string());
    };
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );

    // VSCode 集成终端里的 CLI 没有 mintty 窗口，按进程链找到所属 VSCode 并激活
    if let Some(vscode_pid) = pid.and_then(|session_pid| find_vscode_ancestor(&system, session_pid))
    {
        return focus_vscode_session_window(vscode_pid, directory);
    }

    // Git Bash 由 git-bash.exe 启动 mintty，启动目录保留在 --cd 参数中
    let mut mintty_pids: Vec<u32> = Vec::new();
    let mut directory_matched_pids: Vec<u32> = Vec::new();
    for (pid, process) in system.processes() {
        if !process
            .name()
            .to_string_lossy()
            .eq_ignore_ascii_case("mintty.exe")
        {
            continue;
        }
        mintty_pids.push(pid.as_u32());

        if target.is_empty() {
            continue;
        }
        let Some(launcher) = process.parent().and_then(|parent| system.process(parent)) else {
            continue;
        };
        if !launcher
            .name()
            .to_string_lossy()
            .eq_ignore_ascii_case("git-bash.exe")
        {
            continue;
        }
        if launcher
            .cmd()
            .iter()
            .any(|argument| argument_cd_matches(argument, &target))
        {
            directory_matched_pids.push(pid.as_u32());
        }
    }
    if mintty_pids.is_empty() {
        return Err("未找到 Git Bash 窗口，该会话可能未运行在 Git Bash 中".to_string());
    }

    let candidates = collect_visible_windows();

    let directory_candidates: Vec<&WindowCandidate> = candidates
        .iter()
        .filter(|candidate| directory_matched_pids.contains(&candidate.pid))
        .collect();
    if !directory_candidates.is_empty() {
        return activate_first(&directory_candidates);
    }

    // 启动目录不足以定位时（例如在窗口内手动 cd），再用 Claude Code 写入的终端标题匹配
    let Some(pattern) = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
    else {
        return Err("未匹配到该会话的 Git Bash 窗口".to_string());
    };

    let title_candidates: Vec<&WindowCandidate> = candidates
        .iter()
        .filter(|candidate| mintty_pids.contains(&candidate.pid))
        .filter(|candidate| candidate.title.to_lowercase().contains(&pattern))
        .collect();
    if title_candidates.is_empty() {
        return Err("未匹配到该会话的 Git Bash 窗口".to_string());
    }

    activate_first(&title_candidates)
}

#[cfg(target_os = "windows")]
fn activate_first(candidates: &[&WindowCandidate]) -> Result<(), String> {
    if candidates
        .iter()
        .any(|candidate| activate_window(candidate.hwnd))
    {
        return Ok(());
    }
    Err("找到 Git Bash 窗口，但无法切换到前台".to_string())
}

#[cfg(target_os = "windows")]
fn argument_cd_matches(argument: &std::ffi::OsStr, target: &str) -> bool {
    let Some(value) = argument.to_str() else {
        return false;
    };
    let Some(directory) = value.strip_prefix("--cd=") else {
        return false;
    };
    normalize_path_key(directory) == target
}

#[cfg(target_os = "windows")]
fn normalize_path_key(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches(|character| character == '\\' || character == '.')
        .to_lowercase()
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn set_close_to_tray(state: tauri::State<'_, AppState>, enabled: bool) {
    state.close_to_tray.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
fn set_click_through(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    apply_click_through(&app, &state, enabled)
}

/// 切换窗口鼠标穿透，并同步托盘勾选状态与前端设置。
/// NOTE: 开启后窗口无法再接收点击，托盘菜单与全局快捷键是恢复入口（见 `toggle_main_window`）。
fn apply_click_through(
    app: &tauri::AppHandle,
    state: &AppState,
    enabled: bool,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("主窗口不可用".to_string());
    };

    window
        .set_ignore_cursor_events(enabled)
        .map_err(|err| format!("设置鼠标穿透失败: {err}"))?;
    state.click_through.store(enabled, Ordering::Relaxed);

    if let Ok(item) = state.click_through_item.lock() {
        if let Some(item) = item.as_ref() {
            if let Err(err) = item.set_checked(enabled) {
                eprintln!("pinglet: 同步托盘穿透勾选状态失败: {err}");
            }
        }
    }
    if let Err(err) = app.emit("pinglet://click-through-changed", enabled) {
        eprintln!("pinglet: 广播鼠标穿透状态失败: {err}");
    }

    Ok(())
}

#[tauri::command]
fn is_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;

    app.autolaunch()
        .is_enabled()
        .map_err(|err| format!("读取开机自启状态失败: {err}"))
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    if enabled {
        manager
            .enable()
            .map_err(|err| format!("开启开机自启失败: {err}"))
    } else {
        manager
            .disable()
            .map_err(|err| format!("关闭开机自启失败: {err}"))
    }
}

#[tauri::command]
fn set_global_shortcut(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    shortcut: Option<String>,
) -> Result<(), String> {
    let parsed = match shortcut.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => Some(
            value
                .parse::<Shortcut>()
                .map_err(|err| format!("快捷键格式无效: {err}"))?,
        ),
        _ => None,
    };

    let previous = *state
        .current_shortcut
        .lock()
        .map_err(|_| "快捷键状态锁不可用".to_string())?;

    if let Some(previous) = previous {
        if let Err(err) = app.global_shortcut().unregister(previous) {
            eprintln!("pinglet: 注销旧快捷键失败: {err}");
        }
    }

    if let Some(shortcut) = parsed {
        if let Err(err) = app.global_shortcut().register(shortcut) {
            if let Some(previous) = previous {
                if let Err(restore_err) = app.global_shortcut().register(previous) {
                    eprintln!("pinglet: 恢复旧快捷键失败: {restore_err}");
                }
            }
            return Err(format!("注册快捷键失败: {err}"));
        }
    }

    *state
        .current_shortcut
        .lock()
        .map_err(|_| "快捷键状态锁不可用".to_string())? = parsed;

    Ok(())
}

fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show_item = CheckMenuItemBuilder::with_id(TRAY_MENU_SHOW, "显示")
        .checked(true)
        .build(app)?;
    let click_through_item = CheckMenuItemBuilder::with_id(TRAY_MENU_CLICK_THROUGH, "鼠标穿透")
        .checked(false)
        .build(app)?;
    let settings_item = MenuItemBuilder::with_id(TRAY_MENU_SETTINGS, "设置").build(app)?;
    let quit_item = MenuItemBuilder::with_id(TRAY_MENU_QUIT, "退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&show_item, &click_through_item, &settings_item, &quit_item])
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("默认窗口图标".to_string()))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Pinglet")
        .show_menu_on_left_click(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            TRAY_MENU_SHOW => toggle_window_visibility(app),
            TRAY_MENU_SETTINGS => open_settings(app),
            TRAY_MENU_CLICK_THROUGH => {
                let state = app.state::<AppState>();
                let enabled = !state.click_through.load(Ordering::Relaxed);
                if let Err(err) = apply_click_through(app, &state, enabled) {
                    eprintln!("pinglet: 切换鼠标穿透失败: {err}");
                }
            }
            TRAY_MENU_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    let state = app.state::<AppState>();
    if let Ok(mut item) = state.show_item.lock() {
        *item = Some(show_item);
    }
    if let Ok(mut item) = state.click_through_item.lock() {
        *item = Some(click_through_item);
    }
    state.tray_ready.store(true, Ordering::Relaxed);

    Ok(())
}

/// 同步托盘「显示」勾选状态。
fn sync_show_item(state: &AppState, visible: bool) {
    if let Ok(item) = state.show_item.lock() {
        if let Some(item) = item.as_ref() {
            if let Err(err) = item.set_checked(visible) {
                eprintln!("pinglet: 同步托盘显示勾选状态失败: {err}");
            }
        }
    }
}

/// 托盘「显示」菜单：只切换窗口显隐，勾选状态跟随实际可见性。
fn toggle_window_visibility(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<AppState>();

    let is_visible = match window.is_visible() {
        Ok(true) => {
            if let Err(err) = window.hide() {
                eprintln!("pinglet: 隐藏窗口失败: {err}");
                return;
            }
            false
        }
        Ok(false) => {
            if let Err(err) = window.show() {
                eprintln!("pinglet: 显示窗口失败: {err}");
                return;
            }
            if let Err(err) = window.set_focus() {
                eprintln!("pinglet: 聚焦窗口失败: {err}");
            }
            true
        }
        Err(err) => {
            eprintln!("pinglet: 读取窗口可见性失败: {err}");
            return;
        }
    };

    sync_show_item(&state, is_visible);
}

/// 托盘「设置」菜单：显示窗口并打开设置面板；穿透状态下先关闭穿透，否则面板无法点击。
fn open_settings(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<AppState>();

    if state.click_through.load(Ordering::Relaxed) {
        if let Err(err) = apply_click_through(app, &state, false) {
            eprintln!("pinglet: 关闭鼠标穿透失败: {err}");
        }
    }
    if let Err(err) = window.show() {
        eprintln!("pinglet: 显示窗口失败: {err}");
    } else {
        sync_show_item(&state, true);
    }
    if let Err(err) = window.set_focus() {
        eprintln!("pinglet: 聚焦窗口失败: {err}");
    }
    if let Err(err) = app.emit("pinglet://open-settings", ()) {
        eprintln!("pinglet: 广播打开设置失败: {err}");
    }
}

fn toggle_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<AppState>();

    // 穿透状态下窗口收不到任何点击，托盘左键与全局快捷键优先恢复可点击状态
    if state.click_through.load(Ordering::Relaxed) {
        if let Err(err) = apply_click_through(app, &state, false) {
            eprintln!("pinglet: 恢复鼠标穿透失败: {err}");
        }
        if let Err(err) = window.show() {
            eprintln!("pinglet: 显示窗口失败: {err}");
        }
        if let Err(err) = window.set_focus() {
            eprintln!("pinglet: 聚焦窗口失败: {err}");
        }
        sync_show_item(&state, true);
        return;
    }

    match window.is_visible() {
        Ok(true) => {
            if let Err(err) = window.hide() {
                eprintln!("pinglet: 隐藏窗口失败: {err}");
            } else {
                sync_show_item(&state, false);
            }
        }
        Ok(false) => {
            if let Err(err) = window.show() {
                eprintln!("pinglet: 显示窗口失败: {err}");
            } else {
                sync_show_item(&state, true);
            }
            if let Err(err) = window.set_focus() {
                eprintln!("pinglet: 聚焦窗口失败: {err}");
            }
        }
        Err(err) => eprintln!("pinglet: 读取窗口可见性失败: {err}"),
    }
}

/// 窗口可移动到的四条极限位置（物理像素）。
#[derive(Clone, Copy)]
struct SnapBounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// 计算吸附后的位置并更新吸附状态。
///
/// 吸附采用迟滞：进入 `threshold` 范围时贴边，贴边后必须拖出 `release`
/// 距离才解除该边缘的锁定；否则每帧的小幅移动都会被重新拉回边缘。
fn resolve_snap(
    position: (i32, i32),
    bounds: SnapBounds,
    threshold: i32,
    release: i32,
    edges: &mut SnapEdges,
) -> (i32, i32) {
    let (mut x, mut y) = position;

    if edges.left && (x - bounds.left).abs() > release {
        edges.left = false;
    }
    if edges.right && (x - bounds.right).abs() > release {
        edges.right = false;
    }
    if edges.top && (y - bounds.top).abs() > release {
        edges.top = false;
    }
    if edges.bottom && (y - bounds.bottom).abs() > release {
        edges.bottom = false;
    }

    if !edges.left && (x - bounds.left).abs() < threshold {
        x = bounds.left;
        edges.left = true;
    } else if !edges.right && (x - bounds.right).abs() < threshold {
        x = bounds.right;
        edges.right = true;
    }

    if !edges.top && (y - bounds.top).abs() < threshold {
        y = bounds.top;
        edges.top = true;
    } else if !edges.bottom && (y - bounds.bottom).abs() < threshold {
        y = bounds.bottom;
        edges.bottom = true;
    }

    (x, y)
}

fn snap_window_to_edges(window: &tauri::Window) {
    let Ok(position) = window.outer_position() else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };

    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    if size.width > monitor_size.width || size.height > monitor_size.height {
        return;
    }

    let scale_factor = monitor.scale_factor();
    let threshold = (SNAP_DISTANCE_LOGICAL * scale_factor).round() as i32;
    let release = (SNAP_RELEASE_DISTANCE_LOGICAL * scale_factor).round() as i32;
    let bounds = SnapBounds {
        left: monitor_position.x,
        top: monitor_position.y,
        right: monitor_position.x + monitor_size.width as i32 - size.width as i32,
        bottom: monitor_position.y + monitor_size.height as i32 - size.height as i32,
    };

    let state = window.state::<AppState>();
    let Ok(mut edges) = state.snapped_edges.lock() else {
        return;
    };
    let (x, y) = resolve_snap(
        (position.x, position.y),
        bounds,
        threshold,
        release,
        &mut edges,
    );

    if x != position.x || y != position.y {
        if let Err(err) = window.set_position(tauri::PhysicalPosition::new(x, y)) {
            eprintln!("pinglet: 窗口吸附失败: {err}");
        }
    }
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

fn directory_name(cwd: &str) -> Option<String> {
    let trimmed = cwd.trim_end_matches(|character| character == '\\' || character == '/');
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

fn system_time_ms(time: std::time::SystemTime) -> Option<i64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

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
fn find_transcript(session_id: &str) -> Option<PathBuf> {
    let projects = projects_dir()?;
    for entry in fs::read_dir(projects).ok()?.flatten() {
        let candidate = entry.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 解析展示标题：hook 名称优先；否则读 transcript 标题记录；derived 会话最后回退目录名。
fn resolve_session_title(
    internal_name: &str,
    name_source: Option<&str>,
    session_id: Option<&str>,
    cwd: Option<&str>,
) -> String {
    if name_source.is_some_and(|source| source.eq_ignore_ascii_case("hook")) {
        return internal_name.to_string();
    }

    if let Some(session_id) = session_id.filter(|value| is_safe_session_id(value)) {
        if let Some(title) = transcript_title(session_id) {
            return title;
        }
    }

    if name_source.is_some_and(|source| source.eq_ignore_ascii_case("derived")) {
        if let Some(directory) = cwd.and_then(directory_name) {
            return directory;
        }
    }

    internal_name.to_string()
}

fn title_cache() -> &'static Mutex<HashMap<String, CachedTitle>> {
    TITLE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
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
            if let Some(text) = read_file_slice(&path, head_start, head_end - head_start) {
                if let Some(title) = last_title_record(&text, "ai-title", "aiTitle") {
                    cached.ai_title = Some(title);
                }
            }
            cached.head_scanned_size = head_end;
        }
    }

    let tail_start = cached.file_size.saturating_sub(TITLE_SCAN_OVERLAP_BYTES);
    let start = tail_start.max(size.saturating_sub(TITLE_TAIL_MAX_BYTES));
    if size > start {
        if let Some(text) = read_file_slice(&path, start, size - start) {
            if let Some(title) = last_title_record(&text, "custom-title", "customTitle") {
                cached.custom_title = Some(title);
            }
        }
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

fn position_window_top_right(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };

    let scale = monitor.scale_factor();
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let width = (DEFAULT_WINDOW_WIDTH_LOGICAL * scale).round() as i32;
    let margin = (20.0 * scale).round() as i32;
    let x = monitor_position.x + monitor_size.width as i32 - width - margin;
    let y = monitor_position.y + margin;

    if let Err(err) = window.set_position(tauri::PhysicalPosition::new(x, y)) {
        eprintln!("pinglet: 窗口定位失败: {err}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_main_window(app);
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .setup(|app| {
            if let Err(err) = create_tray(app.handle()) {
                eprintln!("pinglet: 创建托盘失败: {err}");
            }
            if let Some(window) = app.get_webview_window("main") {
                position_window_top_right(&window);
                if let Err(err) = window.show() {
                    eprintln!("pinglet: 显示主窗口失败: {err}");
                }
                let state = app.state::<AppState>();
                if let Ok(visible) = window.is_visible() {
                    sync_show_item(&state, visible);
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                let state = window.state::<AppState>();
                if state.close_to_tray.load(Ordering::Relaxed)
                    && state.tray_ready.load(Ordering::Relaxed)
                {
                    api.prevent_close();
                    if let Err(err) = window.hide() {
                        eprintln!("pinglet: 隐藏到托盘失败: {err}");
                    } else {
                        sync_show_item(&state, false);
                    }
                } else {
                    // 关闭行为设为“退出”时，X 按钮要结束整个应用，而不是留下托盘空转
                    window.app_handle().exit(0);
                }
            }
            WindowEvent::Moved(_) => snap_window_to_edges(window),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            activate_session,
            quit_app,
            set_close_to_tray,
            set_click_through,
            is_autostart_enabled,
            set_autostart,
            set_global_shortcut
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
    fn rejects_unsafe_session_ids() {
        assert!(is_safe_session_id("9adeca97-5414-480b-89ca-153f956e4daa"));
        assert!(!is_safe_session_id("../escape"));
        assert!(!is_safe_session_id("dir/file"));
        assert!(!is_safe_session_id(""));
    }

    fn snap_bounds() -> SnapBounds {
        SnapBounds {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        }
    }

    #[test]
    fn snaps_inside_threshold() {
        let mut edges = SnapEdges::default();
        assert_eq!(
            resolve_snap((5, 5), snap_bounds(), 10, 24, &mut edges),
            (0, 0)
        );
        assert!(edges.left);
        assert!(edges.top);
    }

    #[test]
    fn does_not_snap_outside_threshold() {
        let mut edges = SnapEdges::default();
        assert_eq!(
            resolve_snap((40, 40), snap_bounds(), 10, 24, &mut edges),
            (40, 40)
        );
        assert!(!edges.left);
        assert!(!edges.top);
    }

    #[test]
    fn small_drags_can_pull_window_away_from_edge() {
        let mut edges = SnapEdges::default();
        assert_eq!(
            resolve_snap((3, 100), snap_bounds(), 10, 24, &mut edges),
            (0, 100)
        );
        assert!(edges.left);

        // 仍在吸附距离内的小步拖出，不应被拉回
        assert_eq!(
            resolve_snap((6, 100), snap_bounds(), 10, 24, &mut edges),
            (6, 100)
        );
        assert!(edges.left);

        // 拖出释放距离后解除锁定
        assert_eq!(
            resolve_snap((30, 100), snap_bounds(), 10, 24, &mut edges),
            (30, 100)
        );
        assert!(!edges.left);
    }

    #[test]
    fn snaps_again_after_leaving_edge() {
        let mut edges = SnapEdges::default();
        resolve_snap((3, 100), snap_bounds(), 10, 24, &mut edges);
        resolve_snap((30, 100), snap_bounds(), 10, 24, &mut edges);
        assert_eq!(
            resolve_snap((4, 100), snap_bounds(), 10, 24, &mut edges),
            (0, 100)
        );
        assert!(edges.left);
    }

    #[test]
    fn snaps_to_right_and_bottom_edges() {
        let mut edges = SnapEdges::default();
        assert_eq!(
            resolve_snap((995, 795), snap_bounds(), 10, 24, &mut edges),
            (1000, 800)
        );
        assert!(edges.right);
        assert!(edges.bottom);
    }
}
