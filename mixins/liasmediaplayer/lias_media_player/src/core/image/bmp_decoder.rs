use super::*;
pub fn decode(b: &[u8]) -> Result<DecodedImage> {
    let u16_at = |i| {
        b.get(i..i + 2)
            .map(|s| u16::from_le_bytes(s.try_into().unwrap()))
            .ok_or_else(|| err("Truncated BMP"))
    };
    let u32_at = |i| {
        b.get(i..i + 4)
            .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
            .ok_or_else(|| err("Truncated BMP"))
    };
    let offset = u32_at(10)? as usize;
    let dib = u32_at(14)? as usize;
    if dib < 40
        || 14usize
            .checked_add(dib)
            .is_none_or(|end| end > b.len() || offset < end)
    {
        return Err(err("Unsupported BMP header"));
    }
    let wi = u32_at(18)? as i32;
    let hi = u32_at(22)? as i32;
    if wi <= 0 || hi == i32::MIN {
        return Err(err("Invalid BMP dimensions"));
    }
    let (w, h) = (wi as u32, hi.unsigned_abs());
    let count = pixels(w, h, 1)?;
    let bits = u16_at(28)?;
    if u16_at(26)? != 1 || ![24, 32].contains(&bits) || u32_at(30)? != 0 {
        return Err(err("Only uncompressed 24/32-bit BMP is supported"));
    }
    let channels = bits as usize / 8;
    let stride = (w as usize * channels).div_ceil(4) * 4;
    if offset
        .checked_add(stride * h as usize)
        .is_none_or(|end| end > b.len())
    {
        return Err(err("Truncated BMP pixels"));
    }
    let mut rgba = vec![0; count * 4];
    // BI_RGB's fourth byte is reserved, not alpha (often zero).
    for y in 0..h as usize {
        let sy = if hi < 0 { y } else { h as usize - 1 - y };
        for x in 0..w as usize {
            let s = offset + sy * stride + x * channels;
            let d = (y * w as usize + x) * 4;
            rgba[d..d + 4].copy_from_slice(&[b[s + 2], b[s + 1], b[s], 255]);
        }
    }
    Ok(still(w, h, rgba))
}
