//! FFmpeg fallback for APNG and still AVIF. PAM is parsed incrementally under M2 limits.
use super::{DecodedImage, ImageFrame, MAX_DOWNLOAD, MAX_FRAMES, err, pixels};
use crate::core::{
    process::{Cancellation, ProcessRunner},
    tools::{TempFile, Tool, ToolManager},
};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    process::Command,
    time::Duration,
};

pub struct FfmpegImageDecoder;
impl FfmpegImageDecoder {
    pub fn supports(bytes: &[u8]) -> bool {
        is_avif(bytes) || apng_delays(bytes).ok().flatten().is_some()
    }
    pub fn decode(
        bytes: &[u8],
        tools: &ToolManager,
        cancel: &Cancellation,
    ) -> super::Result<DecodedImage> {
        if bytes.len() > MAX_DOWNLOAD {
            return Err(err("Image download exceeds 8 MiB"));
        }
        let delays = apng_delays(bytes).map_err(err)?;
        if delays.is_none() && !is_avif(bytes) {
            return Err(err("Unsupported image format"));
        }
        let ffmpeg = tools.require(Tool::Ffmpeg, cancel).map_err(err)?;
        let mut temp = TempFile::new(&std::env::temp_dir(), "image").map_err(err)?;
        temp.file.write_all(bytes).map_err(err)?;
        temp.file.flush().map_err(err)?;
        let mut command = Command::new(ffmpeg);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-max_alloc",
            "96000000",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-protocol_whitelist",
            "file,pipe",
        ]);
        if delays.is_some() {
            command.args(["-ignore_loop", "1"]);
        }
        command.arg("-i").arg(&temp.path).args([
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-threads",
            "1",
            "-fps_mode",
            "passthrough",
            "-frames:v",
            "257",
            "-pix_fmt",
            "rgba",
            "-f",
            "image2pipe",
            "-c:v",
            "pam",
            "pipe:1",
        ]);
        ProcessRunner::read(
            &mut command,
            Duration::from_secs(20),
            cancel,
            move |stdout| Self::read_pam(BufReader::new(stdout), delays.as_deref()),
        )
        .map_err(err)
    }
    /// Public for fixture testing and reuse by the later video pipeline.
    pub fn read_pam(mut input: impl BufRead, delays: Option<&[u32]>) -> io::Result<DecodedImage> {
        let mut image = DecodedImage {
            width: 0,
            height: 0,
            frames: Vec::new(),
        };
        while let Some((w, h)) = pam_header(&mut input)? {
            let count = pixels(w, h, image.frames.len() + 1).map_err(io::Error::other)?;
            if !image.frames.is_empty() && (image.width != w || image.height != h) {
                return Err(io::Error::other("Image frame dimensions changed"));
            }
            let delay_ms = if let Some(delays) = delays {
                *delays
                    .get(image.frames.len())
                    .ok_or_else(|| io::Error::other("Unexpected APNG frame"))?
            } else {
                if !image.frames.is_empty() {
                    return Err(io::Error::other("Animated AVIF is not supported"));
                }
                0
            };
            let mut rgba = vec![0; count * 4];
            input.read_exact(&mut rgba)?;
            image.width = w;
            image.height = h;
            image.frames.push(ImageFrame { rgba, delay_ms });
        }
        if image.frames.is_empty() || delays.is_some_and(|d| d.len() != image.frames.len()) {
            return Err(io::Error::other(
                "FFmpeg returned missing or incomplete frames",
            ));
        }
        Ok(image)
    }
}
pub(crate) fn pam_header(input: &mut impl BufRead) -> io::Result<Option<(u32, u32)>> {
    if input.fill_buf()?.is_empty() {
        return Ok(None);
    }
    let mut bytes = 0;
    let mut line = String::new();
    let mut fields = std::collections::HashMap::new();
    loop {
        line.clear();
        let n = input.take(4097 - bytes).read_line(&mut line)? as u64;
        bytes += n;
        if n == 0 || bytes > 4096 {
            return Err(io::Error::other("Truncated or oversized PAM header"));
        }
        let line = line.trim();
        if fields.is_empty() && line == "P7" {
            fields.insert("magic".to_owned(), "P7".to_owned());
            continue;
        }
        if !fields.contains_key("magic") {
            return Err(io::Error::other("Expected PAM P7 header"));
        }
        if line == "ENDHDR" {
            break;
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| io::Error::other("Invalid PAM header"))?;
        if !["WIDTH", "HEIGHT", "DEPTH", "MAXVAL", "TUPLTYPE"].contains(&key)
            || fields.insert(key.into(), value.trim().into()).is_some()
        {
            return Err(io::Error::other("Invalid or duplicate PAM field"));
        }
    }
    if fields.get("DEPTH").map(String::as_str) != Some("4")
        || fields.get("MAXVAL").map(String::as_str) != Some("255")
        || fields.get("TUPLTYPE").map(String::as_str) != Some("RGB_ALPHA")
    {
        return Err(io::Error::other("Expected 8-bit RGBA PAM"));
    }
    let number = |name| {
        fields
            .get(name)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::other("Invalid PAM dimensions"))
    };
    Ok(Some((number("WIDTH")?, number("HEIGHT")?)))
}
fn is_avif(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || bytes.get(4..8) != Some(b"ftyp") {
        return false;
    }
    let len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
    if len < 16 || len > bytes.len() {
        return false;
    }
    bytes.get(8..12) == Some(b"avif")
        || bytes[16..len]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|b| b == b"avif")
}
/// Extract the original rational delays instead of resampling the animation to a guessed FPS.
fn apng_delays(bytes: &[u8]) -> io::Result<Option<Vec<u32>>> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(None);
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: super::MAX_PIXELS * 4,
    });
    let reader = decoder.read_info().map_err(io::Error::other)?;
    let Some(control) = reader.info().animation_control else {
        return Ok(None);
    };
    pixels(
        reader.info().width,
        reader.info().height,
        control.num_frames as usize,
    )
    .map_err(io::Error::other)?;
    let mut pos = 8;
    let mut delays = Vec::new();
    while pos + 12 <= bytes.len() {
        let n = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let end = pos
            .checked_add(12)
            .and_then(|p| p.checked_add(n))
            .filter(|&p| p <= bytes.len())
            .ok_or_else(|| io::Error::other("Truncated PNG chunk"))?;
        if &bytes[pos + 4..pos + 8] == b"fcTL" {
            if n != 26 || delays.len() >= MAX_FRAMES {
                return Err(io::Error::other("Invalid APNG frame control"));
            }
            let data = &bytes[pos + 8..end - 4];
            let num = u16::from_be_bytes(data[20..22].try_into().unwrap()) as u32;
            let den = u16::from_be_bytes(data[22..24].try_into().unwrap()) as u32;
            let den = if den == 0 { 100 } else { den };
            delays.push(super::delay((num * 1000 + den / 2) / den));
        }
        pos = end;
    }
    if delays.len() != control.num_frames as usize {
        return Err(io::Error::other("APNG frame count mismatch"));
    }
    Ok(Some(delays))
}
