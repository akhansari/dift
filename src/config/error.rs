use crate::input::BindingError;
use std::{error::Error, fmt, io, num::TryFromIntError, path::PathBuf};

/// A failure reading, parsing, or validating application configuration.
#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse(kdl::KdlError),
    Validation(ValidationError),
    Binding(BindingError),
    /// The file containing an invalid configuration, with its original cause.
    AtPath {
        path: PathBuf,
        source: Box<ConfigError>,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "Cannot read config {}: {source}", path.display())
            }
            Self::AtPath { path, source } => {
                write!(f, "Invalid config {}: {source}", path.display())
            }
            Self::Parse(error) => error.fmt(f),
            Self::Validation(error) => error.fmt(f),
            Self::Binding(error) => error.fmt(f),
        }
    }
}
impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::AtPath { source, .. } => Some(source.as_ref()),
            Self::Parse(error) => Some(error),
            Self::Validation(error) => Some(error),
            Self::Binding(error) => Some(error),
        }
    }
}
impl From<kdl::KdlError> for ConfigError {
    fn from(error: kdl::KdlError) -> Self {
        Self::Parse(error)
    }
}
impl From<ValidationError> for ConfigError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}
impl From<BindingError> for ConfigError {
    fn from(error: BindingError) -> Self {
        Self::Binding(error)
    }
}

/// A syntactically valid setting that does not satisfy the configuration schema.
#[derive(Debug)]
pub enum ValidationError {
    Duplicate {
        setting: String,
    },
    Unknown {
        setting: String,
    },
    ArgumentsNotAllowed {
        setting: String,
    },
    ChildBlockRequired {
        setting: String,
    },
    ChildBlockNotAllowed {
        setting: String,
    },
    ExpectedValue {
        setting: String,
        expected: &'static str,
    },
    UnsupportedBindingProperty,
    UnknownBindingOperation {
        operation: String,
    },
    ValueTooLarge {
        setting: &'static str,
        source: TryFromIntError,
    },
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { setting } => write!(f, "Duplicate {setting}"),
            Self::Unknown { setting } => write!(f, "Unknown {setting}"),
            Self::ArgumentsNotAllowed { setting } => write!(f, "{setting} cannot have arguments"),
            Self::ChildBlockRequired { setting } => write!(f, "{setting} requires a child block"),
            Self::ChildBlockNotAllowed { setting } => {
                write!(f, "{setting} cannot have child blocks")
            }
            Self::ExpectedValue { setting, expected } => write!(f, "{setting} expects {expected}"),
            Self::UnsupportedBindingProperty => {
                f.write_str("Only a string do property is supported on bindings")
            }
            Self::UnknownBindingOperation { operation } => {
                write!(f, "Unknown binding operation {operation}")
            }
            Self::ValueTooLarge { setting, .. } => write!(f, "{setting} is too large"),
        }
    }
}
impl Error for ValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ValueTooLarge { source, .. } => Some(source),
            _ => None,
        }
    }
}
