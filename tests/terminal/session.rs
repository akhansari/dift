use super::{Result, Screen};
use alacritty_terminal::{
    event::Event,
    tty::{self, ChildEvent, EventedPty, Options, Shell},
};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    process::{Pid, Signal, kill_process_group},
    termios::{InputModes, OptionalActions, Termios, Winsize, tcgetattr, tcsetattr, tcsetwinsize},
};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    os::fd::OwnedFd,
    path::Path,
    process::ExitStatus,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(8);
const RECENT_OUTPUT: usize = 64 * 1024;

pub struct SessionOptions<'a> {
    pub binary: &'a Path,
    pub cwd: &'a Path,
    pub config: &'a Path,
    pub args: &'a [&'a str],
    pub env: &'a [(&'a str, Option<&'a str>)],
}

pub struct Session {
    pty: tty::Pty,
    // Retain the slave to inspect termios after dift has exited.
    slave: OwnedFd,
    original: Termios,
    pub screen: Screen,
    pub clipboard: Vec<String>,
    pub bytes_read: usize,
    recent: Vec<u8>,
    status: Option<ExitStatus>,
    exited: bool,
}

impl Session {
    pub fn start(options: SessionOptions<'_>) -> Result<Self> {
        // Alacritty cannot remove inherited variables. env -u does so in the
        // child, without mutating the test runner's process-wide environment.
        let mut variables = BTreeMap::from([
            ("TERM", Some("xterm-256color")),
            ("COLORTERM", Some("truecolor")),
            ("NO_COLOR", None),
            ("VISUAL", None),
            ("EDITOR", None),
        ]);
        variables.extend(options.env.iter().copied());
        let mut args = Vec::new();
        for (key, value) in &variables {
            if value.is_none() {
                args.extend(["-u".into(), (*key).into()]);
            }
        }
        args.push(path_string(options.binary)?);
        args.extend(["--config".into(), path_string(options.config)?]);
        args.extend(options.args.iter().map(|arg| (*arg).to_owned()));
        let config = Options {
            shell: Some(Shell::new("/usr/bin/env".into(), args)),
            working_directory: Some(options.cwd.to_owned()),
            env: variables
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key.into(), value.into())))
                .collect(),
            ..Options::default()
        };
        // Validate cwd ourselves: Alacritty ignores chdir errors in its child.
        if !options.cwd.is_dir() {
            return Err(format!("Invalid working directory: {}", options.cwd.display()).into());
        }
        let pair = rustix_openpty::openpty(None, Some(&winsize(120, 35)))?;
        let slave = pair.user.try_clone()?;
        let mut original = tcgetattr(&slave)?;
        // Alacritty enables IUTF8 when attaching the PTY on these platforms.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        original.input_modes.insert(InputModes::IUTF8);
        tcsetattr(&slave, OptionalActions::Now, &original)?;
        let pty = tty::from_fd(&config, 0, pair.controller, pair.user)?;
        Ok(Self {
            pty,
            slave,
            original,
            screen: Screen::new(120, 35),
            clipboard: Vec::new(),
            bytes_read: 0,
            recent: Vec::new(),
            status: None,
            exited: false,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pty.child().id()
    }

    pub fn ensure_running(&self) -> Result {
        if self.exited {
            return Err(self.failure("Child exited before completing the operation"));
        }
        Ok(())
    }

    pub fn termios(&self) -> Result<Termios> {
        Ok(tcgetattr(&self.slave)?)
    }

    pub fn restored(&self) -> Result<bool> {
        let current = self.termios()?;
        Ok(current.input_modes == self.original.input_modes
            && current.output_modes == self.original.output_modes
            && current.control_modes == self.original.control_modes
            && current.local_modes == self.original.local_modes
            // SpecialCodes exposes indexing/debugging, but no iterator or equality.
            && format!("{:?}", current.special_codes) == format!("{:?}", self.original.special_codes)
            && current.input_speed() == self.original.input_speed()
            && current.output_speed() == self.original.output_speed())
    }

    pub fn resize(&mut self, columns: u16, rows: u16) -> Result {
        if columns < 2 || rows == 0 {
            return Err("Terminal dimensions require at least two columns and one row".into());
        }
        // OnResize terminates the caller on failure; return the syscall error instead.
        tcsetwinsize(&self.slave, winsize(columns, rows))?;
        self.screen.resize(columns, rows);
        Ok(())
    }

    pub fn send(&mut self, text: &str) -> Result {
        let mut remaining = text.as_bytes();
        let deadline = Instant::now() + WAIT;
        while !remaining.is_empty() {
            match self.pty.file().write(remaining) {
                Ok(0) => return Err(self.failure("PTY closed while sending input")),
                Ok(count) => remaining = &remaining[count..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    wait_fd(self.pty.file(), PollFlags::OUT, Duration::from_millis(20))?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn pump(&mut self, timeout: Duration) -> Result {
        wait_fd(self.pty.file(), PollFlags::IN, timeout)?;
        let mut buffer = [0; 65536];
        // Limit work per call so continuous child output cannot defeat deadlines.
        for _ in 0..16 {
            match self.pty.file().read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    self.bytes_read += count;
                    self.recent.extend_from_slice(&buffer[..count]);
                    if self.recent.len() > RECENT_OUTPUT {
                        self.recent.drain(..self.recent.len() - RECENT_OUTPUT);
                    }
                    self.screen.feed(&buffer[..count]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) =>
                {
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
        let events: Vec<_> = self.screen.events().collect();
        for event in events {
            match event {
                Event::PtyWrite(text) => self.send(&text)?,
                Event::ClipboardStore(_, text) => self.clipboard.push(text),
                _ => {}
            }
        }
        if !self.exited
            && let Some(ChildEvent::Exited(status)) = self.pty.next_child_event()
        {
            self.exited = true;
            self.status = status;
        }
        Ok(())
    }

    pub fn output_contains(&self, bytes: &[u8]) -> bool {
        self.recent
            .windows(bytes.len())
            .any(|window| window == bytes)
    }

    pub fn wait(&mut self, text: &str) -> Result {
        self.wait_for(&format!("text {text:?}"), WAIT, |screen| {
            screen.contains(text)
        })
    }

    pub fn select(&mut self, name: &str) -> Result {
        self.wait_for(&format!("selected {name:?}"), WAIT, |screen| {
            screen.selected(name)
        })
    }

    pub fn wait_for(
        &mut self,
        description: &str,
        timeout: Duration,
        predicate: impl Fn(&Screen) -> bool,
    ) -> Result {
        let deadline = Instant::now() + timeout;
        loop {
            if predicate(&self.screen) {
                return Ok(());
            }
            if self.exited {
                return Err(self.failure(&format!("Child exited waiting for {description}")));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.failure(&format!("Timed out waiting for {description}")));
            }
            self.pump(remaining.min(Duration::from_millis(20)))?;
        }
    }

    /// Used only when the behavior under test is an interval of inactivity.
    pub fn observe(&mut self, duration: Duration) -> Result {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            self.pump(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20)),
            )?;
            if self.exited {
                return Err(self.failure("Child exited during observation"));
            }
        }
        Ok(())
    }

    pub fn quit(&mut self) -> Result {
        self.send("q")?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.exited && Instant::now() < deadline {
            self.pump(Duration::from_millis(10))?;
        }
        if !self.status.is_some_and(|status| status.success()) {
            return Err(self.failure("Expected a successful exit"));
        }
        if !self.restored()? || self.screen.alternate_screen() {
            return Err(self.failure("Exit did not restore the terminal"));
        }
        Ok(())
    }

    pub fn failure(&self, message: &str) -> Box<dyn std::error::Error + Send + Sync> {
        format!(
            "{message}; child status: {:?}\nLast screen:\n{}\nRecent output: {:?}",
            self.status,
            self.screen.text(),
            String::from_utf8_lossy(&self.recent)
        )
        .into()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Alacritty's destructor sends HUP then waits indefinitely. Kill the
        // isolated group first on failure, including any open fake editor.
        if !self.exited
            && let Some(pid) = Pid::from_raw(self.pid() as i32)
        {
            let _ = kill_process_group(pid, Signal::KILL);
        }
    }
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        format!(
            "Alacritty requires a UTF-8 command path: {}",
            path.display()
        )
        .into()
    })
}

fn winsize(columns: u16, rows: u16) -> Winsize {
    Winsize {
        ws_row: rows,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

fn wait_fd(fd: &std::fs::File, flags: PollFlags, duration: Duration) -> Result {
    let timeout = Timespec::try_from(duration)?;
    match poll(&mut [PollFd::new(fd, flags)], Some(&timeout)) {
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
