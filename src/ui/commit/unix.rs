use alacritty_terminal::{
    event::WindowSize,
    tty::{self, Options},
};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    termios::{Winsize, tcsetwinsize, ttyname},
};
use std::io;

pub(super) fn spawn(mut options: Options, size: WindowSize) -> io::Result<tty::Pty> {
    let pair = rustix_openpty::openpty(None, Some(&winsize(size)))?;
    let name = ttyname(&pair.user, Vec::new())?;
    let name = name.to_str().map_err(io::Error::other)?;
    options.env.insert("GPG_TTY".into(), name.into());
    tty::from_fd(&options, 0, pair.controller, pair.user)
}

pub(super) fn resize(pty: &mut tty::Pty, size: WindowSize) -> io::Result<()> {
    // Alacritty's OnResize exits the entire application on syscall failure.
    Ok(tcsetwinsize(pty.file(), winsize(size))?)
}

pub(super) fn read_exhausted(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error())
}

pub(super) fn read_input(input: &mut Vec<u8>) -> io::Result<()> {
    let stdin = io::stdin();
    let mut descriptors = [PollFd::new(&stdin, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: 10_000_000,
    };
    match poll(&mut descriptors, Some(&timeout)) {
        Ok(0) | Err(rustix::io::Errno::INTR) => return Ok(()),
        Ok(_) => {}
        Err(error) => return Err(error.into()),
    }
    let mut buffer = [0; 8192];
    match rustix::io::read(&stdin, &mut buffer[..]) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Terminal input closed",
        )),
        Ok(count) => {
            input.extend_from_slice(&buffer[..count]);
            Ok(())
        }
        Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn winsize(size: WindowSize) -> Winsize {
    Winsize {
        ws_row: size.num_lines,
        ws_col: size.num_cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}
