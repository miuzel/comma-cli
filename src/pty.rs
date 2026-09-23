//! Captured command execution for the auto-refine path.
//!
//! `auto_refine` needs the child's output for its summary. A plain `.output()`
//! capture would buffer everything until the command exits and hand the child
//! pipes instead of a terminal, so instead the command runs on a pseudo-terminal
//! that we relay byte-for-byte:
//!
//! - the child sees a real TTY (colors, progress bars, `vim`/`less` work);
//! - our terminal is switched to raw mode and restored unconditionally — the
//!   RAII guard, the fatal-signal handler and the panic hook all share one
//!   saved `termios`, so Ctrl-C and panics cannot leave it behind;
//! - output is written through to our stdout as it arrives (live) while a
//!   bounded copy is kept for the summary;
//! - the window size is propagated before the child starts and re-synced on
//!   `SIGWINCH`, otherwise full-screen programs render garbled.
//!
//! Platforms without a pty implementation (Windows) and the rare Unix host
//! where no pty can be allocated fall back to [`run_piped`]: output still
//! streams live, but the child sees pipes rather than a TTY — a documented
//! degradation (README/AGENTS.md), never a silent one.

use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

// ── Bounded output capture ──────────────────────────────────────────────────

/// Bytes kept from the start of the relayed output.
const CAPTURE_HEAD_BYTES: usize = 32 * 1024;
/// Bytes kept from the end of the relayed output. The middle is dropped, so an
/// endless command (`yes`, a chatty daemon) cannot exhaust memory; the summary
/// is truncated to 2000 characters later anyway.
const CAPTURE_TAIL_BYTES: usize = 32 * 1024;

/// Bounded collector for relayed output: the first [`CAPTURE_HEAD_BYTES`] and
/// the last [`CAPTURE_TAIL_BYTES`] bytes, with an explicit marker in between
/// when the middle was dropped.
#[derive(Default)]
pub(crate) struct OutputCapture {
    head: Vec<u8>,
    tail: Vec<u8>,
    total: usize,
}

impl OutputCapture {
    /// Append relayed bytes, keeping only the head/tail budget.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len();
        let head_room = CAPTURE_HEAD_BYTES.saturating_sub(self.head.len());
        let (into_head, into_tail) = if head_room == 0 {
            (&[][..], bytes)
        } else if bytes.len() <= head_room {
            (bytes, &[][..])
        } else {
            (&bytes[..head_room], &bytes[head_room..])
        };
        self.head.extend_from_slice(into_head);
        self.tail.extend_from_slice(into_tail);
        let excess = self.tail.len().saturating_sub(CAPTURE_TAIL_BYTES);
        if excess > 0 {
            self.tail.drain(..excess);
        }
    }

    /// Finish the capture. Non-UTF-8 bytes are decoded lossily (the summary is
    /// sanitized later anyway) and a dropped middle is marked.
    pub(crate) fn finish(&self) -> String {
        let dropped = self.total.saturating_sub(self.head.len() + self.tail.len());
        let mut out = String::from_utf8_lossy(&self.head).into_owned();
        if dropped > 0 {
            out.push_str(&format!("\n… [{dropped} bytes of output omitted] …\n"));
            out.push_str(&String::from_utf8_lossy(&self.tail));
        }
        out
    }
}

/// Result of a captured run.
pub(crate) struct CapturedRun {
    /// Exit code of the child; `None` when it was killed by a signal.
    pub code: Option<i32>,
    /// Relayed output (bounded, lossy-decoded, untruncated).
    pub output: String,
}

// ── Unix: pty relay ─────────────────────────────────────────────────────────

#[cfg(unix)]
mod unix_pty {
    use super::*;
    use std::io::IsTerminal;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::process::CommandExt;
    use std::process::Child;
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

    /// Poll slice: short enough to notice a finished child promptly (a
    /// background grandchild may keep the pty open), long enough to stay idle.
    const POLL_MS: libc::c_int = 100;
    /// How long the pty may stay silent after the child exited before we stop
    /// relaying. Commands like `cmd &` leave a background process holding the
    /// slave open; without this the relay would wait for it (the old inherited
    /// stdio path returned as soon as the shell exited).
    const RELAY_GRACE_MS: u32 = 200;

    /// Set by the `SIGWINCH` handler; the relay re-reads the size on its next
    /// pass (poll is never restarted by `SA_RESTART`, so it returns `EINTR`).
    static WINCH: AtomicBool = AtomicBool::new(false);
    /// The user's terminal settings while we hold it in raw mode, reachable
    /// from the signal handler and the panic hook (which cannot run `Drop`).
    static SAVED_TERMIOS: AtomicPtr<libc::termios> = AtomicPtr::new(std::ptr::null_mut());
    static HOOK_INSTALLED: Once = Once::new();

    /// Why a pty run did not produce an outcome.
    pub(super) enum PtyError {
        /// The pty could not be set up; nothing was spawned, so the caller may
        /// safely retry with the portable pipe capture.
        Unavailable(io::Error),
        /// The child was already running (or reaped); never retry — that would
        /// execute the command twice.
        Failed(io::Error),
    }

    // ── Terminal state ──────────────────────────────────────────────────────

    /// Restore the saved terminal settings. Async-signal-safe (only
    /// `tcsetattr`), hence usable from the fatal-signal handler.
    fn restore_termios() {
        let saved = SAVED_TERMIOS.load(Ordering::SeqCst);
        if !saved.is_null() {
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved) };
        }
    }

    extern "C" fn on_winch(_sig: libc::c_int) {
        WINCH.store(true, Ordering::SeqCst);
    }

    /// A fatal signal must not leave the terminal raw: restore, re-raise with
    /// the default disposition so the exit status stays correct.
    extern "C" fn on_fatal(sig: libc::c_int) {
        restore_termios();
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }

    /// `panic = "abort"` means `Drop` never runs on a panic, so chain a hook
    /// that restores the terminal (installed once, keeps the previous hook).
    fn install_restore_hook() {
        HOOK_INSTALLED.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore_termios();
                previous(info);
            }));
        });
    }

    unsafe fn set_handler(
        sig: libc::c_int,
        handler: libc::sighandler_t,
    ) -> io::Result<libc::sigaction> {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = handler;
        action.sa_flags = 0;
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(sig, &action, &mut previous) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(previous)
    }

    /// Dispositions installed for the duration of one relayed command.
    struct SignalGuard {
        saved: Vec<(libc::c_int, libc::sigaction)>,
    }

    impl SignalGuard {
        fn install() -> Self {
            let mut saved = Vec::new();
            let winch =
                unsafe { set_handler(libc::SIGWINCH, on_winch as *const () as libc::sighandler_t) };
            if let Ok(previous) = winch {
                saved.push((libc::SIGWINCH, previous));
            }
            for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
                if let Ok(previous) =
                    unsafe { set_handler(sig, on_fatal as *const () as libc::sighandler_t) }
                {
                    saved.push((sig, previous));
                }
            }
            SignalGuard { saved }
        }
    }

    impl Drop for SignalGuard {
        fn drop(&mut self) {
            for (sig, previous) in &self.saved {
                unsafe { libc::sigaction(*sig, previous, std::ptr::null_mut()) };
            }
        }
    }

    /// The user's terminal in raw mode while the child owns it, restored
    /// unconditionally on `Drop` (every early return included).
    struct RawModeGuard {
        fd: RawFd,
        saved: Box<libc::termios>,
    }

    impl RawModeGuard {
        fn enable(fd: RawFd) -> io::Result<Self> {
            let mut current: libc::termios = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(fd, &mut current) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let saved = Box::new(current);
            let mut raw = current;
            // cfmakeraw(), except that OPOST stays on: the relayed pty output is
            // already CRLF-terminated by the slave's line discipline, and keeping
            // output post-processing guarantees a bare '\n' still returns the
            // cursor to column 0 instead of drawing a staircase.
            raw.c_iflag &= !(libc::IGNBRK
                | libc::BRKINT
                | libc::PARMRK
                | libc::ISTRIP
                | libc::INLCR
                | libc::IGNCR
                | libc::ICRNL
                | libc::IXON);
            raw.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
            raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
            raw.c_cflag |= libc::CS8;
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
                return Err(io::Error::last_os_error());
            }
            SAVED_TERMIOS.store(Box::as_ptr(&saved).cast_mut(), Ordering::SeqCst);
            Ok(RawModeGuard { fd, saved })
        }
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &*self.saved) };
            SAVED_TERMIOS.store(std::ptr::null_mut(), Ordering::SeqCst);
        }
    }

    // ── pty plumbing ────────────────────────────────────────────────────────

    /// Allocate a pty pair. Linux/Android use `posix_openpt` (no `libutil` link
    /// dependency); the other Unixes only have the BSD `openpty`.
    fn open_pty_pair() -> io::Result<(OwnedFd, OwnedFd)> {
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let mut master: libc::c_int = -1;
            let mut slave: libc::c_int = -1;
            let rc = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            unsafe { Ok((OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))) }
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let master_fd =
                unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
            if master_fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
            if unsafe { libc::grantpt(master_fd) } != 0 || unsafe { libc::unlockpt(master_fd) } != 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut name = [0 as libc::c_char; 256];
            let rc = unsafe { libc::ptsname_r(master_fd, name.as_mut_ptr(), name.len()) };
            if rc != 0 {
                return Err(io::Error::from_raw_os_error(rc));
            }
            let slave_fd = unsafe {
                libc::open(
                    name.as_ptr(),
                    libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
                )
            };
            if slave_fd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok((master, unsafe { OwnedFd::from_raw_fd(slave_fd) }))
        }
    }

    fn read_winsize(fd: RawFd) -> Option<libc::winsize> {
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) } == 0
            && size.ws_row > 0
            && size.ws_col > 0
        {
            Some(size)
        } else {
            None
        }
    }

    /// Push our terminal size to the pty (the kernel signals the child's
    /// process group when the size changes, e.g. a `vim` repaint).
    fn sync_winsize(master_fd: RawFd, source_fd: RawFd) {
        if source_fd < 0 {
            return;
        }
        if let Some(size) = read_winsize(source_fd) {
            unsafe { libc::ioctl(master_fd, libc::TIOCSWINSZ, &size) };
        }
    }

    fn read_fd(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n >= 0 {
                return Ok(n as usize);
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
    }

    fn write_all_fd(fd: RawFd, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
            if n >= 0 {
                bytes = &bytes[n as usize..];
                continue;
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
        Ok(())
    }

    /// Relay until the child is done, returning the captured output. Bytes are
    /// written through immediately (stdout raw, no line buffering in between).
    fn relay(
        master_fd: RawFd,
        stdin_fd: RawFd,
        window_fd: RawFd,
        child: &mut Child,
    ) -> io::Result<String> {
        let mut capture = OutputCapture::default();
        let mut buf = [0u8; 8192];
        let mut stdin_open = true;
        let mut child_exited = false;
        let mut idle_ms: u32 = 0;
        let _ = io::stdout().flush();

        let mut pollfds = [
            libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stdin_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        loop {
            pollfds[0].revents = 0;
            pollfds[1].events = if stdin_open { libc::POLLIN } else { 0 };
            pollfds[1].revents = 0;
            let ready = unsafe { libc::poll(pollfds.as_mut_ptr(), 2, POLL_MS) };
            if ready < 0 {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::Interrupted {
                    return Err(err);
                }
            }
            if WINCH.swap(false, Ordering::SeqCst) {
                sync_winsize(master_fd, window_fd);
            }
            if ready == 0 {
                if !child_exited && child.try_wait()?.is_some() {
                    child_exited = true;
                }
                if child_exited {
                    idle_ms += POLL_MS as u32;
                    if idle_ms >= RELAY_GRACE_MS {
                        break;
                    }
                }
                continue;
            }
            idle_ms = 0;

            // Our stdin -> pty (the pty's line discipline does the editing).
            if pollfds[1].revents != 0 {
                match read_fd(stdin_fd, &mut buf) {
                    Ok(0) => stdin_open = false,
                    Ok(n) => {
                        let bytes = &buf[..n];
                        // Ctrl-C is a raw 0x03 byte here (our terminal is in raw
                        // mode with ISIG off) and is forwarded to the child, so
                        // the command is interrupted first — the terminal
                        // convention. The same byte registers the exit intent:
                        // the REPL asks whether to leave once the command is
                        // done, instead of the key doing nothing.
                        if bytes.contains(&0x03) {
                            crate::ui::request_exit();
                        }
                        if write_all_fd(master_fd, bytes).is_err() {
                            stdin_open = false; // slave gone: nothing left to feed
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => stdin_open = false,
                }
            }

            // pty -> our stdout, live, while keeping a bounded copy.
            if pollfds[0].revents != 0 {
                match read_fd(master_fd, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        write_all_fd(libc::STDOUT_FILENO, &buf[..n])?;
                        capture.push(&buf[..n]);
                    }
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(err) if err.raw_os_error() == Some(libc::EIO) => break,
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                    Err(err) => return Err(err),
                }
            }

            if !child_exited && child.try_wait()?.is_some() {
                child_exited = true;
                idle_ms = 0;
            }
        }
        Ok(capture.finish())
    }

    /// Run `command` on a fresh pty, relaying bytes until it exits.
    ///
    /// Everything that can fail *before the child exists* (`Unavailable`) is done
    /// before `command` is mutated, so the caller can safely retry with
    /// `run_piped` — a `pre_exec` closure or stdio override left behind would
    /// refer to a closed fd otherwise.
    pub(super) fn run(command: &mut Command) -> Result<CapturedRun, PtyError> {
        let (master, slave) = open_pty_pair().map_err(PtyError::Unavailable)?;
        let master_fd = master.as_raw_fd();
        let slave_fd = slave.as_raw_fd();

        let stdin_fd = libc::STDIN_FILENO;
        let stdin_is_tty = io::stdin().is_terminal();
        let window_fd = if io::stdout().is_terminal() {
            libc::STDOUT_FILENO
        } else if stdin_is_tty {
            stdin_fd
        } else {
            -1
        };

        install_restore_hook();
        let _signals = SignalGuard::install();
        let _raw = if stdin_is_tty {
            Some(RawModeGuard::enable(stdin_fd).map_err(PtyError::Unavailable)?)
        } else {
            None
        };

        // The child gets the pty as its controlling terminal, in a session of
        // its own: terminal-generated signals (Ctrl-C -> SIGINT) then stay
        // between the pty and the child, and `setsid` requires no controlling
        // terminal, which is exactly the state after `fork`.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                for fd in 0..=2 {
                    if libc::dup2(slave_fd, fd) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        sync_winsize(master_fd, window_fd);

        let mut child = command.spawn().map_err(PtyError::Failed)?;
        // Drop our copy of the slave: without it the master would never read
        // EOF once the child (and everything it spawned) is gone.
        drop(slave);
        let output = relay(master_fd, stdin_fd, window_fd, &mut child).map_err(PtyError::Failed)?;
        let status = child.wait().map_err(PtyError::Failed)?;
        Ok(CapturedRun {
            code: status.code(),
            output,
        })
    }
}

// ── Portable fallback: pipes ────────────────────────────────────────────────

/// Copy one pipe to `writer` as it arrives, flushing every chunk and feeding the
/// bounded capture. Used for Windows (no pty) and as the Unix pty fallback.
fn pump<R: Read, W: Write>(mut reader: R, mut writer: W, capture: Arc<Mutex<OutputCapture>>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let _ = writer.write_all(&buf[..n]);
                let _ = writer.flush();
                if let Ok(mut capture) = capture.lock() {
                    capture.push(&buf[..n]);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

/// Capture a child's output through pipes: live, but the child sees pipes
/// rather than a TTY (no colors/progress bars, no full-screen programs). This is
/// the Windows path and the Unix fallback; `run_tests` exercises it directly so
/// it does not rot unnoticed on the platform that always uses it.
pub(crate) fn run_piped(mut command: Command) -> io::Result<CapturedRun> {
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let capture = Arc::new(Mutex::new(OutputCapture::default()));
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let capture = Arc::clone(&capture);
        readers.push(std::thread::spawn(move || {
            pump(stdout, io::stdout(), capture)
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let capture = Arc::clone(&capture);
        readers.push(std::thread::spawn(move || {
            pump(stderr, io::stderr(), capture)
        }));
    }
    let status = child.wait()?;
    for reader in readers {
        let _ = reader.join();
    }
    let output = capture
        .lock()
        .map(|capture| capture.finish())
        .unwrap_or_default();
    Ok(CapturedRun {
        code: status.code(),
        output,
    })
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Run `command` capturing its output for the auto-refine summary.
///
/// On Unix the command runs on a pty (streaming + full TTY semantics). When no
/// pty can be allocated the command still runs, through the portable pipe
/// capture, and the reason is reported so the downgrade is visible.
// `mut` is only needed by the unix pty path (`unix_pty::run(&mut command)`);
// without this the Windows build warns `unused_mut`, which `clippy -D warnings`
// in CI turns into an error.
#[cfg_attr(not(unix), allow(unused_mut))]
pub(crate) fn run_captured(mut command: Command) -> io::Result<CapturedRun> {
    #[cfg(unix)]
    {
        match unix_pty::run(&mut command) {
            Ok(run) => Ok(run),
            Err(unix_pty::PtyError::Failed(err)) => Err(err),
            Err(unix_pty::PtyError::Unavailable(reason)) => {
                crate::ui::print_info(&t!("info.pty_fallback", "e" => reason));
                run_piped(command)
            }
        }
    }
    #[cfg(not(unix))]
    {
        run_piped(command)
    }
}
