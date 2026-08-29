use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("corrupt record: {0}")]
    Corrupt(String),
    #[error("mdx error: {0}")]
    Mdx(String),
}

pub type Result<T> = std::result::Result<T, Error>;
