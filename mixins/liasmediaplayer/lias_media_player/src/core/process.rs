//! Shell-free subprocesses with bounded diagnostics, deadlines and RAII cleanup.
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    io::{self, Read},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub struct ManagedProcess {
    child: Child,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    drain: Option<JoinHandle<()>>,
}
impl ManagedProcess {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        let mut pipe = child.stderr.take().expect("piped stderr");
        let stderr = Arc::new(Mutex::new(VecDeque::with_capacity(16 * 1024)));
        let ring = stderr.clone();
        let drain = thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut ring = ring.lock();
                for &b in &buf[..n] {
                    if ring.len() == 16 * 1024 {
                        ring.pop_front();
                    }
                    ring.push_back(b);
                }
            }
        });
        Ok(Self {
            child,
            stderr,
            drain: Some(drain),
        })
    }
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().iter().copied().collect::<Vec<_>>())
            .into_owned()
    }
    pub fn terminate(&mut self) {
        if self.drain.is_none() {
            return;
        }
        // Clean up subprocesses that inherited the pipes as well as the direct child.
        #[cfg(unix)]
        {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &self.child.id().to_string()])
                .creation_flags(0x08000000)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
    }
}
impl Drop for ManagedProcess {
    fn drop(&mut self) {
        if self.drain.is_some() {
            self.terminate();
        }
    }
}

pub struct ProcessRunner;
impl ProcessRunner {
    pub fn run(
        command: &mut Command,
        timeout: Duration,
        limit: usize,
        cancel: &Cancellation,
    ) -> io::Result<Vec<u8>> {
        Self::read(command, timeout, cancel, move |stdout| {
            let mut bytes = Vec::new();
            stdout.take(limit as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > limit {
                return Err(io::Error::other("Process output exceeds limit"));
            }
            Ok(bytes)
        })
    }
    /// The decoder reads incrementally; the supervising thread enforces the deadline even
    /// when a child stalls in the middle of a frame. No pipe reader is left detached.
    pub fn read<T: Send + 'static>(
        command: &mut Command,
        timeout: Duration,
        cancel: &Cancellation,
        read: impl FnOnce(ChildStdout) -> io::Result<T> + Send + 'static,
    ) -> io::Result<T> {
        if cancel.is_cancelled() {
            return Err(io::Error::other("Process cancelled"));
        }
        let tool = std::path::Path::new(command.get_program())
            .file_name()
            .unwrap_or(command.get_program())
            .to_string_lossy()
            .into_owned();
        let mut process = ManagedProcess::spawn(command)
            .map_err(|e| io::Error::new(e.kind(), format!("{tool}: {e}")))?;
        let stdout = process.take_stdout().expect("piped stdout");
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let _ = tx.send(read(stdout));
        });
        let start = Instant::now();
        let mut output = None;
        let result = loop {
            if cancel.is_cancelled() {
                break Err(io::Error::other("Process cancelled"));
            }
            if start.elapsed() >= timeout {
                break Err(io::Error::new(io::ErrorKind::TimedOut, "Process timed out"));
            }
            if output.is_none() {
                match rx.try_recv() {
                    Ok(Ok(value)) => output = Some(value),
                    Ok(Err(e)) => break Err(e),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        break Err(io::Error::other("Process reader failed"));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            match process.child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    break Err(io::Error::other(format!("Process exited with {status}")));
                }
                Ok(Some(_)) if output.is_some() => break Ok(output.take().unwrap()),
                Err(e) => break Err(e),
                _ => {}
            }
            thread::sleep(Duration::from_millis(5));
        };
        process.terminate();
        let _ = reader.join();
        result.map_err(|e| {
            let detail = process.stderr();
            log::warn!("{tool}: {e}\n{detail}");
            let clean = regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]")
                .unwrap()
                .replace_all(&detail, "");
            let detail = clean.trim();
            io::Error::new(
                e.kind(),
                if detail.is_empty() {
                    format!("{tool}: {e}")
                } else {
                    format!("{tool}: {e}\n{detail}")
                },
            )
        })
    }
}
