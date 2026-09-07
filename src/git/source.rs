use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::GitError;
use crate::model::WorktreeMetadata;

pub(super) fn activity_metadata(path: &Path) -> Option<WorktreeMetadata> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Some(WorktreeMetadata::Present {
            regular: metadata.is_file(),
            symlink: metadata.is_symlink(),
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
            readonly: metadata.permissions().readonly(),
            link_target: metadata
                .is_symlink()
                .then(|| fs::read_link(path).ok())
                .flatten(),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(WorktreeMetadata::Missing),
        Err(_) => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Entry {
    Missing,
    Regular { bytes: u64 },
    Symlink { bytes: u64 },
    Special { bytes: u64 },
}

impl Entry {
    pub(super) fn size(self) -> u64 {
        match self {
            Self::Missing => 0,
            Self::Regular { bytes } | Self::Symlink { bytes } | Self::Special { bytes } => bytes,
        }
    }
}

/// Expected reasons for unavailable full text stay distinct from I/O failures.
pub(super) enum TextSource {
    Text(Arc<str>),
    Missing,
    NonUtf8,
    Special,
}

pub(super) fn inspect(path: &Path) -> Result<Entry, GitError> {
    let Some(metadata) = optional_read(
        fs::symlink_metadata(path),
        "Inspect working-tree file",
        path,
    )?
    else {
        return Ok(Entry::Missing);
    };
    let bytes = metadata.len();
    Ok(if metadata.is_file() {
        Entry::Regular { bytes }
    } else if metadata.is_symlink() {
        Entry::Symlink { bytes }
    } else {
        Entry::Special { bytes }
    })
}

/// A file can disappear between status collection, inspection, and reading.
pub(super) fn read_file(path: &Path) -> Result<Option<Vec<u8>>, GitError> {
    optional_read(fs::read(path), "Read working-tree file", path)
}

pub(super) fn read_link(path: &Path) -> Result<Option<PathBuf>, GitError> {
    optional_read(fs::read_link(path), "Read symbolic link", path)
}

pub(super) fn read_text(path: &Path, entry: Entry) -> Result<TextSource, GitError> {
    Ok(match entry {
        Entry::Missing => TextSource::Missing,
        Entry::Special { .. } => TextSource::Special,
        Entry::Symlink { .. } => match read_link(path)? {
            Some(target) => TextSource::Text(Arc::from(target.to_string_lossy().into_owned())),
            None => TextSource::Missing,
        },
        Entry::Regular { .. } => match read_file(path)? {
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(text) => TextSource::Text(Arc::from(text)),
                Err(_) => TextSource::NonUtf8,
            },
            None => TextSource::Missing,
        },
    })
}

fn optional_read<T>(
    result: io::Result<T>,
    operation: &'static str,
    path: &Path,
) -> Result<Option<T>, GitError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(GitError::file(operation, path, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn inspection_errors_retain_the_path_operation_and_io_cause() -> crate::AppResult<()> {
        // Given
        let path = Path::new("invalid\0path");
        // When
        let result = inspect(path);
        // Then
        let error = result.err().ok_or("An invalid file path was accepted")?;
        assert!(matches!(
            &error,
            GitError::File { operation: "Inspect working-tree file", path: failed_path, .. }
                if failed_path == path
        ));
        assert!(error.source().is_some_and(|cause| cause.is::<io::Error>()));
        Ok(())
    }
}
