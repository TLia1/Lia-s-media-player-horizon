//! Retained widgets: layout owns geometry, InputState owns focus and pointer capture.
use super::theme::{ThemeRole as R, ThemeSpec};
use crate::platform::{Canvas, InputEvent, Key, Rect, TextureId};
use std::sync::atomic::{AtomicU64, Ordering};
pub type WidgetId = u64;
fn next_id() -> WidgetId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
#[derive(Default, Debug)]
pub struct InputState {
    pub focus: Option<WidgetId>,
    pub capture: Option<WidgetId>,
}
impl InputState {
    pub fn clear(&mut self) {
        self.focus = None;
        self.capture = None;
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    Click(WidgetId),
    Toggle(WidgetId, bool),
    Value(WidgetId, f32),
    Submit(WidgetId, String),
    Changed(WidgetId, String),
    Select(WidgetId, usize),
    Reorder(WidgetId, usize, usize),
    Activate(WidgetId, usize),
    Remove(WidgetId, usize),
}
pub struct EventContext<'a> {
    pub input: &'a mut InputState,
    pub actions: &'a mut Vec<UiAction>,
}
pub struct DrawContext<'a> {
    pub theme: &'a ThemeSpec,
    pub mouse: Option<[f32; 2]>,
    pub focus: Option<WidgetId>,
}
pub trait Widget: Send {
    fn handles_ctrl_scroll(&self) -> bool {
        false
    }
    fn set_interactive(&mut self, _interactive: bool) {}
    fn id(&self) -> WidgetId;
    fn rect(&self) -> Rect;
    fn measure(&self, canvas: &dyn Canvas, max: [f32; 2]) -> [f32; 2];
    fn layout(&mut self, rect: Rect);
    fn arrange(&mut self, _canvas: &dyn Canvas, rect: Rect) {
        self.layout(rect);
    }
    fn draw(&self, canvas: &mut dyn Canvas, ctx: &DrawContext<'_>);
    fn draw_overlay(&self, _canvas: &mut dyn Canvas, _ctx: &DrawContext<'_>) {}
    fn event(&mut self, event: &InputEvent, ctx: &mut EventContext<'_>) -> bool;
    fn focusables(&self, _out: &mut Vec<WidgetId>) {}
    fn focused_rect(&self, id: WidgetId) -> Option<Rect> {
        (self.id() == id).then(|| self.rect())
    }
    fn reveal_focus(&mut self, _id: WidgetId) {}
}
struct Base {
    id: WidgetId,
    rect: Rect,
}
impl Base {
    fn new() -> Self {
        Self {
            id: next_id(),
            rect: Rect::default(),
        }
    }
}
macro_rules! base {
    () => {
        fn id(&self) -> WidgetId {
            self.base.id
        }
        fn rect(&self) -> Rect {
            self.base.rect
        }
        fn layout(&mut self, r: Rect) {
            self.base.rect = r;
        }
    };
}
macro_rules! focusable {
    () => {
        fn focusables(&self, out: &mut Vec<WidgetId>) {
            out.push(self.id());
        }
    };
}
fn hovered(r: Rect, ctx: &DrawContext<'_>) -> bool {
    ctx.mouse.is_some_and(|p| r.contains(p))
}
fn activate(base: &Base, e: &InputEvent, ctx: &mut EventContext<'_>) -> (bool, bool) {
    match e {
        InputEvent::Down { at, button: 0, .. } if base.rect.contains(*at) => {
            ctx.input.focus = Some(base.id);
            ctx.input.capture = Some(base.id);
            (true, false)
        }
        InputEvent::Up { at, button: 0 } if ctx.input.capture == Some(base.id) => {
            ctx.input.capture = None;
            (true, base.rect.contains(*at))
        }
        InputEvent::Drag { button: 0, .. } if ctx.input.capture == Some(base.id) => (true, false),
        InputEvent::Key {
            key: Key::Enter | Key::Space,
            ..
        } if ctx.input.focus == Some(base.id) => (true, true),
        _ => (false, false),
    }
}
fn panel(c: &mut dyn Canvas, b: &Base, ctx: &DrawContext<'_>) {
    c.fill(
        b.rect,
        ctx.theme.color(if hovered(b.rect, ctx) {
            R::RowHover
        } else {
            R::RowBg
        }),
    );
    c.border(
        b.rect,
        ctx.theme.color(if ctx.focus == Some(b.id) {
            R::BorderFocused
        } else {
            R::Border
        }),
    );
}
pub struct Label {
    base: Base,
    pub text: String,
    pub role: R,
}
impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            base: Base::new(),
            text: text.into(),
            role: R::Text,
        }
    }
}
impl Widget for Label {
    base!();
    fn measure(&self, c: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [
            c.text_width(&self.text).min(max[0]),
            c.line_height().min(max[1]),
        ]
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        c.push_clip(self.rect());
        c.text(
            &self.text,
            [self.rect().x, self.rect().y],
            ctx.theme.color(self.role),
            false,
        );
        c.pop_clip();
    }
    fn event(&mut self, _: &InputEvent, _: &mut EventContext<'_>) -> bool {
        false
    }
}
pub struct Button {
    base: Base,
    pub label: String,
    pub enabled: bool,
}
impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            base: Base::new(),
            label: label.into(),
            enabled: true,
        }
    }
}
impl Widget for Button {
    base!();
    fn measure(&self, c: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [
            (c.text_width(&self.label) + 12.).min(max[0]),
            20f32.min(max[1]),
        ]
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        panel(c, &self.base, ctx);
        c.push_clip(self.rect().inset(2.));
        c.text(
            &self.label,
            [
                self.rect().x + 6.,
                self.rect().y + (self.rect().h - c.line_height()) / 2.,
            ],
            ctx.theme
                .color(if self.enabled { R::Text } else { R::TextDim }),
            false,
        );
        c.pop_clip();
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        if !self.enabled {
            return false;
        }
        let (used, clicked) = activate(&self.base, e, ctx);
        if clicked {
            ctx.actions.push(UiAction::Click(self.id()));
        }
        used
    }
    fn focusables(&self, out: &mut Vec<WidgetId>) {
        if self.enabled {
            out.push(self.id());
        }
    }
}
/// An icon button uses a short glyph label and can optionally show an atlas image.
pub struct IconButton {
    pub button: Button,
    pub texture: Option<TextureId>,
}
impl IconButton {
    pub fn new(glyph: impl Into<String>) -> Self {
        Self {
            button: Button::new(glyph),
            texture: None,
        }
    }
}
impl Widget for IconButton {
    fn id(&self) -> WidgetId {
        self.button.id()
    }
    fn rect(&self) -> Rect {
        self.button.rect()
    }
    fn layout(&mut self, r: Rect) {
        self.button.layout(r);
    }
    fn measure(&self, _: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [20f32.min(max[0]), 20f32.min(max[1])]
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        if let Some(id) = self.texture {
            panel(c, &self.button.base, ctx);
            c.texture(id, self.rect().inset(3.), [0., 0., 1., 1.]);
        } else {
            self.button.draw(c, ctx);
        }
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        self.button.event(e, ctx)
    }
    fn focusables(&self, out: &mut Vec<WidgetId>) {
        self.button.focusables(out);
    }
}
pub struct Toggle {
    base: Base,
    pub label: String,
    pub value: bool,
}
impl Toggle {
    pub fn new(label: impl Into<String>, value: bool) -> Self {
        Self {
            base: Base::new(),
            label: label.into(),
            value,
        }
    }
}
impl Widget for Toggle {
    base!();
    focusable!();
    fn measure(&self, c: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [
            (c.text_width(&self.label) + 28.).min(max[0]),
            20f32.min(max[1]),
        ]
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        panel(c, &self.base, ctx);
        c.push_clip(self.rect());
        c.fill(
            Rect::new(self.rect().x + 4., self.rect().y + 5., 10., 10.),
            ctx.theme.color(if self.value { R::Fill } else { R::Track }),
        );
        c.text(
            &self.label,
            [self.rect().x + 20., self.rect().y + 6.],
            ctx.theme.color(R::Text),
            false,
        );
        c.pop_clip();
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        let (used, clicked) = activate(&self.base, e, ctx);
        if clicked {
            self.value = !self.value;
            ctx.actions.push(UiAction::Toggle(self.id(), self.value));
        }
        used
    }
}
pub struct Slider {
    base: Base,
    value: f32,
    pub step: f32,
}
impl Slider {
    pub fn new(value: f32) -> Self {
        Self {
            base: Base::new(),
            value: if value.is_finite() {
                value.clamp(0., 1.)
            } else {
                0.
            },
            step: 0.05,
        }
    }
    pub fn value(&self) -> f32 {
        self.value
    }
    fn change(&mut self, value: f32, ctx: &mut EventContext<'_>) {
        if !value.is_finite() {
            return;
        }
        let v = value.clamp(0., 1.);
        if v != self.value {
            self.value = v;
            ctx.actions.push(UiAction::Value(self.id(), v));
        }
    }
    fn pointer(&mut self, x: f32, ctx: &mut EventContext<'_>) {
        self.change((x - self.rect().x - 4.) / (self.rect().w - 8.).max(1.), ctx);
    }
}
impl Widget for Slider {
    base!();
    focusable!();
    fn measure(&self, _: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [120f32.min(max[0]), 20f32.min(max[1])]
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        let r = self.rect();
        let w = (r.w - 8.).max(0.);
        c.fill(
            Rect::new(r.x + 4., r.y + 9., w, 3.),
            ctx.theme.color(R::Track),
        );
        c.fill(
            Rect::new(r.x + 4., r.y + 9., w * self.value, 3.),
            ctx.theme.color(R::Fill),
        );
        c.fill(
            Rect::new(r.x + 2. + w * self.value, r.y + 5., 4., 11.),
            ctx.theme.color(R::Knob),
        );
        if ctx.focus == Some(self.id()) {
            c.border(r, ctx.theme.color(R::BorderFocused));
        }
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        match e {
            InputEvent::Down { at, button: 0, .. } if self.rect().contains(*at) => {
                ctx.input.focus = Some(self.id());
                ctx.input.capture = Some(self.id());
                self.pointer(at[0], ctx);
                true
            }
            InputEvent::Drag { at, button: 0, .. } | InputEvent::Up { at, button: 0 }
                if ctx.input.capture == Some(self.id()) =>
            {
                self.pointer(at[0], ctx);
                if matches!(e, InputEvent::Up { .. }) {
                    ctx.input.capture = None;
                }
                true
            }
            InputEvent::Key { key, .. } if ctx.input.focus == Some(self.id()) => {
                let v = match key {
                    Key::Left | Key::Down => self.value - self.step,
                    Key::Right | Key::Up => self.value + self.step,
                    Key::Home => 0.,
                    Key::End => 1.,
                    _ => return false,
                };
                self.change(v, ctx);
                true
            }
            _ => false,
        }
    }
}
pub struct SeekBar {
    pub slider: Slider,
    pub duration: f32,
}
impl SeekBar {
    pub fn new(duration: f32) -> Self {
        Self {
            slider: Slider::new(0.),
            duration,
        }
    }
    pub fn position(&self) -> f32 {
        self.slider.value() * self.duration.max(0.)
    }
}
impl Widget for SeekBar {
    fn id(&self) -> WidgetId {
        self.slider.id()
    }
    fn rect(&self) -> Rect {
        self.slider.rect()
    }
    fn layout(&mut self, r: Rect) {
        self.slider.layout(r);
    }
    fn measure(&self, c: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        self.slider.measure(c, max)
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        self.slider.draw(c, ctx);
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        self.duration.is_finite() && self.duration > 0. && self.slider.event(e, ctx)
    }
    fn focusables(&self, out: &mut Vec<WidgetId>) {
        if self.duration.is_finite() && self.duration > 0. {
            out.push(self.id());
        }
    }
}
pub struct TextField {
    base: Base,
    text: String,
    cursor: usize,
    anchor: usize,
    glyphs: Vec<(usize, f32)>,
    pub max_chars: usize,
    pub placeholder: String,
}
impl TextField {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let n = text.len();
        Self {
            base: Base::new(),
            text,
            cursor: n,
            anchor: n,
            glyphs: Vec::new(),
            max_chars: 4096,
            placeholder: String::new(),
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn insert_text(&mut self, text: &str) {
        let selected = self.text[self.selection()].chars().count();
        let available = self
            .max_chars
            .saturating_sub(self.text.chars().count() - selected);
        let text: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(available)
            .collect();
        self.replace(&text);
    }
    fn selection(&self) -> std::ops::Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }
    fn replace(&mut self, s: &str) {
        let r = self.selection();
        self.text.replace_range(r.clone(), s);
        self.glyphs.clear();
        self.cursor = r.start + s.len();
        self.anchor = self.cursor;
    }
    fn hit_cursor(&self, x: f32) -> usize {
        let cursor_width = self
            .glyphs
            .iter()
            .find(|(i, _)| *i == self.cursor)
            .map_or(0., |(_, w)| *w);
        let offset = (cursor_width - (self.rect().w - 10.).max(0.)).max(0.);
        let local = x - self.rect().x - 4. + offset;
        self.glyphs
            .iter()
            .min_by(|a, b| (a.1 - local).abs().total_cmp(&(b.1 - local).abs()))
            .map_or(self.text.len(), |(i, _)| *i)
    }
    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |ch| self.cursor + ch.len_utf8())
    }
}
impl Widget for TextField {
    base!();
    fn arrange(&mut self, c: &dyn Canvas, r: Rect) {
        self.layout(r);
        self.glyphs = self
            .text
            .char_indices()
            .map(|(i, _)| (i, c.text_width(&self.text[..i])))
            .chain(std::iter::once((self.text.len(), c.text_width(&self.text))))
            .collect();
    }
    focusable!();
    fn measure(&self, _: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [160f32.min(max[0]), 20f32.min(max[1])]
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        let r = self.rect();
        c.fill(r, ctx.theme.color(R::FieldBg));
        c.border(
            r,
            ctx.theme.color(if ctx.focus == Some(self.id()) {
                R::BorderFocused
            } else {
                R::Border
            }),
        );
        c.push_clip(r.inset(3.));
        let offset = if ctx.focus == Some(self.id()) {
            (c.text_width(&self.text[..self.cursor]) - (r.w - 10.).max(0.)).max(0.)
        } else {
            0.
        };
        let x = r.x + 4. - offset;
        if ctx.focus == Some(self.id()) {
            let sel = self.selection();
            let left = c.text_width(&self.text[..sel.start]);
            let width = c.text_width(&self.text[sel]);
            c.fill(
                Rect::new(x + left, r.y + 4., width, 12.),
                ctx.theme.color(R::RowSelected),
            );
        }
        c.text(
            if self.text.is_empty() {
                &self.placeholder
            } else {
                &self.text
            },
            [x, r.y + 6.],
            ctx.theme.color(if self.text.is_empty() {
                R::TextDim
            } else {
                R::Text
            }),
            false,
        );
        if ctx.focus == Some(self.id()) {
            let dx = c.text_width(&self.text[..self.cursor]);
            c.fill(
                Rect::new(x + dx, r.y + 4., 1., 12.),
                ctx.theme.color(R::Text),
            );
        }
        c.pop_clip();
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        if let InputEvent::Down { at, button: 0, .. } = e
            && self.rect().contains(*at)
        {
            ctx.input.focus = Some(self.id());
            ctx.input.capture = Some(self.id());
            self.cursor = self.hit_cursor(at[0]);
            self.anchor = self.cursor;
            return true;
        }
        if let InputEvent::Drag { at, button: 0, .. } = e
            && ctx.input.capture == Some(self.id())
        {
            self.cursor = self.hit_cursor(at[0]);
            return true;
        }
        if matches!(e, InputEvent::Up { button: 0, .. }) && ctx.input.capture == Some(self.id()) {
            ctx.input.capture = None;
            return true;
        }
        if ctx.input.focus != Some(self.id()) {
            return false;
        }
        let before = self.text.clone();
        match e {
            InputEvent::Char(ch) if !ch.is_control() => {
                let selected = self.text[self.selection()].chars().count();
                if self.text.chars().count() - selected < self.max_chars {
                    self.replace(&ch.to_string());
                }
            }
            InputEvent::Key { key, mods } => {
                match key {
                    Key::A if mods.ctrl => {
                        self.anchor = 0;
                        self.cursor = self.text.len();
                    }
                    Key::Left | Key::Right | Key::Home | Key::End => {
                        self.cursor = match key {
                            Key::Left => {
                                if !mods.shift && !self.selection().is_empty() {
                                    self.selection().start
                                } else {
                                    self.previous()
                                }
                            }
                            Key::Right => {
                                if !mods.shift && !self.selection().is_empty() {
                                    self.selection().end
                                } else {
                                    self.next()
                                }
                            }
                            Key::Home => 0,
                            _ => self.text.len(),
                        };
                        if !mods.shift {
                            self.anchor = self.cursor;
                        }
                    }
                    Key::Backspace => {
                        if self.selection().is_empty() {
                            self.anchor = self.previous();
                        }
                        self.replace("");
                    }
                    Key::Delete => {
                        if self.selection().is_empty() {
                            self.anchor = self.next();
                        }
                        self.replace("");
                    }
                    Key::Enter => ctx
                        .actions
                        .push(UiAction::Submit(self.id(), self.text.clone())),
                    Key::Escape | Key::Tab => return false,
                    _ => {} // Keep typing keys from reaching the underlying game screen.
                }
            }
            _ => return false,
        }
        if before != self.text {
            ctx.actions
                .push(UiAction::Changed(self.id(), self.text.clone()));
        }
        true
    }
}
#[derive(Clone, Copy)]
pub enum Axis {
    Horizontal,
    Vertical,
    Overlay,
}
pub struct Container {
    base: Base,
    pub children: Vec<Box<dyn Widget>>,
    axis: Axis,
    pub gap: f32,
    sizes: Vec<[f32; 2]>,
}
impl Container {
    fn new(axis: Axis, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            base: Base::new(),
            children,
            axis,
            gap: 4.,
            sizes: Vec::new(),
        }
    }
    pub fn prepare(&mut self, c: &dyn Canvas, max: [f32; 2]) {
        self.sizes = self.children.iter().map(|ch| ch.measure(c, max)).collect();
    }
}
impl Widget for Container {
    fn focused_rect(&self, id: WidgetId) -> Option<Rect> {
        self.children.iter().find_map(|ch| ch.focused_rect(id))
    }
    fn reveal_focus(&mut self, id: WidgetId) {
        for ch in &mut self.children {
            ch.reveal_focus(id);
        }
    }
    fn id(&self) -> WidgetId {
        self.base.id
    }
    fn rect(&self) -> Rect {
        self.base.rect
    }
    fn measure(&self, c: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        let mut size = [0f32; 2];
        for ch in &self.children {
            let s = ch.measure(c, max);
            match self.axis {
                Axis::Horizontal => {
                    size[0] += s[0];
                    size[1] = size[1].max(s[1]);
                }
                Axis::Vertical => {
                    size[1] += s[1];
                    size[0] = size[0].max(s[0]);
                }
                Axis::Overlay => {
                    size[0] = size[0].max(s[0]);
                    size[1] = size[1].max(s[1]);
                }
            }
        }
        let gaps = self.gap * self.children.len().saturating_sub(1) as f32;
        match self.axis {
            Axis::Horizontal => size[0] += gaps,
            Axis::Vertical => size[1] += gaps,
            _ => {}
        }
        [size[0].min(max[0]), size[1].min(max[1])]
    }
    fn layout(&mut self, r: Rect) {
        self.base.rect = r;
        let mut pos = 0.;
        for (i, ch) in self.children.iter_mut().enumerate() {
            let size = self.sizes.get(i).copied().unwrap_or([r.w, 20.]);
            let rect = match self.axis {
                Axis::Horizontal => Rect::new(r.x + pos, r.y, size[0], r.h),
                Axis::Vertical => Rect::new(r.x, r.y + pos, r.w, size[1]),
                Axis::Overlay => r,
            };
            ch.layout(rect);
            pos += match self.axis {
                Axis::Horizontal => size[0],
                _ => size[1],
            } + self.gap;
        }
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        c.push_clip(self.rect());
        for ch in &self.children {
            ch.draw(c, ctx);
        }
        c.pop_clip();
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        if matches!(e, InputEvent::Down { .. } | InputEvent::Scroll { .. })
            && e.position().is_some_and(|p| !self.rect().contains(p))
        {
            return false;
        }
        self.children.iter_mut().rev().any(|ch| ch.event(e, ctx))
    }
    fn focusables(&self, out: &mut Vec<WidgetId>) {
        for ch in &self.children {
            ch.focusables(out);
        }
    }
}
// The wrapper caches natural child sizes during layout_with_canvas, recursively.
macro_rules! container {
    ($name:ident,$axis:ident) => {
        pub struct $name(pub Container);
        impl $name {
            pub fn new(children: Vec<Box<dyn Widget>>) -> Self {
                Self(Container::new(Axis::$axis, children))
            }
        }
        impl Widget for $name {
            fn focused_rect(&self, id: WidgetId) -> Option<Rect> {
                self.0.focused_rect(id)
            }
            fn reveal_focus(&mut self, id: WidgetId) {
                self.0.reveal_focus(id);
            }
            fn id(&self) -> WidgetId {
                self.0.id()
            }
            fn rect(&self) -> Rect {
                self.0.rect()
            }
            fn measure(&self, c: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
                self.0.measure(c, max)
            }
            fn layout(&mut self, r: Rect) {
                self.0.layout(r);
            }
            fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
                self.0.draw(c, ctx);
            }
            fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
                self.0.event(e, ctx)
            }
            fn focusables(&self, out: &mut Vec<WidgetId>) {
                self.0.focusables(out);
            }
            fn arrange(&mut self, c: &dyn Canvas, r: Rect) {
                self.0.prepare(c, [r.w, r.h]);
                self.0.layout(r);
                for ch in &mut self.0.children {
                    ch.arrange(c, ch.rect());
                }
            }
        }
    };
}
container!(Row, Horizontal);
container!(Column, Vertical);
container!(Stack, Overlay);
pub struct ScrollPane {
    base: Base,
    pub child: Box<dyn Widget>,
    pub offset: f32,
    content_height: f32,
}
impl ScrollPane {
    pub fn new(child: Box<dyn Widget>) -> Self {
        Self {
            base: Base::new(),
            child,
            offset: 0.,
            content_height: 0.,
        }
    }
    fn position_child(&mut self) {
        let r = self.rect();
        self.offset = self.offset.clamp(0., (self.content_height - r.h).max(0.));
        self.child.layout(Rect::new(
            r.x,
            r.y - self.offset,
            (r.w - 5.).max(0.),
            self.content_height,
        ));
    }
}
impl Widget for ScrollPane {
    fn focused_rect(&self, id: WidgetId) -> Option<Rect> {
        self.child.focused_rect(id)
    }
    fn reveal_focus(&mut self, id: WidgetId) {
        self.child.reveal_focus(id);
        if let Some(r) = self.child.focused_rect(id) {
            if r.y < self.rect().y {
                self.offset -= self.rect().y - r.y;
            } else if r.bottom() > self.rect().bottom() {
                self.offset += r.bottom() - self.rect().bottom();
            }
            self.position_child();
        }
    }
    fn id(&self) -> WidgetId {
        self.base.id
    }
    fn rect(&self) -> Rect {
        self.base.rect
    }
    fn measure(&self, c: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        let s = self.child.measure(c, [max[0], f32::MAX]);
        [s[0].min(max[0]), s[1].min(max[1]).min(100.)]
    }
    fn layout(&mut self, r: Rect) {
        self.base.rect = r;
        self.position_child();
    }
    fn arrange(&mut self, c: &dyn Canvas, r: Rect) {
        self.content_height = self.child.measure(c, [(r.w - 5.).max(0.), f32::MAX])[1];
        self.layout(r);
        self.child.arrange(c, self.child.rect());
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        c.push_clip(self.rect());
        self.child.draw(c, ctx);
        c.pop_clip();
        let r = self.rect();
        if self.content_height > r.h {
            let h = r.h * r.h / self.content_height;
            let y = r.y + self.offset / self.content_height * r.h;
            c.fill(
                Rect::new(r.right() - 3., y, 3., h),
                ctx.theme.color(R::ScrollThumb),
            );
        }
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        if matches!(e, InputEvent::Down { .. } | InputEvent::Scroll { .. })
            && e.position().is_some_and(|p| !self.rect().contains(p))
        {
            return false;
        }
        if self.child.event(e, ctx) {
            return true;
        }
        if let InputEvent::Scroll { delta, .. } = e {
            self.offset -= delta * 18.;
            self.position_child();
            return true;
        }
        false
    }
    fn focusables(&self, out: &mut Vec<WidgetId>) {
        self.child.focusables(out);
    }
}
pub struct ListView {
    base: Base,
    pub items: Vec<String>,
    pub selected: Option<usize>,
    offset: f32,
    pub row_height: f32,
    pub reorderable: bool,
    pub removable: bool,
    pressed: Option<usize>,
    last_click: Option<(usize, std::time::Instant)>,
    drag: Option<(usize, f32)>,
    drop_row: Option<usize>,
}
impl ListView {
    pub fn new(items: Vec<String>) -> Self {
        Self {
            base: Base::new(),
            items,
            selected: None,
            offset: 0.,
            row_height: 20.,
            reorderable: false,
            removable: false,
            pressed: None,
            last_click: None,
            drag: None,
            drop_row: None,
        }
    }
    pub fn reset_scroll(&mut self) {
        self.offset = 0.;
        self.selected = None;
        self.cancel_drag();
    }
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.drop_row = None;
        self.pressed = None;
    }
    fn row_at(&self, y: f32) -> usize {
        (((y - self.rect().y + self.offset).max(0.) / self.row_height) as usize)
            .min(self.items.len().saturating_sub(1))
    }
    fn clamp(&mut self) {
        self.row_height = self.row_height.max(1.);
        self.offset = self.offset.clamp(
            0.,
            (self.items.len() as f32 * self.row_height - self.rect().h).max(0.),
        );
        if self.selected.is_some_and(|i| i >= self.items.len()) {
            self.selected = None;
        }
    }
}
impl Widget for ListView {
    fn id(&self) -> WidgetId {
        self.base.id
    }
    fn rect(&self) -> Rect {
        self.base.rect
    }
    focusable!();
    fn measure(&self, _: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [
            160f32.min(max[0]),
            (self.items.len() as f32 * self.row_height)
                .min(100.)
                .min(max[1]),
        ]
    }
    fn layout(&mut self, r: Rect) {
        self.base.rect = r;
        self.clamp();
    }
    fn draw(&self, c: &mut dyn Canvas, ctx: &DrawContext<'_>) {
        let r = self.rect();
        c.fill(r, ctx.theme.color(R::ListBg));
        c.push_clip(r);
        let first = (self.offset / self.row_height) as usize;
        let count = (r.h / self.row_height).ceil() as usize + 1;
        for i in first..(first + count).min(self.items.len()) {
            let row = Rect::new(
                r.x,
                r.y + i as f32 * self.row_height - self.offset,
                r.w,
                self.row_height,
            );
            c.fill(
                row,
                ctx.theme.color(if self.selected == Some(i) {
                    R::RowSelected
                } else if hovered(row, ctx) {
                    R::RowHover
                } else {
                    R::RowBg
                }),
            );
            let label = super::text::ellipsize(
                c,
                &self.items[i],
                (row.w - if self.removable { 26. } else { 10. }).max(0.),
            );
            c.text(
                &label,
                [row.x + 4., row.y + (row.h - c.line_height()) / 2.],
                ctx.theme.color(R::Text),
                false,
            );
        }
        if self.removable {
            for i in first..(first + count).min(self.items.len()) {
                let button = Rect::new(
                    r.right() - 18.,
                    r.y + i as f32 * self.row_height - self.offset,
                    14.,
                    self.row_height,
                );
                super::icons::draw(
                    c,
                    button,
                    super::icons::Icon::Close,
                    ctx.theme.color(if hovered(button, ctx) {
                        R::Danger
                    } else {
                        R::TextDim
                    }),
                );
            }
        }
        let total = self.items.len() as f32 * self.row_height;
        if total > r.h && r.h > 0. {
            let thumb = (r.h * r.h / total).max(6.).min(r.h);
            c.fill(
                Rect::new(r.right() - 3., r.y, 3., r.h),
                ctx.theme.color(R::ScrollTrack),
            );
            c.fill(
                Rect::new(
                    r.right() - 3.,
                    r.y + self.offset / (total - r.h) * (r.h - thumb),
                    3.,
                    thumb,
                ),
                ctx.theme.color(R::ScrollThumb),
            );
        }
        if let Some(i) = self.drop_row {
            c.fill(
                Rect::new(r.x, r.y + i as f32 * self.row_height - self.offset, r.w, 2.),
                ctx.theme.color(R::Fill),
            );
        }
        c.pop_clip();
        if ctx.focus == Some(self.id()) {
            c.border(r, ctx.theme.color(R::BorderFocused));
        }
    }
    fn event(&mut self, e: &InputEvent, ctx: &mut EventContext<'_>) -> bool {
        self.clamp();
        if matches!(e, InputEvent::Cancel) {
            self.cancel_drag();
            if ctx.input.capture == Some(self.id()) {
                ctx.input.capture = None;
            }
            return false;
        }
        let index = match e {
            InputEvent::Down { at, button: 0, .. } if self.rect().contains(*at) => {
                ctx.input.focus = Some(self.id());
                ctx.input.capture = Some(self.id());
                let i = ((at[1] - self.rect().y + self.offset) / self.row_height) as usize;
                if self.removable && i < self.items.len() && at[0] >= self.rect().right() - 20. {
                    ctx.actions.push(UiAction::Remove(self.id(), i));
                    self.last_click = None;
                    self.pressed = None;
                    return true;
                }
                self.pressed = (i < self.items.len()).then_some(i);
                self.drag = (self.reorderable && i < self.items.len()).then_some((i, at[1]));
                Some(i)
            }
            InputEvent::Drag { at, button: 0, .. } if ctx.input.capture == Some(self.id()) => {
                if let Some((_, start)) = self.drag
                    && (at[1] - start).abs() >= 4.
                {
                    if at[1] < self.rect().y + 10. {
                        self.offset -= self.row_height;
                    }
                    if at[1] > self.rect().bottom() - 10. {
                        self.offset += self.row_height;
                    }
                    self.clamp();
                    self.drop_row = Some(self.row_at(at[1]));
                }
                return true;
            }
            InputEvent::Up { at, button: 0 } if ctx.input.capture == Some(self.id()) => {
                if let (Some((from, start)), Some(to)) = (self.drag, self.drop_row)
                    && self.rect().contains(*at)
                    && (at[1] - start).abs() >= 4.
                    && from != to
                {
                    ctx.actions.push(UiAction::Reorder(self.id(), from, to));
                    self.selected = Some(to);
                }
                if self.drop_row.is_none()
                    && let Some(i) = self.pressed
                    && self.rect().contains(*at)
                    && self.row_at(at[1]) == i
                {
                    let now = std::time::Instant::now();
                    if self.last_click.is_some_and(|(prev, t)| {
                        prev == i && now.duration_since(t) < std::time::Duration::from_millis(400)
                    }) {
                        ctx.actions.push(UiAction::Activate(self.id(), i));
                        self.last_click = None;
                    } else {
                        self.last_click = Some((i, now));
                    }
                } else {
                    self.last_click = None;
                }
                self.cancel_drag();
                ctx.input.capture = None;
                return true;
            }
            InputEvent::Scroll { at, delta, .. } if self.rect().contains(*at) => {
                self.offset -= delta * self.row_height;
                self.clamp();
                return true;
            }
            InputEvent::Key {
                key: Key::Enter, ..
            } if ctx.input.focus == Some(self.id()) => {
                if let Some(i) = self.selected {
                    ctx.actions.push(UiAction::Activate(self.id(), i));
                }
                return true;
            }
            InputEvent::Key {
                key: Key::Delete, ..
            } if self.removable && ctx.input.focus == Some(self.id()) => {
                if let Some(i) = self.selected {
                    ctx.actions.push(UiAction::Remove(self.id(), i));
                }
                return true;
            }
            InputEvent::Key { key, .. } if ctx.input.focus == Some(self.id()) => Some(match key {
                Key::Up => self.selected.unwrap_or(0).saturating_sub(1),
                Key::Down => self
                    .selected
                    .map_or(0, |i| i + 1)
                    .min(self.items.len().saturating_sub(1)),
                Key::Home => 0,
                Key::End => self.items.len().saturating_sub(1),
                _ => return false,
            }),
            _ => return false,
        };
        if let Some(i) = index.filter(|&i| i < self.items.len()) {
            self.selected = Some(i);
            let top = i as f32 * self.row_height;
            if top < self.offset {
                self.offset = top;
            }
            if top + self.row_height > self.offset + self.rect().h {
                self.offset = top + self.row_height - self.rect().h;
            }
            ctx.actions.push(UiAction::Select(self.id(), i));
        }
        true
    }
}
