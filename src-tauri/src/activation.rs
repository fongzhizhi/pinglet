//! 双击会话时激活已打开的 VSCode / Git Bash 窗口。

use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use serde::Deserialize;
#[cfg(target_os = "windows")]
use std::ffi::OsString;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
use std::process::{Command, Stdio};
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessesToUpdate, System};
#[cfg(target_os = "windows")]
use sysinfo::{Process, ProcessRefreshKind, UpdateKind};
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

/// 进程父链遍历上限，避免异常数据导致死循环。
#[cfg(target_os = "windows")]
const MAX_ANCESTOR_DEPTH: usize = 32;

/// 用目录名匹配 VSCode 窗口标题时，忽略过短路径段（如盘符、单字符目录）以免误命中。
#[cfg(target_os = "windows")]
const MIN_VSCODE_TITLE_SEGMENT_LEN: usize = 3;

/// VSCode 全局存储中记录已打开窗口工作区的相对路径（相对 user-data-dir）。
#[cfg(target_os = "windows")]
const VSCODE_STORAGE_RELATIVE_PATH: &str = "User/globalStorage/storage.json";
/// VSCode CLI 转发 `--reuse-window` 后约 1 秒内退出；超时只影响错误提示，不会撤回已发出的聚焦请求。
#[cfg(target_os = "windows")]
const VSCODE_CLI_TIMEOUT_MS: u64 = 4000;
#[cfg(target_os = "windows")]
const VSCODE_CLI_POLL_INTERVAL_MS: u64 = 50;
/// 创建 CLI 子进程时不弹出控制台窗口（CREATE_NO_WINDOW）。
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 双击会话时只处理 VSCode 与 CLI 两种入口，其他入口直接拒绝。
#[tauri::command]
pub fn activate_session(
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
        activate_vscode_via_window(&directory)
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

/// 只保留进程名确实是 VSCode 的可见窗口，避免误激活其它 Electron 应用。
#[cfg(target_os = "windows")]
fn filter_vscode_windows(
    system: &System,
    candidates: Vec<WindowCandidate>,
) -> Vec<WindowCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| {
            system
                .process(Pid::from_u32(candidate.pid))
                .is_some_and(|process| is_vscode_process_name(&process.name().to_string_lossy()))
        })
        .collect()
}

/// 标题匹配结果：唯一命中才能直接激活；并列意味着同名目录窗口无法仅凭标题区分。
#[cfg(target_os = "windows")]
enum VscodeWindowMatch<'a> {
    Unique(&'a WindowCandidate),
    Ambiguous,
    NoMatch,
}

/// 选择目标 VSCode 窗口：标题命中最深目录段者优先，其次命中段更长者。
#[cfg(target_os = "windows")]
fn select_vscode_window<'a>(
    candidates: &[&'a WindowCandidate],
    directory: &Path,
) -> VscodeWindowMatch<'a> {
    let mut scored: Vec<((usize, usize), &'a WindowCandidate)> = candidates
        .iter()
        .filter_map(|candidate| {
            vscode_title_score(&candidate.title, directory).map(|score| (score, *candidate))
        })
        .collect();
    scored.sort_by_key(|left| std::cmp::Reverse(left.0));

    let Some((best_score, best)) = scored.first().copied() else {
        return VscodeWindowMatch::NoMatch;
    };
    // NOTE: 最高分并列说明标题无法区分（如两个同名目录窗口），交给工作区路径精确定位
    if scored
        .iter()
        .filter(|(score, _)| *score == best_score)
        .count()
        > 1
    {
        return VscodeWindowMatch::Ambiguous;
    }
    VscodeWindowMatch::Unique(best)
}

#[cfg(target_os = "windows")]
fn activate_vscode_windows(
    system: &System,
    windows: &[WindowCandidate],
    directory: &Path,
    allow_unique_fallback: bool,
) -> Result<(), String> {
    let candidates: Vec<&WindowCandidate> = windows.iter().collect();
    if candidates.is_empty() {
        return Err("未找到可激活的 VSCode 窗口".to_string());
    }

    match select_vscode_window(&candidates, directory) {
        VscodeWindowMatch::Unique(target) => return activate_vscode_target(target),
        VscodeWindowMatch::Ambiguous | VscodeWindowMatch::NoMatch => {}
    }

    // NOTE: 标题无法区分同名目录窗口时，只有 VSCode 自己知道窗口与工作区的映射；
    // 以 --reuse-window + folder URI 交给它聚焦，避免激活同名但不同目录的窗口
    if focus_vscode_via_cli(system, windows, directory).is_ok() {
        return Ok(());
    }

    if allow_unique_fallback && candidates.len() == 1 {
        return activate_vscode_target(candidates[0]);
    }
    Err("找到 VSCode，但无法定位该会话所在的窗口".to_string())
}

#[cfg(target_os = "windows")]
fn activate_vscode_target(target: &WindowCandidate) -> Result<(), String> {
    if activate_window(target.hwnd) {
        Ok(())
    } else {
        Err("找到 VSCode 窗口，但无法切换到前台".to_string())
    }
}

/// VSCode 入口：只激活已打开该目录的窗口，不启动新实例、不新开窗口。
#[cfg(target_os = "windows")]
fn activate_vscode_window_by_directory(directory: &Path) -> Result<(), String> {
    let visible = collect_visible_windows();
    if visible.is_empty() {
        return Err("未找到可激活的 VSCode 窗口".to_string());
    }

    // 刷新全部进程：除窗口所属主进程外，还需要子进程命令行里的 user-data-dir
    let mut system = System::new().map_err(|_| "无法读取进程列表".to_string())?;
    system.refresh_processes(ProcessesToUpdate::All, true);

    let windows = filter_vscode_windows(&system, visible);
    activate_vscode_windows(&system, &windows, directory, true)
}

fn activate_vscode_via_window(directory: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        activate_vscode_window_by_directory(directory)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = directory;
        Err("当前平台暂不支持激活窗口".to_string())
    }
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

/// 用会话目录的各级目录名匹配 VSCode 窗口标题，返回（目录深度, 命中段长度）。
///
/// VSCode 标题通常是 `<文件名> - <工作区目录> - Visual Studio Code`，
/// 而会话 cwd 可能位于工作区子目录。更深的目录段比更长的上层目录名更可信，
/// 例如 `C:\work\pinglet\src` 应优先命中标题里的 `src`，而不是 `pinglet`。
#[cfg(target_os = "windows")]
fn vscode_title_score(title: &str, directory: &Path) -> Option<(usize, usize)> {
    let title = title.to_lowercase();
    directory
        .components()
        .enumerate()
        .filter_map(|(depth, component)| {
            let std::path::Component::Normal(name) = component else {
                return None;
            };
            let segment = name.to_str()?.to_lowercase();
            (segment.len() >= MIN_VSCODE_TITLE_SEGMENT_LEN && title.contains(&segment))
                .then_some((depth, segment.len()))
        })
        .max()
}

/// 激活会话所属的 VSCode 窗口；标题无法定位时先按工作区路径精确聚焦，再仅当该进程只有一个窗口才兜底。
#[cfg(target_os = "windows")]
fn focus_vscode_session_window(
    system: &System,
    vscode_pid: u32,
    directory: &Path,
) -> Result<(), String> {
    let candidates: Vec<WindowCandidate> = collect_visible_windows()
        .into_iter()
        .filter(|candidate| candidate.pid == vscode_pid)
        .collect();
    if candidates.is_empty() {
        return Err("找到 VSCode 进程，但未找到可激活的窗口".to_string());
    }

    activate_vscode_windows(system, &candidates, directory, true)
}

/// 通过 VSCode CLI 聚焦已打开目标目录的窗口。
///
/// 只读访问 `<user-data-dir>/User/globalStorage/storage.json`，从中取当前已打开窗口的工作区 folder URI，
/// 再以 `--reuse-window --folder-uri` 让 VSCode 自己定位窗口——同名目录只有它能精确区分。
#[cfg(target_os = "windows")]
fn focus_vscode_via_cli(
    system: &System,
    windows: &[WindowCandidate],
    directory: &Path,
) -> Result<(), String> {
    // 稳定版与 Insiders 可能同时运行，按可执行文件分组，分别用自己的存储与 CLI 定位
    let mut executables: Vec<PathBuf> = Vec::new();
    for window in windows {
        if let Some(executable) = system
            .process(Pid::from_u32(window.pid))
            .and_then(Process::exe)
        {
            push_unique_path(&mut executables, executable.to_path_buf());
        }
    }
    if executables.is_empty() {
        return Err("未找到 VSCode 可执行文件".to_string());
    }

    let mut last_error = String::from("VSCode 已打开的工作区中没有匹配该目录的窗口");
    for executable in executables {
        let folder_uri = vscode_user_data_dirs(system, &executable)
            .into_iter()
            .find_map(|user_data_dir| {
                let storage_path = user_data_dir.join(VSCODE_STORAGE_RELATIVE_PATH);
                let content = std::fs::read_to_string(storage_path).ok()?;
                select_workspace_folder_uri(&parse_open_workspace_folders(&content), directory)
            });
        let Some(folder_uri) = folder_uri else {
            continue;
        };
        match run_vscode_reuse_window(&executable, &folder_uri) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// 汇总候选 user-data-dir：优先采用进程命令行里的 `--user-data-dir`（支持自定义安装），
/// 其次是便携版的 `data/user-data` 与常规安装的 `%APPDATA%\Code`。
#[cfg(target_os = "windows")]
fn vscode_user_data_dirs(system: &System, executable: &Path) -> Vec<PathBuf> {
    let executable_key = normalize_path_key(&executable.to_string_lossy());
    let mut directories: Vec<PathBuf> = Vec::new();
    for process in system.processes().values() {
        // 只采用与目标安装相同的进程，避免稳定版 / Insiders 的 user-data-dir 串用
        let is_same_install = process
            .exe()
            .is_some_and(|path| normalize_path_key(&path.to_string_lossy()) == executable_key);
        if !is_same_install {
            continue;
        }
        if let Some(directory) = user_data_dir_from_arguments(process.cmd()) {
            push_unique_path(&mut directories, directory);
        }
    }

    let is_insiders = executable.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .to_ascii_lowercase()
            .contains("insiders")
    });
    if let Some(parent) = executable.parent() {
        // 便携版把用户数据放在安装目录的 data/user-data 下
        push_unique_path(&mut directories, parent.join("data").join("user-data"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let appdata = PathBuf::from(appdata);
        let profile = if is_insiders {
            "Code - Insiders"
        } else {
            "Code"
        };
        push_unique_path(&mut directories, appdata.join(profile));
        push_unique_path(&mut directories, appdata.join("Code - Insiders"));
    }
    directories
}

#[cfg(target_os = "windows")]
fn push_unique_path(directories: &mut Vec<PathBuf>, directory: PathBuf) {
    if !directories.contains(&directory) {
        directories.push(directory);
    }
}

/// 解析 `--user-data-dir=<路径>` / `--user-data-dir <路径>` 参数（值可能带引号）。
#[cfg(target_os = "windows")]
fn user_data_dir_from_arguments(arguments: &[OsString]) -> Option<PathBuf> {
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let value = argument.to_string_lossy();
        if let Some(directory) = value.strip_prefix("--user-data-dir=") {
            let directory = directory.trim_matches('"');
            if !directory.is_empty() {
                return Some(PathBuf::from(directory));
            }
        }
        if value == "--user-data-dir" {
            let directory = arguments.next()?.to_string_lossy();
            let directory = directory.trim_matches('"');
            if !directory.is_empty() {
                return Some(PathBuf::from(directory));
            }
        }
    }
    None
}

/// VSCode 全局存储中与窗口工作区相关的字段。
#[cfg(target_os = "windows")]
#[derive(Deserialize)]
struct VscodeStorageSnapshot {
    #[serde(rename = "windowsState")]
    windows_state: Option<VscodeWindowsState>,
}

#[cfg(target_os = "windows")]
#[derive(Deserialize)]
struct VscodeWindowsState {
    #[serde(rename = "openedWindows")]
    opened_windows: Option<Vec<VscodeWindowEntry>>,
    #[serde(rename = "lastActiveWindow")]
    last_active_window: Option<VscodeWindowEntry>,
}

#[cfg(target_os = "windows")]
#[derive(Deserialize)]
struct VscodeWindowEntry {
    folder: Option<String>,
}

/// 提取当前已打开窗口的工作区 folder URI。
/// NOTE: 只读取文件夹路径；多根工作区记录在 workspace 字段，无法直接对应单个目录，暂不处理。
#[cfg(target_os = "windows")]
fn parse_open_workspace_folders(content: &str) -> Vec<String> {
    let Ok(storage) = serde_json::from_str::<VscodeStorageSnapshot>(content) else {
        return Vec::new();
    };
    let Some(state) = storage.windows_state else {
        return Vec::new();
    };

    let mut folders: Vec<String> = Vec::new();
    let entries = state
        .opened_windows
        .unwrap_or_default()
        .into_iter()
        .chain(state.last_active_window);
    for entry in entries {
        let Some(folder) = entry.folder.filter(|folder| !folder.is_empty()) else {
            continue;
        };
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
}

/// 选择与目录匹配的已打开工作区：cwd 等于工作区目录或位于其子目录，取匹配最深者。
#[cfg(target_os = "windows")]
fn select_workspace_folder_uri(folders: &[String], directory: &Path) -> Option<String> {
    let target = normalize_path_key(&directory.to_string_lossy());
    if target.is_empty() {
        return None;
    }

    let mut best: Option<(usize, &String)> = None;
    for folder in folders {
        let Some(path) = file_uri_to_path(folder) else {
            continue;
        };
        let key = normalize_path_key(&path);
        // 排除盘符根（如 `d:`），否则任意目录都会命中
        if key.len() <= 2 {
            continue;
        }
        let is_match = target == key
            || target
                .strip_prefix(&key)
                .is_some_and(|rest| rest.starts_with('\\'));
        if !is_match {
            continue;
        }
        if best.is_none_or(|(best_length, _)| key.len() > best_length) {
            best = Some((key.len(), folder));
        }
    }
    best.map(|(_, folder)| folder.clone())
}

/// `file:///d%3A/MyWorking/...` → `d:\MyWorking\...`；非本地 file URI 返回 None。
#[cfg(target_os = "windows")]
fn file_uri_to_path(uri: &str) -> Option<String> {
    let encoded = uri.strip_prefix("file:///")?;
    let decoded = String::from_utf8(percent_decode(encoded.as_bytes())?).ok()?;
    if decoded.is_empty() {
        return None;
    }
    Some(decoded.replace('/', "\\"))
}

#[cfg(target_os = "windows")]
fn percent_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' {
            let high = hex_digit(*input.get(index + 1)?)?;
            let low = hex_digit(*input.get(index + 2)?)?;
            output.push(high * 16 + low);
            index += 3;
        } else {
            output.push(input[index]);
            index += 1;
        }
    }
    Some(output)
}

#[cfg(target_os = "windows")]
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 定位 VSCode CLI 入口 `resources/app/out/cli.js`。
/// 常规安装把资源放在带版本哈希的子目录（如 `69047191fc/resources/app/out/cli.js`）。
#[cfg(target_os = "windows")]
fn find_vscode_cli_script(executable: &Path) -> Option<PathBuf> {
    let install_dir = executable.parent()?;
    let direct = install_dir
        .join("resources")
        .join("app")
        .join("out")
        .join("cli.js");
    if direct.is_file() {
        return Some(direct);
    }

    std::fs::read_dir(install_dir)
        .ok()?
        .flatten()
        .map(|entry| {
            entry
                .path()
                .join("resources")
                .join("app")
                .join("out")
                .join("cli.js")
        })
        .find(|candidate| candidate.is_file())
}

/// 让已在运行的 VSCode 复用并聚焦目标工作区窗口，不等待新窗口创建。
/// NOTE: 直接以 Code.exe 加参数会报 bad option，必须用 ELECTRON_RUN_AS_NODE 运行 `cli.js` 转发请求。
#[cfg(target_os = "windows")]
fn run_vscode_reuse_window(executable: &Path, folder_uri: &str) -> Result<(), String> {
    let cli_script =
        find_vscode_cli_script(executable).ok_or_else(|| "未找到 VSCode CLI 入口".to_string())?;
    let mut command = Command::new(executable);
    command
        .env("ELECTRON_RUN_AS_NODE", "1")
        .arg(&cli_script)
        .arg("--reuse-window")
        .arg("--folder-uri")
        .arg(folder_uri)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 VSCode 定位窗口: {error}"))?;
    let deadline = Instant::now() + Duration::from_millis(VSCODE_CLI_TIMEOUT_MS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("VSCode 命令异常退出: {status}")),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(VSCODE_CLI_POLL_INTERVAL_MS));
            }
            Ok(None) => return Err("等待 VSCode 定位窗口超时".to_string()),
            Err(error) => return Err(format!("等待 VSCode 命令失败: {error}")),
        }
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
        Err("当前平台暂不支持激活窗口".to_string())
    }
}

#[cfg(target_os = "windows")]
fn focus_existing_terminal(
    directory: &Path,
    name: Option<&str>,
    pid: Option<u32>,
) -> Result<(), String> {
    let target = normalize_path_key(&directory.to_string_lossy());
    let mut failures: Vec<String> = Vec::new();
    // 唯一 VSCode 窗口兜底只对确认属于 VSCode 上下文的会话启用，避免把 Windows Terminal 会话切错窗口
    let mut has_vscode_context = false;

    let Ok(mut system) = System::new() else {
        return Err("无法读取进程列表".to_string());
    };
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );

    // 1. VSCode 集成终端：按进程链找到所属 VSCode 并激活
    if let Some(vscode_pid) = pid.and_then(|session_pid| find_vscode_ancestor(&system, session_pid))
    {
        has_vscode_context = true;
        match focus_vscode_session_window(&system, vscode_pid, directory) {
            Ok(()) => return Ok(()),
            Err(err) => failures.push(err),
        }
    }

    // 2. Git Bash 由 git-bash.exe 启动 mintty，启动目录保留在 --cd 参数中
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

    let candidates = collect_visible_windows();

    if !directory_matched_pids.is_empty() {
        let directory_candidates: Vec<&WindowCandidate> = candidates
            .iter()
            .filter(|candidate| directory_matched_pids.contains(&candidate.pid))
            .collect();
        if !directory_candidates.is_empty() {
            match activate_first(&directory_candidates) {
                Ok(()) => return Ok(()),
                Err(err) => failures.push(err),
            }
        }
    }

    // 3. 启动目录不足以定位时（例如在窗口内手动 cd），再用 Claude Code 写入的终端标题匹配
    if let Some(pattern) = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
    {
        let title_candidates: Vec<&WindowCandidate> = candidates
            .iter()
            .filter(|candidate| mintty_pids.contains(&candidate.pid))
            .filter(|candidate| candidate.title.to_lowercase().contains(&pattern))
            .collect();
        if !title_candidates.is_empty() {
            match activate_first(&title_candidates) {
                Ok(()) => return Ok(()),
                Err(err) => failures.push(err),
            }
        } else {
            failures.push("未匹配到该会话的 Git Bash 窗口".to_string());
        }
    }

    if mintty_pids.is_empty() {
        failures.push("未找到 Git Bash 窗口，该会话可能未运行在 Git Bash 中".to_string());
    }

    // 4. 全局 VSCode 兜底：标题命中始终可尝试；唯一窗口只在确认 VSCode 上下文时兜底
    let vscode_windows = filter_vscode_windows(&system, candidates);
    match activate_vscode_windows(&system, &vscode_windows, directory, has_vscode_context) {
        Ok(()) => Ok(()),
        Err(err) => {
            failures.push(err);
            Err(format!("未能激活会话窗口；已尝试: {}", failures.join("；")))
        }
    }
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
        .trim_end_matches(['\\', '.'])
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    fn vscode_candidate(pid: u32, title: &str) -> WindowCandidate {
        WindowCandidate {
            hwnd: std::ptr::null_mut(),
            pid,
            title: title.to_string(),
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn vscode_title_score_prefers_deeper_directory_segment() {
        let directory = Path::new(r"C:\work\pinglet\src");

        // 标题同时命中 pinglet 与 src 时，更深的 src 得分更高
        let deep = vscode_title_score("main.ts - src - pinglet - Visual Studio Code", directory);
        let shallow = vscode_title_score("README - pinglet - Visual Studio Code", directory);
        assert!(deep > shallow);

        // 忽略过短的路径段，避免盘符或单字符目录误命中
        assert!(vscode_title_score("c - Visual Studio Code", directory).is_none());
        assert!(vscode_title_score("其它项目 - Visual Studio Code", directory).is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_vscode_window_prefers_title_match_over_others() {
        let directory = Path::new(r"C:\work\pinglet\src");
        let unmatched_a = vscode_candidate(1, "未知项目 - Visual Studio Code");
        let unmatched_b = vscode_candidate(2, "另一个项目 - Visual Studio Code");
        let matched = vscode_candidate(3, "main.ts - src - pinglet - Visual Studio Code");
        let candidates = [&unmatched_a, &unmatched_b, &matched];

        let VscodeWindowMatch::Unique(selected) = select_vscode_window(&candidates, directory)
        else {
            panic!("expected unique title match");
        };
        assert_eq!(selected.pid, 3);

        // 没有标题命中时不猜测
        let unmatched = [&unmatched_a, &unmatched_b];
        assert!(matches!(
            select_vscode_window(&unmatched, directory),
            VscodeWindowMatch::NoMatch
        ));
        assert!(matches!(
            select_vscode_window(&[], directory),
            VscodeWindowMatch::NoMatch
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_vscode_window_prefers_deeper_directory_match() {
        let directory = Path::new(r"C:\work\pinglet\src");
        let workspace = vscode_candidate(1, "README - pinglet - Visual Studio Code");
        let nested = vscode_candidate(2, "main.ts - src - pinglet - Visual Studio Code");
        let candidates = [&workspace, &nested];

        let VscodeWindowMatch::Unique(selected) = select_vscode_window(&candidates, directory)
        else {
            panic!("expected deeper match");
        };
        assert_eq!(selected.pid, 2);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_vscode_window_reports_ambiguous_same_name_directories() {
        let directory = Path::new(r"C:\work\github\pinglet");
        // 两个窗口标题都只含同名目录段，无法仅凭标题区分
        let first = vscode_candidate(1, "main.ts - pinglet - Visual Studio Code");
        let second = vscode_candidate(2, "README - pinglet - Visual Studio Code");
        let candidates = [&first, &second];

        assert!(matches!(
            select_vscode_window(&candidates, directory),
            VscodeWindowMatch::Ambiguous
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn file_uri_to_path_decodes_drive_and_escaped_characters() {
        assert_eq!(
            file_uri_to_path("file:///d%3A/MyWorking/repository/github/pinglet").as_deref(),
            Some(r"d:\MyWorking\repository\github\pinglet")
        );
        assert_eq!(
            file_uri_to_path("file:///c%3A/Users/Administrator/.claude").as_deref(),
            Some(r"c:\Users\Administrator\.claude")
        );
        // UTF-8 百分号编码（中文路径）与百分号字面量
        assert_eq!(
            file_uri_to_path("file:///d%3A/work/%E4%B8%AD%E6%96%87%20dir").as_deref(),
            Some(r"d:\work\中文 dir")
        );
        assert!(file_uri_to_path("vscode-remote://ssh-remote/home/user").is_none());
        assert!(file_uri_to_path("file:///d%3A/broken%ZZ").is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn parse_open_workspace_folders_collects_opened_and_last_active() {
        let content = r#"{
            "windowsState": {
                "lastActiveWindow": { "folder": "file:///d%3A/work/github/pinglet" },
                "openedWindows": [
                    { "folder": "file:///d%3A/work/gitlab/pinglet" },
                    { "folder": "file:///d%3A/work/github/pinglet" },
                    { "workspace": "file:///d%3A/work/multi.code-workspace" }
                ]
            }
        }"#;
        assert_eq!(
            parse_open_workspace_folders(content),
            vec![
                "file:///d%3A/work/gitlab/pinglet".to_string(),
                "file:///d%3A/work/github/pinglet".to_string(),
            ]
        );
        assert!(parse_open_workspace_folders("not json").is_empty());
        assert!(parse_open_workspace_folders("{}").is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_workspace_folder_uri_matches_exact_and_nested_directories() {
        let folders = vec![
            "file:///d%3A/work/github/pinglet".to_string(),
            "file:///d%3A/work/gitlab/pinglet".to_string(),
        ];

        // 精确匹配同名但不同父目录的工作区
        assert_eq!(
            select_workspace_folder_uri(&folders, Path::new(r"d:\work\github\pinglet")).as_deref(),
            Some("file:///d%3A/work/github/pinglet")
        );
        // cwd 位于工作区子目录时匹配父工作区
        assert_eq!(
            select_workspace_folder_uri(&folders, Path::new(r"d:\work\gitlab\pinglet\src"))
                .as_deref(),
            Some("file:///d%3A/work/gitlab/pinglet")
        );
        // 不在任何工作区内时不猜测
        assert!(select_workspace_folder_uri(&folders, Path::new(r"d:\work\other")).is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_workspace_folder_uri_prefers_deepest_match() {
        let folders = vec![
            "file:///d%3A/work/pinglet".to_string(),
            "file:///d%3A/work/pinglet/src".to_string(),
        ];
        assert_eq!(
            select_workspace_folder_uri(&folders, Path::new(r"d:\work\pinglet\src\ui")).as_deref(),
            Some("file:///d%3A/work/pinglet/src")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_workspace_folder_uri_ignores_drive_root_and_remote_uris() {
        let folders = vec![
            "file:///d%3A/".to_string(),
            "vscode-remote://ssh-remote/home/user/project".to_string(),
        ];
        assert!(select_workspace_folder_uri(&folders, Path::new(r"d:\work\pinglet")).is_none());
    }
}
