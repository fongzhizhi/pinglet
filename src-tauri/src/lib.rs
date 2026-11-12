//! Pinglet 桌面看板的后端入口：组装插件、应用状态与 Tauri 命令。
//! 具体职责按模块拆分：会话采集见 `session`，标题解析见 `title`，窗口激活见 `activation`，
//! 托盘见 `tray`，通知弹窗见 `notify`，窗口行为见 `window`，后台监控见 `monitor`，
//! 设置命令见 `settings`。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tauri::menu::CheckMenuItem;
use tauri::{Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutState};
use tauri_plugin_window_state::StateFlags;

mod activation;
mod monitor;
mod notify;
mod session;
mod settings;
mod title;
mod tray;
mod window;

use crate::monitor::spawn_session_monitor;
use crate::notify::NotificationPopupPayload;
use crate::tray::{TrayIcons, TrayRuntime, create_tray};
use crate::window::{
    SnapEdges, clamp_window_into_work_area, position_window_top_right, show_main_window,
    snap_window_to_edges, sync_window_activity, toggle_main_window,
};

const WINDOW_LABEL: &str = "main";
const NOTIFICATION_WINDOW_LABEL: &str = "notification";
const EVENT_WINDOW_VISIBILITY: &str = "pinglet://window-visibility";
const EVENT_WINDOW_MOVED: &str = "pinglet://window-moved";
const EVENT_WINDOW_HIDE_REQUEST: &str = "pinglet://window-hide-request";
const EVENT_WINDOW_SHOW_REQUEST: &str = "pinglet://window-show-request";
const EVENT_NOTIFICATION_SHOW: &str = "pinglet://notification-show";
/// 应用显示名：托盘 tooltip 与通知弹窗标题共用。
const APP_NAME: &str = "Pinglet";
/// 与 tauri-plugin-window-state 的默认文件名一致，用于判断是否已有可恢复的位置。
const WINDOW_STATE_FILENAME: &str = tauri_plugin_window_state::DEFAULT_FILENAME;

/// 应用级状态：关闭行为、鼠标穿透、托盘可用标记与当前全局快捷键。
struct AppState {
    close_to_tray: AtomicBool,
    click_through: AtomicBool,
    tray_ready: AtomicBool,
    system_notifications: AtomicBool,
    /// 托盘进行中呼吸 / 异常闪烁的开关；关闭时对应状态显示静态图标。
    tray_working_breathing: AtomicBool,
    tray_alert_flicker: AtomicBool,
    tray_alert_unread: AtomicBool,
    window_active: AtomicBool,
    show_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    click_through_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    current_shortcut: Mutex<Option<Shortcut>>,
    snapped_edges: Mutex<SnapEdges>,
    tray_runtime: Mutex<TrayRuntime>,
    tray_icons: Mutex<Option<TrayIcons>>,
    notification_payload: Mutex<Option<NotificationPopupPayload>>,
    /// 每次显示 / 主动隐藏都递增，作废旧的自绘弹窗兜底隐藏计时。
    notification_generation: AtomicU64,
    /// 主窗口隐藏请求代号：显示或新的隐藏请求都会递增，使旧的延迟隐藏失效。
    window_hide_generation: AtomicU64,
    monitor_stop: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            // 与前端默认设置保持一致（默认开启点击关闭时最小化到托盘）；启动后前端会用持久化值覆盖。
            close_to_tray: AtomicBool::new(true),
            click_through: AtomicBool::new(false),
            tray_ready: AtomicBool::new(false),
            system_notifications: AtomicBool::new(true),
            tray_working_breathing: AtomicBool::new(true),
            tray_alert_flicker: AtomicBool::new(true),
            tray_alert_unread: AtomicBool::new(false),
            window_active: AtomicBool::new(true),
            show_item: Mutex::new(None),
            click_through_item: Mutex::new(None),
            current_shortcut: Mutex::new(None),
            snapped_edges: Mutex::new(SnapEdges::default()),
            tray_runtime: Mutex::new(TrayRuntime::default()),
            tray_icons: Mutex::new(None),
            notification_payload: Mutex::new(None),
            notification_generation: AtomicU64::new(0),
            window_hide_generation: AtomicU64::new(0),
            monitor_stop: AtomicBool::new(false),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // NOTE: 单实例插件必须最先注册；第二次启动时回调拿到的是已运行实例的 AppHandle。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        // 只记忆位置：尺寸由前端按内容自适应并持久化在 localStorage。
        // NOTE: 通知弹窗由 Pinglet 自己定位，不能进入 window-state 的位置存档。
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::POSITION)
                .with_denylist(&[NOTIFICATION_WINDOW_LABEL])
                .build(),
        )
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

            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                // NOTE: window-state 已在窗口 ready 时恢复位置；只有没有存档时才回到默认位置。
                let state_path = app
                    .path()
                    .app_config_dir()
                    .ok()
                    .map(|dir| dir.join(WINDOW_STATE_FILENAME));
                if state_path.is_some_and(|path| path.is_file()) {
                    clamp_window_into_work_area(&window);
                } else {
                    position_window_top_right(&window);
                }
                if let Err(err) = window.show() {
                    eprintln!("pinglet: 显示主窗口失败: {err}");
                }
                sync_window_activity(app.handle());
            }

            spawn_session_monitor(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // NOTE: 通知弹窗不参与主窗口的关闭、边缘吸附与活跃度逻辑。
            if window.label() != WINDOW_LABEL {
                return;
            }
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    let state = window.state::<AppState>();
                    if state.close_to_tray.load(Ordering::Relaxed)
                        && state.tray_ready.load(Ordering::Relaxed)
                    {
                        api.prevent_close();
                        if let Err(err) = window.hide() {
                            eprintln!("pinglet: 隐藏到托盘失败: {err}");
                        } else {
                            sync_window_activity(window.app_handle());
                        }
                    } else {
                        // 关闭行为设为“退出”时，X 按钮要结束整个应用，而不是留下托盘空转
                        window.app_handle().exit(0);
                    }
                }
                WindowEvent::Moved(_) => {
                    snap_window_to_edges(window);
                    if let Err(err) = window.emit(EVENT_WINDOW_MOVED, ()) {
                        eprintln!("pinglet: 广播窗口移动失败: {err}");
                    }
                }
                WindowEvent::Resized(_) => sync_window_activity(window.app_handle()),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            session::list_sessions,
            activation::activate_session,
            settings::quit_app,
            settings::set_close_to_tray,
            settings::set_click_through,
            settings::set_activity_preferences,
            notify::get_notification_payload,
            notify::hide_notification_window,
            notify::open_main_window,
            window::hide_main_window_now,
            notify::resize_notification_window,
            window::reset_window_position,
            window::sample_backdrop_luminance,
            settings::is_autostart_enabled,
            settings::set_autostart,
            settings::set_global_shortcut
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<AppState>()
                    .monitor_stop
                    .store(true, Ordering::Relaxed);
            }
        });
}
