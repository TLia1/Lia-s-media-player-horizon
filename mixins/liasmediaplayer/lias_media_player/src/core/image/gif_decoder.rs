//! GIF87a/89a: bounded LZW, local palettes, interlacing and disposal 0–3.
use super::*;
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = self
            .b
            .get(self.at..self.at + n)
            .ok_or_else(|| err("Truncated GIF"))?;
        self.at += n;
        Ok(b)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn word(&mut self) -> Result<u32> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as u32)
    }
    fn blocks(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let n = self.byte()? as usize;
            if n == 0 {
                return Ok(out);
            }
            out.extend_from_slice(self.take(n)?);
        }
    }
    fn palette(&mut self, flags: u8) -> Result<Vec<[u8; 4]>> {
        if flags & 128 == 0 {
            return Ok(Vec::new());
        }
        Ok(self
            .take(3 * (2usize << (flags & 7)))?
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| [p[0], p[1], p[2], 255])
            .collect())
    }
}
pub fn decode(b: &[u8]) -> Result<DecodedImage> {
    let mut r = Reader { b, at: 0 };
    let sig = r.take(6)?;
    if sig != b"GIF87a" && sig != b"GIF89a" {
        return Err(err("Invalid GIF signature"));
    }
    let (w, h) = (r.word()?, r.word()?);
    let count = pixels(w, h, 1)?;
    let flags = r.byte()?;
    let background = r.byte()? as usize;
    r.byte()?;
    let global = r.palette(flags)?;
    let mut canvas = vec![0; count * 4];
    let mut frames = Vec::new();
    let mut disposal = 0;
    let mut transparent = None;
    let mut ms = 100;
    loop {
        match r.byte()? {
            0x3b => break,
            0x21 => {
                let label = r.byte()?;
                if label == 0xf9 {
                    if r.byte()? != 4 {
                        return Err(err("Invalid GIF control block"));
                    }
                    let f = r.byte()?;
                    disposal = (f >> 2) & 7;
                    ms = delay(r.word()? * 10);
                    let index = r.byte()?;
                    transparent = (f & 1 != 0).then_some(index);
                    if r.byte()? != 0 {
                        return Err(err("Invalid GIF control terminator"));
                    }
                } else {
                    r.blocks()?;
                }
            }
            0x2c => {
                pixels(w, h, frames.len() + 1)?;
                let (x, y, fw, fh) = (r.word()?, r.word()?, r.word()?, r.word()?);
                let patch_size = pixels(fw, fh, 1)?;
                let flags = r.byte()?;
                let local = r.palette(flags)?;
                let palette = if local.is_empty() { &global } else { &local };
                if palette.is_empty() {
                    return Err(err("GIF has no color table"));
                }
                let min = r.byte()?;
                let compressed = r.blocks()?;
                let indices = lzw(&compressed, min, patch_size)?;
                if frames.is_empty() && transparent.is_none() {
                    let bg = global.get(background).copied().unwrap_or([0; 4]);
                    for p in canvas.as_chunks_mut::<4>().0 {
                        p.copy_from_slice(&bg);
                    }
                }
                let restore = (disposal == 3).then(|| canvas.clone());
                let rows: Vec<u32> = if flags & 64 != 0 {
                    [(0, 8), (4, 8), (2, 4), (1, 2)]
                        .into_iter()
                        .flat_map(|(start, step)| (start..fh).step_by(step))
                        .collect()
                } else {
                    (0..fh).collect()
                };
                for (source_y, dest_y) in rows.into_iter().enumerate() {
                    for px in 0..fw {
                        let index = indices[source_y * fw as usize + px as usize];
                        if Some(index) == transparent {
                            continue;
                        }
                        let color = palette
                            .get(index as usize)
                            .ok_or_else(|| err("Invalid GIF palette index"))?;
                        if x + px < w && y + dest_y < h {
                            let at = (((y + dest_y) * w + x + px) * 4) as usize;
                            canvas[at..at + 4].copy_from_slice(color);
                        }
                    }
                }
                frames.push(ImageFrame {
                    rgba: canvas.clone(),
                    delay_ms: ms,
                });
                match disposal {
                    2 => {
                        let bg = if transparent.is_some() {
                            [0; 4]
                        } else {
                            global.get(background).copied().unwrap_or([0; 4])
                        };
                        for py in y..(y + fh).min(h) {
                            for px in x..(x + fw).min(w) {
                                let at = ((py * w + px) * 4) as usize;
                                canvas[at..at + 4].copy_from_slice(&bg);
                            }
                        }
                    }
                    3 => canvas = restore.unwrap(),
                    _ => {}
                }
                disposal = 0;
                transparent = None;
                ms = 100;
            }
            _ => return Err(err("Invalid GIF block")),
        }
    }
    if frames.is_empty() {
        return Err(err("GIF has no frames"));
    }
    if frames.len() == 1 {
        frames[0].delay_ms = 0;
    }
    Ok(DecodedImage {
        width: w,
        height: h,
        frames,
    })
}
fn lzw(b: &[u8], min: u8, expected: usize) -> Result<Vec<u8>> {
    if !(2..=8).contains(&min) {
        return Err(err("Invalid GIF LZW code size"));
    }
    let clear = 1usize << min;
    let end = clear + 1;
    let initial = || {
        let mut d: Vec<Vec<u8>> = (0..clear).map(|i| vec![i as u8]).collect();
        d.extend([vec![], vec![]]);
        d
    };
    let mut dict = initial();
    let mut width = min as usize + 1;
    let mut bit = 0;
    let mut previous: Option<Vec<u8>> = None;
    let mut out = Vec::with_capacity(expected);
    loop {
        if bit + width > b.len() * 8 {
            return Err(err("Truncated GIF LZW stream"));
        }
        let mut code = 0;
        for i in 0..width {
            code |= (((b[(bit + i) / 8] >> ((bit + i) % 8)) & 1) as usize) << i;
        }
        bit += width;
        if code == clear {
            dict = initial();
            width = min as usize + 1;
            previous = None;
            continue;
        }
        if code == end {
            break;
        }
        let entry = if code < dict.len() {
            dict[code].clone()
        } else if code == dict.len() {
            let mut v = previous
                .clone()
                .ok_or_else(|| err("Invalid GIF LZW reference"))?;
            v.push(v[0]);
            v
        } else {
            return Err(err("Invalid GIF LZW reference"));
        };
        if entry.is_empty() || out.len() + entry.len() > expected {
            return Err(err("GIF pixel count mismatch"));
        }
        out.extend_from_slice(&entry);
        if let Some(mut p) = previous
            && dict.len() < 4096
        {
            p.push(entry[0]);
            dict.push(p);
            if dict.len() == (1 << width) && width < 12 {
                width += 1;
            }
        }
        previous = Some(entry);
    }
    if out.len() != expected {
        return Err(err("GIF pixel count mismatch"));
    }
    Ok(out)
}
