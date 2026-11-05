//! Chat chips, hover card and a small read-only playlist picker (editing belongs to M6).
use crate::{
    core::{
        chat::ChatAction,
        previews::PreviewCache,
        sources::{MediaKind, MediaRef, SourceRegistry},
    },
    platform::{Canvas, InputEvent, Key, Rect, TextureId, horizon_impl::ModAtlasPacker},
};
use std::time::{Duration, Instant};

pub struct PickerRow {
    pub label: String,
    pub media: Option<MediaRef>,
}
#[derive(Default)]
pub struct ChatOverlay {
    hovered: Option<(MediaRef, Instant)>,
    texture: Option<TextureId>,
    texture_url: Option<String>,
    preview_error: Option<String>,
    pub picker: Option<Vec<PickerRow>>,
    pub picker_title: String,
    offset: usize,
    viewport: [f32; 2],
    pub notice: Option<(String, Instant)>,
}
impl ChatOverlay {
    pub fn clear_hover(&mut self, cache: Option<&mut PreviewCache>, atlas: &mut ModAtlasPacker) {
        self.hovered = None;
        if let Some(c) = cache {
            c.cancel();
        }
        if let Some(t) = self.texture.take() {
            atlas.free(t);
        }
        self.texture_url = None;
        self.preview_error = None;
    }
    pub fn hover(
        &mut self,
        media: Option<MediaRef>,
        cache: Option<&mut PreviewCache>,
        atlas: &mut ModAtlasPacker,
    ) {
        if self.hovered.as_ref().map(|(m, _)| &m.url) == media.as_ref().map(|m| &m.url) {
            return;
        }
        self.clear_hover(cache, atlas);
        self.hovered = media.map(|m| (m, Instant::now()));
    }
    pub fn tick(&mut self, cache: Option<&mut PreviewCache>, atlas: &mut ModAtlasPacker) {
        let Some((media, since)) = &self.hovered else {
            return;
        };
        if since.elapsed() < Duration::from_millis(250) {
            return;
        }
        let Some(cache) = cache else {
            return;
        };
        cache.request(media);
        if self.texture_url.as_deref() == Some(&media.url) {
            return;
        }
        if let Some(preview) = cache.get(&media.url) {
            self.texture_url = Some(media.url.clone());
            if let Some(image) = &preview.image {
                match atlas.allocate(image.width, image.height, &image.frames[0].rgba) {
                    Ok(t) => self.texture = Some(t),
                    Err(_) => self.preview_error = Some("Thumbnail unavailable: atlas full".into()),
                }
            }
        }
    }
    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }
    pub fn show_picker(&mut self, title: impl Into<String>, rows: Vec<PickerRow>) {
        self.picker_title = title.into();
        self.picker = Some(rows);
        self.offset = 0;
    }
    pub fn chips(size: [f32; 2], hidden: usize) -> Vec<(Rect, String, ChatAction)> {
        let mut right = size[0] - 4.;
        let mut chips = Vec::new();
        if hidden > 0 {
            let label = format!("[Hidden ({hidden})]");
            let width = label.len() as f32 * 6. + 8.;
            right -= width;
            chips.push((
                Rect::new(right.max(0.), (size[1] - 32.).max(0.), width, 14.),
                label,
                ChatAction::RevealHidden,
            ));
            right -= 4.;
        }
        chips.push((
            Rect::new((right - 78.).max(0.), (size[1] - 32.).max(0.), 78., 14.),
            "[Playlists]".into(),
            ChatAction::Playlists,
        ));
        chips
    }
    pub fn chip_click(size: [f32; 2], hidden: usize, at: [f32; 2]) -> Option<ChatAction> {
        Self::chips(size, hidden)
            .into_iter()
            .find(|(r, _, _)| r.contains(at))
            .map(|(_, _, a)| a)
    }
    pub fn card_rect(size: [f32; 2], mouse: [f32; 2], height: f32) -> Rect {
        let w = 280f32.min(size[0].max(0.));
        let h = height.min((size[1] - 36.).max(0.));
        Rect::new(
            (mouse[0] + 12.).min(size[0] - w).max(0.),
            (mouse[1] - h - 8.).max(0.).min((size[1] - h - 36.).max(0.)),
            w,
            h,
        )
    }
    pub fn draw_chat(
        &mut self,
        c: &mut dyn Canvas,
        mouse: [f32; 2],
        hidden: usize,
        cache: Option<&PreviewCache>,
    ) {
        self.viewport = c.size();
        for (rect, label, _) in Self::chips(c.size(), hidden) {
            c.fill(
                rect,
                if rect.contains(mouse) {
                    0xee33465a
                } else {
                    0xdd182330
                },
            );
            c.text(&label, [rect.x + 4., rect.y + 3.], 0xffaaddff, true);
        }
        if let Some((media, since)) = &self.hovered
            && since.elapsed() >= Duration::from_millis(250)
        {
            let preview = cache.and_then(|p| p.get(&media.url));
            let image = preview.as_ref().and_then(|p| p.image.as_ref());
            let image_h = if self.texture.is_some() { 110. } else { 0. };
            let r = Self::card_rect(c.size(), mouse, 76. + image_h);
            c.fill(r, 0xf018202c);
            c.border(r, 0xff5b8bb7);
            c.push_clip(r.inset(4.));
            c.text(
                &format!(
                    "[{}] {}",
                    media.label,
                    preview
                        .as_ref()
                        .map_or("Loading preview…", |p| p.title.as_str())
                ),
                [r.x + 6., r.y + 6.],
                0xffeeeeee,
                true,
            );
            let mut y = r.y + 20.;
            if let (Some(t), Some(image)) = (self.texture, image) {
                let scale = (192. / image.width as f32).min(104. / image.height as f32);
                let w = image.width as f32 * scale;
                c.texture(
                    t,
                    Rect::new(r.x + (r.w - w) / 2., y, w, image.height as f32 * scale),
                    [0., 0., 1., 1.],
                );
                y += image_h;
            }
            c.text(&media.url, [r.x + 6., y], 0xffaaaaaa, false);
            y += 12.;
            let detail = self
                .preview_error
                .as_deref()
                .or_else(|| preview.as_ref().and_then(|p| p.error.as_deref()))
                .unwrap_or_else(|| preview.as_ref().map_or("", |p| p.detail.as_str()));
            c.text(detail, [r.x + 6., y], 0xffffcc88, false);
            y += 14.;
            c.text(
                "Click: Open  |  Shift+Click: Queue",
                [r.x + 6., y],
                0xffdddddd,
                false,
            );
            c.text(
                "Ctrl+Click: Browser",
                [r.x + 6., y + 11.],
                0xffdddddd,
                false,
            );
            c.pop_clip();
        }
        self.draw_notice(c);
    }
    fn panel(&self) -> Rect {
        Rect::new(
            10.,
            10.,
            (self.viewport[0] - 20.).max(0.),
            (self.viewport[1] - 20.).max(0.),
        )
    }
    pub fn draw_picker(&mut self, c: &mut dyn Canvas) {
        self.viewport = c.size();
        let Some(rows) = &self.picker else {
            return;
        };
        let r = self.panel();
        c.fill(r, 0xf018202c);
        c.border(r, 0xff5b8bb7);
        c.push_clip(r.inset(5.));
        c.text(&self.picker_title, [r.x + 8., r.y + 8.], 0xffffffff, true);
        c.text(
            "Click: open  |  Shift+Click: queue  |  Esc: close",
            [r.x + 8., r.y + 23.],
            0xffbbbbbb,
            false,
        );
        if rows.is_empty() {
            c.text(
                "No saved playlists yet. Playlist editing arrives in M6.",
                [r.x + 8., r.y + 44.],
                0xffbbbbbb,
                false,
            );
        }
        for (i, row) in rows
            .iter()
            .skip(self.offset)
            .take(self.visible_rows())
            .enumerate()
        {
            let y = r.y + 40. + i as f32 * 20.;
            c.fill(Rect::new(r.x + 5., y, r.w - 10., 18.), 0xff263447);
            c.text(
                &row.label,
                [r.x + 9., y + 4.],
                if row.media.is_some() {
                    0xffaaddff
                } else {
                    0xffeeeeee
                },
                false,
            );
        }
        c.pop_clip();
        self.draw_notice(c);
    }
    fn visible_rows(&self) -> usize {
        ((self.panel().h - 44.).max(20.) / 20.) as usize
    }
    pub fn picker_input(
        &mut self,
        event: &InputEvent,
        _sources: &SourceRegistry,
    ) -> (bool, Option<ChatAction>) {
        let Some(rows) = &self.picker else {
            return (false, None);
        };
        match event {
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.picker = None;
                (false, None)
            }
            InputEvent::Scroll { delta, .. } => {
                let max = rows.len().saturating_sub(self.visible_rows());
                self.offset = (self.offset as f32 - delta * 3.).clamp(0., max as f32) as usize;
                (true, None)
            }
            InputEvent::Down {
                at,
                button: 0,
                mods,
            } => {
                let r = self.panel();
                if at[1] >= r.y + 40.
                    && at[1] < r.bottom() - 4.
                    && at[0] >= r.x
                    && at[0] < r.right()
                {
                    let index = self.offset + ((at[1] - r.y - 40.) / 20.) as usize;
                    let action = rows.get(index).and_then(|row| row.media.as_ref()).map(|m| {
                        if mods.ctrl {
                            ChatAction::Browser(m.url.clone())
                        } else if mods.shift {
                            ChatAction::Enqueue(m.clone())
                        } else {
                            ChatAction::Open(m.clone())
                        }
                    });
                    (true, action)
                } else {
                    (true, None)
                }
            }
            _ => (true, None),
        }
    }
    pub fn draw_notice(&self, c: &mut dyn Canvas) {
        if let Some((text, since)) = &self.notice
            && since.elapsed() < Duration::from_secs(5)
        {
            let width = (c.size()[0] - 12.).max(0.);
            let lines = super::text::wrap(c, text, (width - 8.).max(1.))
                .len()
                .min(4);
            let r = Rect::new(
                6.,
                6.,
                width,
                (lines as f32 * 12. + 8.).min((c.size()[1] - 12.).max(0.)),
            );
            c.fill(r, 0xee182330);
            c.push_clip(r);
            super::text::paragraph(c, r.inset(4.), text, 0xffffdd99);
            c.pop_clip();
        }
    }
}
/// Accept the Java and Rust library entry shapes while retaining source classification.
pub fn playlist_rows(library: &serde_json::Value, sources: &SourceRegistry) -> Vec<PickerRow> {
    let mut rows = Vec::new();
    if let Some(playlists) = library["playlists"].as_array() {
        for list in playlists.iter().take(500) {
            rows.push(PickerRow {
                label: format!(
                    "— {} —",
                    list["name"]
                        .as_str()
                        .or(list["title"].as_str())
                        .unwrap_or("Playlist")
                ),
                media: None,
            });
            if let Some(entries) = list["entries"]
                .as_array()
                .or_else(|| list["items"].as_array())
                .or_else(|| list["urls"].as_array())
            {
                for e in entries.iter().take(500) {
                    let Some(url) = e.as_str().or_else(|| e["url"].as_str()) else {
                        continue;
                    };
                    let Some(media) = sources.identify(url).or_else(|| {
                        crate::core::sources::web_url(url)?;
                        let kind = match e["kind"].as_str()? {
                            "audio" => MediaKind::Audio,
                            "video" => MediaKind::Video,
                            "image" | "gif" => MediaKind::Image,
                            _ => return None,
                        };
                        Some(MediaRef {
                            url: url.into(),
                            kind,
                            entry_kind: kind,
                            source_id: "liasmediaplayer:saved".into(),
                            label: "saved".into(),
                        })
                    }) else {
                        continue;
                    };
                    rows.push(PickerRow {
                        label: e["title"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(url)
                            .chars()
                            .take(256)
                            .collect(),
                        media: Some(media),
                    });
                    if rows.len() >= 2000 {
                        return rows;
                    }
                }
            }
        }
    }
    rows
}
