//! Shared headless terminal support for the opt-in smoke and performance targets.
// Each target uses a different subset of the same harness.
#![allow(dead_code)]

mod screen;
mod session;

pub use screen::Screen;
pub use session::{Session, SessionOptions};

pub type Result<T = ()> = dift::AppResult<T>;
