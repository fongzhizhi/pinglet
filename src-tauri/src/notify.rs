//! 自绘通知弹窗：内容聚合、窗口创建、定位与显示隐藏。

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow};

use crate::session::{AlertItem, BoardKind};
use crate::window::show_main_window;
use crate::{APP_NAME, AppState, EVENT_NOTIFICATION_SHOW, NOTIFICATION_WINDOW_LABEL};

/// 通知弹窗单条最多列举数与标题截断长度，避免内容过长。
const NOTIFICATION_MAX_ITEMS: usize = 3;
const NOTIFICATION_TITLE_MAX_CHARS: usize = 24;
/// 自绘通知弹窗：固定宽度（逻辑像素）、内容高度上下限、右下角边距与前端淡出后的兜底隐藏时长。
const NOTIFICATION_WINDOW_WIDTH_LOGICAL: f64 = 380.0;
const NOTIFICATION_WINDOW_MIN_HEIGHT_LOGICAL: f64 = 96.0;
const NOTIFICATION_WINDOW_HEIGHT_LOGICAL: f64 = 176.0;
const NOTIFICATION_WINDOW_MARGIN_LOGICAL: f64 = 16.0;
const NOTIFICATION_AUTO_HIDE_FALLBACK: Duration = Duration::from_secs(8);
/// 通知弹窗中的一条告警；kind 决定状态文案的颜色。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationPopupItem {
    title: String,
    kind: &'static str,
    status: &'static str,
}

/// 通知弹窗内容：标题汇总 + 最多三条明细 + 剩余数量。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationPopupPayload {
    title: String,
    items: Vec<NotificationPopupItem>,
    more: Option<String>,
}
fn notification_title(blocked: usize, interrupted: usize) -> String {
    let total = blocked + interrupted;
    if interrupted == 0 {
        format!("{APP_NAME} · {total} 个会话待处理")
    } else if blocked == 0 {
        format!("{APP_NAME} · {total} 个会话已中断")
    } else {
        format!("{APP_NAME} · {total} 个会话需要关注")
    }
}

pub(crate) fn notification_popup_payload(
    blocked: usize,
    interrupted: usize,
    items: &[AlertItem],
) -> NotificationPopupPayload {
    let total = items.len();
    let rows = items
        .iter()
        .take(NOTIFICATION_MAX_ITEMS)
        .map(|item| {
            let (kind, status) = match item.kind {
                BoardKind::Interrupted => ("interrupted", "已中断"),
                _ => ("blocked", "等待确认"),
            };
            NotificationPopupItem {
                title: truncate_notification_title(&item.title),
                kind,
                status,
            }
        })
        .collect();
    NotificationPopupPayload {
        title: notification_title(blocked, interrupted),
        items: rows,
        more: (total > NOTIFICATION_MAX_ITEMS).then(|| format!("等 {total} 个会话")),
    }
}

fn truncate_notification_title(title: &str) -> String {
    match title.char_indices().nth(NOTIFICATION_TITLE_MAX_CHARS) {
        Some((index, _)) => format!("{}…", &title[..index]),
        None => title.to_string(),
    }
}
/// 通知弹窗贴主屏工作区右下角（任务栏上方）；工作区不足时退回左上角。
fn resolve_notification_position(
    work_left: i32,
    work_top: i32,
    work_width: i32,
    work_height: i32,
    window_width: i32,
    window_height: i32,
    margin: i32,
) -> (i32, i32) {
    let max_x = work_left + (work_width - window_width).max(0);
    let max_y = work_top + (work_height - window_height).max(0);
    (
        (max_x - margin).max(work_left),
        (max_y - margin).max(work_top),
    )
}

fn position_notification_window(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    let monitor = app
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };

    let scale = monitor.scale_factor();
    let (window_width, window_height) = window
        .outer_size()
        .map(|size| (size.width as i32, size.height as i32))
        .unwrap_or_else(|_| {
            (
                (NOTIFICATION_WINDOW_WIDTH_LOGICAL * scale).round() as i32,
                (NOTIFICATION_WINDOW_HEIGHT_LOGICAL * scale).round() as i32,
            )
        });
    position_notification_window_sized(app, window, window_width, window_height);
}

/// 以物理尺寸把弹窗贴到主屏工作区右下角；尺寸可能来自前端实测内容高度。
fn position_notification_window_sized(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    window_width: i32,
    window_height: i32,
) {
    let monitor = app
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };

    let scale = monitor.scale_factor();
    let work_area = monitor.work_area();
    let margin = (NOTIFICATION_WINDOW_MARGIN_LOGICAL * scale).round() as i32;
    let (x, y) = resolve_notification_position(
        work_area.position.x,
        work_area.position.y,
        work_area.size.width as i32,
        work_area.size.height as i32,
        window_width,
        window_height,
        margin,
    );

    if let Err(err) = window.set_position(tauri::PhysicalPosition::new(x, y)) {
        eprintln!("pinglet: 定位通知弹窗失败: {err}");
    }
}

/// 首次弹窗时按需创建窗口，之后复用同一个隐藏窗口。
/// 窗口不抢焦点、不进任务栏，透明无边框，只由 Pinglet 控制显示与隐藏。
fn ensure_notification_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(NOTIFICATION_WINDOW_LABEL) {
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(
        app,
        NOTIFICATION_WINDOW_LABEL,
        WebviewUrl::App("notification.html".into()),
    )
    .title(APP_NAME)
    .inner_size(
        NOTIFICATION_WINDOW_WIDTH_LOGICAL,
        NOTIFICATION_WINDOW_HEIGHT_LOGICAL,
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
    .map_err(|err| format!("创建通知弹窗失败: {err}"))?;

    // NOTE: 仅 focused(false) 只影响首次创建，后续 show() 仍会抢焦点；
    // 关闭可聚焦性让窗口带上 WS_EX_NOACTIVATE，点击时不会激活。
    if let Err(err) = window.set_focusable(false) {
        eprintln!("pinglet: 设置通知弹窗不可聚焦失败: {err}");
    }
    Ok(window)
}

/// 同步 WebView2 控制器的可见性。
/// NOTE: 裸 `ShowWindow` 只改变原生窗口，WebView2 仍认为页面可见：隐藏期间渲染器可能被
/// 遮挡检测挂起，再次显示时不再出帧（空白卡片）。显式切换控制器可见性让渲染器随窗口
/// 一起暂停与恢复，第二次及之后的弹窗才能正常绘制新内容。
#[cfg(target_os = "windows")]
fn set_notification_webview_visible(window: &tauri::WebviewWindow, visible: bool) {
    let dispatched = window.with_webview(move |platform| {
        // SAFETY: controller 属于仍存活的 webview；with_webview 回调在主线程执行。
        if let Err(err) = unsafe { platform.controller().SetIsVisible(visible) } {
            eprintln!("pinglet: 同步通知弹窗渲染可见性失败: {err}");
        }
    });
    if let Err(err) = dispatched {
        eprintln!("pinglet: 派发通知弹窗渲染可见性失败: {err}");
    }
}

/// 以 `SW_SHOWNOACTIVATE` 显示通知弹窗。
/// WARN: 不能用 `window.show()`——tao 创建后会清除 MARKER_DONT_FOCUS，show() 走 SW_SHOW，
/// 即使窗口带 WS_EX_NOACTIVATE 也会被设为前台，抢走用户当前应用的焦点。
fn show_notification_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = window
            .hwnd()
            .map_err(|err| format!("获取通知弹窗句柄失败: {err}"))?;
        // SAFETY: hwnd 来自仍存活的 WebviewWindow，仅执行窗口显示且不激活窗口。
        unsafe {
            let _ = ShowWindow(hwnd.0 as _, SW_SHOWNOACTIVATE);
        }
        set_notification_webview_visible(window, true);
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        window
            .show()
            .map_err(|err| format!("显示通知弹窗失败: {err}"))
    }
}

/// 用裸 `ShowWindow(SW_HIDE)` 隐藏通知弹窗。
/// WARN: 不能用 `window.hide()`——显示走的是裸 `ShowWindow`，tao 的 `WindowFlags::VISIBLE`
/// 始终为 false，`hide()` 的 `apply_diff` 因 diff 为空直接返回，窗口不会真正隐藏。
fn hide_notification_window_raw(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = window
            .hwnd()
            .map_err(|err| format!("获取通知弹窗句柄失败: {err}"))?;
        set_notification_webview_visible(window, false);
        // SAFETY: hwnd 来自仍存活的 WebviewWindow；ShowWindow 允许从兜底线程调用。
        unsafe {
            let _ = ShowWindow(hwnd.0 as _, SW_HIDE);
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        window
            .hide()
            .map_err(|err| format!("隐藏通知弹窗失败: {err}"))
    }
}

/// 更新内容并显示通知弹窗；前端负责 5 秒淡出，Rust 侧另有兜底隐藏。
pub(crate) fn show_notification_popup(
    app: &tauri::AppHandle,
    payload: NotificationPopupPayload,
) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut slot = state
            .notification_payload
            .lock()
            .map_err(|_| "通知内容锁不可用".to_string())?;
        *slot = Some(payload);
    }

    let window = ensure_notification_window(app)?;
    position_notification_window(app, &window);
    show_notification_window(&window)?;
    // NOTE: 首次创建时事件可能早于页面加载；页面加载后会主动调用 get_notification_payload。
    if let Err(err) = window.emit(EVENT_NOTIFICATION_SHOW, ()) {
        eprintln!("pinglet: 广播通知内容失败: {err}");
    }
    schedule_notification_auto_hide(app);
    Ok(())
}

/// 兜底隐藏：前端 5 秒淡出未生效时，8 秒后由 Rust 收起；再次弹窗会作废旧计时。
fn schedule_notification_auto_hide(app: &tauri::AppHandle) {
    let generation = app
        .state::<AppState>()
        .notification_generation
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    let thread_app = app.clone();
    let result = std::thread::Builder::new()
        .name("pinglet-notification-hide".to_string())
        .spawn(move || {
            std::thread::sleep(NOTIFICATION_AUTO_HIDE_FALLBACK);
            if thread_app
                .state::<AppState>()
                .notification_generation
                .load(Ordering::Relaxed)
                != generation
            {
                return;
            }
            if let Some(window) = thread_app.get_webview_window(NOTIFICATION_WINDOW_LABEL)
                && let Err(err) = hide_notification_window_raw(&window)
            {
                eprintln!("pinglet: 自动隐藏通知弹窗失败: {err}");
            }
        });
    if let Err(err) = result {
        eprintln!("pinglet: 启动通知隐藏计时失败: {err}");
    }
}

/// 通知弹窗加载时读取最新内容；窗口复用时通过事件触发重新读取。
#[tauri::command]
pub fn get_notification_payload(
    state: tauri::State<'_, AppState>,
) -> Option<NotificationPopupPayload> {
    state
        .notification_payload
        .lock()
        .ok()
        .and_then(|payload| payload.clone())
}

/// 前端点击关闭或倒计时结束后调用；递增代号让 Rust 兜底计时提前失效。
#[tauri::command]
pub fn hide_notification_window(app: tauri::AppHandle) {
    app.state::<AppState>()
        .notification_generation
        .fetch_add(1, Ordering::Relaxed);
    if let Some(window) = app.get_webview_window(NOTIFICATION_WINDOW_LABEL)
        && let Err(err) = hide_notification_window_raw(&window)
    {
        eprintln!("pinglet: 隐藏通知弹窗失败: {err}");
    }
}

/// 点击通知卡片：收起弹窗并唤起主窗口。
#[tauri::command]
pub fn open_main_window(app: tauri::AppHandle) {
    hide_notification_window(app.clone());
    show_main_window(&app);
}
/// 前端按内容实测高度调整弹窗，避免透明空白区域遮挡桌面点击。
#[tauri::command]
pub fn resize_notification_window(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    let window = app
        .get_webview_window(NOTIFICATION_WINDOW_LABEL)
        .ok_or_else(|| "通知弹窗尚未创建".to_string())?;
    let scale = window
        .scale_factor()
        .map_err(|err| format!("读取通知弹窗缩放失败: {err}"))?;
    let clamped = height.clamp(
        NOTIFICATION_WINDOW_MIN_HEIGHT_LOGICAL,
        NOTIFICATION_WINDOW_HEIGHT_LOGICAL,
    );
    let physical_width = (NOTIFICATION_WINDOW_WIDTH_LOGICAL * scale).round() as u32;
    let physical_height = (clamped * scale).round() as u32;
    window
        .set_size(tauri::PhysicalSize::new(physical_width, physical_height))
        .map_err(|err| format!("调整通知弹窗尺寸失败: {err}"))?;
    position_notification_window_sized(
        &app,
        &window,
        physical_width as i32,
        physical_height as i32,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_title_describes_alert_kinds() {
        assert_eq!(notification_title(2, 0), "Pinglet · 2 个会话待处理");
        assert_eq!(notification_title(0, 3), "Pinglet · 3 个会话已中断");
        assert_eq!(notification_title(1, 2), "Pinglet · 3 个会话需要关注");
    }

    #[test]
    fn notification_popup_limits_and_truncates_items() {
        let long_title = "很".repeat(NOTIFICATION_TITLE_MAX_CHARS + 5);
        let items: Vec<AlertItem> = (0..5)
            .map(|index| AlertItem {
                key: format!("pid:{index}"),
                title: if index == 0 {
                    long_title.clone()
                } else {
                    format!("会话{index}")
                },
                kind: if index % 2 == 0 {
                    BoardKind::Blocked
                } else {
                    BoardKind::Interrupted
                },
            })
            .collect();

        let payload = notification_popup_payload(3, 2, &items);
        assert_eq!(payload.title, "Pinglet · 5 个会话需要关注");
        assert_eq!(payload.items.len(), NOTIFICATION_MAX_ITEMS);
        assert!(payload.items[0].title.ends_with('…'));
        assert_eq!(payload.items[0].kind, "blocked");
        assert_eq!(payload.items[0].status, "等待确认");
        assert_eq!(payload.items[1].kind, "interrupted");
        assert_eq!(payload.items[1].status, "已中断");
        assert_eq!(payload.more.as_deref(), Some("等 5 个会话"));
        // 只列举前 3 条
        assert!(payload.items.iter().all(|item| item.title != "会话4"));
    }

    #[test]
    fn notification_position_sticks_to_work_area_corner() {
        assert_eq!(
            resolve_notification_position(0, 0, 1920, 1040, 380, 176, 16),
            (1524, 848)
        );
        // 工作区比窗口还小时退回左上角
        assert_eq!(
            resolve_notification_position(100, 50, 300, 100, 380, 176, 16),
            (100, 50)
        );
    }
}
