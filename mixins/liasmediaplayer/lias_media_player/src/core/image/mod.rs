//! Bounded decoders. All frames are composited RGBA, ready for an atlas upload.
pub mod bmp_decoder;
pub mod ffmpeg_image;
pub mod gif_decoder;
use std::{
    io::{Cursor, Read},
    time::Duration,
};
pub const MAX_DOWNLOAD: usize = 8 * 1024 * 1024;
pub const MAX_PIXELS: usize = 24_000_000;
pub const MAX_FRAMES: usize = 256;
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ImageError(pub String);
pub type Result<T> = std::result::Result<T, ImageError>;
pub(crate) fn err(e: impl std::fmt::Display) -> ImageError {
    ImageError(e.to_string())
}
#[derive(Debug, Clone)]
pub struct ImageFrame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<ImageFrame>,
}
impl DecodedImage {
    pub fn weight(&self) -> usize {
        self.frames.iter().map(|f| f.rgba.len()).sum()
    }
    /// Tier 1 reserves at most 512² pixels per image window, retaining aspect ratio.
    pub fn fit(mut self, side: u32) -> Self {
        let factor = (side as f64 / self.width.max(self.height) as f64).min(1.);
        if factor >= 1. {
            return self;
        }
        let w = (self.width as f64 * factor).round().max(1.) as u32;
        let h = (self.height as f64 * factor).round().max(1.) as u32;
        for frame in &mut self.frames {
            let mut out = vec![0; w as usize * h as usize * 4];
            for y in 0..h {
                for x in 0..w {
                    let src = (((y as u64 * self.height as u64 / h as u64) * self.width as u64
                        + x as u64 * self.width as u64 / w as u64)
                        * 4) as usize;
                    let dst = ((y * w + x) * 4) as usize;
                    out[dst..dst + 4].copy_from_slice(&frame.rgba[src..src + 4]);
                }
            }
            frame.rgba = out;
        }
        self.width = w;
        self.height = h;
        self
    }
}
pub(crate) fn pixels(w: u32, h: u32, frames: usize) -> Result<usize> {
    let count = w as u64 * h as u64;
    if w == 0
        || h == 0
        || w > 8192
        || h > 8192
        || frames == 0
        || frames > MAX_FRAMES
        || count * frames as u64 > MAX_PIXELS as u64
    {
        return Err(err(
            "Image exceeds the 8192-side, 256-frame or 24-megapixel limit",
        ));
    }
    Ok(count as usize)
}
pub(crate) fn delay(ms: u32) -> u32 {
    if ms == 0 { 100 } else { ms.max(20) }
}
fn still(w: u32, h: u32, rgba: Vec<u8>) -> DecodedImage {
    DecodedImage {
        width: w,
        height: h,
        frames: vec![ImageFrame { rgba, delay_ms: 0 }],
    }
}
pub struct ImageLoader;
impl ImageLoader {
    pub fn download(url: &str) -> Result<Vec<u8>> {
        let url =
            super::sources::web_url(url).ok_or_else(|| err("Expected an HTTP(S) image URL"))?;
        let response = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .redirects(5)
            .build()
            .get(url.as_str())
            .set("User-Agent", "LiasMediaPlayer/4.0")
            .call()
            .map_err(err)?;
        if response
            .header("Content-Length")
            .and_then(|s| s.parse::<usize>().ok())
            .is_some_and(|n| n > MAX_DOWNLOAD)
        {
            return Err(err("Image download exceeds 8 MiB"));
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_DOWNLOAD as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        if bytes.len() > MAX_DOWNLOAD {
            return Err(err("Image download exceeds 8 MiB"));
        }
        Ok(bytes)
    }
    pub fn decode_with_tools(
        bytes: &[u8],
        tools: &super::tools::ToolManager,
        cancel: &super::process::Cancellation,
    ) -> Result<DecodedImage> {
        if ffmpeg_image::FfmpegImageDecoder::supports(bytes) {
            ffmpeg_image::FfmpegImageDecoder::decode(bytes, tools, cancel)
        } else {
            Self::decode(bytes)
        }
    }
    pub fn decode(bytes: &[u8]) -> Result<DecodedImage> {
        if bytes.len() > MAX_DOWNLOAD {
            return Err(err("Image download exceeds 8 MiB"));
        }
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            decode_png(bytes)
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            decode_jpeg(bytes)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            gif_decoder::decode(bytes)
        } else if bytes.starts_with(b"BM") {
            bmp_decoder::decode(bytes)
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            decode_webp(bytes)
        } else {
            Err(err("Unsupported image format (AVIF/APNG need FFmpeg)"))
        }
    }
}
fn decode_png(bytes: &[u8]) -> Result<DecodedImage> {
    let mut d = png::Decoder::new(Cursor::new(bytes));
    d.set_limits(png::Limits {
        bytes: MAX_PIXELS * 4,
    });
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(err)?;
    let info = r.info();
    pixels(info.width, info.height, 1)?;
    if info.animation_control.is_some() {
        return Err(err("APNG requires the FFmpeg decoder"));
    }
    let mut buf = vec![
        0;
        r.output_buffer_size()
            .ok_or_else(|| err("PNG size overflow"))?
    ];
    let f = r.next_frame(&mut buf).map_err(err)?;
    let b = &buf[..f.buffer_size()];
    let mut rgba = Vec::with_capacity(f.width as usize * f.height as usize * 4);
    match f.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(b),
        png::ColorType::Rgb => {
            for p in b.as_chunks::<3>().0.iter() {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &v in b {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in b.as_chunks::<2>().0.iter() {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        _ => return Err(err("Unsupported PNG color type")),
    }
    Ok(still(f.width, f.height, rgba))
}
fn decode_jpeg(bytes: &[u8]) -> Result<DecodedImage> {
    let mut d = zune_jpeg::JpegDecoder::new(bytes);
    d.decode_headers().map_err(err)?;
    let info = d.info().ok_or_else(|| err("Missing JPEG dimensions"))?;
    let count = pixels(info.width as u32, info.height as u32, 1)?;
    let b = d.decode().map_err(err)?;
    let mut rgba = Vec::with_capacity(count * 4);
    // zune's default output is RGB, including grayscale/CMYK inputs.
    if b.len() == count {
        for v in b {
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
        return Ok(still(info.width as u32, info.height as u32, rgba));
    }
    if b.len() != count * 3 {
        return Err(err("Unexpected JPEG output colorspace"));
    }
    for p in b.as_chunks::<3>().0.iter() {
        rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    Ok(still(info.width as u32, info.height as u32, rgba))
}
fn decode_webp(bytes: &[u8]) -> Result<DecodedImage> {
    let mut d = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(err)?;
    d.set_memory_limit(MAX_PIXELS * 4);
    let (w, h) = d.dimensions();
    let n = if d.is_animated() {
        d.num_frames() as usize
    } else {
        1
    };
    let count = pixels(w, h, n)?;
    if d.is_animated() {
        d.set_background_color([0; 4]).map_err(err)?;
    }
    let mut frames = Vec::new();
    for _ in 0..n {
        let mut b = vec![
            0;
            d.output_buffer_size()
                .ok_or_else(|| err("WebP size overflow"))?
        ];
        let ms = if d.is_animated() {
            delay(d.read_frame(&mut b).map_err(err)?)
        } else {
            d.read_image(&mut b).map_err(err)?;
            0
        };
        let rgba = if d.has_alpha() {
            b
        } else {
            let mut out = Vec::with_capacity(count * 4);
            for p in b.as_chunks::<3>().0.iter() {
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            out
        };
        frames.push(ImageFrame { rgba, delay_ms: ms });
    }
    Ok(DecodedImage {
        width: w,
        height: h,
        frames,
    })
}
