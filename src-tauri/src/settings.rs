//! 设置相关命令：关闭行为、鼠标穿透、开机自启、全局快捷键与动效偏好。

use std::sync::atomic::Ordering;
use std::time::Instant;

use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::AppState;
use crate::tray::refresh_tray;
use crate::window::apply_click_through;

#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn set_close_to_tray(state: tauri::State<'_, AppState>, enabled: bool) {
    state.close_to_tray.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn set_click_through(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    apply_click_through(&app, &state, enabled)
}
#[tauri::command]
pub fn is_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;

    app.autolaunch()
        .is_enabled()
        .map_err(|err| format!("读取开机自启状态失败: {err}"))
}

#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
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
pub fn set_global_shortcut(
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

    if let Some(previous) = previous
        && let Err(err) = app.global_shortcut().unregister(previous)
    {
        eprintln!("pinglet: 注销旧快捷键失败: {err}");
    }

    if let Some(shortcut) = parsed
        && let Err(err) = app.global_shortcut().register(shortcut)
    {
        if let Some(previous) = previous
            && let Err(restore_err) = app.global_shortcut().register(previous)
        {
            eprintln!("pinglet: 恢复旧快捷键失败: {restore_err}");
        }
        return Err(format!("注册快捷键失败: {err}"));
    }

    *state
        .current_shortcut
        .lock()
        .map_err(|_| "快捷键状态锁不可用".to_string())? = parsed;

    Ok(())
}

/// 前端设置变化时同步通知消息与托盘动效开关，托盘效果立即按新状态生效。
#[tauri::command]
pub fn set_activity_preferences(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    system_notifications: bool,
    tray_working_breathing: bool,
    tray_alert_flicker: bool,
) {
    state
        .system_notifications
        .store(system_notifications, Ordering::Relaxed);
    state
        .tray_working_breathing
        .store(tray_working_breathing, Ordering::Relaxed);
    state
        .tray_alert_flicker
        .store(tray_alert_flicker, Ordering::Relaxed);
    refresh_tray(&app, Instant::now());
}
