//! lexicon-core: pure-local dictionary + spaced repetition engine.
//! No cloud. All data stays on-device in a SQLite db.

pub mod error;
pub mod mdx_parser;
pub mod model;
pub mod sm2;
pub mod storage;

pub use error::{Error, Result};
pub use model::{Card, Entry, FieldValue, ReviewStatus};
