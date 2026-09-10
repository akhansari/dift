use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line},
    term::{Config, TermMode, cell::Flags},
    vte::ansi::{Color, Processor},
};
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Clone, Copy)]
struct Size {
    columns: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn columns(&self) -> usize {
        self.columns
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn total_lines(&self) -> usize {
        self.rows
    }
}

struct Listener(Sender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

pub struct Screen {
    term: Term<Listener>,
    parser: Processor,
    events: Receiver<Event>,
    size: Size,
}

impl Screen {
    pub fn new(columns: u16, rows: u16) -> Self {
        let size = Size {
            columns: usize::from(columns).max(alacritty_terminal::term::MIN_COLUMNS),
            rows: usize::from(rows).max(1),
        };
        let (sender, events) = mpsc::channel();
        Self {
            term: Term::new(
                Config {
                    scrolling_history: 0,
                    ..Config::default()
                },
                &size,
                Listener(sender),
            ),
            parser: Processor::new(),
            events,
            size,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }

    pub fn resize(&mut self, columns: u16, rows: u16) {
        self.size = Size {
            columns: usize::from(columns).max(alacritty_terminal::term::MIN_COLUMNS),
            rows: usize::from(rows).max(1),
        };
        self.term.resize(self.size);
    }

    pub fn rows(&self) -> usize {
        self.size.rows
    }
    pub fn columns(&self) -> usize {
        self.size.columns
    }

    pub fn cell(&self, x: usize, y: usize) -> &alacritty_terminal::term::cell::Cell {
        &self.term.grid()[Line(y as i32)][Column(x)]
    }

    /// Keep one string per terminal cell; a wide glyph's continuation is empty.
    pub fn cell_text(&self, x: usize, y: usize) -> String {
        let cell = self.cell(x, y);
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            return String::new();
        }
        std::iter::once(cell.c)
            .chain(cell.zerowidth().into_iter().flatten().copied())
            .collect()
    }

    pub fn row(&self, y: usize) -> String {
        (0..self.columns()).map(|x| self.cell_text(x, y)).collect()
    }

    pub fn text(&self) -> String {
        (0..self.rows())
            .map(|y| self.row(y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn contains(&self, text: &str) -> bool {
        (0..self.rows()).any(|y| self.row(y).contains(text))
    }

    pub fn selected(&self, name: &str) -> bool {
        let width = (self.columns() / 3).min(34);
        (0..self.rows().saturating_sub(3)).any(|y| {
            self.cell(0, y).c == '▎'
                && (1..width)
                    .map(|x| self.cell_text(x, y))
                    .collect::<String>()
                    .split_whitespace()
                    .any(|word| word == name)
        }) || (name == "All changes"
            && (0..self.rows().saturating_sub(3))
                .any(|y| self.cell(0, y).c == '▎' && self.row(y).contains("All changes")))
    }

    pub fn badge(&self, name: &str, badge: &str) -> bool {
        let width = (self.columns() / 3).min(34);
        (0..self.rows().saturating_sub(3)).any(|y| {
            let row: String = (0..width).map(|x| self.cell_text(x, y)).collect();
            row.split_once(name)
                .is_some_and(|(_, tail)| tail.split_whitespace().next() == Some(badge))
        })
    }

    pub fn locate(&self, text: &str) -> super::Result<(usize, usize)> {
        (0..self.rows())
            .find_map(|y| {
                (0..self.columns())
                    .find(|&x| {
                        (x..self.columns())
                            .map(|col| self.cell_text(col, y))
                            .collect::<String>()
                            .starts_with(text)
                    })
                    .map(|x| (x, y))
            })
            .ok_or_else(|| format!("Missing {text:?} in terminal:\n{}", self.text()).into())
    }

    pub fn highlighted(&self) -> Vec<String> {
        (0..self.rows())
            .filter_map(|y| {
                let text: String = (0..self.columns())
                    .filter(|&x| self.cell(x, y).flags.contains(Flags::INVERSE))
                    .map(|x| self.cell_text(x, y))
                    .collect();
                (!text.is_empty()).then_some(text)
            })
            .collect()
    }

    pub fn alternate_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Match the existing startup benchmark: unified gutter at column 34,
    /// source at column 48, excluding the status/prompt rows and right border.
    pub fn visible_source(&self) -> (bool, bool) {
        let mut readable = false;
        let mut highlighted = false;
        for y in 0..self.rows().saturating_sub(3) {
            let gutter: String = (34..48.min(self.columns()))
                .map(|x| self.cell(x, y).c)
                .collect();
            let bytes = gutter.as_bytes();
            if bytes.len() != 14 || !matches!(bytes[12], b' ' | b'+' | b'-' | b'!') {
                continue;
            }
            if ![&bytes[..5], &bytes[6..11]].iter().any(|field| {
                field.iter().any(u8::is_ascii_digit)
                    && field.iter().all(|c| c.is_ascii_digit() || *c == b' ')
            }) {
                continue;
            }
            readable = true;
            let mut first: Option<Color> = None;
            for x in 48..self.columns().saturating_sub(1) {
                let cell = self.cell(x, y);
                if !cell.c.is_whitespace() {
                    highlighted |= first.is_some_and(|color| color != cell.fg);
                    first.get_or_insert(cell.fg);
                }
            }
        }
        (readable, highlighted)
    }
}
