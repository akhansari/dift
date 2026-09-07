use crate::input::BindingError;
use std::{io, num::TryFromIntError, path::PathBuf};
use thiserror::Error;

/// A failure reading, parsing, or validating application configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Cannot read config {}: {source}", .path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{0}")]
    Parse(#[from] kdl::KdlError),
    #[error("{0}")]
    Validation(#[from] ValidationError),
    #[error("{0}")]
    Binding(#[from] BindingError),
    /// The file containing an invalid configuration, with its original cause.
    #[error("Invalid config {}: {source}", .path.display())]
    AtPath {
        path: PathBuf,
        source: Box<ConfigError>,
    },
}

/// A syntactically valid setting that does not satisfy the configuration schema.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Duplicate {setting}")]
    Duplicate { setting: String },
    #[error("Unknown {setting}")]
    Unknown { setting: String },
    #[error("{setting} cannot have arguments")]
    ArgumentsNotAllowed { setting: String },
    #[error("{setting} requires a child block")]
    ChildBlockRequired { setting: String },
    #[error("{setting} cannot have child blocks")]
    ChildBlockNotAllowed { setting: String },
    #[error("{setting} expects {expected}")]
    ExpectedValue {
        setting: String,
        expected: &'static str,
    },
    #[error("Only a string do property is supported on bindings")]
    UnsupportedBindingProperty,
    #[error("Unknown binding operation {operation}")]
    UnknownBindingOperation { operation: String },
    #[error("{setting} is too large")]
    ValueTooLarge {
        setting: &'static str,
        source: TryFromIntError,
    },
}
