//! 主窗口显隐、边缘吸附、鼠标穿透与背景亮度采样。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};
use tauri_plugin_window_state::{AppHandleExt as WindowStateAppHandleExt, StateFlags};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Graphics::Gdi::{CLR_INVALID, GetDC, GetPixel, ReleaseDC};

use crate::tray::{refresh_tray, sync_show_item};
use crate::{
    AppState, EVENT_WINDOW_HIDE_REQUEST, EVENT_WINDOW_SHOW_REQUEST, EVENT_WINDOW_VISIBILITY,
    WINDOW_LABEL,
};

/// 前端收起动画约 160ms；超过这个时间仍未收到隐藏命令就直接隐藏，避免窗口卡在可见态。
const WINDOW_HIDE_FALLBACK: Duration = Duration::from_millis(260);
const SNAP_DISTANCE_LOGICAL: f64 = 10.0;
/// 已吸附后需要拖出的距离；大于吸附距离，避免小步移动被反复拉回边缘。
const SNAP_RELEASE_DISTANCE_LOGICAL: f64 = 24.0;
/// 默认窗口宽度（逻辑像素），与 tauri.conf.json / 前端默认值保持一致。
const DEFAULT_WINDOW_WIDTH_LOGICAL: f64 = 320.0;
/// 自动主题采样：窗口底边下方 6 逻辑像素处横向取 12 个点，取亮度中位数。
const BACKDROP_SAMPLE_OFFSET_LOGICAL: f64 = 6.0;
const BACKDROP_SAMPLE_COUNT: i32 = 12;
/// 当前吸附在哪些边缘；用于拖出时保持迟滞，避免每帧都被吸回。
#[derive(Clone, Copy, Default)]
pub(crate) struct SnapEdges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}
/// 切换窗口鼠标穿透，并同步托盘勾选状态与前端设置。
/// NOTE: 开启后窗口无法再接收点击，托盘菜单与全局快捷键是恢复入口（见 `toggle_main_window`）。
pub(crate) fn apply_click_through(
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

    if let Ok(item) = state.click_through_item.lock()
        && let Some(item) = item.as_ref()
        && let Err(err) = item.set_checked(enabled)
    {
        eprintln!("pinglet: 同步托盘穿透勾选状态失败: {err}");
    }
    if let Err(err) = app.emit("pinglet://click-through-changed", enabled) {
        eprintln!("pinglet: 广播鼠标穿透状态失败: {err}");
    }

    Ok(())
}
/// 恢复默认位置（主屏右上角）并把位置写回 window-state，下次启动不再使用旧坐标。
#[tauri::command]
pub fn reset_window_position(app: tauri::AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return Err("主窗口不可用".to_string());
    };

    position_window_top_right(&window);
    // NOTE: set_position 是投递到事件循环的异步消息；用一次同步 getter 等待它生效，
    // 再写状态文件，避免保存到重置前的坐标。
    let _ = window.outer_position();
    app.save_window_state(StateFlags::POSITION)
        .map_err(|err| format!("保存窗口位置失败: {err}"))
}

/// 采样窗口正下方（工作区内）的背景亮度，供自动主题判定深浅色。
/// NOTE: 窗口贴近工作区底边或采样点无效时返回 None，前端保持当前主题。
#[tauri::command]
pub fn sample_backdrop_luminance(window: tauri::WebviewWindow) -> Option<f32> {
    #[cfg(target_os = "windows")]
    {
        sample_backdrop_luminance_windows(&window)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        None
    }
}

fn median_luminance(samples: &mut [f32]) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(f32::total_cmp);
    let middle = samples.len() / 2;
    if samples.len().is_multiple_of(2) {
        Some((samples[middle - 1] + samples[middle]) / 2.0)
    } else {
        Some(samples[middle])
    }
}

#[cfg(target_os = "windows")]
fn sample_backdrop_luminance_windows(window: &tauri::WebviewWindow) -> Option<f32> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let monitor = window.current_monitor().ok()??;
    let work_area = monitor.work_area();
    let work_left = work_area.position.x;
    let work_top = work_area.position.y;
    let work_right = work_left + work_area.size.width as i32;
    let work_bottom = work_top + work_area.size.height as i32;

    let offset = (BACKDROP_SAMPLE_OFFSET_LOGICAL * monitor.scale_factor()).round() as i32;
    let sample_y = position.y + size.height as i32 + offset;
    if sample_y < work_top || sample_y >= work_bottom {
        return None;
    }

    let window_left = position.x.max(work_left);
    let window_right = (position.x + size.width as i32).min(work_right);
    if window_right <= window_left {
        return None;
    }

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    if screen_dc.is_null() {
        return None;
    }

    let span = window_right - window_left;
    let mut samples = Vec::with_capacity(BACKDROP_SAMPLE_COUNT as usize);
    for index in 0..BACKDROP_SAMPLE_COUNT {
        let offset_x =
            (span as f64 * (index as f64 + 0.5) / f64::from(BACKDROP_SAMPLE_COUNT)) as i32;
        let sample_x = (window_left + offset_x).clamp(work_left, work_right - 1);
        let color = unsafe { GetPixel(screen_dc, sample_x, sample_y) };
        if color == CLR_INVALID {
            continue;
        }
        let red = (color & 0xFF) as f32 / 255.0;
        let green = ((color >> 8) & 0xFF) as f32 / 255.0;
        let blue = ((color >> 16) & 0xFF) as f32 / 255.0;
        samples.push(0.2126 * red + 0.7152 * green + 0.0722 * blue);
    }
    unsafe { ReleaseDC(std::ptr::null_mut(), screen_dc) };

    median_luminance(&mut samples)
}
/// 读取窗口真实显隐并广播给前端；窗口回到前台时清除托盘未读提醒。
/// NOTE: Windows 上 WebView2 在窗口隐藏后会节流后台计时器，前端因此不能作为
/// 显隐的权威来源；轮询与心跳的启停统一由这里发出的 `pinglet://window-visibility` 决定。
pub(crate) fn sync_window_activity(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let active = visible && !window.is_minimized().unwrap_or(false);

    let state = app.state::<AppState>();
    let previous = state.window_active.swap(active, Ordering::Relaxed);
    sync_show_item(&state, visible);

    if active && state.tray_alert_unread.swap(false, Ordering::Relaxed) {
        refresh_tray(app, Instant::now());
    }

    if previous != active
        && let Err(err) = window.emit(EVENT_WINDOW_VISIBILITY, active)
    {
        eprintln!("pinglet: 广播窗口可见性失败: {err}");
    }
}

/// 真正隐藏主窗口；调用方负责代号校验。
fn hide_main_window_internal(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return Ok(());
    };
    if !window.is_visible().unwrap_or(false) {
        return Ok(());
    }
    window
        .hide()
        .map_err(|err| format!("隐藏窗口失败: {err}"))?;
    sync_window_activity(app);
    Ok(())
}

/// 隐藏入口：先通知前端播放收起动画；前端无响应时由 260ms 兜底直接隐藏。
/// NOTE: 每次显示都会递增代号（见 `show_main_window`），延迟隐藏不会误杀刚重新显示的窗口。
fn request_hide_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }

    let state = app.state::<AppState>();
    let generation = state.window_hide_generation.fetch_add(1, Ordering::Relaxed) + 1;

    if let Err(err) = window.emit(EVENT_WINDOW_HIDE_REQUEST, generation) {
        eprintln!("pinglet: 广播窗口隐藏请求失败: {err}");
        if let Err(err) = hide_main_window_internal(app) {
            eprintln!("pinglet: {err}");
        }
        return;
    }

    let thread_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("pinglet-window-hide".to_string())
        .spawn(move || {
            std::thread::sleep(WINDOW_HIDE_FALLBACK);
            let state = thread_app.state::<AppState>();
            if state.window_hide_generation.load(Ordering::Relaxed) != generation {
                return;
            }
            if let Err(err) = hide_main_window_internal(&thread_app) {
                eprintln!("pinglet: 兜底{err}");
            }
        });
    if let Err(err) = spawned {
        eprintln!("pinglet: 启动隐藏兜底线程失败: {err}");
    }
}

/// 显示并聚焦主窗口：穿透状态下恢复可点击，最小化时先还原。
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    let state = app.state::<AppState>();
    // 先递增代号，取消可能仍在等待的延迟隐藏
    state.window_hide_generation.fetch_add(1, Ordering::Relaxed);

    // 穿透状态下窗口收不到任何点击，必须先恢复可点击
    if state.click_through.load(Ordering::Relaxed)
        && let Err(err) = apply_click_through(app, &state, false)
    {
        eprintln!("pinglet: 关闭鼠标穿透失败: {err}");
    }
    if window.is_minimized().unwrap_or(false)
        && let Err(err) = window.unminimize()
    {
        eprintln!("pinglet: 还原窗口失败: {err}");
    }
    if let Err(err) = window.show() {
        eprintln!("pinglet: 显示窗口失败: {err}");
        return;
    }
    if let Err(err) = window.set_focus() {
        eprintln!("pinglet: 聚焦窗口失败: {err}");
    }
    // show() 之后通知前端从收起态展开；前端未收起时该请求无副作用
    if let Err(err) = window.emit(EVENT_WINDOW_SHOW_REQUEST, ()) {
        eprintln!("pinglet: 广播窗口显示请求失败: {err}");
    }
    sync_window_activity(app);
}

/// 托盘「显示」菜单：只切换窗口显隐，勾选状态跟随实际可见性。
pub(crate) fn toggle_window_visibility(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };

    match window.is_visible() {
        Ok(true) => request_hide_main_window(app),
        Ok(false) => show_main_window(app),
        Err(err) => eprintln!("pinglet: 读取窗口可见性失败: {err}"),
    }
}

/// 托盘「设置」菜单：显示窗口并打开设置面板；穿透状态下先关闭穿透，否则面板无法点击。
pub(crate) fn open_settings(app: &tauri::AppHandle) {
    show_main_window(app);
    if let Err(err) = app.emit("pinglet://open-settings", ()) {
        eprintln!("pinglet: 广播打开设置失败: {err}");
    }
}

pub(crate) fn toggle_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };

    // 最小化时 is_visible() 仍为 true，先还原而不是隐藏
    if window.is_minimized().unwrap_or(false) {
        show_main_window(app);
        return;
    }

    let state = app.state::<AppState>();
    // 穿透状态下窗口收不到任何点击，托盘左键与全局快捷键优先恢复可点击状态
    if state.click_through.load(Ordering::Relaxed) {
        show_main_window(app);
        return;
    }

    match window.is_visible() {
        Ok(true) => request_hide_main_window(app),
        Ok(false) => show_main_window(app),
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

pub(crate) fn snap_window_to_edges(window: &tauri::Window) {
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

    if (x != position.x || y != position.y)
        && let Err(err) = window.set_position(tauri::PhysicalPosition::new(x, y))
    {
        eprintln!("pinglet: 窗口吸附失败: {err}");
    }
}

pub(crate) fn position_window_top_right(window: &tauri::WebviewWindow) {
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

/// 恢复的位置可能来自已断开的显示器；必要时平移回当前显示器工作区。
pub(crate) fn clamp_window_into_work_area(window: &tauri::WebviewWindow) {
    let Ok(position) = window.outer_position() else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let monitor = match window.current_monitor() {
        Ok(Some(monitor)) => Some(monitor),
        _ => window.primary_monitor().ok().flatten(),
    };
    let Some(monitor) = monitor else {
        return;
    };

    let work_area = monitor.work_area();
    let left = work_area.position.x;
    let top = work_area.position.y;
    let right = left + work_area.size.width as i32;
    let bottom = top + work_area.size.height as i32;
    let width = size.width as i32;
    let height = size.height as i32;

    let x = position.x.clamp(left, (right - width).max(left));
    let y = position.y.clamp(top, (bottom - height).max(top));
    if (x != position.x || y != position.y)
        && let Err(err) = window.set_position(tauri::PhysicalPosition::new(x, y))
    {
        eprintln!("pinglet: 修正窗口位置失败: {err}");
    }
}
/// 收起动画结束后由前端调用；代号过期说明窗口已重新显示，直接忽略。
#[tauri::command]
pub fn hide_main_window_now(app: tauri::AppHandle, generation: u64) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.window_hide_generation.load(Ordering::Relaxed) != generation {
        return Ok(());
    }
    // 本次隐藏已执行，递增代号让兜底线程与迟到的重复调用失效
    state.window_hide_generation.fetch_add(1, Ordering::Relaxed);
    hide_main_window_internal(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    #[test]
    fn median_luminance_handles_odd_and_even_samples() {
        assert_eq!(median_luminance(&mut []), None);
        assert_eq!(median_luminance(&mut [0.9, 0.1, 0.5]), Some(0.5));
        assert_eq!(median_luminance(&mut [0.4, 0.2]), Some(0.3));
    }
}
