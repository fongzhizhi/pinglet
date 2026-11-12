//! 后台会话监控线程：5 秒采集驱动托盘与通知弹窗。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::Manager;

use crate::AppState;
use crate::notify::{notification_popup_payload, show_notification_popup};
use crate::session::{AlertItem, SessionSnapshot, build_snapshot, collect_sessions};
use crate::tray::refresh_tray;
use crate::window::sync_window_activity;

/// 托盘监视线程以 80ms（约 12.5fps）为粒度推进动画帧；会话采集周期为 5 秒。
/// NOTE: 隐藏窗口后 WebView2 会节流前端定时器，托盘与通知弹窗必须由 Rust 侧自行采集。
const TRAY_MONITOR_TICK: Duration = Duration::from_millis(80);
const SESSION_MONITOR_INTERVAL: Duration = Duration::from_secs(5);
/// 启动后台会话监控：Rust 侧独立轮询，不依赖会被 WebView2 节流的 JS 计时器。
pub(crate) fn spawn_session_monitor(app: tauri::AppHandle) {
    let thread_app = app.clone();
    if let Err(err) = std::thread::Builder::new()
        .name("pinglet-session-monitor".to_string())
        .spawn(move || session_monitor_loop(thread_app))
    {
        eprintln!("pinglet: 启动会话监控线程失败: {err}");
    }
}

fn session_monitor_loop(app: tauri::AppHandle) {
    let mut first_scan = true;
    let mut last_poll: Option<Instant> = None;

    loop {
        if app.state::<AppState>().monitor_stop.load(Ordering::Relaxed) {
            break;
        }

        let now = Instant::now();
        let should_poll = match last_poll {
            None => true,
            Some(last) => now.duration_since(last) >= SESSION_MONITOR_INTERVAL,
        };
        if should_poll {
            last_poll = Some(now);
            let snapshot = build_snapshot(&collect_sessions().rows);
            poll_session_state(&app, snapshot, &mut first_scan);
        }

        // 动画帧时间驱动：每 tick 重新计算帧索引，掉帧时不会整体变慢
        refresh_tray(&app, now);
        std::thread::sleep(TRAY_MONITOR_TICK);
    }
}

fn poll_session_state(app: &tauri::AppHandle, snapshot: SessionSnapshot, first_scan: &mut bool) {
    sync_window_activity(app);

    let state = app.state::<AppState>();
    let active = state.window_active.load(Ordering::Relaxed);
    let system_notifications = state.system_notifications.load(Ordering::Relaxed);

    let new_alerts: Vec<AlertItem> = {
        let Ok(mut runtime) = state.tray_runtime.lock() else {
            return;
        };
        let new_alerts = snapshot
            .alert_items
            .iter()
            .filter(|item| !runtime.alert_keys.contains(&item.key))
            .map(|item| AlertItem {
                key: item.key.clone(),
                title: item.title.clone(),
                kind: item.kind,
            })
            .collect();
        runtime.blocked = snapshot.blocked;
        runtime.interrupted = snapshot.interrupted;
        runtime.working = snapshot.working;
        runtime.alert_keys = snapshot.alert_keys;
        new_alerts
    };

    let has_alert = snapshot.blocked + snapshot.interrupted > 0;
    if !has_alert {
        state.tray_alert_unread.store(false, Ordering::Relaxed);
    } else if !*first_scan && !active && !new_alerts.is_empty() {
        state.tray_alert_unread.store(true, Ordering::Relaxed);
    }

    // 首次扫描只播种，避免启动时把历史告警全部弹成通知。
    // NOTE: 待处理告警的去重键带状态时间戳（见 alert_event_key），每次新的等待请求都会
    // 进入 new_alerts，即使中间的 busy 状态短于轮询周期也能识别；同一次等待不会重复弹。
    // 因此不设全局冷却——否则同一会话短时间内第二次告警会被静默丢弃。
    if !*first_scan && system_notifications && !active && !new_alerts.is_empty() {
        let payload =
            notification_popup_payload(snapshot.blocked, snapshot.interrupted, &new_alerts);
        if let Err(err) = show_notification_popup(app, payload) {
            eprintln!("pinglet: 显示通知弹窗失败: {err}");
        }
    }

    *first_scan = false;
    refresh_tray(app, Instant::now());
}
