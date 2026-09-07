pub mod config;
pub mod git;
pub mod input;
pub mod model;
pub mod render;
pub mod state;
mod text;
pub mod ui;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
