#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("mise executable not found")]
    NotFound,
    #[error("mise failed (exit {code}): {stderr}")]
    CommandFailed { code: i32, stderr: String },
    #[error("invalid JSON from mise: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}
