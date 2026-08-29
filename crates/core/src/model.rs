use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A raw dictionary headword + definition body, before per-field processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub headword: String,
    /// Section name -> raw definition(s). e.g. "phonetic" -> "[h]/h/[/h]"
    pub fields: HashMap<String, Vec<String>>,
}

/// One parsed field value (strips markup tags, keeps structured content).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FieldValue {
    Text(String),
    List(Vec<String>),
}

/// A flashcard in the SM2 queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: i64,
    pub headword: String,
    /// Which definition the card covers (for multi-sense words).
    pub sense: usize,
    pub due: Option<chrono::NaiveDateTime>,
    pub interval_days: f64,
    pub ease: f64,
    pub reps: u32,
    pub lapses: u32,
    pub status: ReviewStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReviewStatus {
    New,
    Learning,
    Review,
    Relearning,
}
