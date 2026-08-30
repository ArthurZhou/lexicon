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

    /// Run `f` inside a single transaction. All inserts/updates made by `f`
    /// are committed together, which is dramatically faster than per-row
    /// autocommit for bulk imports.
    pub fn transaction<T, F>(&mut self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Db) -> Result<T>,
    {
        self.conn.execute_batch("BEGIN")?;
        let r = f(self);
        match r {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Insert (or update) a raw entry. The definition is stored as-is (HTML).
    pub fn insert_entry(&self, headword: &str, definition: &str) -> Result<()> {
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

    /// Look up a word following @@@LINK= aliases (MDX redirects such as
    /// "10p" -> "ten pence"). Returns the resolved definition, or None when
    /// neither the word nor its aliases exist. Guarded against cycles.
    pub fn lookup_resolved(&self, word: &str) -> Result<Option<String>> {
        let mut current = word.trim().to_string();
        let mut hops = 0usize;
        loop {
            let def = match self.lookup(&current)? {
                Some(d) => d,
                None => return Ok(None),
            };
            let target = link_target(&def);
            match target {
                Some(t) if hops < 8 && !t.trim().eq_ignore_ascii_case(word) => {
                    current = t.trim().to_string();
                    hops += 1;
                }
                // unevaluated fallback branch kept short for clarity
                _ => return Ok(Some(def)),
            }
        }
    }

    /// Raw entry lookup (no alias resolution). Used by CLI import.
    pub fn raw_lookup(&self, word: &str) -> Result<Option<String>> {
        self.lookup(word)
    }

    pub fn stats(&self) -> Result<(usize, usize)> {
        let words: usize = self.conn.query_row("SELECT COUNT(*) FROM words", [], |r| r.get(0))?;
        let cards: usize = self.conn.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0))?;
        Ok((words, cards))
    }

    /// Number of cards due right now (due <= now or never scheduled).
    pub fn due_count(&self, now: &chrono::NaiveDateTime) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cards WHERE due IS NULL OR due <= ?1",
            params![now.to_string()],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Prefix search over known headwords. Returns (headword, definition).
    /// Definitions can be NULL for words without an imported entry.
    pub fn search(&self, prefix: &str, limit: i64) -> Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT w.headword, e.definition
             FROM words w LEFT JOIN entries e ON e.headword = w.headword
             WHERE w.headword LIKE ?1
             ORDER BY w.headword ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![format!("{prefix}%"), limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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

    /// List due cards restricted to a headword prefix range [start, end).
    /// Use this to constrain a review session to a section of the wordlist.
    pub fn due_cards_in_range(
        &self,
        now: &chrono::NaiveDateTime,
        start: &str,
        end: &str,
        limit: i64,
    ) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, w.headword FROM cards c JOIN words w ON w.id = c.word_id
             WHERE (c.due IS NULL OR c.due <= ?1)
               AND w.headword >= ?2 AND w.headword < ?3
             ORDER BY w.headword, c.due IS NULL DESC, c.due ASC LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![now.to_string(), start, end, limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
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

    /// Due cards with their definitions, ready for a front-end.
    pub fn due_card_details(
        &self,
        now: &chrono::NaiveDateTime,
        range: Option<(&str, &str)>,
        limit: i64,
    ) -> Result<Vec<(i64, String, String)>> {
        let sql = match range {
            Some(_) => {
                "SELECT c.id, w.headword, e.definition FROM cards c
                 JOIN words w ON w.id = c.word_id
                 LEFT JOIN entries e ON e.headword = w.headword
                 WHERE (c.due IS NULL OR c.due <= ?1)
                   AND w.headword >= ?2 AND w.headword < ?3
                 ORDER BY c.due IS NULL DESC, c.due ASC LIMIT ?4"
            }
            None => {
                "SELECT c.id, w.headword, e.definition FROM cards c
                 JOIN words w ON w.id = c.word_id
                 LEFT JOIN entries e ON e.headword = w.headword
                 WHERE (c.due IS NULL OR c.due <= ?1)
                 ORDER BY c.due IS NULL DESC, c.due ASC LIMIT ?2"
            }
        };
        let mut stmt = self.conn.prepare(sql)?;
        let map = |r: &rusqlite::Row| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        };
        let rows = match range {
            Some((s, e)) => stmt.query_map(params![now.to_string(), s, e, limit], map)?,
            None => stmt.query_map(params![now.to_string(), limit], map)?,
        };
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Apply a grade to a card by id and persist it.
    pub fn grade_card(&self, card_id: i64, grade: u8) -> Result<()> {
        if let Some(mut card) = self.load_card(card_id)? {
            let now = chrono::Local::now().naive_local();
            crate::sm2::apply_review(&mut card, crate::sm2::Rating::new(grade, now));
            self.save_card(&card)?;
        }
        Ok(())
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

/// If the definition is an MDX redirect ("@@@LINK=target"), return Some(target).
/// MDX records often carry a trailing NUL byte ("@@@LINK=ten pence\r\n\0"),
/// so we strip NULs and whitespace before returning the target.
fn link_target(def: &str) -> Option<String> {
    const PREFIX: &str = "@@@LINK=";
    let s = def.trim_start();
    if s.len() > PREFIX.len() && s[..PREFIX.len()] == *PREFIX {
        let t = s[PREFIX.len()..].trim_end_matches('\0').trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}

fn status_str(s: ReviewStatus) -> &'static str {
    match s {
        ReviewStatus::New => "New",
        ReviewStatus::Learning => "Learning",
        ReviewStatus::Review => "Review",
        ReviewStatus::Relearning => "Relearning",
    }
}