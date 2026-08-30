//! SQLite persistence + range tables for fast lookup.

use crate::error::Result;
use crate::model::{Card, CardType, Difficulty, ReviewLogEntry, ReviewStatus, Wordlist};
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
    card_type TEXT NOT NULL DEFAULT 'word',
    difficulty TEXT NOT NULL DEFAULT 'easy',
    source TEXT NOT NULL DEFAULT '',
    phrase TEXT NOT NULL DEFAULT '',
    due TEXT,
    interval_days REAL NOT NULL DEFAULT 0,
    ease REAL NOT NULL DEFAULT 2.5,
    reps INTEGER NOT NULL DEFAULT 0,
    lapses INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'New',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY(word_id) REFERENCES words(id)
);

CREATE TABLE IF NOT EXISTS review_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    card_id INTEGER NOT NULL,
    reviewed_at TEXT NOT NULL,
    grade INTEGER NOT NULL,
    is_new INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS wordlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS wordlist_words (
    wordlist_id INTEGER NOT NULL,
    headword TEXT NOT NULL,
    PRIMARY KEY (wordlist_id, headword)
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS phrases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    phrase_type TEXT NOT NULL,
    text TEXT NOT NULL,
    def_en TEXT,
    def_zh TEXT,
    example_en TEXT,
    example_zh TEXT,
    UNIQUE(source, phrase_type, text)
);

CREATE INDEX IF NOT EXISTS idx_cards_due ON cards(due);
CREATE INDEX IF NOT EXISTS idx_entries_headword ON entries(headword);
CREATE INDEX IF NOT EXISTS idx_review_card ON review_log(card_id);
CREATE INDEX IF NOT EXISTS idx_review_date ON review_log(reviewed_at);
"#;

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Db> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        // Keep sort/temp working even where a temp file dir is blocked or slow:
        // RANDOM()-style whole-table sorts (random_word) run in memory. This is
        // also a net win for a small local dictionary.
        conn.execute_batch("PRAGMA temp_store = MEMORY")?;
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
        self.add_new_card_full(headword, CardType::Word, Difficulty::Easy, "", "")
    }

    /// Add a card with type/difficulty. For word cards `source`/`phrase`
    /// are empty; for phrase/pattern cards `phrase` is the display text.
    /// Idempotent per (word_id, card_type, phrase).
    pub fn add_new_card_full(
        &self,
        headword: &str,
        card_type: CardType,
        difficulty: Difficulty,
        source: &str,
        phrase: &str,
    ) -> Result<i64> {
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
                "SELECT id FROM cards WHERE word_id = ?1 AND card_type = ?2 AND phrase = ?3",
                params![word_id, card_type.as_str(), phrase],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO cards (word_id, card_type, difficulty, source, phrase) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![word_id, card_type.as_str(), difficulty.as_str(), source, phrase],
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
            "SELECT c.id, w.headword, c.due, c.interval_days, c.ease, c.reps, c.lapses, c.status,
                    c.card_type, c.difficulty, c.source, c.phrase, c.created_at
             FROM cards c JOIN words w ON w.id = c.word_id WHERE c.id = ?1",
        )?;
        let card = stmt
            .query_row(params![card_id], |r| {
                let due: Option<String> = r.get(2)?;
                let status: String = r.get(7)?;
                let ctype: String = r.get(8)?;
                let diff: String = r.get(9)?;
                let created: String = r.get(12)?;
                Ok(Card {
                    id: r.get(0)?,
                    headword: r.get(1)?,
                    sense: 0,
                    card_type: parse_card_type(&ctype),
                    difficulty: parse_difficulty(&diff),
                    source: r.get(10)?,
                    phrase: r.get(11)?,
                    due: due.and_then(|s| chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok()),
                    interval_days: r.get(3)?,
                    ease: r.get(4)?,
                    reps: r.get(5)?,
                    lapses: r.get(6)?,
                    status: parse_status(&status),
                    created_at: chrono::NaiveDateTime::parse_from_str(
                        &created,
                        "%Y-%m-%d %H:%M:%S",
                    )
                    .unwrap_or_else(|_| chrono::Local::now().naive_local()),
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

    /// Apply a grade to a card by id, persist it and log the review
    /// (with a new/review marker used by the daily limits).
    pub fn grade_card(&self, card_id: i64, grade: u8) -> Result<()> {
        if let Some(mut card) = self.load_card(card_id)? {
            let now = chrono::Local::now().naive_local();
            let is_new = card.status == ReviewStatus::New;
            crate::sm2::apply_review(&mut card, crate::sm2::Rating::new(grade, now));
            self.save_card(&card)?;
            self.conn.execute(
                "INSERT INTO review_log (card_id, reviewed_at, grade, is_new) VALUES (?1, ?2, ?3, ?4)",
                params![
                    card_id,
                    now.format("%Y-%m-%d %H:%M:%S").to_string(),
                    grade as i64,
                    if is_new { 1 } else { 0 },
                ],
            )?;
        }
        Ok(())
    }

    /// Full review history for one card (oldest first).
    pub fn card_history(&self, card_id: i64) -> Result<Vec<ReviewLogEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT reviewed_at, grade, is_new FROM review_log WHERE card_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![card_id], |r| {
            let ts: String = r.get(0)?;
            Ok(ReviewLogEntry {
                card_id,
                reviewed_at: chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%d %H:%M:%S")
                    .unwrap_or_else(|_| chrono::Local::now().naive_local()),
                grade: r.get(1)?,
                is_new: r.get::<_, i64>(2)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Reviews done TODAY (local): (new_done, review_done).
    pub fn today_progress(&self) -> Result<(u32, u32)> {
        let mut stmt = self.conn.prepare(
            "SELECT is_new, COUNT(*) FROM review_log
             WHERE date(reviewed_at) = date('now', 'localtime')
             GROUP BY is_new",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        let mut new_done = 0u32;
        let mut review_done = 0u32;
        for r in rows {
            let (is_new, n) = r?;
            if is_new != 0 {
                new_done = n as u32;
            } else {
                review_done = n as u32;
            }
        }
        Ok((new_done, review_done))
    }

    /// Daily limits from settings: (new_per_day, review_per_day).
    pub fn daily_limits(&self) -> Result<(u32, u32)> {
        let new_pd: u32 = self
            .get_setting("new_per_day")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let review_pd: u32 = self
            .get_setting("review_per_day")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);
        Ok((new_pd, review_pd))
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Set a card's difficulty flag (easy/hard), used to pick prompt style.
    pub fn set_card_difficulty(&self, card_id: i64, difficulty: Difficulty) -> Result<()> {
        self.conn.execute(
            "UPDATE cards SET difficulty = ?1 WHERE id = ?2",
            params![difficulty.as_str(), card_id],
        )?;
        Ok(())
    }

    /// Persist a reviewed card back to the DB.
    pub fn save_card(&self, card: &Card) -> Result<()> {
        self.conn.execute(
            "UPDATE cards SET due = ?1, interval_days = ?2, ease = ?3, reps = ?4, lapses = ?5, status = ?6,
                    card_type = ?8, difficulty = ?9 WHERE id = ?7",
            params![
                card.due.map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()),
                card.interval_days,
                card.ease,
                card.reps,
                card.lapses,
                status_str(card.status),
                card.id,
                card.card_type.as_str(),
                card.difficulty.as_str(),
            ],
        )?;
        Ok(())
    }

    // ---- wordlists ----

    /// Create a word list by name (idempotent), returning its id.
    pub fn create_wordlist(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO wordlists (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            params![name],
        )?;
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM wordlists WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )?)
    }

    /// Add headwords to a word list. Returns how many were new.
    pub fn add_wordlist_words(&self, wordlist_id: i64, headwords: &[String]) -> Result<usize> {
        let mut added = 0usize;
        for h in headwords {
            let n = self.conn.execute(
                "INSERT OR IGNORE INTO wordlist_words (wordlist_id, headword) VALUES (?1, ?2)",
                params![wordlist_id, h],
            )?;
            added += n;
        }
        Ok(added)
    }

    /// All word lists with their word counts.
    pub fn wordlists(&self) -> Result<Vec<Wordlist>> {
        let mut stmt = self.conn.prepare(
            "SELECT w.id, w.name,
                    (SELECT COUNT(*) FROM wordlist_words x WHERE x.wordlist_id = w.id)
             FROM wordlists w ORDER BY w.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Wordlist {
                id: r.get(0)?,
                name: r.get(1)?,
                size: r.get::<_, i64>(2)? as usize,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn delete_wordlist(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM wordlist_words WHERE wordlist_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM wordlists WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Due queue shaped by daily limits and optional wordlist scope.
    /// New cards (status New) are capped by new_per_day - new_done_today;
    /// due cards are capped by review_per_day - review_done_today.
    /// Returns (cards, new_left, review_left); each card is
    /// (card_id, headword, card_type, difficulty, phrase).
    pub fn daily_due_queue(
        &self,
        now: &chrono::NaiveDateTime,
        limit: i64,
        wordlist_id: Option<i64>,
    ) -> Result<(Vec<(i64, String, String, String, String)>, u32, u32)> {
        let (new_done, review_done) = self.today_progress()?;
        let (new_pd, review_pd) = self.daily_limits()?;
        let new_left = new_pd.saturating_sub(new_done);
        let review_left = review_pd.saturating_sub(review_done);

        let wl_clause = match wordlist_id {
            Some(0) | None => String::new(),
            Some(id) => format!(
                " AND w.headword IN (SELECT headword FROM wordlist_words WHERE wordlist_id = {id})"
            ),
        };

        // due cards first (oldest due first), then new cards.
        let mut out: Vec<(i64, String, String, String, String)> = Vec::new();

        let review_take = review_left.min(limit as u32);
        if review_take > 0 {
            let sql = format!(
                "SELECT c.id, w.headword, c.card_type, c.difficulty, c.phrase
                 FROM cards c JOIN words w ON w.id = c.word_id
                 WHERE c.due IS NOT NULL AND c.due <= ?1 AND c.status != 'New'{wl_clause}
                 ORDER BY c.due ASC LIMIT ?2"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![now.to_string(), review_take],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )?;
            for r in rows {
                out.push(r?);
            }
        }
        let new_take = (limit as u32 - out.len() as u32).min(new_left);
        if new_take > 0 {
            let sql = format!(
                "SELECT c.id, w.headword, c.card_type, c.difficulty, c.phrase
                 FROM cards c JOIN words w ON w.id = c.word_id
                 WHERE c.status = 'New'{wl_clause}
                 ORDER BY c.created_at ASC, c.id ASC LIMIT ?1"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![new_take], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok((out, new_left, review_left))
    }

    // ---- learning records ----

    /// Card list with per-card study stats for the records page.
    /// Returns (id, headword, card_type, difficulty, status, reps, lapses,
    /// due, last_review, created_at).
    pub fn card_records(&self, limit: i64) -> Result<Vec<(i64, String, String, String, String, i64, i64, Option<String>, Option<String>, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, w.headword, c.card_type, c.difficulty, c.status, c.reps, c.lapses,
                    c.due,
                    (SELECT MAX(reviewed_at) FROM review_log WHERE card_id = c.id),
                    c.created_at
             FROM cards c JOIN words w ON w.id = c.word_id
             ORDER BY c.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, String>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Pick a random word for "surprise me" mode.
    pub fn random_word(&self) -> Result<Option<String>> {
        Ok(self.conn
            .query_row(
                "SELECT headword FROM words WHERE headword IN (SELECT headword FROM entries)
                 ORDER BY RANDOM() LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    // ---- phrase bank ----

    /// Store an extracted phrasal verb / idiom / pattern.
    pub fn insert_phrase(
        &self,
        source: &str,
        phrase_type: &str,
        text: &str,
        def_en: &str,
        def_zh: &str,
        example_en: &str,
        example_zh: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO phrases (source, phrase_type, text, def_en, def_zh, example_en, example_zh)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![source, phrase_type, text, def_en, def_zh, example_en, example_zh],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Phrases extracted from one headword's definition.
    pub fn phrases_for(
        &self,
        source: &str,
    ) -> Result<Vec<(i64, String, String, String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, phrase_type, text, def_en, def_zh, example_en, example_zh
             FROM phrases WHERE source = ?1 ORDER BY phrase_type, text",
        )?;
        let rows = stmt.query_map(params![source], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn phrase_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM phrases", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Browse extracted phrases.
    pub fn all_phrases(&self, limit: i64, offset: i64) -> Result<Vec<(i64, String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source, phrase_type, text, def_zh FROM phrases
             ORDER BY source, text LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit, offset], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

}

fn parse_card_type(s: &str) -> CardType {
    match s {
        "phrase" => CardType::Phrase,
        "pattern" => CardType::Pattern,
        _ => CardType::Word,
    }
}

fn parse_difficulty(s: &str) -> Difficulty {
    match s {
        "hard" => Difficulty::Hard,
        _ => Difficulty::Easy,
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