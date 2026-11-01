//! Errors from reading and writing the favorites file.

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StripError {
    #[error("could not read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("could not save {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("could not encode the favorites: {0}")]
    Encode(#[from] serde_json::Error),
}
