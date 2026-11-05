//! Tier 1 atlas and canvas. The tiny Game/HudBatch adapter lives in the injected bridge.
use super::{Canvas, Rect, TextureId};
use std::collections::BTreeMap;
pub const ATLAS_SIZE: u32 = 1024;
pub const ATLAS_SPRITE: &str = "horizon/modimage";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasError {
    InvalidSize,
    InvalidPixels,
    Full,
    UnknownTexture,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}
struct Shelf {
    y: u32,
    height: u32,
    free: Vec<(u32, u32)>,
}
/// Stable handles and shelf allocation. Freed spans coalesce; live textures never move.
/// A full atlas returns Full rather than evicting a texture still used by a window.
pub struct ModAtlasPacker {
    pixels: Vec<u8>,
    shelves: Vec<Shelf>,
    entries: BTreeMap<u64, AtlasRect>,
    next: u64,
    dirty: bool,
}
impl Default for ModAtlasPacker {
    fn default() -> Self {
        Self::new()
    }
}
impl ModAtlasPacker {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            shelves: Vec::new(),
            entries: BTreeMap::new(),
            next: 1,
            dirty: true,
        }
    }
    fn validate(w: u32, h: u32, pixels: &[u8]) -> Result<(), AtlasError> {
        if w == 0 || h == 0 || w > ATLAS_SIZE || h > ATLAS_SIZE {
            return Err(AtlasError::InvalidSize);
        }
        if pixels.len() != (w * h * 4) as usize {
            return Err(AtlasError::InvalidPixels);
        }
        Ok(())
    }
    pub fn allocate(&mut self, w: u32, h: u32, pixels: &[u8]) -> Result<TextureId, AtlasError> {
        Self::validate(w, h, pixels)?;
        let slot = self
            .shelves
            .iter()
            .enumerate()
            .filter(|(_, s)| s.height >= h)
            .filter_map(|(i, s)| s.free.iter().position(|&(_, len)| len >= w).map(|j| (i, j)))
            .min_by_key(|&(i, _)| self.shelves[i].height);
        let (i, j) = if let Some(slot) = slot {
            slot
        } else {
            let y = self.shelves.last().map_or(0, |s| s.y + s.height);
            if y + h > ATLAS_SIZE {
                return Err(AtlasError::Full);
            }
            self.shelves.push(Shelf {
                y,
                height: h,
                free: vec![(0, ATLAS_SIZE)],
            });
            (self.shelves.len() - 1, 0)
        };
        let shelf = &mut self.shelves[i];
        let (x, len) = shelf.free[j];
        if len == w {
            shelf.free.remove(j);
        } else {
            shelf.free[j] = (x + w, len - w);
        }
        let rect = AtlasRect {
            x,
            y: shelf.y,
            w,
            h,
        };
        let id = TextureId(self.next);
        self.next += 1;
        self.entries.insert(id.0, rect);
        self.write(rect, pixels);
        Ok(id)
    }
    fn write(&mut self, r: AtlasRect, pixels: &[u8]) {
        for row in 0..r.h as usize {
            let dst = ((r.y as usize + row) * ATLAS_SIZE as usize + r.x as usize) * 4;
            let src = row * r.w as usize * 4;
            self.pixels[dst..dst + r.w as usize * 4]
                .copy_from_slice(&pixels[src..src + r.w as usize * 4]);
        }
        self.dirty = true;
    }
    pub fn update(&mut self, id: TextureId, pixels: &[u8]) -> Result<(), AtlasError> {
        let r = self.region(id).ok_or(AtlasError::UnknownTexture)?;
        Self::validate(r.w, r.h, pixels)?;
        self.write(r, pixels);
        Ok(())
    }
    pub fn free(&mut self, id: TextureId) -> bool {
        let Some(r) = self.entries.remove(&id.0) else {
            return false;
        };
        for row in r.y..r.y + r.h {
            let start = ((row * ATLAS_SIZE + r.x) * 4) as usize;
            self.pixels[start..start + (r.w * 4) as usize].fill(0);
        }
        let s = self.shelves.iter_mut().find(|s| s.y == r.y).unwrap();
        s.free.push((r.x, r.w));
        s.free.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for &(x, len) in &s.free {
            if let Some(last) = merged.last_mut()
                && last.0 + last.1 == x
            {
                last.1 += len;
                continue;
            }
            merged.push((x, len));
        }
        s.free = merged;
        while self
            .shelves
            .last()
            .is_some_and(|s| s.free == [(0, ATLAS_SIZE)])
        {
            self.shelves.pop();
        }
        self.dirty = true;
        true
    }
    pub fn region(&self, id: TextureId) -> Option<AtlasRect> {
        self.entries.get(&id.0).copied()
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    /// Failed uploads remain dirty for retry. Called from the mutable frame hook.
    pub fn flush(&mut self, upload: impl FnOnce(u32, u32, &[u8]) -> bool) -> bool {
        if !self.dirty {
            return true;
        }
        if upload(ATLAS_SIZE, ATLAS_SIZE, &self.pixels) {
            self.dirty = false;
            true
        } else {
            false
        }
    }
    /// Resource-pack reloads can replace Horizon's atlas, requiring another upload.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
}
/// Operations implemented against the pinned Horizon types in bridge.rs.
pub trait HudBackend {
    fn size(&self) -> [f32; 2];
    fn fill(&mut self, rect: Rect, argb: u32);
    fn sprite(&mut self, source: [f32; 4], rect: Rect);
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool);
    fn text_width(&self, text: &str) -> f32;
    fn vertex_count(&self) -> usize;
    fn clip_since(&mut self, start: usize, rect: Rect);
}
pub struct HorizonCanvas<'a, B: HudBackend> {
    backend: B,
    atlas: &'a ModAtlasPacker,
    clips: Vec<(usize, Rect)>,
}
impl<'a, B: HudBackend> HorizonCanvas<'a, B> {
    pub fn new(backend: B, atlas: &'a ModAtlasPacker) -> Self {
        Self {
            backend,
            atlas,
            clips: Vec::new(),
        }
    }
}
impl<B: HudBackend> Canvas for HorizonCanvas<'_, B> {
    fn size(&self) -> [f32; 2] {
        self.backend.size()
    }
    fn fill(&mut self, r: Rect, c: u32) {
        if r.w > 0. && r.h > 0. {
            self.backend.fill(r, c);
        }
    }
    fn texture(&mut self, id: TextureId, r: Rect, uv: [f32; 4]) {
        if let Some(a) = self.atlas.region(id) {
            if !uv.iter().all(|v| v.is_finite()) {
                return;
            }
            let [u0, v0, u1, v1] = uv.map(|v| v.clamp(0., 1.));
            if u1 <= u0 || v1 <= v0 {
                return;
            }
            self.backend.sprite(
                [
                    a.x as f32 + u0 * a.w as f32,
                    a.y as f32 + v0 * a.h as f32,
                    (u1 - u0) * a.w as f32,
                    (v1 - v0) * a.h as f32,
                ],
                r,
            );
        }
    }
    fn text(&mut self, t: &str, p: [f32; 2], c: u32, s: bool) {
        self.backend.text(t, p, c, s);
    }
    fn text_width(&self, t: &str) -> f32 {
        self.backend.text_width(t)
    }
    fn push_clip(&mut self, r: Rect) {
        let r = self
            .clips
            .last()
            .map_or(r, |&(_, parent)| parent.intersect(r));
        self.clips.push((self.backend.vertex_count(), r));
    }
    fn pop_clip(&mut self) {
        if let Some((start, r)) = self.clips.pop() {
            self.backend.clip_since(start, r);
        }
    }
}
impl<B: HudBackend> Drop for HorizonCanvas<'_, B> {
    fn drop(&mut self) {
        while !self.clips.is_empty() {
            self.pop_clip();
        }
    }
}
