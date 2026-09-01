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

/// What a card tests. Word is a single headword (Chinese->English with
/// difficulty-graded prompts), Phrase is a collocation / fixed phrase,
/// Pattern is a sentence pattern or special usage.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CardType {
    Word,
    Phrase,
    Pattern,
}

impl CardType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CardType::Word => "word",
            CardType::Phrase => "phrase",
            CardType::Pattern => "pattern",
        }
    }
}

/// Prompt difficulty for Chinese->English cards. Three tiers matched to how
/// familiar the learner is with the card (auto-adjusted by SM2 grades and
/// manually overridable):
///  - Easy:   cloze example sentence + the word's Chinese meaning.
///  - Medium: cloze example sentence + its Chinese translation (no meaning).
///  - Hard:   no example, only first letter + Chinese meaning.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub fn as_str(&self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
        }
    }

    /// Parse the difficulty stored in SQLite; unknown values fall back to Easy.
    pub fn parse(s: &str) -> Difficulty {
        match s {
            "hard" => Difficulty::Hard,
            "medium" => Difficulty::Medium,
            _ => Difficulty::Easy,
        }
    }

    /// Make the prompt easier (more help). Used when the learner forgets.
    pub fn step_down(&mut self) {
        *self = match self {
            Difficulty::Easy => Difficulty::Easy,
            Difficulty::Medium => Difficulty::Easy,
            Difficulty::Hard => Difficulty::Medium,
        };
    }

    /// Make the prompt harder (less help). Used when the learner finds the
    /// card easy, but only after they have seen it a few times.
    pub fn step_up(&mut self) {
        *self = match self {
            Difficulty::Easy => Difficulty::Medium,
            _ => Difficulty::Hard,
        };
    }
}

/// A flashcard in the SM2 queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: i64,
    pub headword: String,
    /// Which definition the card covers (for multi-sense words).
    pub sense: usize,
    pub card_type: CardType,
    /// Human difficulty flag (user-settable), used to pick the prompt style.
    pub difficulty: Difficulty,
    /// For Phrase/Pattern cards: source headword + the phrase text itself.
    /// e.g. source "break", phrase "break away (from sb/sth)".
    pub source: String,
    pub phrase: String,
    pub due: Option<chrono::NaiveDateTime>,
    pub interval_days: f64,
    pub ease: f64,
    pub reps: u32,
    pub lapses: u32,
    pub status: ReviewStatus,
    /// UTC timestamp when the card was created (for daily-new limiting).
    pub created_at: chrono::NaiveDateTime,
}

/// One entry in the review log (one grading action).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewLogEntry {
    pub card_id: i64,
    pub reviewed_at: chrono::NaiveDateTime,
    pub grade: u8,
    /// true when this was the card's first-ever review (counts toward
    /// the daily new-card limit).
    pub is_new: bool,
}

/// A word list (gaokao / IELTS / CET...) that constrains the study range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wordlist {
    pub id: i64,
    pub name: String,
    /// Number of headwords currently in the list.
    pub size: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReviewStatus {
    New,
    Learning,
    Review,
    Relearning,
}
