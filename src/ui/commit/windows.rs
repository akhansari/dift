use alacritty_terminal::{
    event::{OnResize, WindowSize},
    tty::{self, Options},
};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::{io, time::Duration};

pub(super) fn spawn(options: Options, size: WindowSize) -> io::Result<tty::Pty> {
    tty::new(&options, size, 0)
}

pub(super) fn resize(pty: &mut tty::Pty, size: WindowSize) -> io::Result<()> {
    pty.on_resize(size);
    Ok(())
}

pub(super) fn read_exhausted(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
}

pub(super) fn read_input(input: &mut Vec<u8>) -> io::Result<()> {
    if !event::poll(Duration::from_millis(10))? {
        return Ok(());
    }
    match event::read()? {
        Event::Paste(text) => input.extend_from_slice(text.as_bytes()),
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            if key.modifiers.contains(KeyModifiers::ALT) {
                input.push(0x1b);
            }
            match key.code {
                KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let ch = ch.to_ascii_uppercase();
                    if ('@'..='_').contains(&ch) {
                        input.push(ch as u8 & 0x1f);
                    } else if ch == ' ' {
                        input.push(0);
                    }
                }
                KeyCode::Char(ch) => {
                    input.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes())
                }
                KeyCode::Enter => input.push(b'\r'),
                KeyCode::Backspace => input.push(0x7f),
                KeyCode::Tab => input.push(b'\t'),
                KeyCode::BackTab => input.extend_from_slice(b"\x1b[Z"),
                KeyCode::Esc => input.push(0x1b),
                KeyCode::Up => input.extend_from_slice(b"\x1b[A"),
                KeyCode::Down => input.extend_from_slice(b"\x1b[B"),
                KeyCode::Right => input.extend_from_slice(b"\x1b[C"),
                KeyCode::Left => input.extend_from_slice(b"\x1b[D"),
                KeyCode::Home => input.extend_from_slice(b"\x1b[H"),
                KeyCode::End => input.extend_from_slice(b"\x1b[F"),
                KeyCode::Insert => input.extend_from_slice(b"\x1b[2~"),
                KeyCode::Delete => input.extend_from_slice(b"\x1b[3~"),
                KeyCode::PageUp => input.extend_from_slice(b"\x1b[5~"),
                KeyCode::PageDown => input.extend_from_slice(b"\x1b[6~"),
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}
