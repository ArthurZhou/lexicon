//! lexicon-core: pure-local dictionary + spaced repetition engine.
//! No cloud. All data stays on-device in a SQLite db.

pub mod definition;
pub mod error;
pub mod mdx_parser;
pub mod model;
pub mod prompt;
pub mod sm2;
pub mod storage;
pub mod study;

pub use error::{Error, Result};
pub use model::{Card, CardType, Difficulty, Entry, FieldValue, ReviewStatus};
