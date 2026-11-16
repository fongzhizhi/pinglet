//! 自绘通知弹窗：内容聚合、窗口创建、定位与显示隐藏。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::session::{AlertItem, BoardKind};
use crate::window::{hide_auxiliary_window, show_auxiliary_window, show_main_window};
use crate::{APP_NAME, AppState, EVENT_NOTIFICATION_SHOW, NOTIFICATION_WINDOW_LABEL};

/// 通知弹窗最多列举数与标题截断长度，避免内容过长。
const NOTIFICATION_MAX_ITEMS: usize = 6;
const NOTIFICATION_TITLE_MAX_CHARS: usize = 24;
/// 自绘通知弹窗：固定宽度（逻辑像素）、内容高度上下限、右下角边距与前端淡出后的兜底隐藏时长。
const NOTIFICATION_WINDOW_WIDTH_LOGICAL: f64 = 300.0;
const NOTIFICATION_WINDOW_MIN_HEIGHT_LOGICAL: f64 = 96.0;
const NOTIFICATION_WINDOW_HEIGHT_LOGICAL: f64 = 480.0;
/// 底部距任务栏的间隙；工作区已排除任务栏，这里只留一点视觉呼吸空间。
const NOTIFICATION_WINDOW_MARGIN_LOGICAL: f64 = 6.0;
const NOTIFICATION_AUTO_HIDE_FALLBACK: Duration = Duration::from_secs(8);
/// 鼠标离开托盘或通知弹窗后延迟隐藏：只留出从托盘移到卡片上的缓冲，接近系统 tooltip 的手感。
const NOTIFICATION_HOVER_HIDE_DELAY: Duration = Duration::from_millis(250);
/// 悬浮托盘停留多久才展示通知，避免鼠标无意划过时误触发。
const NOTIFICATION_HOVER_SHOW_DELAY: Duration = Duration::from_millis(400);
/// 悬浮卡片的最长展示时长，防止 Enter / Leave 事件丢失后卡片一直不消失。
const NOTIFICATION_MAX_VISIBLE: Duration = Duration::from_secs(12);

/// 通知弹窗中的一条告警；每条独立成卡。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationPopupItem {
    key: String,
    title: String,
    kind: &'static str,
    status: &'static str,
    /// 点击卡片时激活会话所需字段；仅用于本地 IPC，不在卡片上展示。
    cwd: Option<String>,
    entrypoint: Option<String>,
    name: String,
    pid: Option<u32>,
}

/// 通知弹窗内容：独立卡片列表 + 剩余数量 + 是否自动隐藏。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationPopupPayload {
    items: Vec<NotificationPopupItem>,
    more: Option<String>,
    auto_hide: bool,
}

pub(crate) fn notification_popup_payload(
    items: &[AlertItem],
    auto_hide: bool,
) -> NotificationPopupPayload {
    let total = items.len();
    let rows = items
        .iter()
        .take(NOTIFICATION_MAX_ITEMS)
        .map(|item| {
            let (kind, status) = match item.kind {
                BoardKind::Interrupted => ("interrupted", "已中断"),
                _ => ("blocked", "待处理"),
            };
            NotificationPopupItem {
                key: item.key.clone(),
                title: truncate_notification_title(&item.title),
                kind,
                status,
                cwd: item.cwd.clone(),
                entrypoint: item.entrypoint.clone(),
                name: item.name.clone(),
                pid: item.pid,
            }
        })
        .collect();
    NotificationPopupPayload {
        items: rows,
        more: (total > NOTIFICATION_MAX_ITEMS).then(|| format!("等 {total} 个会话")),
        auto_hide,
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

/// 更新内容并显示通知弹窗；前端负责 5 秒淡出，Rust 侧另有兜底隐藏。
pub(crate) fn show_notification_popup(
    app: &tauri::AppHandle,
    payload: NotificationPopupPayload,
) -> Result<(), String> {
    let auto_hide = payload.auto_hide;
    {
        let state = app.state::<AppState>();
        let mut slot = state
            .notification_payload
            .lock()
            .map_err(|_| "通知内容锁不可用".to_string())?;
        *slot = Some(payload);
        drop(slot);
        state
            .notification_tray_visible
            .store(false, Ordering::Relaxed);
        if let Ok(mut shown_at) = state.notification_shown_at.lock() {
            *shown_at = Some(Instant::now());
        }
    }

    let window = ensure_notification_window(app)?;
    position_notification_window(app, &window);
    show_auxiliary_window(&window)?;
    // NOTE: 首次创建时事件可能早于页面加载；页面加载后会主动调用 get_notification_payload。
    if let Err(err) = window.emit(EVENT_NOTIFICATION_SHOW, ()) {
        eprintln!("pinglet: 广播通知内容失败: {err}");
    }
    if auto_hide {
        // 悬浮查看时同样不强制隐藏，鼠标离开后由前端重新触发延迟隐藏
        schedule_notification_hide_after(app, NOTIFICATION_AUTO_HIDE_FALLBACK, true);
    }
    Ok(())
}

/// 鼠标进入托盘：延迟展示，快速划过不弹出；期间离开会取消本次展示。
pub(crate) fn schedule_notification_from_tray(app: &tauri::AppHandle) {
    let generation = app
        .state::<AppState>()
        .notification_generation
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    let thread_app = app.clone();
    let result = std::thread::Builder::new()
        .name("pinglet-notification-show".to_string())
        .spawn(move || {
            std::thread::sleep(NOTIFICATION_HOVER_SHOW_DELAY);
            let state = thread_app.state::<AppState>();
            if state.notification_generation.load(Ordering::Relaxed) != generation {
                return;
            }
            show_notification_from_tray_now(&thread_app);
        });
    if let Err(err) = result {
        eprintln!("pinglet: 启动通知展示计时失败: {err}");
    }
}

/// 展示当前全部未处理告警。
/// 只在主面板隐藏或最小化、通知消息开启且确有告警时显示，避免与面板内状态重复。
fn show_notification_from_tray_now(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if !state.system_notifications.load(Ordering::Relaxed) {
        return;
    }
    if state.window_active.load(Ordering::Relaxed) {
        return;
    }
    let items = state
        .latest_alerts
        .lock()
        .map(|alerts| alerts.clone())
        .unwrap_or_default();
    if items.is_empty() {
        return;
    }

    // 卡片已在显示：鼠标快速移出又移入时只取消隐藏计时，不重绘，避免一闪而过
    if state.notification_tray_visible.load(Ordering::Relaxed) {
        state
            .notification_generation
            .fetch_add(1, Ordering::Relaxed);
        return;
    }

    let payload = notification_popup_payload(&items, false);
    match show_notification_popup(app, payload) {
        Ok(()) => {
            state
                .notification_tray_visible
                .store(true, Ordering::Relaxed);
            schedule_notification_max_visible(app);
        }
        Err(err) => eprintln!("pinglet: 显示托盘通知失败: {err}"),
    }
}

/// 悬浮卡片最长展示兜底：即使鼠标一直停在托盘上或事件丢失，到时也会收起。
fn schedule_notification_max_visible(app: &tauri::AppHandle) {
    let thread_app = app.clone();
    let result = std::thread::Builder::new()
        .name("pinglet-notification-max-visible".to_string())
        .spawn(move || {
            std::thread::sleep(NOTIFICATION_MAX_VISIBLE);
            let state = thread_app.state::<AppState>();
            let stale = state
                .notification_shown_at
                .lock()
                .ok()
                .and_then(|shown_at| *shown_at)
                .is_none_or(|shown_at| shown_at.elapsed() >= NOTIFICATION_MAX_VISIBLE);
            if !stale || !state.notification_tray_visible.load(Ordering::Relaxed) {
                return;
            }
            state
                .notification_generation
                .fetch_add(1, Ordering::Relaxed);
            if let Some(window) = thread_app.get_webview_window(NOTIFICATION_WINDOW_LABEL)
                && let Err(err) = hide_auxiliary_window(&window)
            {
                eprintln!("pinglet: 强制隐藏通知弹窗失败: {err}");
            }
            state.notification_hovered.store(false, Ordering::Relaxed);
            state
                .notification_tray_visible
                .store(false, Ordering::Relaxed);
        });
    if let Err(err) = result {
        eprintln!("pinglet: 启动通知最长展示计时失败: {err}");
    }
}

/// 托盘或弹窗鼠标离开后延迟隐藏；期间鼠标重新进入会递增代号取消本次计时。
pub(crate) fn schedule_notification_hide_after(
    app: &tauri::AppHandle,
    delay: Duration,
    respect_hover: bool,
) {
    let generation = app
        .state::<AppState>()
        .notification_generation
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    let thread_app = app.clone();
    let result = std::thread::Builder::new()
        .name("pinglet-notification-hide".to_string())
        .spawn(move || {
            std::thread::sleep(delay);
            let state = thread_app.state::<AppState>();
            if state.notification_generation.load(Ordering::Relaxed) != generation {
                return;
            }
            if respect_hover && state.notification_hovered.load(Ordering::Relaxed) {
                return;
            }
            if let Some(window) = thread_app.get_webview_window(NOTIFICATION_WINDOW_LABEL)
                && let Err(err) = hide_auxiliary_window(&window)
            {
                eprintln!("pinglet: 自动隐藏通知弹窗失败: {err}");
            }
            state.notification_hovered.store(false, Ordering::Relaxed);
            state
                .notification_tray_visible
                .store(false, Ordering::Relaxed);
        });
    if let Err(err) = result {
        eprintln!("pinglet: 启动通知隐藏计时失败: {err}");
    }
}

/// 托盘鼠标离开：短暂缓冲后隐藏；鼠标重新移入会取消本次隐藏。
pub(crate) fn schedule_notification_hide_from_tray(app: &tauri::AppHandle) {
    schedule_notification_hide_after(app, NOTIFICATION_HOVER_HIDE_DELAY, true);
}

/// 前端上报鼠标是否停在通知弹窗上；进入时立即取消尚未触发的延迟隐藏。
#[tauri::command]
pub fn set_notification_hover(state: tauri::State<'_, AppState>, hovered: bool) {
    state.notification_hovered.store(hovered, Ordering::Relaxed);
    if hovered {
        state
            .notification_generation
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// 弹窗鼠标离开后的延迟隐藏入口。
#[tauri::command]
pub fn schedule_notification_hide(app: tauri::AppHandle) {
    schedule_notification_hide_after(&app, NOTIFICATION_HOVER_HIDE_DELAY, true);
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
    let state = app.state::<AppState>();
    state
        .notification_generation
        .fetch_add(1, Ordering::Relaxed);
    state.notification_hovered.store(false, Ordering::Relaxed);
    state
        .notification_tray_visible
        .store(false, Ordering::Relaxed);
    if let Some(window) = app.get_webview_window(NOTIFICATION_WINDOW_LABEL)
        && let Err(err) = hide_auxiliary_window(&window)
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

    fn alert(index: usize, title: String, kind: BoardKind) -> AlertItem {
        AlertItem {
            key: format!("pid:{index}"),
            title,
            kind,
            cwd: Some(format!("C:\\work\\demo{index}")),
            entrypoint: Some("vscode".to_string()),
            name: format!("demo{index}"),
            pid: Some(index as u32),
        }
    }

    #[test]
    fn notification_popup_limits_and_truncates_items() {
        let long_title = "很".repeat(NOTIFICATION_TITLE_MAX_CHARS + 5);
        let items: Vec<AlertItem> = (0..8)
            .map(|index| {
                alert(
                    index,
                    if index == 0 {
                        long_title.clone()
                    } else {
                        format!("会话{index}")
                    },
                    if index % 2 == 0 {
                        BoardKind::Blocked
                    } else {
                        BoardKind::Interrupted
                    },
                )
            })
            .collect();

        let payload = notification_popup_payload(&items, true);
        assert_eq!(payload.items.len(), NOTIFICATION_MAX_ITEMS);
        assert!(payload.items[0].title.ends_with('…'));
        assert_eq!(payload.items[0].kind, "blocked");
        assert_eq!(payload.items[0].status, "待处理");
        assert_eq!(payload.items[1].kind, "interrupted");
        assert_eq!(payload.items[1].status, "已中断");
        assert_eq!(payload.more.as_deref(), Some("等 8 个会话"));
        assert!(payload.auto_hide);
        // 每条卡片都携带点击激活所需字段
        assert_eq!(payload.items[0].cwd.as_deref(), Some("C:\\work\\demo0"));
        assert_eq!(payload.items[0].pid, Some(0));
        assert_eq!(payload.items[0].name, "demo0");
        assert!(payload.items.iter().all(|item| item.title != "会话7"));
    }

    #[test]
    fn tray_popup_keeps_all_items_and_skips_auto_hide() {
        let items: Vec<AlertItem> = (0..2)
            .map(|index| alert(index, format!("会话{index}"), BoardKind::Blocked))
            .collect();

        let payload = notification_popup_payload(&items, false);
        assert_eq!(payload.items.len(), 2);
        assert!(payload.more.is_none());
        assert!(!payload.auto_hide);
    }

    #[test]
    fn notification_position_sticks_to_work_area_corner() {
        assert_eq!(
            resolve_notification_position(0, 0, 1920, 1040, 380, 176, 6),
            (1534, 858)
        );
        // 工作区比窗口还小时退回左上角
        assert_eq!(
            resolve_notification_position(100, 50, 300, 100, 380, 176, 6),
            (100, 50)
        );
    }
}
