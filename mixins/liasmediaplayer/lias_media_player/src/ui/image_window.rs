use super::widgets::*;
use crate::{
    core::image::DecodedImage,
    platform::{Canvas, InputEvent, Key, Rect, TextureId},
};
use parking_lot::Mutex;
use std::{sync::Arc, time::Duration};

pub struct ImageView {
    pub image: Option<Arc<DecodedImage>>,
    pub texture: Option<TextureId>,
    pub status: String,
    pub frame: usize,
    pub paused: bool,
    pub zoom: f32,
    pub pan: [f32; 2],
    elapsed_ms: f64,
    pub dirty: bool,
}
impl Default for ImageView {
    fn default() -> Self {
        Self {
            image: None,
            texture: None,
            status: "Loading image…".into(),
            frame: 0,
            paused: false,
            zoom: 1.,
            pan: [0., 0.],
            elapsed_ms: 0.,
            dirty: false,
        }
    }
}
impl ImageView {
    pub fn step(&mut self, forward: bool) {
        if let Some(image) = &self.image {
            self.frame = if forward {
                (self.frame + 1) % image.frames.len()
            } else {
                (self.frame + image.frames.len() - 1) % image.frames.len()
            };
            self.paused = true;
            self.elapsed_ms = 0.;
            self.dirty = true;
        }
    }
    pub fn advance(&mut self, dt: Duration) {
        let Some(image) = &self.image else {
            return;
        };
        if self.paused || image.frames.len() <= 1 {
            return;
        }
        let total: u64 = image.frames.iter().map(|f| f.delay_ms.max(20) as u64).sum();
        self.elapsed_ms += (dt.as_secs_f64() * 1000.) % total as f64;
        for _ in 0..image.frames.len() {
            let duration = image.frames[self.frame].delay_ms.max(20) as f64;
            if self.elapsed_ms < duration {
                break;
            }
            self.elapsed_ms -= duration;
            self.frame = (self.frame + 1) % image.frames.len();
            self.dirty = true;
        }
    }
}
pub struct ImageWindow {
    id: WidgetId,
    rect: Rect,
    view: Arc<Mutex<ImageView>>,
    interactive: bool,
    drag: Option<([f32; 2], [f32; 2])>,
}
impl ImageWindow {
    pub fn new(view: Arc<Mutex<ImageView>>) -> Self {
        Self {
            id: Label::new("").id(),
            rect: Rect::default(),
            view,
            interactive: true,
            drag: None,
        }
    }
    fn picture_rect(&self) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y,
            self.rect.w,
            (self.rect.h - if self.interactive { 22. } else { 0. }).max(0.),
        )
    }
    fn reset(view: &mut ImageView) {
        view.zoom = 1.;
        view.pan = [0., 0.];
    }
}
impl Widget for ImageWindow {
    fn handles_ctrl_scroll(&self) -> bool {
        true
    }
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
    fn set_interactive(&mut self, interactive: bool) {
        self.interactive = interactive;
    }
    fn focusables(&self, out: &mut Vec<WidgetId>) {
        out.push(self.id);
    }
    fn draw(&self, c: &mut dyn Canvas, _: &DrawContext<'_>) {
        let v = self.view.lock();
        let area = self.picture_rect();
        c.fill(area, 0xff15171b);
        c.push_clip(area);
        if let (Some(image), Some(texture)) = (&v.image, v.texture) {
            let scale = (area.w / image.width as f32).min(area.h / image.height as f32) * v.zoom;
            let w = image.width as f32 * scale;
            let h = image.height as f32 * scale;
            c.texture(
                texture,
                Rect::new(
                    area.x + (area.w - w) / 2. + v.pan[0],
                    area.y + (area.h - h) / 2. + v.pan[1],
                    w,
                    h,
                ),
                [0., 0., 1., 1.],
            );
        } else {
            // Wrap errors within the body instead of clipping the useful explanation.
            let mut line = String::new();
            let mut y = area.y + 8.;
            for word in v.status.split_whitespace() {
                if !line.is_empty()
                    && c.text_width(&format!("{line} {word}")) > (area.w - 12.).max(1.)
                {
                    c.text(&line, [area.x + 6., y], 0xffffcccc, false);
                    line.clear();
                    y += 12.;
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            c.text(&line, [area.x + 6., y], 0xffeeeeee, false);
        }
        c.pop_clip();
        if self.interactive {
            let bar = Rect::new(self.rect.x, area.bottom(), self.rect.w, 22.);
            c.fill(bar, 0xff252a33);
            c.push_clip(bar);
            for (i, label) in [if v.paused { "Play" } else { "Pause" }, "<", ">", "Fit"]
                .iter()
                .enumerate()
            {
                c.text(
                    label,
                    [bar.x + i as f32 * 38. + 4., bar.y + 6.],
                    0xffffffff,
                    false,
                );
            }
            if let Some(image) = &v.image {
                c.text(
                    &format!("{}/{}", v.frame + 1, image.frames.len()),
                    [bar.x + 156., bar.y + 6.],
                    0xffcccccc,
                    false,
                );
            }
            c.pop_clip();
        }
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        let mut v = self.view.lock();
        match e {
            InputEvent::Cancel => {
                self.drag = None;
                false
            }
            InputEvent::Down { at, button: 0, .. } if self.rect.contains(*at) => {
                ctx.input.focus = Some(self.id);
                ctx.input.capture = Some(self.id);
                if at[1] >= self.picture_rect().bottom() && self.interactive {
                    match ((at[0] - self.rect.x) / 38.) as usize {
                        0 => v.paused = !v.paused,
                        1 => v.step(false),
                        2 => v.step(true),
                        3 => Self::reset(&mut v),
                        _ => {}
                    }
                    self.drag = None;
                } else {
                    self.drag = Some((*at, v.pan));
                }
                true
            }
            InputEvent::Drag { at, button: 0, .. } if ctx.input.capture == Some(self.id) => {
                if let Some((start, pan)) = self.drag {
                    v.pan = [pan[0] + at[0] - start[0], pan[1] + at[1] - start[1]];
                }
                true
            }
            InputEvent::Up { button: 0, .. } if ctx.input.capture == Some(self.id) => {
                self.drag = None;
                ctx.input.capture = None;
                true
            }
            InputEvent::Scroll { at, delta, mods }
                if mods.ctrl && self.picture_rect().contains(*at) && delta.is_finite() =>
            {
                let area = self.picture_rect();
                let old = v.zoom;
                v.zoom = (old * (delta * 0.15).clamp(-2., 2.).exp()).clamp(0.1, 16.);
                let ratio = v.zoom / old;
                for (i, center) in [area.x + area.w / 2., area.y + area.h / 2.]
                    .into_iter()
                    .enumerate()
                {
                    v.pan[i] = (at[i] - center) - (at[i] - center - v.pan[i]) * ratio;
                }
                true
            }
            InputEvent::Key { key, .. } if ctx.input.focus == Some(self.id) => {
                match key {
                    Key::Space => v.paused = !v.paused,
                    Key::Left => v.step(false),
                    Key::Right => v.step(true),
                    Key::Home => Self::reset(&mut v),
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }
}
