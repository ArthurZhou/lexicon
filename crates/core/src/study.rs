//! Study-domain operations on top of storage: building the phrase bank and
//! parsing wordlist files. Pure logic — the caller does any file I/O.

use crate::definition::parse_definition;
use crate::model::{CardType, Difficulty};
use crate::storage::Db;
use crate::Result;

/// Extract every phrase/idiom from all entries into the phrase bank.
/// Idempotent: clears the bank and rebuilds it.
pub fn rebuild_phrase_bank(d: &Db) -> Result<usize> {
    d.conn().execute_batch("DELETE FROM phrases")?;
    let mut stmt = d.conn().prepare("SELECT headword, definition FROM entries")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut n = 0usize;
    for r in rows {
        let (head, def) = r?;
        let parsed = parse_definition(&head, &def);
        for p in parsed.phrases {
            d.insert_phrase(
                &head,
                &p.kind,
                &p.text,
                &p.def_en,
                &p.def_zh,
                &p.example_en,
                &p.example_zh,
            )?;
            n += 1;
        }
    }
    Ok(n)
}

/// One parsed wordlist line.
#[derive(Debug, Clone, PartialEq)]
pub struct WordlistLine {
    pub card_type: CardType,
    pub headword: String,
    /// For phrase/pattern cards: the phrase text itself (defaults to the
    /// headword when the line does not carry one).
    pub phrase: String,
}

/// Parse one wordlist line. Supported forms (empty lines and `#` comments
/// are skipped, returning None):
///   headword                       -> default type
///   headword<TAB>word|phrase|pattern
///   word|phrase|pattern:headword|phrase-text
///   headword|phrase-text
pub fn parse_wordlist_line(line: &str, default_type: &str) -> Option<WordlistLine> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    // type is either a trailing tab field or a leading "type:" prefix
    let (type_str, rest) = match line.split_once('\t') {
        Some((h, t)) if ["word", "phrase", "pattern"].contains(&t) => (t, h),
        _ => match line.split_once(':') {
            Some((t, r)) if ["word", "phrase", "pattern"].contains(&t) => (t, r),
            _ => (default_type, line),
        },
    };
    let (head, phrase) = match rest.split_once('|') {
        Some((h, p)) => (h.trim(), p.trim().to_string()),
        None => (rest.trim(), String::new()),
    };
    if head.is_empty() {
        return None;
    }
    let card_type = match type_str {
        "phrase" => CardType::Phrase,
        "pattern" => CardType::Pattern,
        _ => CardType::Word,
    };
    Some(WordlistLine {
        card_type,
        headword: head.to_string(),
        phrase: if phrase.is_empty() { head.to_string() } else { phrase },
    })
}

/// Default difficulty for freshly imported cards.
pub fn initial_difficulty() -> Difficulty {
    Difficulty::Easy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_headword() {
        let l = parse_wordlist_line("apple", "word").unwrap();
        assert_eq!(l.card_type, CardType::Word);
        assert_eq!(l.headword, "apple");
        assert_eq!(l.phrase, "apple");
    }

    #[test]
    fn parses_tab_type() {
        let l = parse_wordlist_line("break away\tphrase", "word").unwrap();
        assert_eq!(l.card_type, CardType::Phrase);
        assert_eq!(l.headword, "break away");
        assert_eq!(l.phrase, "break away");
    }

    #[test]
    fn parses_typed_line_with_phrase_text() {
        let l = parse_wordlist_line("pattern:break|break away from sb", "word").unwrap();
        assert_eq!(l.card_type, CardType::Pattern);
        assert_eq!(l.headword, "break");
        assert_eq!(l.phrase, "break away from sb");
    }

    #[test]
    fn skips_comments_and_empty() {
        assert!(parse_wordlist_line("", "word").is_none());
        assert!(parse_wordlist_line("# comment", "word").is_none());
        assert!(parse_wordlist_line("   ", "word").is_none());
    }
}
