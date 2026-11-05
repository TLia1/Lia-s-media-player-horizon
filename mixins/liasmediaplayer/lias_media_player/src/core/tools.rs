//! On-demand tool provisioning. Call on a worker, never on Horizon's render thread.
//! Downloads trust TLS to named publishers; magic/version checks are not signatures.
use super::{
    process::{Cancellation, ProcessRunner},
    sources::web_url,
};
use parking_lot::Mutex;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static INSTALL: Mutex<()> = Mutex::new(());
static SERIAL: AtomicU64 = AtomicU64::new(0);
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_BINARY: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
}
impl Tool {
    pub fn name(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Self::YtDlp, false) => "yt-dlp",
            (Self::YtDlp, true) => "yt-dlp.exe",
            (Self::Ffmpeg, false) => "ffmpeg",
            (Self::Ffmpeg, true) => "ffmpeg.exe",
        }
    }
    fn version_arg(self) -> &'static str {
        if self == Self::YtDlp {
            "--version"
        } else {
            "-version"
        }
    }
}
#[derive(Debug, Clone)]
pub struct ToolPolicy {
    pub prefer_system: bool,
    pub auto_download: bool,
}
impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            prefer_system: true,
            auto_download: true,
        }
    }
}
#[derive(Clone)]
pub struct ToolManager {
    pub root: Option<PathBuf>,
    pub legacy_bin: Option<PathBuf>,
    pub policy: ToolPolicy,
}
impl ToolManager {
    pub fn new(root: Option<&Path>) -> Self {
        Self {
            root: root.map(Path::to_path_buf),
            legacy_bin: None,
            policy: ToolPolicy::default(),
        }
    }
    pub fn candidates(&self, tool: Tool) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(root) = &self.root {
            paths.push(root.join("bin").join(tool.name()));
        }
        if let Some(bin) = &self.legacy_bin {
            paths.push(bin.join(tool.name()));
        }
        if self.policy.prefer_system
            && let Some(path) = std::env::var_os("PATH")
        {
            paths.extend(
                std::env::split_paths(&path)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join(tool.name())),
            );
        }
        paths
    }
    pub fn locate(&self, tool: Tool, cancel: &Cancellation) -> Option<PathBuf> {
        self.candidates(tool)
            .into_iter()
            .take_while(|_| !cancel.is_cancelled())
            .find(|path| path.is_file() && check_version(path, tool, cancel).is_ok())
    }
    pub fn require(&self, tool: Tool, cancel: &Cancellation) -> io::Result<PathBuf> {
        self.provision(tool, cancel, false)
    }
    /// Explicit download keeps the previous working executable until atomic replacement succeeds.
    pub fn redownload(&self, tool: Tool, cancel: &Cancellation) -> io::Result<PathBuf> {
        self.provision(tool, cancel, true)
    }
    fn provision(&self, tool: Tool, cancel: &Cancellation, force: bool) -> io::Result<PathBuf> {
        let _lock = loop {
            if cancel.is_cancelled() {
                return Err(io::Error::other("Tool lookup cancelled"));
            }
            if let Some(lock) = INSTALL.try_lock_for(Duration::from_millis(100)) {
                break lock;
            }
        };
        if !force && let Some(path) = self.locate(tool, cancel) {
            return Ok(path);
        }
        if cancel.is_cancelled() {
            return Err(io::Error::other("Tool lookup cancelled"));
        }
        if !force && !self.policy.auto_download {
            return Err(io::Error::other(format!(
                "{} is missing; install it on PATH or in the mod's bin folder",
                tool.name()
            )));
        }
        let root = self.root.as_ref().ok_or_else(|| {
            io::Error::other("No data directory for tool download; install the tool on PATH")
        })?;
        let bin = root.join("bin");
        fs::create_dir_all(&bin)?;
        let (url, archive) = download_source(tool)?;
        let mut download = TempFile::new(&bin, "download")?;
        download_tls(&url, &mut download.file, cancel)?;
        download.file.sync_all()?;
        let bytes = match archive {
            Archive::Binary => read_bounded(File::open(&download.path)?, MAX_BINARY)?,
            Archive::Zip => extract_zip(File::open(&download.path)?, tool.name())?,
            Archive::TarXz => extract_tar(&download.path, tool.name(), cancel)?,
        };
        install_binary(&bin, tool, &bytes, cancel)
    }
}
fn check_version(path: &Path, tool: Tool, cancel: &Cancellation) -> io::Result<()> {
    let out = ProcessRunner::run(
        Command::new(path).arg(tool.version_arg()),
        Duration::from_secs(5),
        64 * 1024,
        cancel,
    )?;
    let text = String::from_utf8_lossy(&out);
    let valid = match tool {
        Tool::Ffmpeg => text.starts_with("ffmpeg version "),
        Tool::YtDlp => text.trim().starts_with(|c: char| c.is_ascii_digit()),
    };
    if valid {
        Ok(())
    } else {
        Err(io::Error::other("Unrecognised tool version output"))
    }
}
#[derive(Clone, Copy)]
enum Archive {
    Binary,
    Zip,
    TarXz,
}
fn download_source(tool: Tool) -> io::Result<(String, Archive)> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    if tool == Tool::YtDlp {
        let name = match (os, arch) {
            ("linux", "x86_64") => "yt-dlp_linux",
            ("linux", "aarch64") => "yt-dlp_linux_aarch64",
            ("windows", "x86_64") => "yt-dlp.exe",
            ("windows", "aarch64") => "yt-dlp_arm64.exe",
            ("macos", "x86_64" | "aarch64") => "yt-dlp_macos",
            _ => return Err(io::Error::other("Install yt-dlp on PATH for this platform")),
        };
        return Ok((
            format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{name}"),
            Archive::Binary,
        ));
    }
    let (platform, extension, kind) = match (os, arch) {
        ("linux", "x86_64") => ("linux64", "tar.xz", Archive::TarXz),
        ("linux", "aarch64") => ("linuxarm64", "tar.xz", Archive::TarXz),
        ("windows", "x86_64") => ("win64", "zip", Archive::Zip),
        ("macos", "x86_64") => {
            return Ok((
                "https://evermeet.cx/ffmpeg/getrelease/zip".into(),
                Archive::Zip,
            ));
        }
        _ => {
            return Err(io::Error::other(
                "Install a native FFmpeg on PATH for this platform",
            ));
        }
    };
    Ok((
        format!(
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n9.0-latest-{platform}-gpl-9.0.{extension}"
        ),
        kind,
    ))
}
pub fn allowed_download_url(raw: &str) -> bool {
    web_url(raw).is_some_and(|u| {
        u.scheme() == "https"
            && u.port_or_known_default() == Some(443)
            && matches!(
                u.host_str(),
                Some(
                    "github.com"
                        | "objects.githubusercontent.com"
                        | "release-assets.githubusercontent.com"
                        | "evermeet.cx"
                )
            )
    })
}
fn download_tls(raw: &str, out: &mut File, cancel: &Cancellation) -> io::Result<()> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(30))
        .build();
    let mut url = raw.to_owned();
    let start = Instant::now();
    for _ in 0..6 {
        if !allowed_download_url(&url) {
            return Err(io::Error::other(
                "Refused tool download host or non-TLS redirect",
            ));
        }
        if cancel.is_cancelled() {
            return Err(io::Error::other("Tool download cancelled"));
        }
        let response = agent.get(&url).call().map_err(io::Error::other)?;
        if (300..400).contains(&response.status()) {
            let location = response
                .header("Location")
                .ok_or_else(|| io::Error::other("Redirect without Location"))?;
            url = url::Url::parse(&url)
                .and_then(|u| u.join(location))
                .map_err(io::Error::other)?
                .to_string();
            continue;
        }
        if response.status() != 200 {
            return Err(io::Error::other("Tool download did not return HTTP 200"));
        }
        let mut reader = response.into_reader();
        let mut buf = [0; 64 * 1024];
        let mut total = 0;
        loop {
            if cancel.is_cancelled() || start.elapsed() > Duration::from_secs(300) {
                return Err(io::Error::other("Tool download cancelled or timed out"));
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_ARCHIVE {
                return Err(io::Error::other("Tool download exceeds 512 MiB"));
            }
            out.write_all(&buf[..n])?;
        }
        return Ok(());
    }
    Err(io::Error::other("Too many tool download redirects"))
}
pub fn executable_magic(bytes: &[u8], os: &str) -> bool {
    match os {
        "linux" => bytes.starts_with(b"\x7fELF"),
        "windows" => bytes.starts_with(b"MZ"),
        "macos" => bytes.get(..4).is_some_and(|b| {
            [
                b"\xfe\xed\xfa\xce",
                b"\xfe\xed\xfa\xcf",
                b"\xce\xfa\xed\xfe",
                b"\xcf\xfa\xed\xfe",
                b"\xca\xfe\xba\xbe",
                b"\xbe\xba\xfe\xca",
                b"\xca\xfe\xba\xbf",
            ]
            .contains(&b.try_into().unwrap())
        }),
        _ => false,
    }
}
fn read_bounded(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        Err(io::Error::other("Extracted tool exceeds size limit"))
    } else {
        Ok(bytes)
    }
}
fn safe_member(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains(['\\', ':'])
        && !name.split('/').any(|c| c == "..")
        && !name.chars().any(char::is_control)
}
/// Extract only the requested regular file into memory; archive paths never become output paths.
pub fn extract_zip(reader: impl Read + io::Seek, name: &str) -> io::Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(reader).map_err(io::Error::other)?;
    if zip.len() > 4096 {
        return Err(io::Error::other("Too many archive entries"));
    }
    let mut found = None;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(io::Error::other)?;
        let entry_name = entry.name().map_err(io::Error::other)?.into_owned();
        if !safe_member(&entry_name) || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(io::Error::other("Unsafe archive member"));
        }
        if entry_name.rsplit('/').next() == Some(name) && !entry.is_dir() {
            if found.is_some() {
                return Err(io::Error::other("Duplicate executable in archive"));
            }
            found = Some(read_bounded(entry, MAX_BINARY)?);
        }
    }
    found.ok_or_else(|| io::Error::other("Executable missing from zip"))
}
fn extract_tar(path: &Path, name: &str, cancel: &Cancellation) -> io::Result<Vec<u8>> {
    let listing = ProcessRunner::run(
        Command::new("tar").arg("-tJf").arg(path),
        Duration::from_secs(60),
        1024 * 1024,
        cancel,
    )?;
    let listing = String::from_utf8(listing).map_err(io::Error::other)?;
    let mut selected = None;
    for member in listing.lines() {
        if !safe_member(member) {
            return Err(io::Error::other("Unsafe tar member"));
        }
        if member.rsplit('/').next() == Some(name) {
            if selected.is_some() {
                return Err(io::Error::other("Duplicate executable in archive"));
            }
            selected = Some(member);
        }
    }
    let member = selected.ok_or_else(|| io::Error::other("Executable missing from tar"))?;
    // -O writes bytes to a pipe, so neither symlinks nor archive paths can write to disk.
    ProcessRunner::run(
        Command::new("tar")
            .arg("-xOJf")
            .arg(path)
            .arg("--")
            .arg(member),
        Duration::from_secs(60),
        MAX_BINARY,
        cancel,
    )
}
fn install_binary(
    bin: &Path,
    tool: Tool,
    bytes: &[u8],
    cancel: &Cancellation,
) -> io::Result<PathBuf> {
    if !executable_magic(bytes, std::env::consts::OS) {
        return Err(io::Error::other(
            "Downloaded file is not a native executable",
        ));
    }
    let mut temp = TempFile::new(bin, tool.name())?;
    temp.file.write_all(bytes)?;
    temp.file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp.path, fs::Permissions::from_mode(0o755))?;
    }
    // Close the writable handle before execution (Linux ETXTBSY).
    let path = temp.path.clone();
    drop(temp.file);
    let cleanup = RemoveFile(path.clone());
    check_version(&path, tool, cancel)?;
    let target = bin.join(tool.name());
    fs::rename(&path, &target)?;
    drop(cleanup);
    Ok(target)
}
struct RemoveFile(PathBuf);
impl Drop for RemoveFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
/// Unique files created with O_EXCL and removed on all exit paths.
pub(crate) struct TempFile {
    pub path: PathBuf,
    pub file: File,
    _cleanup: RemoveFile,
}
impl TempFile {
    pub fn new(root: &Path, label: &str) -> io::Result<Self> {
        let path = root.join(format!(
            ".lmp-{label}-{}-{}.part",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            _cleanup: RemoveFile(path.clone()),
            path,
            file,
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn install_validates_before_replacing_and_cleans_staging_files() {
        let root = std::env::temp_dir().join(format!("lmp-install-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let tool = ToolManager::new(None).locate(Tool::Ffmpeg, &Cancellation::default());
        if let Some(tool) = tool {
            let bytes = fs::read(tool).unwrap();
            let installed =
                install_binary(&root, Tool::Ffmpeg, &bytes, &Cancellation::default()).unwrap();
            assert!(check_version(&installed, Tool::Ffmpeg, &Cancellation::default()).is_ok());
            assert!(
                install_binary(
                    &root,
                    Tool::Ffmpeg,
                    b"\x7fELFbroken",
                    &Cancellation::default()
                )
                .is_err()
            );
            assert_eq!(fs::read(installed).unwrap(), bytes);
            assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
            let mut tar = Command::new("tar");
            tar.arg("-cJf")
                .arg(root.join("fixture.tar.xz"))
                .arg("-C")
                .arg(&root)
                .arg("ffmpeg");
            ProcessRunner::run(
                &mut tar,
                Duration::from_secs(10),
                1024,
                &Cancellation::default(),
            )
            .unwrap();
            assert_eq!(
                extract_tar(
                    &root.join("fixture.tar.xz"),
                    "ffmpeg",
                    &Cancellation::default()
                )
                .unwrap(),
                bytes
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

/// At most one settings tool job is admitted by Runtime. Dropping it cancels subprocesses.
pub struct ToolTask {
    rx: std::sync::mpsc::Receiver<String>,
    cancel: Cancellation,
}
impl ToolTask {
    pub fn start(tools: ToolManager, download: Option<Tool>) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let cancel = Cancellation::default();
        let worker = cancel.clone();
        std::thread::spawn(move || {
            let result = if let Some(tool) = download {
                match tools.redownload(tool, &worker) {
                    Ok(path) => format!("Installed {}: {}", tool.name(), path.display()),
                    Err(e) => format!("{}: {e}", tool.name()),
                }
            } else {
                [Tool::YtDlp, Tool::Ffmpeg]
                    .into_iter()
                    .map(|tool| {
                        tools.locate(tool, &worker).map_or_else(
                            || format!("{}: missing or not executable", tool.name()),
                            |path| format!("{}: {}", tool.name(), path.display()),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let _ = tx.send(if worker.is_cancelled() {
                "Tool operation cancelled".into()
            } else {
                result
            });
        });
        Self { rx, cancel }
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn poll(&self) -> Option<String> {
        self.rx.try_recv().ok()
    }
}
impl Drop for ToolTask {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
