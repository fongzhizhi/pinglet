//! 会话悬浮详情：独立透明置顶小窗，避免详情被主面板尺寸裁切。

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::window::{hide_auxiliary_window, show_auxiliary_window};
use crate::{APP_NAME, AppState, EVENT_TOOLTIP_SHOW, TOOLTIP_WINDOW_LABEL, WINDOW_LABEL};

/// 详情内容的最大宽度（逻辑像素）：超过后文本换行，窗口按实测内容收缩。
const TOOLTIP_MAX_WIDTH_LOGICAL: f64 = 360.0;
/// 内容宽度的下限，避免极端短文本让窗口过窄。
const TOOLTIP_MIN_WIDTH_LOGICAL: f64 = 120.0;
/// 内容高度的下限；上限由所在显示器工作区决定。
const TOOLTIP_MIN_HEIGHT_LOGICAL: f64 = 32.0;
/// 创建窗口时的初始尺寸：足够宽，避免内容先被压缩后再重排。
const TOOLTIP_INITIAL_WIDTH_LOGICAL: f64 = 380.0;
const TOOLTIP_INITIAL_HEIGHT_LOGICAL: f64 = 480.0;
/// 鼠标与详情窗口之间的间距（逻辑像素），与主窗口内 tooltip 的定位间距一致。
const TOOLTIP_OFFSET_LOGICAL: f64 = 12.0;

/// 主窗口传给详情页的内容；字段与前端 `SessionTooltip` 一致。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TooltipPayload {
    title: String,
    lines: Vec<String>,
    hint: Option<String>,
}

/// 详情页加载 / 重新显示时拉取的内容与代号。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TooltipRequestSnapshot {
    payload: TooltipPayload,
    generation: u64,
}

/// 主窗口悬停到新会话时更新内容；页面渲染并实测尺寸后回调 `resize_tooltip_window`。
/// NOTE: 必须用 async 命令：同步命令在 IPC 主线程执行，而首次创建窗口要等主线程处理构建请求，
/// 在命令线程里同步等待会死锁。
#[tauri::command]
pub async fn show_tooltip_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    payload: TooltipPayload,
    pointer_x: f64,
    pointer_y: f64,
) -> Result<(), String> {
    let generation = state.tooltip_generation.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut slot) = state.tooltip_payload.lock() {
        *slot = Some(payload);
    }
    if let Ok(mut slot) = state.tooltip_pointer.lock() {
        *slot = Some((pointer_x, pointer_y));
    }
    if let Ok(mut slot) = state.tooltip_size.lock() {
        *slot = None;
    }

    let is_new_window = app.get_webview_window(TOOLTIP_WINDOW_LABEL).is_none();
    let window = ensure_tooltip_window(&app)?;
    if is_new_window {
        // NOTE: 首次创建时先显示窗口，让 WebView2 建立透明合成；此时窗口透明且无内容，
        // 不会产生可见闪烁，页面渲染后由 resize_tooltip_window 重新定位。
        if let Err(err) = show_auxiliary_window(&window) {
            eprintln!("pinglet: 预显示悬浮详情失败: {err}");
        }
    } else if let Err(err) = hide_auxiliary_window(&window) {
        // 旧内容先隐藏，等页面重新渲染并上报尺寸后再显示，避免闪一帧上一个会话的详情
        eprintln!("pinglet: 隐藏旧悬浮详情失败: {err}");
    }
    // NOTE: 首次创建时事件可能早于页面加载；页面加载后会主动调用 get_tooltip_payload。
    if let Err(err) = window.emit(EVENT_TOOLTIP_SHOW, generation) {
        eprintln!("pinglet: 广播悬浮详情失败: {err}");
    }
    Ok(())
}

/// 详情页加载时读取最新内容；窗口复用时通过事件触发重新读取。
#[tauri::command]
pub fn get_tooltip_payload(state: tauri::State<'_, AppState>) -> Option<TooltipRequestSnapshot> {
    let payload = state
        .tooltip_payload
        .lock()
        .ok()
        .and_then(|slot| slot.clone())?;
    Some(TooltipRequestSnapshot {
        payload,
        generation: state.tooltip_generation.load(Ordering::Relaxed),
    })
}

/// 详情页实测内容尺寸后调用：按当前鼠标位置定位并显示窗口。
#[tauri::command]
pub fn resize_tooltip_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    generation: u64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // 代号过期说明鼠标已离开或切换到了别的会话，旧渲染结果不再显示
    if state.tooltip_generation.load(Ordering::Relaxed) != generation {
        return Ok(());
    }
    let Some(window) = app.get_webview_window(TOOLTIP_WINDOW_LABEL) else {
        return Ok(());
    };
    let Some(pointer) = state.tooltip_pointer.lock().ok().and_then(|slot| *slot) else {
        return Ok(());
    };

    let width = width.clamp(TOOLTIP_MIN_WIDTH_LOGICAL, TOOLTIP_MAX_WIDTH_LOGICAL);
    let height = height.max(TOOLTIP_MIN_HEIGHT_LOGICAL);
    if let Ok(mut slot) = state.tooltip_size.lock() {
        *slot = Some((width, height));
    }
    apply_tooltip_geometry(&window, pointer, width, height, true)
}

/// 鼠标移动时让详情窗口跟随；尺寸沿用上一次实测结果。
#[tauri::command]
pub fn move_tooltip_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pointer_x: f64,
    pointer_y: f64,
) -> Result<(), String> {
    if let Ok(mut slot) = state.tooltip_pointer.lock() {
        *slot = Some((pointer_x, pointer_y));
    }
    let Some((width, height)) = state.tooltip_size.lock().ok().and_then(|slot| *slot) else {
        return Ok(());
    };
    let Some(window) = app.get_webview_window(TOOLTIP_WINDOW_LABEL) else {
        return Ok(());
    };
    apply_tooltip_geometry(&window, (pointer_x, pointer_y), width, height, false)
}

/// 鼠标离开会话行、滚轮滚动或点击时收起详情。
#[tauri::command]
pub fn hide_tooltip_window(app: tauri::AppHandle) {
    hide_tooltip_now(&app);
}

/// 供主窗口隐藏 / 收起动画兜底调用；递增代号让页面迟到的尺寸上报失效。
pub(crate) fn hide_tooltip_now(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    state.tooltip_generation.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut slot) = state.tooltip_size.lock() {
        *slot = None;
    }
    if let Some(window) = app.get_webview_window(TOOLTIP_WINDOW_LABEL)
        && let Err(err) = hide_auxiliary_window(&window)
    {
        eprintln!("pinglet: 隐藏悬浮详情失败: {err}");
    }
}

/// 首次悬停时按需创建窗口，之后复用同一个隐藏窗口。
/// 窗口透明无边框、不抢焦点、不进任务栏，并设置为鼠标穿透，避免挡住主窗口的悬停判定。
fn ensure_tooltip_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(TOOLTIP_WINDOW_LABEL) {
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(
        app,
        TOOLTIP_WINDOW_LABEL,
        WebviewUrl::App("tooltip.html".into()),
    )
    .title(APP_NAME)
    .inner_size(
        TOOLTIP_INITIAL_WIDTH_LOGICAL,
        TOOLTIP_INITIAL_HEIGHT_LOGICAL,
    )
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .visible(false)
    .build()
    .map_err(|err| format!("创建悬浮详情窗口失败: {err}"))?;

    // NOTE: 仅 focused(false) 只影响首次创建，后续显示仍可能抢焦点；
    // 关闭可聚焦性让窗口带上 WS_EX_NOACTIVATE，鼠标事件继续落到主窗口。
    if let Err(err) = window.set_focusable(false) {
        eprintln!("pinglet: 设置悬浮详情窗口不可聚焦失败: {err}");
    }
    // NOTE: 不能用 set_ignore_cursor_events——它会给窗口加 WS_EX_LAYERED，
    // 与 WebView2 的透明 DirectComposition 合成冲突，窗口显示后没有内容。
    if let Err(err) = enable_tooltip_click_through(&window) {
        eprintln!("pinglet: 设置悬浮详情窗口穿透失败: {err}");
    }
    Ok(window)
}

/// 只加 `WS_EX_TRANSPARENT` 让鼠标事件穿透，避免 `WS_EX_LAYERED` 破坏透明渲染。
#[cfg(target_os = "windows")]
fn enable_tooltip_click_through(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_TRANSPARENT,
    };

    let hwnd = window
        .hwnd()
        .map_err(|err| format!("获取悬浮详情句柄失败: {err}"))?;
    // SAFETY: hwnd 来自仍存活的窗口；只调整扩展样式，不改变窗口生命周期。
    unsafe {
        let style = GetWindowLongPtrW(hwnd.0 as _, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd.0 as _, GWL_EXSTYLE, style | WS_EX_TRANSPARENT as isize);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn enable_tooltip_click_through(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// 按鼠标在主窗口内的逻辑坐标定位详情窗口，并同步尺寸与可见性。
fn apply_tooltip_geometry(
    window: &tauri::WebviewWindow,
    pointer: (f64, f64),
    width_logical: f64,
    height_logical: f64,
    resize: bool,
) -> Result<(), String> {
    let app = window.app_handle();
    let main = app
        .get_webview_window(WINDOW_LABEL)
        .ok_or_else(|| "主窗口不可用".to_string())?;
    let scale = main
        .scale_factor()
        .map_err(|err| format!("读取窗口缩放失败: {err}"))?;
    let main_position = main
        .outer_position()
        .map_err(|err| format!("读取主窗口位置失败: {err}"))?;
    let monitor = main
        .current_monitor()
        .map_err(|err| format!("读取显示器信息失败: {err}"))?
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return Err("无法定位显示器".to_string());
    };

    let work_area = monitor.work_area();
    let work_left = work_area.position.x;
    let work_top = work_area.position.y;
    let work_right = work_area.position.x + work_area.size.width as i32;
    let work_bottom = work_area.position.y + work_area.size.height as i32;
    // 内容再长也不能超过所在显示器的工作区，否则窗口会被屏幕裁掉
    let max_height_logical =
        ((work_bottom - work_top) as f64 / scale).max(TOOLTIP_MIN_HEIGHT_LOGICAL);
    let height_logical = height_logical.min(max_height_logical);

    let window_width = (width_logical * scale).round().max(1.0) as u32;
    let window_height = (height_logical * scale).round().max(1.0) as u32;
    let pointer_x = main_position.x + (pointer.0 * scale).round() as i32;
    let pointer_y = main_position.y + (pointer.1 * scale).round() as i32;
    let offset = (TOOLTIP_OFFSET_LOGICAL * scale).round() as i32;
    let (x, y) = resolve_tooltip_position(
        pointer_x,
        pointer_y,
        window_width as i32,
        window_height as i32,
        work_left,
        work_top,
        work_right,
        work_bottom,
        offset,
    );

    if resize {
        window
            .set_size(tauri::PhysicalSize::new(window_width, window_height))
            .map_err(|err| format!("调整悬浮详情尺寸失败: {err}"))?;
    }
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|err| format!("定位悬浮详情失败: {err}"))?;
    if resize {
        // 只在尺寸上报（新内容渲染完成）时显示；鼠标移动只需挪动已显示的窗口
        show_auxiliary_window(window)?;
    }
    Ok(())
}

/// 默认放在鼠标右下方；越界时翻到左 / 上方，最后夹取到工作区。
#[allow(clippy::too_many_arguments)]
fn resolve_tooltip_position(
    pointer_x: i32,
    pointer_y: i32,
    tooltip_width: i32,
    tooltip_height: i32,
    work_left: i32,
    work_top: i32,
    work_right: i32,
    work_bottom: i32,
    offset: i32,
) -> (i32, i32) {
    let mut x = pointer_x + offset;
    let mut y = pointer_y + offset;
    if x + tooltip_width > work_right {
        x = pointer_x - tooltip_width - offset;
    }
    if y + tooltip_height > work_bottom {
        y = pointer_y - tooltip_height - offset;
    }

    let max_x = (work_right - tooltip_width).max(work_left);
    let max_y = (work_bottom - tooltip_height).max(work_top);
    (x.clamp(work_left, max_x), y.clamp(work_top, max_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_position_prefers_cursor_bottom_right() {
        assert_eq!(
            resolve_tooltip_position(100, 200, 300, 180, 0, 0, 1920, 1040, 12),
            (112, 212)
        );
    }

    #[test]
    fn tooltip_position_flips_near_right_and_bottom_edges() {
        assert_eq!(
            resolve_tooltip_position(1700, 100, 300, 180, 0, 0, 1920, 1040, 12),
            (1388, 112)
        );
        assert_eq!(
            resolve_tooltip_position(100, 900, 300, 180, 0, 0, 1920, 1040, 12),
            (112, 708)
        );
    }

    #[test]
    fn tooltip_position_clamps_into_work_area() {
        // 窗口比工作区还大时钉在工作区左上角
        assert_eq!(
            resolve_tooltip_position(500, 500, 1200, 700, 100, 50, 800, 600, 12),
            (100, 50)
        );
        // 鼠标贴近工作区左上角时不出现负坐标
        assert_eq!(
            resolve_tooltip_position(102, 52, 300, 180, 100, 50, 1920, 1040, 12),
            (114, 64)
        );
    }
}
