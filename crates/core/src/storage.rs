//! SQLite persistence + range tables for fast lookup.

use crate::error::Result;
use rusqlite::{params, Connection};
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
}
