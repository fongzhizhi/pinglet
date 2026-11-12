//! 双击会话时激活已打开的 VSCode / Git Bash 窗口。

use std::path::{Path, PathBuf};

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

/// 选择目标 VSCode 窗口：标题命中最深目录段者优先，其次命中段更长者；
/// 没有任何标题命中时，仅在允许且候选唯一的情况下兜底。
#[cfg(target_os = "windows")]
fn select_vscode_window<'a>(
    candidates: &[&'a WindowCandidate],
    directory: &Path,
    allow_unique_fallback: bool,
) -> Result<&'a WindowCandidate, String> {
    if candidates.is_empty() {
        return Err("未找到可激活的 VSCode 窗口".to_string());
    }

    let mut scored: Vec<((usize, usize), &'a WindowCandidate)> = candidates
        .iter()
        .filter_map(|candidate| {
            vscode_title_score(&candidate.title, directory).map(|score| (score, *candidate))
        })
        .collect();
    scored.sort_by_key(|left| std::cmp::Reverse(left.0));

    if let Some((_, target)) = scored.first() {
        return Ok(*target);
    }
    if allow_unique_fallback && candidates.len() == 1 {
        return Ok(candidates[0]);
    }
    Err("找到 VSCode，但无法定位该会话所在的窗口".to_string())
}

#[cfg(target_os = "windows")]
fn activate_vscode_windows(
    windows: &[WindowCandidate],
    directory: &Path,
    allow_unique_fallback: bool,
) -> Result<(), String> {
    let candidates: Vec<&WindowCandidate> = windows.iter().collect();
    let target = select_vscode_window(&candidates, directory, allow_unique_fallback)?;
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

    // 同一进程可能有多个顶层窗口；重复 PID 会让 sysinfo 漏掉该进程
    let mut pids: Vec<Pid> = visible
        .iter()
        .map(|candidate| Pid::from_u32(candidate.pid))
        .collect();
    pids.sort_unstable();
    pids.dedup();
    let mut system = System::new().map_err(|_| "无法读取进程列表".to_string())?;
    system.refresh_processes(ProcessesToUpdate::Some(&pids), true);

    let windows = filter_vscode_windows(&system, visible);
    activate_vscode_windows(&windows, directory, true)
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

    let refs: Vec<&WindowCandidate> = candidates.iter().collect();
    let target = select_vscode_window(&refs, directory, true)?;
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
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );

    // 1. VSCode 集成终端：按进程链找到所属 VSCode 并激活
    if let Some(vscode_pid) = pid.and_then(|session_pid| find_vscode_ancestor(&system, session_pid))
    {
        has_vscode_context = true;
        match focus_vscode_session_window(vscode_pid, directory) {
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
    match activate_vscode_windows(&vscode_windows, directory, has_vscode_context) {
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
    fn select_vscode_window_prefers_title_match_over_fallback() {
        let directory = Path::new(r"C:\work\pinglet\src");
        let unmatched_a = vscode_candidate(1, "未知项目 - Visual Studio Code");
        let unmatched_b = vscode_candidate(2, "另一个项目 - Visual Studio Code");
        let matched = vscode_candidate(3, "main.ts - src - pinglet - Visual Studio Code");
        let candidates = [&unmatched_a, &unmatched_b, &matched];

        let selected = select_vscode_window(&candidates, directory, false).expect("title match");
        assert_eq!(selected.pid, 3);

        // 唯一窗口但未命中标题：仅在允许兜底时激活
        let unique = [&unmatched_a];
        assert!(select_vscode_window(&unique, directory, false).is_err());
        assert_eq!(
            select_vscode_window(&unique, directory, true)
                .expect("unique fallback")
                .pid,
            1
        );

        // 多个候选都未命中时拒绝猜测
        let ambiguous = [&unmatched_a, &unmatched_b];
        assert!(select_vscode_window(&ambiguous, directory, true).is_err());
        assert!(select_vscode_window(&[], directory, true).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn select_vscode_window_prefers_deeper_directory_match() {
        let directory = Path::new(r"C:\work\pinglet\src");
        let workspace = vscode_candidate(1, "README - pinglet - Visual Studio Code");
        let nested = vscode_candidate(2, "main.ts - src - pinglet - Visual Studio Code");
        let candidates = [&workspace, &nested];

        let selected = select_vscode_window(&candidates, directory, false).expect("deeper match");
        assert_eq!(selected.pid, 2);
    }
}
