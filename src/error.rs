use std::path::PathBuf;

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum MakerpmError {
    #[error("failed to read {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("validation failed with {count} error(s)")]
    Validation { count: usize },

    #[error("failed to download {url}")]
    Fetch {
        url: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("checksum mismatch for {filename}: expected {expected}, got {actual}")]
    #[diagnostic(severity(Error))]
    ChecksumMismatch {
        filename: String,
        expected: String,
        actual: String,
    },

    #[error("offline mode: remote source {filename} is not cached")]
    #[diagnostic(severity(Error))]
    OfflineUncached { filename: String },

    #[error("failed to create cache directory {path}")]
    CacheDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("rpmbuild failed with exit code {exit_code}")]
    #[diagnostic(severity(Error))]
    RpmbuildFailed {
        exit_code: i32,
        stderr_tail: String,
    },
}
