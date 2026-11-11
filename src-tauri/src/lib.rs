use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessesToUpdate, System};
#[cfg(target_os = "windows")]
use sysinfo::{Process, ProcessRefreshKind, UpdateKind};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_window_state::{AppHandleExt as WindowStateAppHandleExt, StateFlags};

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Graphics::Gdi::{CLR_INVALID, GetDC, GetPixel, ReleaseDC};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOWNOACTIVATE,
    SetForegroundWindow, ShowWindow,
};
#[cfg(target_os = "windows")]
use windows_sys::core::BOOL;

const SESSION_STATUS_UNKNOWN: &str = "unknown";
const SESSION_KIND_INTERACTIVE: &str = "interactive";
const WINDOW_LABEL: &str = "main";
const NOTIFICATION_WINDOW_LABEL: &str = "notification";
const EVENT_WINDOW_VISIBILITY: &str = "pinglet://window-visibility";
const EVENT_WINDOW_MOVED: &str = "pinglet://window-moved";
const EVENT_WINDOW_HIDE_REQUEST: &str = "pinglet://window-hide-request";
const EVENT_WINDOW_SHOW_REQUEST: &str = "pinglet://window-show-request";
const EVENT_NOTIFICATION_SHOW: &str = "pinglet://notification-show";
/// 前端收起动画约 160ms；超过这个时间仍未收到隐藏命令就直接隐藏，避免窗口卡在可见态。
const WINDOW_HIDE_FALLBACK: Duration = Duration::from_millis(260);
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
/// transcript 未找到时的负缓存有效期；新会话的 transcript 可能稍后才落盘。
const TRANSCRIPT_MISS_CACHE_TTL: Duration = Duration::from_secs(30);
/// 默认窗口宽度（逻辑像素），与 tauri.conf.json / 前端默认值保持一致。
const DEFAULT_WINDOW_WIDTH_LOGICAL: f64 = 320.0;
/// 托盘监视线程以 80ms（约 12.5fps）为粒度推进动画帧；会话采集周期为 5 秒。
/// NOTE: 隐藏窗口后 WebView2 会节流前端定时器，托盘与通知弹窗必须由 Rust 侧自行采集。
const TRAY_MONITOR_TICK: Duration = Duration::from_millis(80);
const SESSION_MONITOR_INTERVAL: Duration = Duration::from_secs(5);
/// 托盘动画总开关：真机出现高频 set_icon 导致闪烁 / 图标消失时置 false 退回静态图标。
const TRAY_ANIMATION_ENABLED: bool = true;
/// 进行中：透明度呼吸（100% ↔ 25%）叠加轻微缩放（100% ↔ 90%），周期约 1.5 秒。
/// NOTE: 16px 托盘下纯 alpha 变化不够醒目，叠加轻微缩放让呼吸在深浅任务栏上都可见。
const TRAY_WORKING_FRAME_COUNT: usize = 15;
const TRAY_WORKING_PERIOD: Duration = Duration::from_millis(1500);
const TRAY_WORKING_MIN_ALPHA: f64 = 0.25;
const TRAY_WORKING_MIN_SCALE: f64 = 0.9;
/// 告警：整枚图标隐藏 / 显示交替闪烁，每 500ms 切换一次，周期 1 秒。
/// NOTE: 16px 下缩放下压不够醒目；借鉴微信消息提示，直接消失再出现最容易被注意到。
const TRAY_ALERT_FRAME_COUNT: usize = 10;
const TRAY_ALERT_PERIOD: Duration = Duration::from_millis(1000);
/// 隐藏帧保留的最低 alpha。
/// NOTE: Windows 会把 alpha 全为 0 的 32bpp 图标按不透明黑色绘制（RGB 为黑时就是黑方块）；
/// 保留 1/255 才能走 alpha 混合，视觉上仍不可见。
const TRAY_HIDDEN_ALPHA: u8 = 1;
/// 托盘专用图标：原三色信号图形 + 深绿空心圆角描边，内部保持透明。
/// NOTE: 深绿在亮 / 暗任务栏上都有明确轮廓，同时比纯黑更接近品牌色。
const TRAY_BADGE_BORDER_COLOR: [u8; 3] = [4, 120, 87];
/// 图形缩到 0.85，与 2px 描边留出间距，避免红色弧线贴到边框。
const TRAY_BADGE_GLYPH_SCALE: f64 = 0.85;
/// 圆角半径占短边的比例，接近 Windows 11 应用图标的圆角。
const TRAY_BADGE_CORNER_RATIO: f64 = 0.22;
/// 描边宽度占短边的比例，32px 下约 2px，缩放到 16px 托盘后约 1px。
const TRAY_BADGE_BORDER_RATIO: f64 = 0.0625;
/// 通知弹窗单条最多列举数与标题截断长度，避免内容过长。
const NOTIFICATION_MAX_ITEMS: usize = 3;
const NOTIFICATION_TITLE_MAX_CHARS: usize = 24;
const NOTIFICATION_TITLE_PREFIX: &str = "Pinglet";
/// 自绘通知弹窗：固定宽度（逻辑像素）、内容高度上下限、右下角边距与前端淡出后的兜底隐藏时长。
const NOTIFICATION_WINDOW_WIDTH_LOGICAL: f64 = 380.0;
const NOTIFICATION_WINDOW_MIN_HEIGHT_LOGICAL: f64 = 96.0;
const NOTIFICATION_WINDOW_HEIGHT_LOGICAL: f64 = 176.0;
const NOTIFICATION_WINDOW_MARGIN_LOGICAL: f64 = 16.0;
const NOTIFICATION_AUTO_HIDE_FALLBACK: Duration = Duration::from_secs(8);
/// 自动主题采样：窗口底边下方 6 逻辑像素处横向取 12 个点，取亮度中位数。
const BACKDROP_SAMPLE_OFFSET_LOGICAL: f64 = 6.0;
const BACKDROP_SAMPLE_COUNT: i32 = 12;
/// 与 tauri-plugin-window-state 的默认文件名一致，用于判断是否已有可恢复的位置。
const WINDOW_STATE_FILENAME: &str = tauri_plugin_window_state::DEFAULT_FILENAME;

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
static TRANSCRIPT_MISS_CACHE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

/// 与前端 `classify()` 一致的看板四态；托盘与通知只需要异常 / 进行中的聚合计数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoardKind {
    Blocked,
    Interrupted,
    Working,
    Done,
}

impl BoardKind {
    fn classify(status: &str, alive: Option<bool>) -> Self {
        // NOTE: 进程退出时无论最后状态是什么都归为已中断，与前端保持一致。
        if alive == Some(false) {
            return Self::Interrupted;
        }
        match status.to_ascii_lowercase().as_str() {
            "waiting" | "blocked" => Self::Blocked,
            "failed" | "crashed" | "stopped" => Self::Interrupted,
            "idle" | "done" => Self::Done,
            _ => Self::Working,
        }
    }

    fn is_alert(self) -> bool {
        matches!(self, Self::Blocked | Self::Interrupted)
    }
}

struct AlertItem {
    key: String,
    title: String,
    kind: BoardKind,
}

/// 通知弹窗中的一条告警；kind 决定状态文案的颜色。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationPopupItem {
    title: String,
    kind: &'static str,
    status: &'static str,
}

/// 通知弹窗内容：标题汇总 + 最多三条明细 + 剩余数量。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationPopupPayload {
    title: String,
    items: Vec<NotificationPopupItem>,
    more: Option<String>,
}

#[derive(Default)]
struct SessionSnapshot {
    blocked: usize,
    interrupted: usize,
    working: usize,
    alert_keys: HashSet<String>,
    alert_items: Vec<AlertItem>,
}

/// 与前端 `getRowKey()` 相同的身份规则，用于判断哪些警告是新增的。
fn session_key(row: &SessionRow) -> String {
    if let Some(pid) = row.pid {
        return format!("pid:{pid}");
    }
    if let Some(cwd) = row.cwd.as_deref() {
        return format!("cwd:{cwd}");
    }
    format!("name:{}", row.name)
}

/// 告警事件身份：会话身份 + 待处理状态时间戳。
/// NOTE: 只用 session_key 无法区分「同一会话再次进入待处理」——当 waiting → busy → waiting
/// 的 busy 间隔短于 5 秒轮询周期时，轮询看不到离开告警态的那一刻，alert_keys 永不释放，
/// 第二次请求会被当成旧告警静默丢弃。待处理停留期间 statusUpdatedAt 不变，带上时间戳既能
/// 识别新一次请求，也不会在同一次等待中重复弹窗。
/// 已中断仍只用会话身份：进程已死，文件被重写不应再次唤醒弹窗。
fn alert_event_key(row: &SessionRow, kind: BoardKind) -> String {
    let key = session_key(row);
    if kind == BoardKind::Blocked {
        if let Some(timestamp) = row.status_updated_at {
            return format!("{key}:{timestamp}");
        }
    }
    key
}

fn build_snapshot(rows: &[SessionRow]) -> SessionSnapshot {
    let mut snapshot = SessionSnapshot::default();
    for row in rows {
        let kind = BoardKind::classify(&row.status, row.alive);
        match kind {
            BoardKind::Blocked => snapshot.blocked += 1,
            BoardKind::Interrupted => snapshot.interrupted += 1,
            BoardKind::Working => snapshot.working += 1,
            BoardKind::Done => {}
        }
        if kind.is_alert() {
            let key = alert_event_key(row, kind);
            snapshot.alert_keys.insert(key.clone());
            snapshot.alert_items.push(AlertItem {
                key,
                title: row.title.clone(),
                kind,
            });
        }
    }
    snapshot
}

fn tray_tooltip(blocked: usize, interrupted: usize, working: usize) -> String {
    let mut parts = vec![NOTIFICATION_TITLE_PREFIX.to_string()];
    if blocked > 0 {
        parts.push(format!("{blocked} 待处理"));
    }
    if interrupted > 0 {
        parts.push(format!("{interrupted} 已中断"));
    }
    if working > 0 {
        parts.push(format!("{working} 进行中"));
    }
    parts.join(" · ")
}

fn notification_title(blocked: usize, interrupted: usize) -> String {
    let total = blocked + interrupted;
    if interrupted == 0 {
        format!("{NOTIFICATION_TITLE_PREFIX} · {total} 个会话待处理")
    } else if blocked == 0 {
        format!("{NOTIFICATION_TITLE_PREFIX} · {total} 个会话已中断")
    } else {
        format!("{NOTIFICATION_TITLE_PREFIX} · {total} 个会话需要关注")
    }
}

fn notification_popup_payload(
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayIconKind {
    Idle,
    Working,
    Alert,
}

/// 三种托盘状态在启动时预生成动画帧，运行期只切换帧引用，不做重复缩放。
struct TrayIcons {
    idle: Vec<Image<'static>>,
    working: Vec<Image<'static>>,
    alert: Vec<Image<'static>>,
}

impl TrayIcons {
    fn get(&self, kind: TrayIconKind, frame: usize) -> &Image<'static> {
        let frames = match kind {
            TrayIconKind::Idle => &self.idle,
            TrayIconKind::Working => &self.working,
            TrayIconKind::Alert => &self.alert,
        };
        let index = frame % frames.len();
        &frames[index]
    }
}

/// 托盘专用图标：原三色信号图形 + 深绿空心圆角描边。
/// NOTE: 圆底会裁掉方形画布四角的红色弧线，只能把图形缩到 0.765；圆角方框能容纳更大的图形。
fn build_tray_badge_icon(base: &Image<'_>) -> Image<'static> {
    let width = base.width();
    let height = base.height();
    let expected_len = width as usize * height as usize * 4;
    if width == 0 || height == 0 || base.rgba().len() < expected_len {
        return base.clone().to_owned();
    }

    let glyph = scale_tray_icon(base, TRAY_BADGE_GLYPH_SCALE);
    let side = f64::from(width.min(height));
    let outer_radius = side * TRAY_BADGE_CORNER_RATIO;
    let border = side * TRAY_BADGE_BORDER_RATIO;
    let inner_radius = (outer_radius - border).max(0.0);
    let mut pixels = vec![0_u8; expected_len];

    for y in 0..height {
        for x in 0..width {
            let outer = rounded_rect_coverage(x, y, width, height, 0.0, outer_radius);
            if outer <= 0.0 {
                continue;
            }

            // 外框覆盖率减去内框覆盖率就是描边环；环内像素保持透明，只叠图形。
            let inner = if border > 0.0 {
                rounded_rect_coverage(x, y, width, height, border, inner_radius)
            } else {
                0.0
            };
            let ring = (outer - inner).clamp(0.0, 1.0);
            let index = ((y * width + x) * 4) as usize;
            let glyph_alpha = f64::from(glyph.rgba()[index + 3]) / 255.0;
            let alpha = glyph_alpha + ring * (1.0 - glyph_alpha);
            if alpha <= 0.0 {
                continue;
            }

            for channel in 0..3 {
                let border_color = f64::from(TRAY_BADGE_BORDER_COLOR[channel]);
                let glyph_color = f64::from(glyph.rgba()[index + channel]);
                let premultiplied =
                    glyph_color * glyph_alpha + border_color * ring * (1.0 - glyph_alpha);
                pixels[index + channel] = (premultiplied / alpha).round().clamp(0.0, 255.0) as u8;
            }
            pixels[index + 3] = (alpha * 255.0).round() as u8;
        }
    }

    Image::new_owned(pixels, width, height)
}

/// 像素中心到圆角矩形边界的近似距离，返回 0–1 覆盖率做 1px 抗锯齿。
/// `inset` 表示从画布边缘向内收缩的距离，用于计算描边内边界。
fn rounded_rect_coverage(x: u32, y: u32, width: u32, height: u32, inset: f64, radius: f64) -> f64 {
    let half_width = f64::from(width) / 2.0 - inset;
    let half_height = f64::from(height) / 2.0 - inset;
    let offset_x = (f64::from(x) + 0.5 - f64::from(width) / 2.0).abs() - (half_width - radius);
    let offset_y = (f64::from(y) + 0.5 - f64::from(height) / 2.0).abs() - (half_height - radius);
    let outside = offset_x.max(0.0).hypot(offset_y.max(0.0));
    let inside = offset_x.max(offset_y).min(0.0);
    (0.5 - (outside + inside - radius)).clamp(0.0, 1.0)
}

fn build_tray_icons(base: &Image<'_>) -> TrayIcons {
    TrayIcons {
        idle: vec![base.clone().to_owned()],
        working: build_breathing_frames(
            base,
            TRAY_WORKING_FRAME_COUNT,
            TRAY_WORKING_MIN_ALPHA,
            TRAY_WORKING_MIN_SCALE,
        ),
        alert: build_alert_frames(base, TRAY_ALERT_FRAME_COUNT),
    }
}

/// 托盘当前展示的计数与上次写入的图标帧 / tooltip，避免每 80ms 重复设置。
struct TrayRuntime {
    blocked: usize,
    interrupted: usize,
    working: usize,
    alert_keys: HashSet<String>,
    last_icon: Option<(TrayIconKind, usize)>,
    last_animation_kind: Option<TrayIconKind>,
    animation_started: Option<Instant>,
    last_tooltip: Option<String>,
}

impl Default for TrayRuntime {
    fn default() -> Self {
        Self {
            blocked: 0,
            interrupted: 0,
            working: 0,
            alert_keys: HashSet::new(),
            last_icon: Some((TrayIconKind::Idle, 0)),
            last_animation_kind: None,
            animation_started: None,
            last_tooltip: Some(NOTIFICATION_TITLE_PREFIX.to_string()),
        }
    }
}

/// 生成进行中呼吸帧：透明度与尺寸按同一条余弦曲线从 100% 平滑降到最低值再回升。
fn build_breathing_frames(
    base: &Image<'_>,
    frame_count: usize,
    min_alpha: f64,
    min_scale: f64,
) -> Vec<Image<'static>> {
    let frame_count = frame_count.max(1);
    (0..frame_count)
        .map(|frame| {
            let phase = frame as f64 / frame_count as f64 * std::f64::consts::TAU;
            let breath = 0.5 + 0.5 * phase.cos();
            let alpha = min_alpha + (1.0 - min_alpha) * breath;
            let scale = min_scale + (1.0 - min_scale) * breath;
            let dimmed = scale_tray_alpha(base, alpha);
            scale_tray_icon(&dimmed, scale)
        })
        .collect()
}

/// 生成告警帧：前半周期显示完整图标，后半周期整枚近乎全透明，形成闪烁。
fn build_alert_frames(base: &Image<'_>, frame_count: usize) -> Vec<Image<'static>> {
    let frame_count = frame_count.max(1);
    let hidden = build_hidden_tray_frame(base);
    (0..frame_count)
        .map(|frame| {
            if alert_frame_visible(frame, frame_count) {
                base.clone().to_owned()
            } else {
                hidden.clone()
            }
        })
        .collect()
}

/// 生成「隐藏」帧：保留底图像素，alpha 压到最低可见值，避免 Windows 按黑色方块绘制。
fn build_hidden_tray_frame(base: &Image<'_>) -> Image<'static> {
    let width = base.width();
    let height = base.height();
    let source = base.rgba();
    let expected_len = width as usize * height as usize * 4;
    if width == 0 || height == 0 || source.len() < expected_len {
        return base.clone().to_owned();
    }

    let mut pixels = source[..expected_len].to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = TRAY_HIDDEN_ALPHA;
    }
    Image::new_owned(pixels, width, height)
}

/// 前半周期可见、后半周期隐藏；帧数为奇数时隐藏段多一帧。
fn alert_frame_visible(frame: usize, frame_count: usize) -> bool {
    frame < frame_count / 2
}

/// 只调整 alpha 通道，保留原图像素颜色；透明底上的图形实现整体呼吸。
fn scale_tray_alpha(base: &Image<'_>, alpha_scale: f64) -> Image<'static> {
    let width = base.width();
    let height = base.height();
    let source = base.rgba();
    let expected_len = width as usize * height as usize * 4;
    if width == 0 || height == 0 || source.len() < expected_len {
        return base.clone().to_owned();
    }

    let alpha_scale = alpha_scale.clamp(0.0, 1.0);
    let mut pixels = source[..expected_len].to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = (f64::from(pixel[3]) * alpha_scale).round() as u8;
    }
    Image::new_owned(pixels, width, height)
}

/// 围绕画布中心缩放图标；缩小后四周补透明像素，保证弹跳不越界。
/// NOTE: 采样先乘 alpha 再插值（预乘 alpha），避免透明边缘与图形混色产生暗边。
fn scale_tray_icon(base: &Image<'_>, scale: f64) -> Image<'static> {
    let width = base.width();
    let height = base.height();
    let source = base.rgba();
    let expected_len = width as usize * height as usize * 4;
    if width == 0 || height == 0 || source.len() < expected_len {
        return base.clone().to_owned();
    }

    let scale = scale.clamp(0.0, 1.0);
    if scale >= 1.0 {
        return base.clone().to_owned();
    }

    let mut pixels = vec![0u8; expected_len];
    let center_x = f64::from(width) / 2.0;
    let center_y = f64::from(height) / 2.0;

    for y in 0..height {
        for x in 0..width {
            // 目标像素中心反向映射到源图坐标，再转换为双线性采样的网格坐标
            let source_x = (f64::from(x) + 0.5 - center_x) / scale + center_x;
            let source_y = (f64::from(y) + 0.5 - center_y) / scale + center_y;
            let grid_x = source_x - 0.5;
            let grid_y = source_y - 0.5;
            let x0 = grid_x.floor() as i64;
            let y0 = grid_y.floor() as i64;
            let fraction_x = grid_x - x0 as f64;
            let fraction_y = grid_y - y0 as f64;

            let mut channels = [0.0_f64; 4];
            for (offset_y, weight_y) in [(0_i64, 1.0 - fraction_y), (1, fraction_y)] {
                for (offset_x, weight_x) in [(0_i64, 1.0 - fraction_x), (1, fraction_x)] {
                    let sample_x = x0 + offset_x;
                    let sample_y = y0 + offset_y;
                    if sample_x < 0
                        || sample_y < 0
                        || sample_x >= i64::from(width)
                        || sample_y >= i64::from(height)
                    {
                        continue;
                    }
                    let index = ((sample_y as u32 * width + sample_x as u32) * 4) as usize;
                    let alpha = f64::from(source[index + 3]) / 255.0;
                    let weight = weight_x * weight_y;
                    channels[0] += f64::from(source[index]) * alpha * weight;
                    channels[1] += f64::from(source[index + 1]) * alpha * weight;
                    channels[2] += f64::from(source[index + 2]) * alpha * weight;
                    channels[3] += alpha * weight;
                }
            }

            let output_alpha = channels[3];
            if output_alpha <= 0.0 {
                continue;
            }
            let index = ((y * width + x) * 4) as usize;
            pixels[index] = (channels[0] / output_alpha).round().clamp(0.0, 255.0) as u8;
            pixels[index + 1] = (channels[1] / output_alpha).round().clamp(0.0, 255.0) as u8;
            pixels[index + 2] = (channels[2] / output_alpha).round().clamp(0.0, 255.0) as u8;
            pixels[index + 3] = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    Image::new_owned(pixels, width, height)
}

/// 按经过时间计算动画帧，避免依赖 tick 次数导致掉帧后动画变慢。
fn tray_animation_frame(elapsed: Duration, frame_count: usize, period: Duration) -> usize {
    if frame_count <= 1 {
        return 0;
    }
    let period_ms = period.as_millis().max(1);
    let elapsed_ms = elapsed.as_millis() % period_ms;
    ((elapsed_ms * frame_count as u128) / period_ms) as usize
}

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
    system_notifications: AtomicBool,
    do_not_disturb: AtomicBool,
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
            do_not_disturb: AtomicBool::new(false),
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

/// 解析 Claude Code 的内部会话文件；字段随版本变化时只需改这里。
/// NOTE: 异步命令会在线程池执行，避免文件扫描阻塞主线程。
#[tauri::command(async)]
fn list_sessions() -> ListResult {
    collect_sessions()
}

/// 前端轮询与后台托盘监控共用的扫描入口。
fn collect_sessions() -> ListResult {
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
    scored.sort_by(|left, right| right.0.cmp(&left.0));

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

/// 前端设置变化时同步免打扰与通知消息开关，托盘闪烁立即按新状态生效。
#[tauri::command]
fn set_notification_preferences(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    system_notifications: bool,
    do_not_disturb: bool,
) {
    state
        .system_notifications
        .store(system_notifications, Ordering::Relaxed);
    let previous_dnd = state.do_not_disturb.swap(do_not_disturb, Ordering::Relaxed);
    // 关闭免打扰时重新评估当前告警：窗口仍隐藏且仍有告警，则恢复未读闪烁
    if previous_dnd && !do_not_disturb && !state.window_active.load(Ordering::Relaxed) {
        let has_alert = state
            .tray_runtime
            .lock()
            .map(|runtime| runtime.blocked + runtime.interrupted > 0)
            .unwrap_or(false);
        if has_alert {
            state.tray_alert_unread.store(true, Ordering::Relaxed);
        }
    }
    refresh_tray(&app, Instant::now());
}

/// 恢复默认位置（主屏右上角）并把位置写回 window-state，下次启动不再使用旧坐标。
#[tauri::command]
fn reset_window_position(app: tauri::AppHandle) -> Result<(), String> {
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
fn sample_backdrop_luminance(window: tauri::WebviewWindow) -> Option<f32> {
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
    if samples.len() % 2 == 0 {
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

    let default_icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("默认窗口图标".to_string()))?;
    let tray_base = build_tray_badge_icon(&default_icon);
    let tray_icons = build_tray_icons(&tray_base);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icons.idle[0].clone())
        .tooltip(NOTIFICATION_TITLE_PREFIX)
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

    {
        let state = app.state::<AppState>();
        if let Ok(mut item) = state.show_item.lock() {
            *item = Some(show_item);
        }
        if let Ok(mut item) = state.click_through_item.lock() {
            *item = Some(click_through_item);
        }
        if let Ok(mut icons) = state.tray_icons.lock() {
            *icons = Some(tray_icons);
        }
        state.tray_ready.store(true, Ordering::Relaxed);
    }
    refresh_tray(app, Instant::now());

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

/// 读取窗口真实显隐并广播给前端；窗口回到前台时清除托盘未读提醒。
/// NOTE: Windows 上 WebView2 在窗口隐藏后会节流后台计时器，前端因此不能作为
/// 显隐的权威来源；轮询与心跳的启停统一由这里发出的 `pinglet://window-visibility` 决定。
fn sync_window_activity(app: &tauri::AppHandle) {
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

    if previous != active {
        if let Err(err) = window.emit(EVENT_WINDOW_VISIBILITY, active) {
            eprintln!("pinglet: 广播窗口可见性失败: {err}");
        }
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
fn show_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    let state = app.state::<AppState>();
    // 先递增代号，取消可能仍在等待的延迟隐藏
    state.window_hide_generation.fetch_add(1, Ordering::Relaxed);

    // 穿透状态下窗口收不到任何点击，必须先恢复可点击
    if state.click_through.load(Ordering::Relaxed) {
        if let Err(err) = apply_click_through(app, &state, false) {
            eprintln!("pinglet: 关闭鼠标穿透失败: {err}");
        }
    }
    if window.is_minimized().unwrap_or(false) {
        if let Err(err) = window.unminimize() {
            eprintln!("pinglet: 还原窗口失败: {err}");
        }
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
fn toggle_window_visibility(app: &tauri::AppHandle) {
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
fn open_settings(app: &tauri::AppHandle) {
    show_main_window(app);
    if let Err(err) = app.emit("pinglet://open-settings", ()) {
        eprintln!("pinglet: 广播打开设置失败: {err}");
    }
}

fn toggle_main_window(app: &tauri::AppHandle) {
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

/// 根据当前计数与动画相位更新托盘图标帧与 tooltip，只在内容变化时写入。
fn refresh_tray(app: &tauri::AppHandle, now: Instant) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = app.state::<AppState>();
    let dnd = state.do_not_disturb.load(Ordering::Relaxed);
    let unread = state.tray_alert_unread.load(Ordering::Relaxed);
    let active = state.window_active.load(Ordering::Relaxed);

    let Ok(icons_guard) = state.tray_icons.lock() else {
        return;
    };
    let Some(icons) = icons_guard.as_ref() else {
        return;
    };
    let Ok(mut runtime) = state.tray_runtime.lock() else {
        return;
    };

    // 优先级：可播放的告警动画 > 进行中呼吸 > 静态空闲
    let has_alert = runtime.blocked + runtime.interrupted > 0;
    let alert_animated = has_alert && unread && !dnd && !active;
    let kind = if alert_animated {
        TrayIconKind::Alert
    } else if runtime.working > 0 {
        TrayIconKind::Working
    } else {
        TrayIconKind::Idle
    };
    let display_kind = if TRAY_ANIMATION_ENABLED {
        kind
    } else {
        TrayIconKind::Idle
    };
    let (frame_count, period) = if TRAY_ANIMATION_ENABLED {
        match display_kind {
            TrayIconKind::Working => (TRAY_WORKING_FRAME_COUNT, TRAY_WORKING_PERIOD),
            TrayIconKind::Alert => (TRAY_ALERT_FRAME_COUNT, TRAY_ALERT_PERIOD),
            TrayIconKind::Idle => (1, Duration::ZERO),
        }
    } else {
        (1, Duration::ZERO)
    };

    // 状态切换时重新起算动画，保证告警弹跳从完整图标开始
    if runtime.last_animation_kind != Some(display_kind) {
        runtime.last_animation_kind = Some(display_kind);
        runtime.animation_started = Some(now);
    }
    let frame = if frame_count <= 1 {
        0
    } else {
        let started = runtime.animation_started.unwrap_or(now);
        tray_animation_frame(now.duration_since(started), frame_count, period)
    };
    let icon_key = (display_kind, frame);

    if runtime.last_icon != Some(icon_key) {
        match tray.set_icon(Some(icons.get(display_kind, frame).clone())) {
            Ok(()) => runtime.last_icon = Some(icon_key),
            Err(err) => eprintln!("pinglet: 更新托盘图标失败: {err}"),
        }
    }

    let tooltip = tray_tooltip(runtime.blocked, runtime.interrupted, runtime.working);
    if runtime.last_tooltip.as_deref() != Some(tooltip.as_str()) {
        match tray.set_tooltip(Some(tooltip.as_str())) {
            Ok(()) => runtime.last_tooltip = Some(tooltip),
            Err(err) => eprintln!("pinglet: 更新托盘提示失败: {err}"),
        }
    }
}

/// 启动后台会话监控：Rust 侧独立轮询，不依赖会被 WebView2 节流的 JS 计时器。
fn spawn_session_monitor(app: tauri::AppHandle) {
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
    let dnd = state.do_not_disturb.load(Ordering::Relaxed);
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
    } else if !*first_scan && !active && !dnd && !new_alerts.is_empty() {
        state.tray_alert_unread.store(true, Ordering::Relaxed);
    }

    // 首次扫描只播种，避免启动时把历史告警全部弹成通知。
    // NOTE: 待处理告警的去重键带状态时间戳（见 alert_event_key），每次新的等待请求都会
    // 进入 new_alerts，即使中间的 busy 状态短于轮询周期也能识别；同一次等待不会重复弹。
    // 因此不设全局冷却——否则同一会话短时间内第二次告警会被静默丢弃。
    if !*first_scan && system_notifications && !dnd && !active && !new_alerts.is_empty() {
        let payload =
            notification_popup_payload(snapshot.blocked, snapshot.interrupted, &new_alerts);
        if let Err(err) = show_notification_popup(app, payload) {
            eprintln!("pinglet: 显示通知弹窗失败: {err}");
        }
    }

    *first_scan = false;
    refresh_tray(app, Instant::now());
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
/// NOTE: 未命中结果缓存一段时间，避免为尚未生成 transcript 的新会话重复扫描目录。
fn find_transcript(session_id: &str) -> Option<PathBuf> {
    if let Ok(cache) = transcript_miss_cache().lock() {
        if let Some(checked_at) = cache.get(session_id) {
            if checked_at.elapsed() < TRANSCRIPT_MISS_CACHE_TTL {
                return None;
            }
        }
    }

    let projects = projects_dir()?;
    for entry in fs::read_dir(projects).ok()?.flatten() {
        let candidate = entry.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            if let Ok(mut cache) = transcript_miss_cache().lock() {
                cache.remove(session_id);
            }
            return Some(candidate);
        }
    }

    if let Ok(mut cache) = transcript_miss_cache().lock() {
        cache.retain(|_, checked_at| checked_at.elapsed() < TRANSCRIPT_MISS_CACHE_TTL);
        cache.insert(session_id.to_string(), Instant::now());
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

fn transcript_miss_cache() -> &'static Mutex<HashMap<String, Instant>> {
    TRANSCRIPT_MISS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
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

/// 恢复的位置可能来自已断开的显示器；必要时平移回当前显示器工作区。
fn clamp_window_into_work_area(window: &tauri::WebviewWindow) {
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
    if x != position.x || y != position.y {
        if let Err(err) = window.set_position(tauri::PhysicalPosition::new(x, y)) {
            eprintln!("pinglet: 修正窗口位置失败: {err}");
        }
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
    .title(NOTIFICATION_TITLE_PREFIX)
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
fn show_notification_popup(
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
            if let Some(window) = thread_app.get_webview_window(NOTIFICATION_WINDOW_LABEL) {
                if let Err(err) = hide_notification_window_raw(&window) {
                    eprintln!("pinglet: 自动隐藏通知弹窗失败: {err}");
                }
            }
        });
    if let Err(err) = result {
        eprintln!("pinglet: 启动通知隐藏计时失败: {err}");
    }
}

/// 通知弹窗加载时读取最新内容；窗口复用时通过事件触发重新读取。
#[tauri::command]
fn get_notification_payload(state: tauri::State<'_, AppState>) -> Option<NotificationPopupPayload> {
    state
        .notification_payload
        .lock()
        .ok()
        .and_then(|payload| payload.clone())
}

/// 前端点击关闭或倒计时结束后调用；递增代号让 Rust 兜底计时提前失效。
#[tauri::command]
fn hide_notification_window(app: tauri::AppHandle) {
    app.state::<AppState>()
        .notification_generation
        .fetch_add(1, Ordering::Relaxed);
    if let Some(window) = app.get_webview_window(NOTIFICATION_WINDOW_LABEL) {
        if let Err(err) = hide_notification_window_raw(&window) {
            eprintln!("pinglet: 隐藏通知弹窗失败: {err}");
        }
    }
}

/// 点击通知卡片：收起弹窗并唤起主窗口。
#[tauri::command]
fn open_main_window(app: tauri::AppHandle) {
    hide_notification_window(app.clone());
    show_main_window(&app);
}

/// 收起动画结束后由前端调用；代号过期说明窗口已重新显示，直接忽略。
#[tauri::command]
fn hide_main_window_now(app: tauri::AppHandle, generation: u64) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.window_hide_generation.load(Ordering::Relaxed) != generation {
        return Ok(());
    }
    // 本次隐藏已执行，递增代号让兜底线程与迟到的重复调用失效
    state.window_hide_generation.fetch_add(1, Ordering::Relaxed);
    hide_main_window_internal(&app)
}

/// 前端按内容实测高度调整弹窗，避免透明空白区域遮挡桌面点击。
#[tauri::command]
fn resize_notification_window(app: tauri::AppHandle, height: f64) -> Result<(), String> {
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
                            sync_window_activity(&window.app_handle());
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
                WindowEvent::Resized(_) => sync_window_activity(&window.app_handle()),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            activate_session,
            quit_app,
            set_close_to_tray,
            set_click_through,
            set_notification_preferences,
            get_notification_payload,
            hide_notification_window,
            open_main_window,
            hide_main_window_now,
            resize_notification_window,
            reset_window_position,
            sample_backdrop_luminance,
            is_autostart_enabled,
            set_autostart,
            set_global_shortcut
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

    fn session_row(
        pid: Option<u32>,
        cwd: Option<&str>,
        status: &str,
        alive: Option<bool>,
    ) -> SessionRow {
        SessionRow {
            pid,
            name: "demo".to_string(),
            title: "演示会话".to_string(),
            session_id: None,
            cwd: cwd.map(str::to_string),
            status: status.to_string(),
            status_updated_at: None,
            alive,
            entrypoint: None,
        }
    }

    #[test]
    fn classifies_like_frontend() {
        // 进程退出优先于状态字段，与前端 classify() 一致
        assert_eq!(
            BoardKind::classify("done", Some(false)),
            BoardKind::Interrupted
        );
        assert_eq!(
            BoardKind::classify("waiting", Some(true)),
            BoardKind::Blocked
        );
        assert_eq!(BoardKind::classify("BLOCKED", None), BoardKind::Blocked);
        for status in ["failed", "crashed", "stopped"] {
            assert_eq!(
                BoardKind::classify(status, Some(true)),
                BoardKind::Interrupted
            );
        }
        for status in ["idle", "done"] {
            assert_eq!(BoardKind::classify(status, Some(true)), BoardKind::Done);
        }
        for status in [
            "busy", "shell", "working", "starting", "running", "resuming", "adopted", "unknown",
        ] {
            assert_eq!(BoardKind::classify(status, Some(true)), BoardKind::Working);
        }
    }

    #[test]
    fn session_key_prefers_pid_then_cwd() {
        assert_eq!(
            session_key(&session_row(Some(42), Some("C:\\demo"), "busy", Some(true))),
            "pid:42"
        );
        assert_eq!(
            session_key(&session_row(None, Some("C:\\demo"), "busy", Some(true))),
            "cwd:C:\\demo"
        );
        assert_eq!(
            session_key(&session_row(None, None, "busy", Some(true))),
            "name:demo"
        );
    }

    fn session_row_at(
        pid: Option<u32>,
        status: &str,
        status_updated_at: Option<i64>,
    ) -> SessionRow {
        SessionRow {
            status_updated_at,
            ..session_row(pid, None, status, Some(true))
        }
    }

    #[test]
    fn alert_event_key_distinguishes_repeated_blocked_requests() {
        let first_wait = session_row_at(Some(7), "waiting", Some(1000));
        let same_wait = session_row_at(Some(7), "waiting", Some(1000));
        let next_wait = session_row_at(Some(7), "waiting", Some(2000));

        // 同一次等待期间 statusUpdatedAt 不变，轮询重复采集不能重复触发弹窗
        assert_eq!(
            alert_event_key(&first_wait, BoardKind::Blocked),
            "pid:7:1000"
        );
        assert_eq!(
            alert_event_key(&same_wait, BoardKind::Blocked),
            "pid:7:1000"
        );
        // 再次进入待处理会刷新 statusUpdatedAt，应视为新的告警事件
        assert_ne!(
            alert_event_key(&next_wait, BoardKind::Blocked),
            alert_event_key(&first_wait, BoardKind::Blocked)
        );
    }

    #[test]
    fn alert_event_key_falls_back_without_blocked_timestamp() {
        let row = session_row_at(Some(3), "waiting", None);

        assert_eq!(alert_event_key(&row, BoardKind::Blocked), "pid:3");
    }

    #[test]
    fn interrupted_alert_key_ignores_status_timestamp() {
        let row = session_row_at(Some(9), "failed", Some(1000));
        let touched = session_row_at(Some(9), "failed", Some(2000));

        // 进程已死：会话文件被重写不应再次唤醒弹窗
        assert_eq!(alert_event_key(&row, BoardKind::Interrupted), "pid:9");
        assert_eq!(alert_event_key(&touched, BoardKind::Interrupted), "pid:9");
    }

    #[test]
    fn snapshot_counts_states_and_alert_keys() {
        let rows = [
            session_row(Some(1), Some("C:\\a"), "waiting", Some(true)),
            session_row(Some(2), Some("C:\\b"), "failed", Some(true)),
            session_row(Some(3), Some("C:\\c"), "busy", Some(true)),
            session_row(Some(4), Some("C:\\d"), "done", Some(false)),
            session_row(Some(5), Some("C:\\e"), "idle", Some(true)),
        ];

        let snapshot = build_snapshot(&rows);
        assert_eq!(snapshot.blocked, 1);
        assert_eq!(snapshot.interrupted, 2);
        assert_eq!(snapshot.working, 1);
        assert_eq!(snapshot.alert_items.len(), 3);
        assert!(snapshot.alert_keys.contains("pid:1"));
        // 进程已退出时即使状态是 done 也算已中断
        assert!(snapshot.alert_keys.contains("pid:4"));
        assert!(!snapshot.alert_keys.contains("pid:3"));
    }

    #[test]
    fn tooltip_omits_zero_counts() {
        assert_eq!(tray_tooltip(0, 0, 0), "Pinglet");
        assert_eq!(tray_tooltip(1, 0, 2), "Pinglet · 1 待处理 · 2 进行中");
        assert_eq!(tray_tooltip(0, 3, 0), "Pinglet · 3 已中断");
        assert_eq!(
            tray_tooltip(2, 3, 4),
            "Pinglet · 2 待处理 · 3 已中断 · 4 进行中"
        );
    }

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

    #[test]
    fn median_luminance_handles_odd_and_even_samples() {
        assert_eq!(median_luminance(&mut []), None);
        assert_eq!(median_luminance(&mut [0.9, 0.1, 0.5]), Some(0.5));
        assert_eq!(median_luminance(&mut [0.4, 0.2]), Some(0.3));
    }

    #[test]
    fn tray_icon_frames_keep_canvas_size_and_alpha_range() {
        let base = Image::new_owned(vec![255_u8; 20 * 20 * 4], 20, 20);
        let icons = build_tray_icons(&base);

        assert_eq!(icons.idle.len(), 1);
        assert_eq!(icons.working.len(), TRAY_WORKING_FRAME_COUNT);
        assert_eq!(icons.alert.len(), TRAY_ALERT_FRAME_COUNT);

        // 所有帧保持与底图相同的画布尺寸，不改变图标尺寸
        for image in icons
            .idle
            .iter()
            .chain(icons.working.iter())
            .chain(icons.alert.iter())
        {
            assert_eq!((image.width(), image.height()), (20, 20));
        }

        // 呼吸帧同时降低 alpha 并轻微收缩：首帧完全不透明且铺满画布，
        // 最暗帧中心保留颜色、四角因收缩变成透明
        let center = ((10 * 20 + 10) * 4) as usize;
        let alphas: Vec<u8> = icons
            .working
            .iter()
            .map(|image| image.rgba()[center + 3])
            .collect();
        assert_eq!(alphas[0], 255);
        let darkest_index = alphas
            .iter()
            .enumerate()
            .min_by_key(|(_, alpha)| **alpha)
            .map(|(index, _)| index)
            .expect("darkest frame");
        // 帧数不整除半周期时，采样到的最暗帧会略高于理论下限
        let min_alpha = (TRAY_WORKING_MIN_ALPHA * 255.0).round() as u8;
        assert!(alphas[darkest_index] >= min_alpha);
        assert!(alphas[darkest_index] <= min_alpha + 3);
        assert_eq!(
            &icons.working[darkest_index].rgba()[center..center + 3],
            &[255, 255, 255]
        );
        assert_eq!(icons.working[darkest_index].rgba()[3], 0);

        // 告警帧前半周期显示完整图标，后半周期整枚透明，形成隐藏 / 显示闪烁
        assert_eq!(icons.alert[0].rgba()[3], 255);
        assert_eq!(icons.alert[0].rgba()[(19 * 20 + 19) * 4 + 3], 255);
        let hidden_frame = TRAY_ALERT_FRAME_COUNT / 2;
        for frame in 0..TRAY_ALERT_FRAME_COUNT {
            let is_visible = frame < hidden_frame;
            let corner_alpha = icons.alert[frame].rgba()[(19 * 20 + 19) * 4 + 3];
            let center_alpha = icons.alert[frame].rgba()[center + 3];
            let expected_alpha = if is_visible { 255 } else { TRAY_HIDDEN_ALPHA };
            assert_eq!(corner_alpha, expected_alpha);
            assert_eq!(center_alpha, expected_alpha);
        }
    }

    #[test]
    fn tray_badge_icon_draws_outline_and_keeps_interior_transparent() {
        // 用透明底 + 中央不透明绿色方块模拟当前透明图标
        let mut pixels = vec![0_u8; 32 * 32 * 4];
        for y in 8..24 {
            for x in 8..24 {
                let index = ((y * 32 + x) * 4) as usize;
                pixels[index] = 46;
                pixels[index + 1] = 213;
                pixels[index + 2] = 115;
                pixels[index + 3] = 255;
            }
        }
        let base = Image::new_owned(pixels, 32, 32);
        let tray = build_tray_badge_icon(&base);

        assert_eq!((tray.width(), tray.height()), (32, 32));
        // 圆角外四角保持透明
        assert_eq!(tray.rgba()[3], 0);
        // 顶边中点为深绿描边
        let border_pixel = ((16 * 32 + 1) * 4) as usize;
        assert_eq!(
            &tray.rgba()[border_pixel..border_pixel + 4],
            &[4, 120, 87, 255]
        );
        // 描边与图形之间的内部区域不填充
        let interior_pixel = ((4 * 32 + 16) * 4) as usize;
        assert_eq!(tray.rgba()[interior_pixel + 3], 0);
        // 中央图形保留原色
        let center = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(&tray.rgba()[center..center + 4], &[46, 213, 115, 255]);
    }

    #[test]
    fn alert_frames_alternate_between_visible_and_hidden() {
        assert!(alert_frame_visible(0, TRAY_ALERT_FRAME_COUNT));
        assert!(alert_frame_visible(
            TRAY_ALERT_FRAME_COUNT / 2 - 1,
            TRAY_ALERT_FRAME_COUNT
        ));
        assert!(!alert_frame_visible(
            TRAY_ALERT_FRAME_COUNT / 2,
            TRAY_ALERT_FRAME_COUNT
        ));
        assert!(!alert_frame_visible(
            TRAY_ALERT_FRAME_COUNT - 1,
            TRAY_ALERT_FRAME_COUNT
        ));
        // 帧数为奇数时隐藏段多一帧，避免显示帧数超过半个周期
        assert!(alert_frame_visible(3, 9));
        assert!(!alert_frame_visible(4, 9));
    }

    #[test]
    fn tray_animation_frame_is_time_driven() {
        assert_eq!(
            tray_animation_frame(
                Duration::ZERO,
                TRAY_WORKING_FRAME_COUNT,
                TRAY_WORKING_PERIOD
            ),
            0
        );
        assert_eq!(
            tray_animation_frame(
                TRAY_WORKING_PERIOD / 2,
                TRAY_WORKING_FRAME_COUNT,
                TRAY_WORKING_PERIOD
            ),
            TRAY_WORKING_FRAME_COUNT / 2
        );
        // 一个周期后从头播放，且始终落在有效帧范围内
        assert_eq!(
            tray_animation_frame(
                TRAY_WORKING_PERIOD,
                TRAY_WORKING_FRAME_COUNT,
                TRAY_WORKING_PERIOD
            ),
            0
        );
        let last = tray_animation_frame(
            TRAY_ALERT_PERIOD - Duration::from_millis(1),
            TRAY_ALERT_FRAME_COUNT,
            TRAY_ALERT_PERIOD,
        );
        assert_eq!(last, TRAY_ALERT_FRAME_COUNT - 1);
        assert_eq!(
            tray_animation_frame(Duration::from_secs(1), 1, TRAY_ALERT_PERIOD),
            0
        );
    }

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
