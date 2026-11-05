//! Floating window geometry, z-order, input capture and snapping, with no game imports.
use super::{
    theme::{self, ThemeRole as R, ThemeSpec},
    widgets::*,
};
use crate::platform::{Canvas, InputEvent, Key, Rect};
use std::collections::BTreeSet;

pub struct SnapEngine;
impl SnapEngine {
    pub const THRESHOLD: f32 = 6.;
    pub fn axis(start: f32, length: f32, guides: &[f32], threshold: f32) -> f32 {
        let mut best = threshold.max(0.) + 1.;
        let mut delta = 0.;
        for line in [start, start + length, start + length / 2.] {
            for guide in guides {
                let d = guide - line;
                if d.abs() < best {
                    best = d.abs();
                    delta = d;
                }
            }
        }
        if best <= threshold {
            start + delta
        } else {
            start
        }
    }
    pub fn snap(r: Rect, viewport: Rect, others: impl Iterator<Item = Rect>) -> Rect {
        let mut xs = vec![viewport.x, viewport.right(), viewport.x + viewport.w / 2.];
        let mut ys = vec![viewport.y, viewport.bottom(), viewport.y + viewport.h / 2.];
        for other in others {
            xs.extend([other.x, other.right(), other.x + other.w / 2.]);
            ys.extend([other.y, other.bottom(), other.y + other.h / 2.]);
        }
        Rect::new(
            Self::axis(r.x, r.w, &xs, Self::THRESHOLD),
            Self::axis(r.y, r.h, &ys, Self::THRESHOLD),
            r.w,
            r.h,
        )
    }
}
pub struct FloatingWindow {
    pub id: u64,
    pub title: String,
    pub rect: Rect,
    pub min_size: [f32; 2],
    pub compact: bool,
    pub hidden: bool,
    pub pinned: bool,
    pub theatre: bool,
    pub content: Box<dyn Widget>,
}
impl FloatingWindow {
    pub fn new(title: impl Into<String>, rect: Rect, content: Box<dyn Widget>) -> Self {
        Self {
            id: content.id(),
            title: title.into(),
            rect,
            min_size: [120., 80.],
            compact: false,
            hidden: false,
            pinned: true,
            theatre: false,
            content,
        }
    }
    fn frame(&self, viewport: Rect) -> Rect {
        if self.theatre {
            viewport.inset(6.)
        } else {
            self.rect
        }
    }
    fn body(&self, viewport: Rect, interactive: bool) -> Rect {
        let r = self.frame(viewport);
        if interactive {
            if self.compact {
                Rect::new(r.x + 1., r.y + 14., (r.w - 2.).max(0.), (r.h - 15.).max(0.))
            } else {
                Rect::new(r.x + 4., r.y + 24., (r.w - 8.).max(0.), (r.h - 28.).max(0.))
            }
        } else {
            r
        }
    }
}
#[derive(Clone, Copy)]
enum Capture {
    Move {
        id: u64,
        origin: Rect,
        at: [f32; 2],
    },
    Resize {
        id: u64,
        origin: Rect,
        at: [f32; 2],
        left: bool,
        top: bool,
    },
    Content(u64),
}
impl Capture {
    fn id(self) -> u64 {
        match self {
            Self::Move { id, .. } | Self::Resize { id, .. } | Self::Content(id) => id,
        }
    }
}
pub struct WindowManager {
    windows: Vec<FloatingWindow>,
    pub theme: ThemeSpec,
    viewport: Rect,
    input: InputState,
    focused: Option<u64>,
    capture: Option<Capture>,
    buttons: BTreeSet<i32>,
    actions: Vec<UiAction>,
}
impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}
impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            theme: theme::dark(),
            viewport: Rect::default(),
            input: InputState::default(),
            focused: None,
            capture: None,
            buttons: BTreeSet::new(),
            actions: Vec::new(),
        }
    }
    pub fn has_focus(&self) -> bool {
        self.focused.is_some()
    }
    pub fn windows(&self) -> &[FloatingWindow] {
        &self.windows
    }
    pub fn window_mut(&mut self, id: u64) -> Option<&mut FloatingWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }
    pub fn open(&mut self, window: FloatingWindow) {
        if self.windows.iter().any(|w| w.id == window.id) {
            return;
        }
        self.focused = Some(window.id);
        self.windows.push(window);
    }
    pub fn close(&mut self, id: u64) {
        self.windows.retain(|w| w.id != id);
        if self.focused == Some(id) {
            self.focused = None;
            self.input.clear();
        }
        if self.capture.is_some_and(|c| c.id() == id) {
            self.capture = None;
            self.input.capture = None;
        }
    }
    pub fn clear(&mut self) {
        self.windows.clear();
        self.cancel_input();
        self.actions.clear();
    }
    pub fn cancel_input(&mut self) {
        self.input.clear();
        self.capture = None;
        self.focused = None;
        self.buttons.clear();
    }
    pub fn reveal(&mut self) {
        for w in &mut self.windows {
            w.hidden = false;
        }
        self.focused = self.windows.last().map(|w| w.id);
    }
    pub fn take_actions(&mut self) -> Vec<UiAction> {
        std::mem::take(&mut self.actions)
    }
    fn fit(r: Rect, v: Rect, min: [f32; 2]) -> Rect {
        let w = r.w.max(min[0]).min(v.w.max(0.));
        let h = r.h.max(min[1]).min(v.h.max(0.));
        Rect::new(
            r.x.clamp(v.x, (v.right() - w).max(v.x)),
            r.y.clamp(v.y, (v.bottom() - h).max(v.y)),
            w,
            h,
        )
    }
    pub fn draw(&mut self, c: &mut dyn Canvas, interactive: bool, mouse: Option<[f32; 2]>) {
        let [width, height] = c.size();
        self.viewport = Rect::new(0., 0., width, height);
        let top = mouse.and_then(|p| {
            self.windows
                .iter()
                .rev()
                .find(|w| !w.hidden && w.frame(self.viewport).contains(p))
                .map(|w| w.id)
        });
        let addon_actions = crate::api::window_actions();
        for w in &mut self.windows {
            w.rect = Self::fit(w.rect, self.viewport, w.min_size);
            if w.hidden || (!interactive && !w.pinned) {
                continue;
            }
            let r = w.frame(self.viewport);
            let body = w.body(self.viewport, interactive);
            let title_height = if w.compact { 13. } else { 20. };
            let button_width = if w.compact { 14. } else { 20. };
            let text_y = if w.compact { 3. } else { 6. };
            c.next_layer();
            c.push_clip(r);
            c.fill(r, self.theme.color(R::WindowBg));
            if interactive {
                c.fill(
                    Rect::new(r.x, r.y, r.w, title_height),
                    self.theme.color(R::TitleBarBg),
                );
                c.push_clip(Rect::new(
                    r.x + 4.,
                    r.y,
                    (r.w - 8. - (4 + addon_actions.len()) as f32 * button_width).max(0.),
                    title_height,
                ));
                let title = super::text::ellipsize(
                    c,
                    &w.title,
                    (r.w - 8. - (4 + addon_actions.len()) as f32 * button_width).max(0.),
                );
                c.text(
                    &title,
                    [r.x + 4., r.y + text_y],
                    self.theme.color(R::Text),
                    false,
                );
                c.pop_clip();
                for (i, label) in [
                    if w.pinned { "P" } else { "p" },
                    if w.theatre { "R" } else { "T" },
                    "_",
                    "X",
                ]
                .iter()
                .enumerate()
                {
                    let b = Rect::new(
                        r.right() - 4. * button_width + i as f32 * button_width,
                        r.y,
                        button_width,
                        title_height,
                    );
                    if top == Some(w.id) && mouse.is_some_and(|p| b.contains(p)) {
                        c.fill(b, self.theme.color(R::RowHover));
                    }
                    if w.compact {
                        use super::icons::{self, Icon};
                        let icon = [
                            Icon::Pin,
                            if w.theatre {
                                Icon::Restore
                            } else {
                                Icon::Expand
                            },
                            Icon::Minimize,
                            Icon::Close,
                        ][i];
                        icons::draw(
                            c,
                            b,
                            icon,
                            self.theme.color(if i == 0 && w.pinned {
                                R::IconActive
                            } else {
                                R::Icon
                            }),
                        );
                    } else {
                        c.text(
                            label,
                            [b.x + (button_width - 6.) / 2., b.y + text_y],
                            self.theme.color(R::Icon),
                            false,
                        );
                    }
                }
                for (i, action) in addon_actions.iter().enumerate() {
                    let b = Rect::new(
                        r.right() - 5. * button_width - i as f32 * button_width,
                        r.y,
                        button_width,
                        title_height,
                    );
                    if top == Some(w.id) && mouse.is_some_and(|p| b.contains(p)) {
                        c.fill(b, self.theme.color(R::RowHover));
                    }
                    c.text(
                        &action.label,
                        [b.x + (button_width - 6.) / 2., b.y + text_y],
                        self.theme.color(R::Icon),
                        false,
                    );
                }
                c.border(
                    r,
                    self.theme.color(if self.focused == Some(w.id) {
                        R::BorderFocused
                    } else {
                        R::Border
                    }),
                );
                for (x, y) in [
                    (r.x, r.y),
                    (r.right() - 4., r.y),
                    (r.x, r.bottom() - 4.),
                    (r.right() - 4., r.bottom() - 4.),
                ] {
                    c.fill(Rect::new(x, y, 4., 4.), self.theme.color(R::Border));
                }
            }
            w.content.set_interactive(interactive);
            w.content.arrange(c, body);
            c.push_clip(body);
            let ctx = DrawContext {
                theme: &self.theme,
                mouse: if interactive && top == Some(w.id) {
                    mouse
                } else {
                    None
                },
                focus: if interactive && self.focused == Some(w.id) {
                    self.input.focus
                } else {
                    None
                },
            };
            w.content.draw(c, &ctx);
            c.pop_clip();
            c.pop_clip();
            w.content.draw_overlay(c, &ctx);
            if interactive
                && top == Some(w.id)
                && let Some(at) = mouse.filter(|p| p[1] < r.y + title_height)
            {
                let first = r.right() - 4. * button_width;
                let text = if at[0] >= first {
                    [
                        if w.pinned {
                            "Unpin from gameplay"
                        } else {
                            "Pin during gameplay"
                        },
                        if w.theatre {
                            "Restore size"
                        } else {
                            "Theatre mode"
                        },
                        "Hide player · M to reveal",
                        "Close player",
                    ][((at[0] - first) / button_width).floor() as usize]
                        .to_owned()
                } else {
                    w.title.clone()
                };
                super::text::tooltip(
                    c,
                    Rect::new(at[0], r.y, 0., title_height),
                    &text,
                    self.theme.color(R::PanelBg),
                    self.theme.color(R::Text),
                );
            }
        }
    }
    pub fn event(&mut self, e: &InputEvent) -> bool {
        if matches!(e, InputEvent::Cancel) {
            self.cancel_input();
            return false;
        }
        match e {
            InputEvent::Down { at, button, .. } => {
                let Some(index) = self
                    .windows
                    .iter()
                    .rposition(|w| !w.hidden && w.frame(self.viewport).contains(*at))
                else {
                    self.input.focus = None;
                    self.focused = None;
                    return false;
                };
                let w = self.windows.remove(index);
                let id = w.id;
                self.windows.push(w);
                if self.focused != Some(id) {
                    self.input.clear();
                }
                self.focused = Some(id);
                self.buttons.insert(*button);
                if *button != 0 {
                    return true;
                }
                self.input.focus = None;
                let w = self.windows.last_mut().unwrap();
                let r = w.frame(self.viewport);
                let left = at[0] < r.x + 5.;
                let right = at[0] >= r.right() - 5.;
                let top = at[1] < r.y + 5.;
                let bottom = at[1] >= r.bottom() - 5.;
                if !w.theatre && (left || right) && (top || bottom) {
                    self.capture = Some(Capture::Resize {
                        id,
                        origin: w.rect,
                        at: *at,
                        left,
                        top,
                    });
                    return true;
                }
                let title_height = if w.compact { 13. } else { 20. };
                let button_width = if w.compact { 14. } else { 20. };
                if at[1] < r.y + title_height {
                    if at[0] >= r.right() - 4. * button_width {
                        match ((at[0] - (r.right() - 4. * button_width)) / button_width) as usize {
                            0 => w.pinned = !w.pinned,
                            1 => w.theatre = !w.theatre,
                            2 => w.hidden = true,
                            _ => {
                                self.close(id);
                            }
                        }
                    } else if let Some(action) = crate::api::window_actions()
                        .iter()
                        .enumerate()
                        .find(|(i, _)| {
                            Rect::new(
                                r.right() - 5. * button_width - *i as f32 * button_width,
                                r.y,
                                button_width,
                                title_height,
                            )
                            .contains(*at)
                        })
                        .map(|(_, action)| action)
                    {
                        crate::api::publish(crate::api::MediaEvent::WindowAction {
                            id: action.id.clone(),
                            handle: crate::api::MediaHandle(id),
                        });
                    } else if !w.theatre {
                        self.capture = Some(Capture::Move {
                            id,
                            origin: w.rect,
                            at: *at,
                        });
                    }
                    return true;
                }
                self.capture = Some(Capture::Content(id));
                w.content.event(
                    e,
                    &mut EventContext {
                        input: &mut self.input,
                        actions: &mut self.actions,
                    },
                );
                true
            }
            InputEvent::Drag {
                at,
                button: 0,
                mods,
            } => {
                let Some(capture) = self.capture else {
                    return self.buttons.contains(&0);
                };
                let id = capture.id();
                let Some(index) = self.windows.iter().position(|w| w.id == id) else {
                    return false;
                };
                match capture {
                    Capture::Move {
                        origin, at: start, ..
                    } => {
                        let mut r = Rect::new(
                            origin.x + at[0] - start[0],
                            origin.y + at[1] - start[1],
                            origin.w,
                            origin.h,
                        );
                        if !mods.shift {
                            r = SnapEngine::snap(
                                r,
                                self.viewport,
                                self.windows
                                    .iter()
                                    .filter(|w| w.id != id && !w.hidden)
                                    .map(|w| w.frame(self.viewport)),
                            );
                        }
                        self.windows[index].rect =
                            Self::fit(r, self.viewport, self.windows[index].min_size);
                    }
                    Capture::Resize {
                        origin,
                        at: start,
                        left,
                        top,
                        ..
                    } => {
                        let min = self.windows[index].min_size;
                        let dx = at[0] - start[0];
                        let dy = at[1] - start[1];
                        let x = if left {
                            (origin.x + dx)
                                .min(origin.right() - min[0])
                                .max(self.viewport.x)
                        } else {
                            origin.x
                        };
                        let y = if top {
                            (origin.y + dy)
                                .min(origin.bottom() - min[1])
                                .max(self.viewport.y)
                        } else {
                            origin.y
                        };
                        let right = if left {
                            origin.right()
                        } else {
                            (origin.right() + dx)
                                .max(x + min[0])
                                .min(self.viewport.right())
                        };
                        let bottom = if top {
                            origin.bottom()
                        } else {
                            (origin.bottom() + dy)
                                .max(y + min[1])
                                .min(self.viewport.bottom())
                        };
                        self.windows[index].rect =
                            Self::fit(Rect::new(x, y, right - x, bottom - y), self.viewport, min);
                    }
                    Capture::Content(_) => {
                        self.windows[index].content.event(
                            e,
                            &mut EventContext {
                                input: &mut self.input,
                                actions: &mut self.actions,
                            },
                        );
                    }
                }
                true
            }
            InputEvent::Up { button, .. } => {
                let used = self.buttons.remove(button);
                if *button == 0 {
                    if let Some(Capture::Content(id)) = self.capture
                        && let Some(w) = self.windows.iter_mut().find(|w| w.id == id)
                    {
                        w.content.event(
                            e,
                            &mut EventContext {
                                input: &mut self.input,
                                actions: &mut self.actions,
                            },
                        );
                    }
                    self.capture = None;
                    self.input.capture = None;
                }
                used
            }
            InputEvent::Scroll { at, delta, mods } => {
                let Some(w) = self
                    .windows
                    .iter_mut()
                    .rev()
                    .find(|w| !w.hidden && w.frame(self.viewport).contains(*at))
                else {
                    return false;
                };
                if (!mods.ctrl || w.content.handles_ctrl_scroll())
                    && w.body(self.viewport, true).contains(*at)
                    && w.content.event(
                        e,
                        &mut EventContext {
                            input: &mut self.input,
                            actions: &mut self.actions,
                        },
                    )
                {
                    return true;
                }
                if mods.ctrl && !w.theatre && delta.is_finite() {
                    let r = w.rect;
                    let factor = (delta * 0.1).clamp(-2., 2.).exp();
                    let width = (r.w * factor).max(w.min_size[0]).min(self.viewport.w);
                    let height = (r.h * factor).max(w.min_size[1]).min(self.viewport.h);
                    let x = at[0] - (at[0] - r.x) * width / r.w.max(1.);
                    let y = at[1] - (at[1] - r.y) * height / r.h.max(1.);
                    w.rect = Self::fit(Rect::new(x, y, width, height), self.viewport, w.min_size);
                }
                true
            }
            InputEvent::Key {
                key: Key::Tab,
                mods,
            } => {
                if self.focused.is_none() {
                    self.focused = self.windows.iter().rev().find(|w| !w.hidden).map(|w| w.id);
                }
                let Some(w) = self
                    .windows
                    .iter_mut()
                    .find(|w| Some(w.id) == self.focused && !w.hidden)
                else {
                    return false;
                };
                let mut ids = Vec::new();
                w.content.focusables(&mut ids);
                if ids.is_empty() {
                    return false;
                }
                let next = match ids.iter().position(|id| Some(*id) == self.input.focus) {
                    Some(i) if mods.shift => (i + ids.len() - 1) % ids.len(),
                    Some(i) => (i + 1) % ids.len(),
                    None if mods.shift => ids.len() - 1,
                    None => 0,
                };
                self.input.focus = Some(ids[next]);
                w.content.reveal_focus(ids[next]);
                true
            }
            InputEvent::Key { .. } | InputEvent::Char(_) => {
                if let Some(w) = self
                    .windows
                    .iter_mut()
                    .find(|w| Some(w.id) == self.focused && !w.hidden)
                {
                    w.content.event(
                        e,
                        &mut EventContext {
                            input: &mut self.input,
                            actions: &mut self.actions,
                        },
                    )
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
