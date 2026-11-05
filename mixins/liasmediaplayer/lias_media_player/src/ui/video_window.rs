//! Video and audio transport controls share the same actor-backed view.
use super::widgets::*;
use crate::{
    core::player::{PlaybackState, PlayerCommand, PlayerControl, PlayerSnapshot},
    platform::{Canvas, InputEvent, Key, Rect, TextureId},
};
use parking_lot::Mutex;
use std::sync::Arc;

pub struct PlaybackView {
    pub control: Arc<PlayerControl>,
    pub texture: Option<TextureId>,
    pub size: Option<(u32, u32)>,
    pub error: Option<String>,
    pub queue_open: bool,
    pub auto_quality: bool,
    pub details_open: bool,
}
impl PlaybackView {
    pub fn new(control: Arc<PlayerControl>) -> Self {
        Self {
            control,
            texture: None,
            size: None,
            error: None,
            queue_open: false,
            auto_quality: true,
            details_open: false,
        }
    }
    pub fn send(&mut self, command: PlayerCommand) {
        if let Err(e) = self.control.send(command) {
            self.error = Some(e);
        }
    }
}
pub struct ErrorPanel;
impl ErrorPanel {
    pub fn draw(c: &mut dyn Canvas, area: Rect, message: &str) {
        Self::draw_scrolled(c, area, message, 0);
    }
    pub fn draw_scrolled(c: &mut dyn Canvas, area: Rect, message: &str, offset: usize) -> usize {
        let area = area.inset(6.);
        c.push_clip(area);
        c.fill(area, 0xf0271921);
        let tool = if message.to_lowercase().contains("yt-dlp") {
            "yt-dlp"
        } else if message.to_lowercase().contains("ffmpeg") {
            "FFmpeg"
        } else if message.to_lowercase().contains("ffprobe") {
            "FFprobe"
        } else {
            "Media"
        };
        let header =
            super::text::ellipsize(c, &format!("{tool} failed · Retry below"), area.w - 8.);
        c.text(&header, [area.x + 4., area.y + 4.], 0xffff909b, false);
        let detail = Rect::new(
            area.x + 4.,
            area.y + 20.,
            (area.w - 8.).max(0.),
            (area.h - 36.).max(0.),
        );
        let lines = super::text::wrap(c, message, detail.w);
        let count = (detail.h / 12.).floor().max(0.) as usize;
        let offset = offset.min(lines.len().saturating_sub(count));
        c.push_clip(detail);
        for (i, line) in lines.iter().skip(offset).take(count).enumerate() {
            c.text(
                line,
                [detail.x, detail.y + i as f32 * 12.],
                0xffffdce0,
                false,
            );
        }
        c.pop_clip();
        if lines.len() > count {
            let hint = super::text::ellipsize(
                c,
                "Scroll for details · Full error in game log",
                area.w - 8.,
            );
            c.text(&hint, [area.x + 4., area.bottom() - 12.], 0xffbda9ae, false);
        }
        c.pop_clip();
        lines.len().saturating_sub(count)
    }
}
pub struct NowPlayingBanner;
impl NowPlayingBanner {
    pub fn draw(c: &mut dyn Canvas, snapshot: &PlayerSnapshot) {
        let [width, height] = c.size();
        let area = Rect::new(
            8.,
            (height - 24.).max(0.),
            (width - 16.).clamp(0., 380.),
            18.,
        );
        c.push_clip(area);
        c.fill(area, 0xcc151a24);
        c.text(
            &format!("Now playing: {}", snapshot.title),
            [area.x + 5., area.y + 5.],
            0xffeeeeee,
            false,
        );
        c.pop_clip();
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Play,
    Previous,
    Next,
    Back,
    Forward,
    Speaker,
    Repeat,
    Shuffle,
    Queue,
    More,
}
#[derive(Clone)]
struct TransportLayout {
    buttons: Vec<(Control, Rect)>,
    seek: Rect,
    time: Option<Rect>,
    bar: Rect,
}
impl TransportLayout {
    fn new(area: Rect, audio: bool) -> Self {
        let bar = Rect::new(area.x, area.bottom() - 16., area.w, 16.);
        let mut controls = vec![Control::Play];
        if area.w >= 210. {
            controls.push(Control::Previous);
        }
        if !audio && area.w >= 360. {
            controls.extend([Control::Back, Control::Forward]);
        }
        controls.push(Control::Next);
        let mut buttons = Vec::new();
        let mut x = bar.x + 2.;
        for control in controls {
            buttons.push((control, Rect::new(x, bar.y, 14., 16.)));
            x += 15.;
        }
        let mut right = bar.right() - 2.;
        for control in [
            Control::More,
            Control::Queue,
            Control::Shuffle,
            Control::Repeat,
            Control::Speaker,
        ] {
            right -= 14.;
            buttons.push((control, Rect::new(right, bar.y, 14., 16.)));
            right -= 1.;
        }
        let time = (area.w >= 240.).then(|| {
            let r = Rect::new(x + 2., bar.y, 69., 16.);
            x += 73.;
            r
        });
        Self {
            buttons,
            seek: Rect::new(x + 2., bar.y + 3., (right - x - 4.).max(1.), 10.),
            time,
            bar,
        }
    }
    fn button(&self, control: Control) -> Rect {
        self.buttons
            .iter()
            .find(|(a, _)| *a == control)
            .map_or(Rect::default(), |(_, r)| *r)
    }
}

pub const QUEUE_WIDTH: f32 = 120.;
pub const QUEUE_HEIGHT: f32 = 84.;
pub const AUDIO_HEIGHT: f32 = 31.;
pub const DETAIL_HEIGHT: f32 = 100.;
pub struct VideoWindow {
    id: WidgetId,
    rect: Rect,
    view: Arc<Mutex<PlaybackView>>,
    audio_only: bool,
    interactive: bool,
    drag: Option<bool>,
    queue_offset: usize,
    selected: Option<usize>,
    queue_drag: Option<(usize, f32)>,
    queue_drop: Option<usize>,
    last_click: Option<(usize, std::time::Instant)>,
    error_offset: usize,
    error_max: std::cell::Cell<usize>,
    unmuted_volume: f32,
    volume_open: std::cell::Cell<bool>,
    tooltip: std::cell::RefCell<Option<(String, Rect)>>,
}
pub struct AudioBar;
impl AudioBar {
    pub fn window(view: Arc<Mutex<PlaybackView>>) -> VideoWindow {
        VideoWindow::with_kind(view, true)
    }
}
impl VideoWindow {
    pub fn new(view: Arc<Mutex<PlaybackView>>) -> Self {
        Self::with_kind(view, false)
    }
    fn with_kind(view: Arc<Mutex<PlaybackView>>, audio_only: bool) -> Self {
        Self {
            id: Label::new("").id(),
            rect: Rect::default(),
            view,
            audio_only,
            interactive: true,
            drag: None,
            queue_offset: 0,
            selected: None,
            queue_drag: None,
            queue_drop: None,
            last_click: None,
            error_offset: 0,
            error_max: std::cell::Cell::new(0),
            unmuted_volume: 1.,
            volume_open: std::cell::Cell::new(false),
            tooltip: std::cell::RefCell::new(None),
        }
    }
    fn player_area(&self, view: &PlaybackView) -> Rect {
        let mut r = self.rect;
        if self.interactive {
            if self.audio_only {
                r.h = 16.;
            } else if view.queue_open {
                r.w = (r.w - QUEUE_WIDTH).max(0.);
            }
        }
        r
    }
    fn picture(&self, view: &PlaybackView) -> Rect {
        let mut r = self.player_area(view);
        if self.interactive {
            r.h = (r.h - 16.).max(0.);
        }
        r
    }
    fn queue_area(&self, view: &PlaybackView) -> Rect {
        if self.audio_only {
            Rect::new(
                self.rect.x,
                self.rect.y + 16.,
                self.rect.w,
                (self.rect.h - 16.).max(0.),
            )
        } else {
            let p = self.player_area(view);
            Rect::new(p.right(), p.y, QUEUE_WIDTH.min(self.rect.w), p.h)
        }
    }
    fn volume_area(&self, t: &TransportLayout) -> Rect {
        // Popover stays inside the player; in the short audio bar it runs horizontally.
        let b = t.button(Control::Speaker);
        Rect::new(
            (b.x - 54.).max(self.rect.x),
            t.bar.y,
            64f32.min(self.rect.w),
            16.,
        )
    }
    fn queue_row(&self, view: &PlaybackView, at: [f32; 2], len: usize) -> Option<usize> {
        let r = self.queue_area(view);
        if !r.contains(at) || at[1] < r.y + 14. {
            return None;
        }
        let rows = ((r.h - 14.) / 14.).floor().max(1.) as usize;
        let offset = self.queue_offset.min(len.saturating_sub(rows));
        let index = offset + ((at[1] - r.y - 14.) / 14.) as usize;
        (index < len).then_some(index)
    }
    fn set_drag_value(&self, view: &mut PlaybackView, at: [f32; 2], volume: bool) {
        let s = view.control.snapshot();
        let t = TransportLayout::new(self.player_area(view), self.audio_only);
        let r = if volume {
            self.volume_area(&t).inset(3.)
        } else {
            t.seek
        };
        let v = ((at[0] - r.x) / r.w.max(1.)).clamp(0., 1.);
        if volume {
            view.send(PlayerCommand::SetVolume(v));
        } else if let Some(d) = s.duration_secs.filter(|_| !s.is_live) {
            view.send(PlayerCommand::Seek(v as f64 * d));
        }
    }
    fn draw_queue(
        &self,
        c: &mut dyn Canvas,
        ctx: &DrawContext<'_>,
        view: &PlaybackView,
        s: &PlayerSnapshot,
    ) {
        use super::theme::ThemeRole as R;
        let r = self.queue_area(view);
        c.fill(r, ctx.theme.color(R::PanelBg));
        c.push_clip(r);
        c.border(r, ctx.theme.color(R::Border));
        let label = super::text::ellipsize(c, &format!("Up next · {}", s.queue.len()), r.w - 19.);
        c.text(
            &label,
            [r.x + 4., r.y + 3.],
            ctx.theme.color(R::TextSubtle),
            false,
        );
        super::icons::draw(
            c,
            Rect::new(r.right() - 14., r.y, 14., 14.),
            super::icons::Icon::Close,
            ctx.theme.color(R::Icon),
        );
        let rows = ((r.h - 14.) / 14.).floor().max(0.) as usize;
        let offset = self.queue_offset.min(s.queue.len().saturating_sub(rows));
        if s.queue.is_empty() {
            super::text::paragraph(
                c,
                Rect::new(r.x + 4., r.y + 20., (r.w - 8.).max(0.), (r.h - 20.).max(0.)),
                "Nothing queued",
                ctx.theme.color(R::TextDim),
            );
        }
        for (i, title) in s.queue_titles.iter().enumerate().skip(offset).take(rows) {
            let row = Rect::new(
                r.x + 1.,
                r.y + 14. + (i - offset) as f32 * 14.,
                (r.w - 2.).max(0.),
                14.,
            );
            if self.selected == Some(i) || ctx.mouse.is_some_and(|p| row.contains(p)) {
                c.fill(
                    row,
                    ctx.theme.color(if self.selected == Some(i) {
                        R::RowSelected
                    } else {
                        R::RowHover
                    }),
                );
            }
            let label = super::text::ellipsize(c, &format!("{}. {title}", i + 1), row.w - 20.);
            c.text(
                &label,
                [row.x + 3., row.y + 3.],
                ctx.theme.color(R::Text),
                false,
            );
            let remove = Rect::new(row.right() - 14., row.y, 14., 14.);
            super::icons::draw(
                c,
                remove,
                super::icons::Icon::Close,
                ctx.theme
                    .color(if ctx.mouse.is_some_and(|p| remove.contains(p)) {
                        R::Danger
                    } else {
                        R::TextDim
                    }),
            );
            if self.queue_drop == Some(i) {
                c.fill(Rect::new(row.x, row.y, row.w, 1.), ctx.theme.color(R::Fill));
            }
        }
        if s.queue.len() > rows && rows > 0 {
            let height = r.h - 14.;
            let thumb = (height * rows as f32 / s.queue.len() as f32).max(4.);
            c.fill(
                Rect::new(
                    r.right() - 2.,
                    r.y + 14. + offset as f32 / (s.queue.len() - rows) as f32 * (height - thumb),
                    2.,
                    thumb,
                ),
                ctx.theme.color(R::ScrollThumb),
            );
        }
        c.pop_clip();
    }
}
impl Widget for VideoWindow {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn measure(&self, _: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        max
    }
    fn layout(&mut self, rect: Rect) {
        self.rect = rect;
    }
    fn set_interactive(&mut self, value: bool) {
        self.interactive = value;
    }
    fn focusables(&self, out: &mut Vec<WidgetId>) {
        out.push(self.id);
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        use super::{
            icons::{self, Icon},
            theme::ThemeRole as R,
        };
        let view = self.view.lock();
        let s = view.control.snapshot();
        let picture = self.picture(&view);
        c.push_clip(self.rect);
        if !self.audio_only {
            c.fill(picture, ctx.theme.color(R::Placeholder));
            c.push_clip(picture);
            if let (Some(texture), Some((w, h))) = (view.texture, view.size) {
                let scale = (picture.w / w as f32).min(picture.h / h as f32);
                c.texture(
                    texture,
                    Rect::new(
                        picture.x + (picture.w - w as f32 * scale) / 2.,
                        picture.y + (picture.h - h as f32 * scale) / 2.,
                        w as f32 * scale,
                        h as f32 * scale,
                    ),
                    [0., 0., 1., 1.],
                );
            }
            if let Some(error) = view.error.as_ref().or(s.error.as_ref()) {
                self.error_max.set(ErrorPanel::draw_scrolled(
                    c,
                    picture,
                    error,
                    self.error_offset,
                ));
            } else if s.state != PlaybackState::Playing || s.sound_only {
                let label = super::text::ellipsize(
                    c,
                    &format!("{:?} · {}", s.state, s.title),
                    picture.w - 8.,
                );
                c.text(
                    &label,
                    [picture.x + 4., picture.y + 5.],
                    ctx.theme.color(R::TextSubtle),
                    false,
                );
            }
            c.pop_clip();
        }
        if !self.interactive {
            if self.audio_only {
                c.fill(self.rect, ctx.theme.color(R::TitleBarBg));
                let title = super::text::ellipsize(
                    c,
                    &format!("{:?} · {}", s.state, s.title),
                    self.rect.w - 8.,
                );
                c.text(
                    &title,
                    [self.rect.x + 4., self.rect.y + 5.],
                    ctx.theme.color(R::Text),
                    false,
                );
                if let Some(d) = s.duration_secs.filter(|d| *d > 0.) {
                    c.fill(
                        Rect::new(
                            self.rect.x,
                            self.rect.bottom() - 2.,
                            self.rect.w * (s.position_secs / d).clamp(0., 1.) as f32,
                            2.,
                        ),
                        ctx.theme.color(R::Fill),
                    );
                }
            }
            c.pop_clip();
            return;
        }
        let t = TransportLayout::new(self.player_area(&view), self.audio_only);
        c.fill(t.bar, ctx.theme.color(R::ControlBarBg));
        let mut tooltip = None;
        for (control, r) in &t.buttons {
            let (icon, label, active) = match control {
                Control::Play => (
                    if matches!(
                        s.state,
                        PlaybackState::Playing
                            | PlaybackState::Buffering
                            | PlaybackState::Resolving
                    ) {
                        Icon::Pause
                    } else {
                        Icon::Play
                    },
                    if s.state == PlaybackState::Failed {
                        "Retry"
                    } else if s.state == PlaybackState::Paused {
                        "Play"
                    } else {
                        "Play / pause"
                    },
                    false,
                ),
                Control::Previous => (Icon::Previous, "Previous track", false),
                Control::Next => (Icon::Next, "Next track", false),
                Control::Back => (Icon::Back, "Back 10 seconds", false),
                Control::Forward => (Icon::Forward, "Forward 10 seconds", false),
                Control::Speaker => (
                    Icon::Speaker,
                    "Mute · Scroll to change volume",
                    s.volume <= 0.,
                ),
                Control::Repeat => (
                    Icon::Repeat,
                    if s.looping {
                        "Repeat one"
                    } else if s.repeat_all {
                        "Repeat all"
                    } else {
                        "Repeat off"
                    },
                    s.looping || s.repeat_all,
                ),
                Control::Shuffle => (
                    Icon::Shuffle,
                    if s.shuffle {
                        "Shuffle on"
                    } else {
                        "Shuffle off"
                    },
                    s.shuffle,
                ),
                Control::Queue => (Icon::Queue, "Show / hide queue", view.queue_open),
                Control::More => (
                    Icon::More,
                    "Playback options / error details",
                    view.details_open,
                ),
            };
            if ctx.mouse.is_some_and(|p| r.contains(p)) {
                c.fill(*r, ctx.theme.color(R::RowHover));
                tooltip = Some((label, *r));
            }
            icons::draw(
                c,
                *r,
                icon,
                ctx.theme
                    .color(if *control == Control::Previous && !s.has_previous {
                        R::IconDisabled
                    } else if active {
                        R::IconActive
                    } else {
                        R::Icon
                    }),
            );
            if *control == Control::Repeat && s.looping {
                c.fill(
                    Rect::new(r.x + 6., r.y + 7., 2., 3.),
                    ctx.theme.color(R::IconActive),
                );
            }
        }
        if let Some(r) = t.time {
            let time = if s.is_live {
                "LIVE".into()
            } else {
                format!(
                    "{} / {}",
                    clock(s.position_secs),
                    s.duration_secs.map(clock).unwrap_or("--:--".into())
                )
            };
            let text = super::text::ellipsize(c, &time, r.w);
            c.text(
                &text,
                [r.x, r.y + 4.],
                ctx.theme.color(R::TextSubtle),
                false,
            );
        }
        let track = Rect::new(t.seek.x, t.seek.y + 3., t.seek.w, 3.);
        c.fill(track, ctx.theme.color(R::Track));
        if let Some(d) = s.duration_secs.filter(|d| *d > 0. && !s.is_live) {
            let fraction = (s.position_secs / d).clamp(0., 1.) as f32;
            c.fill(
                Rect::new(track.x, track.y, track.w * fraction, track.h),
                ctx.theme.color(R::Fill),
            );
            c.fill(
                Rect::new(
                    track.x + (track.w - 2.).max(0.) * fraction,
                    track.y - 2.,
                    2.,
                    7.,
                ),
                ctx.theme.color(R::Knob),
            );
        }
        if view.queue_open {
            self.draw_queue(c, ctx, &view, &s);
        }
        if view.details_open {
            let area = if self.audio_only {
                Rect::new(
                    self.rect.x,
                    self.rect.y + 16.,
                    self.rect.w,
                    (self.rect.h - 16.).max(0.),
                )
            } else {
                picture
            };
            c.fill(area, ctx.theme.color(R::PanelBg));
            if let Some(error) = view.error.as_ref().or(s.error.as_ref()) {
                self.error_max
                    .set(ErrorPanel::draw_scrolled(c, area, error, self.error_offset));
            } else {
                for (i, (_, label)) in [
                    format!("Speed: {:.2}x", s.speed),
                    format!(
                        "Quality: {}",
                        if view.auto_quality {
                            "Auto".into()
                        } else {
                            format!("{}p", s.target_height)
                        }
                    ),
                    format!("Frame rate: {} fps", s.fps),
                    "Clear upcoming tracks".into(),
                ]
                .iter()
                .enumerate()
                .filter(|(i, _)| !self.audio_only || *i == 0 || *i == 3)
                .enumerate()
                {
                    let row = Rect::new(
                        area.x + 3.,
                        area.y + 3. + i as f32 * 18.,
                        (area.w - 6.).max(0.),
                        18.,
                    );
                    if row.bottom() > area.bottom() {
                        break;
                    }
                    if ctx.mouse.is_some_and(|p| row.contains(p)) {
                        c.fill(row, ctx.theme.color(R::RowHover));
                    }
                    let text = super::text::ellipsize(c, label, row.w - 6.);
                    c.text(
                        &text,
                        [row.x + 3., row.y + 4.],
                        ctx.theme.color(R::Text),
                        false,
                    );
                }
            }
        }
        let speaker = t.button(Control::Speaker);
        let volume = self.volume_area(&t);
        let over_volume = ctx
            .mouse
            .is_some_and(|p| speaker.contains(p) || (self.volume_open.get() && volume.contains(p)));
        self.volume_open.set(over_volume || self.drag == Some(true));
        if self.volume_open.get() {
            c.fill(volume, ctx.theme.color(R::PanelBg));
            c.border(volume, ctx.theme.color(R::Border));
            c.fill(
                Rect::new(volume.x + 3., volume.y + 6., (volume.w - 6.).max(0.), 3.),
                ctx.theme.color(R::Track),
            );
            c.fill(
                Rect::new(
                    volume.x + 3.,
                    volume.y + 6.,
                    (volume.w - 6.).max(0.) * s.volume,
                    3.,
                ),
                ctx.theme.color(R::Fill),
            );
        }
        c.pop_clip();
        *self.tooltip.borrow_mut() = tooltip.map(|(label, r)| {
            let label = if label.starts_with("Mute") {
                format!(
                    "Volume {}% · Click mute · Scroll",
                    (s.volume * 100.).round()
                )
            } else {
                label.into()
            };
            (label, r)
        });
        if view.queue_open
            && self.queue_drag.is_none()
            && let Some(at) = ctx.mouse
            && let Some(i) = self.queue_row(&view, at, s.queue.len())
        {
            let r = self.queue_area(&view);
            let label = if at[0] >= r.right() - 16. {
                "Remove from queue".into()
            } else {
                s.queue_titles.get(i).cloned().unwrap_or_default()
            };
            *self.tooltip.borrow_mut() = Some((label, Rect::new(at[0], at[1] - 7., 0., 14.)));
        }
    }
    fn draw_overlay(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        if !self.interactive {
            return;
        }
        if let Some((label, r)) = self.tooltip.borrow().as_ref() {
            super::text::tooltip(
                c,
                *r,
                label,
                ctx.theme.color(super::ThemeRole::PanelBg),
                ctx.theme.color(super::ThemeRole::Text),
            );
        }
    }
    fn event(&mut self, event: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        let view_arc = self.view.clone();
        let mut view = view_arc.lock();
        let s = view.control.snapshot();
        let t = TransportLayout::new(self.player_area(&view), self.audio_only);
        match event {
            InputEvent::Cancel => {
                self.drag = None;
                self.queue_drag = None;
                self.queue_drop = None;
                self.volume_open.set(false);
                false
            }
            InputEvent::Down { at, button: 0, .. }
                if self.interactive && self.rect.contains(*at) =>
            {
                ctx.input.focus = Some(self.id);
                ctx.input.capture = Some(self.id);
                if view.queue_open && self.queue_area(&view).contains(*at) {
                    let r = self.queue_area(&view);
                    if at[1] < r.y + 14. && at[0] >= r.right() - 16. {
                        view.queue_open = false;
                        return true;
                    }
                    if let Some(i) = self.queue_row(&view, *at, s.queue.len()) {
                        if at[0] >= r.right() - 16. {
                            view.send(PlayerCommand::RemoveQueue(i));
                            self.last_click = None;
                        } else {
                            self.selected = Some(i);
                            self.queue_drag = Some((i, at[1]));
                        }
                    }
                    return true;
                }
                if self.volume_open.get()
                    && self.volume_area(&t).contains(*at)
                    && !t.button(Control::Speaker).contains(*at)
                {
                    self.drag = Some(true);
                    self.set_drag_value(&mut view, *at, true);
                    return true;
                }
                if let Some((control, _)) = t.buttons.iter().find(|(_, r)| r.contains(*at)) {
                    let command = match control {
                        Control::Play => {
                            self.error_offset = 0;
                            view.error = None;
                            Some(if s.state == PlaybackState::Failed {
                                PlayerCommand::Retry
                            } else {
                                PlayerCommand::Toggle
                            })
                        }
                        Control::Previous => s.has_previous.then_some(PlayerCommand::Previous),
                        Control::Next => Some(PlayerCommand::Next),
                        Control::Back => Some(PlayerCommand::Seek(s.position_secs - 10.)),
                        Control::Forward => Some(PlayerCommand::Seek(s.position_secs + 10.)),
                        Control::Speaker => {
                            if s.volume > 0. {
                                self.unmuted_volume = s.volume;
                                Some(PlayerCommand::SetVolume(0.))
                            } else {
                                Some(PlayerCommand::SetVolume(self.unmuted_volume))
                            }
                        }
                        Control::Repeat => Some(PlayerCommand::CycleRepeat),
                        Control::Shuffle => Some(PlayerCommand::SetShuffle(!s.shuffle)),
                        Control::Queue => {
                            view.queue_open = !view.queue_open;
                            view.details_open = false;
                            None
                        }
                        Control::More => {
                            view.details_open = !view.details_open;
                            view.queue_open = false;
                            None
                        }
                    };
                    if let Some(command) = command {
                        view.send(command);
                    }
                    return true;
                }
                if t.seek.contains(*at) {
                    self.drag = Some(false);
                    return true;
                }
                if view.details_open && view.error.is_none() && s.error.is_none() {
                    let area = if self.audio_only {
                        Rect::new(
                            self.rect.x,
                            self.rect.y + 16.,
                            self.rect.w,
                            self.rect.h - 16.,
                        )
                    } else {
                        self.picture(&view)
                    };
                    let mut i = ((at[1] - area.y - 3.) / 18.) as usize;
                    if self.audio_only && i == 1 {
                        i = 3;
                    }
                    let command = match i {
                        0 => Some(PlayerCommand::SetSpeed(if s.speed >= 2. {
                            0.5
                        } else {
                            s.speed + 0.25
                        })),
                        1 if !self.audio_only => {
                            view.auto_quality = false;
                            Some(PlayerCommand::SetTargetHeight(match s.target_height {
                                360 => 480,
                                480 => 720,
                                720 => 1080,
                                _ => 360,
                            }))
                        }
                        2 if !self.audio_only => {
                            Some(PlayerCommand::SetFps(if s.fps == 30 { 60 } else { 30 }))
                        }
                        3 => Some(PlayerCommand::ClearQueue),
                        _ => None,
                    };
                    if let Some(command) = command {
                        view.send(command);
                    }
                }
                true
            }
            InputEvent::Drag { at, button: 0, .. } if ctx.input.capture == Some(self.id) => {
                if self.drag == Some(true) {
                    self.set_drag_value(&mut view, *at, true);
                }
                if let Some((_, start)) = self.queue_drag
                    && (at[1] - start).abs() > 4.
                {
                    let r = self.queue_area(&view);
                    if at[1] < r.y + 22. {
                        self.queue_offset = self.queue_offset.saturating_sub(1);
                    }
                    if at[1] > r.bottom() - 8. {
                        self.queue_offset =
                            (self.queue_offset + 1).min(s.queue.len().saturating_sub(1));
                    }
                    self.queue_drop = self.queue_row(&view, *at, s.queue.len());
                }
                true
            }
            InputEvent::Up { at, button: 0 } if ctx.input.capture == Some(self.id) => {
                if let Some(volume) = self.drag.take() {
                    self.set_drag_value(&mut view, *at, volume);
                }
                if let Some((from, start)) = self.queue_drag.take() {
                    if let Some(to) = self.queue_drop.take()
                        && (at[1] - start).abs() > 4.
                    {
                        view.send(PlayerCommand::MoveQueue(from, to));
                        self.selected = Some(to);
                        self.last_click = None;
                    } else if self.queue_row(&view, *at, s.queue.len()) == Some(from) {
                        let now = std::time::Instant::now();
                        if self.last_click.is_some_and(|(i, t)| {
                            i == from
                                && now.duration_since(t) < std::time::Duration::from_millis(400)
                        }) {
                            view.send(PlayerCommand::JumpQueue(from));
                            self.last_click = None;
                        } else {
                            self.last_click = Some((from, now));
                        }
                    }
                }
                ctx.input.capture = None;
                true
            }
            InputEvent::Scroll { at, delta, .. } if self.interactive && self.rect.contains(*at) => {
                if t.button(Control::Speaker).contains(*at)
                    || self.volume_open.get() && self.volume_area(&t).contains(*at)
                {
                    view.send(PlayerCommand::SetVolume(s.volume + delta.signum() * 0.05));
                } else if view.queue_open && self.queue_area(&view).contains(*at) {
                    let rows = ((self.queue_area(&view).h - 14.) / 14.).floor().max(1.) as usize;
                    self.queue_offset = (self.queue_offset as isize - delta.signum() as isize)
                        .clamp(0, s.queue.len().saturating_sub(rows) as isize)
                        as usize;
                } else if s.error.is_some() || view.error.is_some() {
                    self.error_offset = (self.error_offset.min(self.error_max.get()) as isize
                        - delta.signum() as isize * 3)
                        .clamp(0, self.error_max.get() as isize)
                        as usize;
                }
                true
            }
            InputEvent::Key { key, .. } if ctx.input.focus == Some(self.id) => {
                let command = match key {
                    Key::Enter if view.queue_open => self.selected.map(PlayerCommand::JumpQueue),
                    Key::Delete if view.queue_open => self.selected.map(PlayerCommand::RemoveQueue),
                    Key::Space => Some(PlayerCommand::Toggle),
                    Key::Left => Some(PlayerCommand::Seek(s.position_secs - 10.)),
                    Key::Right => Some(PlayerCommand::Seek(s.position_secs + 10.)),
                    Key::Up => Some(PlayerCommand::SetVolume(s.volume + 0.05)),
                    Key::Down => Some(PlayerCommand::SetVolume(s.volume - 0.05)),
                    Key::Home => Some(PlayerCommand::Seek(0.)),
                    _ => None,
                };
                if let Some(command) = command {
                    view.send(command);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{player::Player, tools::ToolManager};
    use crate::platform::Modifiers;
    use crate::ui::widgets::InputState;
    use std::time::{Duration, Instant};

    #[test]
    fn compact_transport_keeps_shuffle_repeat_and_queue_in_bounds() {
        for width in [158., 198., 268., 358., 498.] {
            let area = Rect::new(10., 20., width, 16.);
            let layout = TransportLayout::new(area, true);
            for control in [
                Control::Play,
                Control::Next,
                Control::Shuffle,
                Control::Repeat,
                Control::Queue,
                Control::More,
            ] {
                let r = layout.button(control);
                assert!(r.w > 0. && r.x >= area.x && r.right() <= area.right());
                assert_eq!(r.intersect(layout.seek).w, 0.);
            }
        }
    }

    #[test]
    fn queue_single_click_selects_drag_moves_and_x_removes() {
        let root = std::env::temp_dir().join(format!("lmp-queue-ui-{}", std::process::id()));
        let mut tools = ToolManager::new(Some(&root));
        tools.policy.prefer_system = false;
        tools.policy.auto_download = false;
        let player = Player::new(tools, true);
        let control = player.control.clone();
        let urls: Vec<_> = (0..3)
            .map(|i| format!("https://example.org/{i}.mp3"))
            .collect();
        control
            .send(PlayerCommand::Load(
                "https://example.org/current.mp3".into(),
            ))
            .unwrap();
        control
            .send(PlayerCommand::EnqueueBatch(urls.clone()))
            .unwrap();
        let wait_for = |expected: &[String]| {
            let start = Instant::now();
            while control.snapshot().queue != expected {
                assert!(start.elapsed() < Duration::from_secs(2));
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait_for(&urls);
        let view = Arc::new(Mutex::new(PlaybackView::new(control.clone())));
        view.lock().queue_open = true;
        let mut widget = AudioBar::window(view);
        widget.set_interactive(true);
        widget.layout(Rect::new(0., 0., 198., 100.));
        let mut input = InputState::default();
        let mut actions = Vec::new();
        let mut event = |event| {
            widget.event(
                &event,
                &mut EventContext {
                    input: &mut input,
                    actions: &mut actions,
                },
            )
        };
        let down = |at| InputEvent::Down {
            at,
            button: 0,
            mods: Modifiers::default(),
        };
        let up = |at| InputEvent::Up { at, button: 0 };
        event(down([10., 36.]));
        event(up([10., 36.]));
        assert_eq!(control.snapshot().queue, urls);
        event(down([10., 36.]));
        event(InputEvent::Drag {
            at: [10., 64.],
            button: 0,
            mods: Modifiers::default(),
        });
        event(up([10., 64.]));
        wait_for(&[urls[1].clone(), urls[2].clone(), urls[0].clone()]);
        event(down([190., 36.]));
        event(up([190., 36.]));
        wait_for(&[urls[2].clone(), urls[0].clone()]);
        player.join();
    }
}
