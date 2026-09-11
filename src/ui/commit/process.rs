use alacritty_terminal::{
    event::WindowSize,
    tty::{self, ChildEvent, EventedPty, EventedReadWrite, Options, Shell},
};
use std::{
    io::{self, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

pub(super) const HANDOFF_DELAY: Duration = Duration::from_secs(1);

/// The UI owns this nonblocking PTY until completion, including during handoff.
pub(super) struct Process {
    pty: tty::Pty,
    started: Instant,
    size: WindowSize,
    output: Vec<u8>,
    input: Vec<u8>,
    written: usize,
    finished: bool,
}

impl Process {
    pub(super) fn start(root: &Path, message: &str, columns: u16, rows: u16) -> io::Result<Self> {
        // Alacritty ignores chdir errors in its child.
        if !root.is_dir() {
            return Err(io::Error::other("The repository directory is unavailable"));
        }
        let options = Options {
            shell: Some(Shell::new(
                "git".into(),
                vec!["commit".into(), "-m".into(), message.into()],
            )),
            working_directory: Some(root.to_owned()),
            #[cfg(windows)]
            escape_args: true,
            ..Options::default()
        };
        let size = window_size(columns, rows);
        let pty = platform::spawn(options, size)?;
        Ok(Self {
            pty,
            started: Instant::now(),
            size,
            output: Vec::new(),
            input: Vec::new(),
            written: 0,
            finished: false,
        })
    }

    pub(super) fn handoff_due(&self) -> bool {
        self.started.elapsed() >= HANDOFF_DELAY
    }

    pub(super) fn poll(&mut self) -> io::Result<Option<Result<(), String>>> {
        // Observe exit before the timeout, then collect the final output. Do not
        // wait for EOF: hooks can leave descendants holding the slave open.
        let exited = self.pty.next_child_event();
        self.finished |= exited.is_some();
        self.read_output()?;
        if let Some(ChildEvent::Exited(status)) = exited {
            return Ok(Some(match status {
                Some(status) if status.success() => Ok(()),
                Some(status) => Err(format!("Git commit failed ({status})")),
                None => Err("Git commit ended without an exit status".into()),
            }));
        }
        self.flush_input()?;
        Ok(None)
    }

    fn read_output(&mut self) -> io::Result<()> {
        let mut buffer = [0; 65536];
        // Bound each turn so continuous hook output cannot postpone handoff.
        for _ in 0..16 {
            match self.pty.reader().read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => self.output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if platform::read_exhausted(&error) => break,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn flush_input(&mut self) -> io::Result<()> {
        while self.written < self.input.len() {
            match self.pty.writer().write(&self.input[self.written..]) {
                Ok(0) => break,
                Ok(count) => self.written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        if self.written == self.input.len() {
            self.input.clear();
            self.written = 0;
        }
        Ok(())
    }

    pub(super) fn resize(&mut self, columns: u16, rows: u16) -> io::Result<()> {
        let size = window_size(columns, rows);
        if self.size.num_cols != size.num_cols || self.size.num_lines != size.num_lines {
            platform::resize(&mut self.pty, size)?;
            self.size = size;
        }
        Ok(())
    }

    pub(super) fn show_output(&mut self) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&self.output)?;
        stdout.flush()?;
        self.output.clear();
        Ok(())
    }

    pub(super) fn read_input(&mut self) -> io::Result<()> {
        platform::read_input(&mut self.input)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // On an I/O error, avoid Alacritty's indefinite wait on a child which
        // ignores SIGHUP. The process group belongs exclusively to this PTY.
        #[cfg(unix)]
        if !self.finished
            && let Some(pid) = rustix::process::Pid::from_raw(self.pty.child().id() as i32)
        {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
}

fn window_size(columns: u16, rows: u16) -> WindowSize {
    WindowSize {
        num_cols: columns.max(1),
        num_lines: rows.max(1),
        cell_width: 0,
        cell_height: 0,
    }
}
