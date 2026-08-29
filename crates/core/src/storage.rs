//! SQLite persistence + range tables for fast lookup.

use crate::error::Result;
use crate::model::{Card, ReviewStatus};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Db {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    headword TEXT NOT NULL,
    sense INTEGER NOT NULL DEFAULT 0,
    definition TEXT NOT NULL,      -- JSON: FieldValue map
    UNIQUE(headword, sense)
);

CREATE TABLE IF NOT EXISTS words (
    id INTEGER PRIMARY KEY,
    headword TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS ranges (
    prefix TEXT NOT NULL,
    start_word_id INTEGER NOT NULL,
    end_word_id INTEGER NOT NULL,
    PRIMARY KEY(prefix)
);

CREATE TABLE IF NOT EXISTS cards (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    word_id INTEGER NOT NULL,
    sense INTEGER NOT NULL DEFAULT 0,
    due TEXT,
    interval_days REAL NOT NULL DEFAULT 0,
    ease REAL NOT NULL DEFAULT 2.5,
    reps INTEGER NOT NULL DEFAULT 0,
    lapses INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'New',
    FOREIGN KEY(word_id) REFERENCES words(id)
);

CREATE INDEX IF NOT EXISTS idx_cards_due ON cards(due);
CREATE INDEX IF NOT EXISTS idx_entries_headword ON entries(headword);
"#;

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Db> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Insert (or update) a raw entry. The definition is stored as-is (HTML).
    pub fn insert_entry(&mut self, headword: &str, definition: &str) -> Result<()> {
        // upsert: keep first insertion's definition
        self.conn.execute(
            "INSERT OR IGNORE INTO entries (headword, definition) VALUES (?1, ?2)",
            params![headword, definition],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO words (headword) VALUES (?1)",
            params![headword],
        )?;
        Ok(())
    }

    pub fn lookup(&self, word: &str) -> Result<Option<String>> {
        let mut stmt =
            self.conn
                .prepare("SELECT definition FROM entries WHERE headword = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![word])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn stats(&self) -> Result<(usize, usize)> {
        let words: usize = self.conn.query_row("SELECT COUNT(*) FROM words", [], |r| r.get(0))?;
        let cards: usize = self.conn.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0))?;
        Ok((words, cards))
    }

    /// Add a new card for `headword` (idempotent per word).
    pub fn add_new_card(&self, headword: &str) -> Result<i64> {
        let word_id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM words WHERE headword = ?1", params![headword], |r| r.get(0))
            .optional()?;
        let word_id = match word_id {
            Some(id) => id,
            None => {
                // unknown word: skip
                return Ok(-1);
            }
        };
        // check existing card
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM cards WHERE word_id = ?1",
                params![word_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO cards (word_id) VALUES (?1)",
            params![word_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List due review cards (those with due <= now or due IS NULL for new).
    pub fn due_cards(&self, now: &chrono::NaiveDateTime, limit: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, w.headword FROM cards c JOIN words w ON w.id = c.word_id
             WHERE c.due IS NULL OR c.due <= ?1 ORDER BY c.due IS NULL DESC, c.due ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now.to_string(), limit], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Load a card for review.
    pub fn load_card(&self, card_id: i64) -> Result<Option<Card>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, w.headword, c.due, c.interval_days, c.ease, c.reps, c.lapses, c.status
             FROM cards c JOIN words w ON w.id = c.word_id WHERE c.id = ?1",
        )?;
        let card = stmt
            .query_row(params![card_id], |r| {
                let due: Option<String> = r.get(2)?;
                let status: String = r.get(7)?;
                Ok(Card {
                    id: r.get(0)?,
                    headword: r.get(1)?,
                    sense: 0,
                    due: due.and_then(|s| chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok()),
                    interval_days: r.get(3)?,
                    ease: r.get(4)?,
                    reps: r.get(5)?,
                    lapses: r.get(6)?,
                    status: parse_status(&status),
                })
            })
            .optional()?;
        Ok(card)
    }

    /// Persist a reviewed card back to the DB.
    pub fn save_card(&self, card: &Card) -> Result<()> {
        self.conn.execute(
            "UPDATE cards SET due = ?1, interval_days = ?2, ease = ?3, reps = ?4, lapses = ?5, status = ?6 WHERE id = ?7",
            params![
                card.due.map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()),
                card.interval_days,
                card.ease,
                card.reps,
                card.lapses,
                status_str(card.status),
                card.id,
            ],
        )?;
        Ok(())
    }
}

fn parse_status(s: &str) -> ReviewStatus {
    match s {
        "Learning" => ReviewStatus::Learning,
        "Review" => ReviewStatus::Review,
        "Relearning" => ReviewStatus::Relearning,
        _ => ReviewStatus::New,
    }
}

fn status_str(s: ReviewStatus) -> &'static str {
    match s {
        ReviewStatus::New => "New",
        ReviewStatus::Learning => "Learning",
        ReviewStatus::Review => "Review",
        ReviewStatus::Relearning => "Relearning",
    }
}