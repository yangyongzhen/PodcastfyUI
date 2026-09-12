//! Application-wide error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("extraction failed: {0}")]
    Extraction(String),
    #[error("llm failed: {0}")]
    Llm(String),
    #[error("tts failed: {0}")]
    Tts(String),
    #[error("{0}")]
    Other(String),
}
