use crate::mdx_parser;
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
    Mdx(#[from] mdx_parser::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
