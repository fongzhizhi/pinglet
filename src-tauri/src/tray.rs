//! 托盘图标合成、动画帧与状态刷新。

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::Manager;
use tauri::image::Image;
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::window::{
    apply_click_through, open_settings, toggle_main_window, toggle_window_visibility,
};
use crate::{APP_NAME, AppState};

const TRAY_ID: &str = "pinglet-tray";
const TRAY_MENU_SHOW: &str = "show";
const TRAY_MENU_SETTINGS: &str = "settings";
const TRAY_MENU_CLICK_THROUGH: &str = "click-through";
const TRAY_MENU_QUIT: &str = "quit";
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
fn tray_tooltip(blocked: usize, interrupted: usize, working: usize) -> String {
    let mut parts = vec![APP_NAME.to_string()];
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayIconKind {
    Idle,
    Working,
    Alert,
}

/// 三种托盘状态在启动时预生成动画帧，运行期只切换帧引用，不做重复缩放。
pub(crate) struct TrayIcons {
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
pub(crate) struct TrayRuntime {
    pub(crate) blocked: usize,
    pub(crate) interrupted: usize,
    pub(crate) working: usize,
    pub(crate) alert_keys: HashSet<String>,
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
            last_tooltip: Some(APP_NAME.to_string()),
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
    for pixel in pixels.as_chunks_mut::<4>().0 {
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
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[3] = (f64::from(pixel[3]) * alpha_scale).round() as u8;
    }
    Image::new_owned(pixels, width, height)
}

/// 围绕画布中心缩放图标；缩小后四周补透明像素，避免呼吸收缩时越界。
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
pub(crate) fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
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
    // 直接使用应用图标（图形已在 SVG 中放大 1.7 倍），不再叠加托盘专用描边。
    let tray_icons = build_tray_icons(&default_icon);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icons.idle[0].clone())
        .tooltip(APP_NAME)
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
pub(crate) fn sync_show_item(state: &AppState, visible: bool) {
    if let Ok(item) = state.show_item.lock()
        && let Some(item) = item.as_ref()
        && let Err(err) = item.set_checked(visible)
    {
        eprintln!("pinglet: 同步托盘显示勾选状态失败: {err}");
    }
}
/// 根据当前计数与动画相位更新托盘图标帧与 tooltip，只在内容变化时写入。
pub(crate) fn refresh_tray(app: &tauri::AppHandle, now: Instant) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = app.state::<AppState>();
    let alert_flicker = state.tray_alert_flicker.load(Ordering::Relaxed);
    let working_breathing = state.tray_working_breathing.load(Ordering::Relaxed);
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

    // 优先级：可播放的告警动画 > 进行中呼吸 > 静态空闲；
    // 对应动效关闭时仍保留状态，只回退为静态图标。
    let has_alert = runtime.blocked + runtime.interrupted > 0;
    let alert_animated = has_alert && unread && !active && alert_flicker;
    let (display_kind, frame_count, period) = if !TRAY_ANIMATION_ENABLED {
        (TrayIconKind::Idle, 1, Duration::ZERO)
    } else if alert_animated {
        (
            TrayIconKind::Alert,
            TRAY_ALERT_FRAME_COUNT,
            TRAY_ALERT_PERIOD,
        )
    } else if runtime.working > 0 {
        let frames = if working_breathing {
            TRAY_WORKING_FRAME_COUNT
        } else {
            1
        };
        let period = if working_breathing {
            TRAY_WORKING_PERIOD
        } else {
            Duration::ZERO
        };
        (TrayIconKind::Working, frames, period)
    } else {
        (TrayIconKind::Idle, 1, Duration::ZERO)
    };

    // 状态切换时重新起算动画，保证告警闪烁从完整图标开始
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
