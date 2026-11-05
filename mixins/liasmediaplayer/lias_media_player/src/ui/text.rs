//! Measured text layout shared by notices, diagnostics and compact controls.
use crate::platform::{Canvas, Rect};

pub fn ellipsize(c: &dyn Canvas, text: &str, width: f32) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if c.text_width(&text) <= width {
        return text;
    }
    let mut out = String::new();
    if c.text_width("…") > width {
        return out;
    }
    for ch in text.chars() {
        if c.text_width(&format!("{out}{ch}…")) > width {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// Break even unspaced URLs and preserve paragraph boundaries. Work is bounded
/// because external tool diagnostics can be large.
pub fn wrap(c: &dyn Canvas, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for ch in text.chars().take(16 * 1024) {
        if ch == '\n' {
            lines.push(std::mem::take(&mut line));
        } else if !ch.is_control() {
            if !line.is_empty() && c.text_width(&format!("{line}{ch}")) > width.max(1.) {
                if let Some(space) = line.rfind(' ').filter(|space| *space > 0) {
                    let rest = line[space + 1..].to_owned();
                    lines.push(line[..space].to_owned());
                    line = rest;
                } else {
                    lines.push(std::mem::take(&mut line));
                }
            }
            if ch != ' ' || !line.is_empty() {
                line.push(ch);
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub fn paragraph(c: &mut dyn Canvas, area: Rect, text: &str, color: u32) {
    c.push_clip(area);
    let step = c.line_height() + 3.;
    let count = (area.h / step).floor().max(0.) as usize;
    let lines = wrap(c, text, area.w);
    for (i, line) in lines.iter().take(count).enumerate() {
        let line = if i + 1 == count && lines.len() > count {
            ellipsize(c, &format!("{line}…"), area.w)
        } else {
            line.clone()
        };
        c.text(&line, [area.x, area.y + i as f32 * step], color, false);
    }
    c.pop_clip();
}

/// Hover help is drawn outside the window clip, but always inside the viewport.
pub fn tooltip(c: &mut dyn Canvas, anchor: Rect, text: &str, background: u32, color: u32) {
    let [vw, vh] = c.size();
    let width = (c.text_width(text) + 8.).min(320.).min((vw - 8.).max(0.));
    let height =
        (wrap(c, text, width - 8.).len().clamp(1, 4) as f32 * (c.line_height() + 3.) + 6.).min(vh);
    let y = if anchor.y >= height + 2. {
        anchor.y - height - 2.
    } else {
        anchor.bottom() + 2.
    };
    let rect = Rect::new(
        anchor.x.clamp(0., (vw - width).max(0.)),
        y.clamp(0., (vh - height).max(0.)),
        width,
        height,
    );
    c.fill(rect, background | 0xff000000);
    paragraph(c, rect.inset(3.), text, color);
}
