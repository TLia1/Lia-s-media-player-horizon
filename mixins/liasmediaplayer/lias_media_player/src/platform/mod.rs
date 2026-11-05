//! Host-independent drawing and input seam. Coordinates are GUI pixels; colors are ARGB.
pub mod horizon_impl;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
    pub fn contains(self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[1] >= self.y && p[0] < self.right() && p[1] < self.bottom()
    }
    pub fn intersect(self, b: Self) -> Self {
        let x = self.x.max(b.x);
        let y = self.y.max(b.y);
        Self::new(
            x,
            y,
            (self.right().min(b.right()) - x).max(0.),
            (self.bottom().min(b.bottom()) - y).max(0.),
        )
    }
    pub fn inset(self, n: f32) -> Self {
        Self::new(
            self.x + n,
            self.y + n,
            (self.w - 2. * n).max(0.),
            (self.h - 2. * n).max(0.),
        )
    }
    pub fn array(self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub(crate) u64);

pub trait Canvas {
    fn size(&self) -> [f32; 2];
    fn fill(&mut self, rect: Rect, argb: u32);
    /// UVs are normalized within this texture's allocation, not within the atlas.
    fn texture(&mut self, id: TextureId, rect: Rect, uv: [f32; 4]);
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool);
    fn text_width(&self, text: &str) -> f32;
    fn line_height(&self) -> f32 {
        9.
    }
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);
    /// Horizon draws in submission order; callers finish one window before the next.
    fn next_layer(&mut self) {}
    fn border(&mut self, r: Rect, color: u32) {
        self.fill(Rect::new(r.x, r.y, r.w, 1.), color);
        self.fill(Rect::new(r.x, r.bottom() - 1., r.w, 1.), color);
        self.fill(Rect::new(r.x, r.y, 1., r.h), color);
        self.fill(Rect::new(r.right() - 1., r.y, 1., r.h), color);
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}
impl Modifiers {
    pub fn from_bits(bits: u32) -> Self {
        Self {
            shift: bits & 1 != 0,
            ctrl: bits & 2 != 0,
            alt: bits & 4 != 0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Space,
    Tab,
    Escape,
    A,
    V,
    Other,
}
impl Key {
    pub fn from_name(name: &str) -> Self {
        match name.strip_prefix("key.keyboard.").unwrap_or(name) {
            "left" => Self::Left,
            "right" => Self::Right,
            "up" => Self::Up,
            "down" => Self::Down,
            "home" => Self::Home,
            "end" => Self::End,
            "backspace" => Self::Backspace,
            "delete" => Self::Delete,
            "enter" => Self::Enter,
            "space" => Self::Space,
            "tab" => Self::Tab,
            "escape" => Self::Escape,
            "a" => Self::A,
            "v" => Self::V,
            _ => Self::Other,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Down {
        at: [f32; 2],
        button: i32,
        mods: Modifiers,
    },
    Up {
        at: [f32; 2],
        button: i32,
    },
    Drag {
        at: [f32; 2],
        button: i32,
        mods: Modifiers,
    },
    Scroll {
        at: [f32; 2],
        delta: f32,
        mods: Modifiers,
    },
    Key {
        key: Key,
        mods: Modifiers,
    },
    Char(char),
    /// Screen changes and focus loss must end drags and clear keyboard focus.
    Cancel,
}
impl InputEvent {
    pub fn position(&self) -> Option<[f32; 2]> {
        match self {
            Self::Down { at, .. }
            | Self::Up { at, .. }
            | Self::Drag { at, .. }
            | Self::Scroll { at, .. } => Some(*at),
            _ => None,
        }
    }
}
