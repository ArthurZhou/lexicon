//! Card prompt building: turns a stored card + dictionary entry into a
//! graded study prompt. Pure logic — no I/O beyond dictionary reads.
//!
//! Prompt tiers are matched to how familiar the learner is with the card
//! (auto-adjusted by SM2 grades, see `sm2::apply_review`):
//!   word  easy:   挖空例句 + 词义（提示里给例句中文翻译）
//!   word  medium: 挖空例句（不给词义，提示只给例句中文翻译）
//!   word  hard:   首字母 + 字母数 + 词义，不给例句
//!   phrase/pattern easy:   中文释义 + 挖空例句
//!   phrase/pattern medium: 中文释义 + 例句中文翻译（不给英文例句）
//!   phrase/pattern hard:   词组首字母骨架 + 中文释义
//! Falls back to the hard style whenever the entry has no usable example.

use crate::definition::parse_definition;
use crate::storage::Db;

/// Resolve an @@@LINK= alias to its real definition, falling back to the
/// stored definition when the target is missing (defensive).
pub fn resolve_def(d: &Db, headword: &str, def: &str) -> String {
    match d.lookup_resolved(headword) {
        Ok(Some(text)) => text,
        _ => def.to_string(),
    }
}

/// Build the review prompt for one card.
/// Returns a JSON object: {kind, question, hint, answer, extra}.
pub fn build_card_prompt(
    d: &Db,
    _card_id: i64,
    headword: &str,
    ctype: &str,
    diff: &str,
    phrase: &str,
) -> serde_json::Value {
    let def = d.lookup_resolved(headword).ok().flatten().unwrap_or_default();
    let parsed = parse_definition(headword, &def);

    match ctype {
        "phrase" | "pattern" => {
            let want_pattern = ctype == "pattern";
            // Fixed phrase / collocation: prefer the phrase bank (already
            // extracted, deduped) so the gloss matches THIS phrase, falling
            // back to whatever the headword definition parses.
            let bank = d.phrases_for(headword).unwrap_or_default();
            let (zh, ex_en, ex_zh) = bank
                .iter()
                .find(|(_, pt, text, ..)| {
                    norm_phrase(text) == norm_phrase(phrase)
                        && (!want_pattern || pt == "id" || pt == "sd")
                })
                .map(|(_, _, _, de, dz, xe, xz)| {
                    (if dz.is_empty() { de.clone() } else { dz.clone() }, xe.clone(), xz.clone())
                })
                .or_else(|| {
                    parsed.phrases.iter().find(|p| p.text.contains(phrase)).map(|p| {
                        (
                            if p.def_zh.is_empty() { p.def_en.clone() } else { p.def_zh.clone() },
                            p.example_en.clone(),
                            p.example_zh.clone(),
                        )
                    })
                })
                .unwrap_or_default();
            let zh = if zh.is_empty() {
                // bank lookup failed: use the source word's first sense
                // instead of dumping the whole entry
                parsed.senses_zh.first().cloned().unwrap_or_default()
            } else {
                zh
            };
            let masked = if ex_en.is_empty() { String::new() } else { cloze_example(&ex_en, phrase) };
            let question = match diff {
                "hard" => format!("{}\n{}", initials_mask(phrase), zh),
                "medium" => {
                    if masked.is_empty() {
                        format!("{}\n{}", initials_mask(phrase), zh)
                    } else {
                        zh.clone()
                    }
                }
                _ => {
                    if masked.is_empty() {
                        format!("{}\n{}", initials_mask(phrase), zh)
                    } else {
                        format!("{zh}\n{masked}")
                    }
                }
            };
            let hint = if diff == "hard" { String::new() } else { ex_zh.clone() };
            serde_json::json!({
                "kind": ctype,
                "question": question,
                "hint": hint,
                "answer": phrase,
                "extra": {
                    "source": headword,
                    "example": ex_en,
                    "example_zh": ex_zh,
                }
            })
        }
        _ => {
            // Word: Chinese->English, tiered hints.
            let zh = if parsed.senses_zh.is_empty() {
                plainify(&def).lines().next().unwrap_or_default().to_string()
            } else {
                parsed.senses_zh[..parsed.senses_zh.len().min(2)].join("；")
            };
            let (ex_en, ex_zh) = parsed
                .examples
                .first()
                .map(|(e, z)| (e.clone(), z.clone()))
                .unwrap_or_default();
            let masked =
                if ex_en.is_empty() { String::new() } else { cloze_example(&ex_en, headword) };
            // Hard style (also the fallback when no example is available):
            // first letter + letter count + Chinese meaning, no example.
            let hard_question = format!("{}  {}", first_letter_mask(headword), zh);
            let (question, hint) = match diff {
                "hard" => (hard_question, String::new()),
                "medium" => {
                    if masked.is_empty() {
                        (hard_question, String::new())
                    } else {
                        (masked.clone(), format!("例句翻译：{ex_zh}"))
                    }
                }
                _ => {
                    if masked.is_empty() {
                        (hard_question, String::new())
                    } else {
                        (masked, format!("词义：{zh}\n例句翻译：{ex_zh}"))
                    }
                }
            };
            serde_json::json!({
                "kind": "word",
                "question": question,
                "hint": hint,
                "answer": headword,
                "extra": {
                    "pos": parsed.pos,
                    "phonetic": parsed.phonetic,
                    "senses_zh": parsed.senses_zh,
                    "example": ex_en,
                    "example_zh": ex_zh,
                    "difficulty": diff,
                }
            })
        }
    }
}

/// Replace occurrences of `target` (a headword or phrase; and simple
/// inflections: plural, -ed/-ing/-'s, irregular verb forms) in `sentence`
/// with "____". Returns "" when the target does not appear in the sentence
/// (so callers can fall back to a no-example prompt style instead of
/// showing the answer).
pub fn cloze_example(sentence: &str, target: &str) -> String {
    let t = norm_phrase(target);
    if t.is_empty() {
        return String::new();
    }
    if t.contains(' ') {
        cloze_multi(sentence, &t)
    } else {
        cloze_single(sentence, &t)
    }
}

/// Cloze for a single-word target.
fn cloze_single(sentence: &str, hw: &str) -> String {
    let mut out = String::with_capacity(sentence.len() + 8);
    let mut word = String::new();
    let mut replaced = false;
    for c in sentence.chars() {
        if c.is_ascii_alphabetic() || c == '\'' || c == '-' {
            word.push(c);
        } else {
            if !word.is_empty() {
                if masks_headword(&word, hw) {
                    out.push_str("____");
                    replaced = true;
                } else {
                    out.push_str(&word);
                }
                word.clear();
            }
            out.push(c);
        }
    }
    if !word.is_empty() {
        if masks_headword(&word, hw) {
            out.push_str("____");
            replaced = true;
        } else {
            out.push_str(&word);
        }
    }
    if replaced { out } else { String::new() }
}

/// Cloze for a multi-word phrase target: mask each word that belongs to the
/// phrase (in order); wildcard parts like "sb"/"sth" are left visible.
fn cloze_multi(sentence: &str, phrase_norm: &str) -> String {
    let parts: Vec<&str> = phrase_norm.split_whitespace().collect();
    if parts.is_empty() {
        return sentence.to_string();
    }
    let mut want = 0usize; // index into parts we are looking for
    let mut out = String::with_capacity(sentence.len() + 16);
    let mut word = String::new();
    let mut replaced = false;
    for c in sentence.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphabetic() || c == '\'' || c == '-' {
            word.push(c);
        } else {
            if !word.is_empty() {
                let core = word
                    .trim_matches(|c: char| !c.is_ascii_alphabetic())
                    .trim_end_matches("'s")
                    .to_lowercase();
                if want < parts.len() && word_matches_part(&core, parts[want]) {
                    out.push_str("____");
                    want += 1;
                    replaced = true;
                } else {
                    out.push_str(&word);
                }
                word.clear();
            }
            out.push(c);
        }
    }
    // only keep the mask when we matched the FIRST word of the phrase
    // (otherwise we'd blank random words); require at least 2 matches.
    if replaced && want >= 2 {
        out.trim_end().to_string()
    } else {
        String::new()
    }
}

/// True when the lowercase word core matches the next phrase part (allowing
/// light inflection and wildcard parts like "sb", "sth", "oneself").
fn word_matches_part(core: &str, part: &str) -> bool {
    let part = part.trim_matches(|c: char| !c.is_ascii_alphabetic());
    if part.is_empty() {
        return false;
    }
    if ["sb", "sth", "oneself", "yourself"].contains(&part) {
        return false; // never blank these out
    }
    core == part
        || stems(core).contains(&part.to_string())
        || irregular_base(core) == Some(part)
}

/// True when `word` is the headword (case-insensitive) or a light inflection
/// of it (trailing 's, plural -s/-es, -ed/-ing with doubling, -er, -ly, or
/// a common irregular verb form like "broke" for "break").
fn masks_headword(word: &str, hw: &str) -> bool {
    let core = word
        .trim_matches(|c: char| !c.is_ascii_alphabetic())
        .trim_end_matches("'s")
        .to_lowercase();
    if core == hw {
        return true;
    }
    if stems(&core).iter().any(|s| s == hw) {
        return true;
    }
    irregular_base(&core) == Some(hw)
}

/// Common irregular verb/past forms so the cloze can still mask
/// "broke" for "break", "ran" for "run", etc.
fn irregular_base(word: &str) -> Option<&'static str> {
    const FORMS: &[(&str, &str)] = &[
        ("was", "be"), ("were", "be"), ("been", "be"),
        ("began", "begin"), ("begun", "begin"),
        ("bent", "bend"),
        ("bit", "bite"), ("bitten", "bite"),
        ("blew", "blow"), ("blown", "blow"),
        ("broke", "break"), ("broken", "break"),
        ("brought", "bring"),
        ("built", "build"),
        ("bought", "buy"),
        ("caught", "catch"),
        ("chose", "choose"), ("chosen", "choose"),
        ("came", "come"),
        ("cost", "cost"),
        ("cut", "cut"),
        ("dealt", "deal"),
        ("did", "do"), ("done", "do"),
        ("drew", "draw"), ("drawn", "draw"),
        ("drank", "drink"), ("drunk", "drink"),
        ("drove", "drive"), ("driven", "drive"),
        ("ate", "eat"), ("eaten", "eat"),
        ("fell", "fall"), ("fallen", "fall"),
        ("fed", "feed"),
        ("felt", "feel"),
        ("fought", "fight"),
        ("found", "find"),
        ("flew", "fly"), ("flown", "fly"),
        ("forgot", "forget"), ("forgotten", "forget"),
        ("froze", "freeze"), ("frozen", "freeze"),
        ("got", "get"), ("gotten", "get"),
        ("gave", "give"), ("given", "give"),
        ("grew", "grow"), ("grown", "grow"),
        ("had", "have"),
        ("heard", "hear"),
        ("held", "hold"),
        ("hid", "hide"), ("hidden", "hide"),
        ("kept", "keep"),
        ("knew", "know"), ("known", "know"),
        ("laid", "lay"),
        ("led", "lead"),
        ("left", "leave"),
        ("lent", "lend"),
        ("lost", "lose"),
        ("made", "make"),
        ("meant", "mean"),
        ("met", "meet"),
        ("paid", "pay"),
        ("put", "put"),
        ("read", "read"),
        ("rang", "ring"), ("rung", "ring"),
        ("ran", "run"),
        ("rode", "ride"), ("ridden", "ride"),
        ("rose", "rise"), ("risen", "rise"),
        ("said", "say"),
        ("saw", "see"), ("seen", "see"),
        ("sold", "sell"),
        ("sent", "send"),
        ("shook", "shake"), ("shaken", "shake"),
        ("shot", "shoot"),
        ("sang", "sing"), ("sung", "sing"),
        ("sank", "sink"), ("sunk", "sink"),
        ("sat", "sit"),
        ("slept", "sleep"),
        ("spoke", "speak"), ("spoken", "speak"),
        ("spent", "spend"),
        ("stood", "stand"),
        ("stuck", "stick"),
        ("struck", "strike"),
        ("swam", "swim"), ("swum", "swim"),
        ("took", "take"), ("taken", "take"),
        ("taught", "teach"),
        ("told", "tell"),
        ("thought", "think"),
        ("threw", "throw"), ("thrown", "throw"),
        ("understood", "understand"),
        ("woke", "wake"), ("woken", "wake"),
        ("wore", "wear"), ("worn", "wear"),
        ("won", "win"),
        ("wrote", "write"), ("written", "write"),
    ];
    FORMS.iter().find(|(w, _)| *w == word).map(|(_, base)| *base)
}

/// Candidate base forms of an inflected word: strips common suffixes and
/// collapses doubled consonants ("running" -> ["running", "runn", "run"]).
fn stems(word: &str) -> Vec<String> {
    let mut out = vec![word.to_string()];
    let push = |w: String, out: &mut Vec<String>| {
        if !w.is_empty() && !out.contains(&w) {
            out.push(w);
        }
    };
    for suf in ["ies", "es", "s", "ed", "ing", "er", "est", "ly"] {
        if let Some(stem) = word.strip_suffix(suf) {
            if suf == "ies" {
                push(format!("{stem}y"), &mut out);
            }
            // collapse doubled final consonant: running -> runn -> run
            let chars: Vec<char> = stem.chars().collect();
            if chars.len() >= 2 && chars[chars.len() - 1] == chars[chars.len() - 2] {
                push(chars[..chars.len() - 1].iter().collect(), &mut out);
            }
            push(stem.to_string(), &mut out);
        }
    }
    out
}

/// "break away" -> "b---- a---" — first letters + letter counts, so the
/// learner recalls the whole fixed phrase from its skeleton.
pub fn initials_mask(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .map(|w| {
            let letters: Vec<char> =
                w.chars().filter(|c| c.is_ascii_alphabetic() || *c == '-').collect();
            if letters.is_empty() {
                return w.to_string();
            }
            let n = letters.len();
            let first = letters[0].to_ascii_uppercase();
            if n <= 1 {
                first.to_string()
            } else {
                format!("{first}{}", "-".repeat(n - 1))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "apple" -> "A----（5 个字母）"
pub fn first_letter_mask(headword: &str) -> String {
    let letters: Vec<char> = headword.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let n = letters.len();
    if n == 0 {
        return headword.to_string();
    }
    format!("{}{}（{}个字母）", letters[0].to_ascii_uppercase(), "-".repeat(n - 1), n)
}

/// Normalize a phrase text for matching: drop stress/separator glyphs
/// (ˈ ˌ ↔ | / ) and trim, so "give sb a ˈbreak" matches "give sb a break".
pub fn norm_phrase(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{02C8}' | '\u{02CC}' | '\u{2194}' | '\u{007C}' | '\u{002F}'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Convert an MDX definition (HTML with markup, styles and media links)
/// into readable plain text. This is what kills the "raw source code" look:
/// scripts, styles, images, audio and sound:// refs are dropped, block
/// boundaries become line breaks, entities are decoded.
pub fn plainify(html: &str) -> String {
    let mut s = html.to_string();

    // drop whole style/script/audio blocks
    for pat in [
        r"(?is)<style\b[^>]*>.*?</style>",
        r"(?is)<script\b[^>]*>.*?</script>",
        r"(?is)<audio\b[^>]*>.*?</audio>",
    ] {
        let re = regex::Regex::new(pat).unwrap();
        s = re.replace_all(&s, "").to_string();
    }
    // drop self-closing / void tags we never show
    let re_void = regex::Regex::new(
        r"(?is)<(?:link|img|source|meta|input|area|base|track|iframe|object|embed)\b[^>]*/?>",
    )
    .unwrap();
    s = re_void.replace_all(&s, "").to_string();

    // <br> becomes a newline
    let re_br = regex::Regex::new(r"(?i)<br\s*/?>").unwrap();
    s = re_br.replace_all(&s, "\n").to_string();

    // block containers become newlines
    let re_close = regex::Regex::new(
        r"(?i)</(?:div|p|li|tr|h[1-6]|section|ul|ol|table|dl|dt|dd)>",
    )
    .unwrap();
    s = re_close.replace_all(&s, "\n").to_string();

    // common dictionary "groups" (example, definition, phonetic, part-of-speech) start a new line
    let re_group = regex::Regex::new(
        r#"(?i)<span[^>]*class="[^"]*(?:x-g|def-g|pos-g|top-g|block-g|d-g|ei-g)[^"]*">"#,
    )
    .unwrap();
    s = re_group.replace_all(&s, "\n").to_string();

    // strip any remaining tags
    let re_tag = regex::Regex::new(r"<[^>]*>").unwrap();
    s = re_tag.replace_all(&s, "").to_string();

    // decode common entities
    for (e, c) in [
        ("&nbsp;", " "),
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&hellip;", "…"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
    ] {
        s = s.replace(e, c);
    }

    // drop stray NUL bytes carried over from MDX records
    s = s.replace('\0', "");

    // tidy: trim lines, drop empties
    let mut lines: Vec<String> = s
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    // keep at most 60 lines so a huge entry doesn't flood the screen
    if lines.len() > 60 {
        lines.truncate(60);
        lines.push("…".into());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloze_replaces_headword() {
        assert_eq!(cloze_example("I ate an apple.", "apple"), "I ate an ____.");
        assert_eq!(cloze_example("She has two apples.", "apple"), "She has two ____.");
        assert_eq!(cloze_example("He was running.", "run"), "He was ____.");
    }

    #[test]
    fn cloze_leaves_unrelated_words() {
        // no replacement -> empty string so the caller falls back to the
        // no-example prompt style
        assert_eq!(cloze_example("A happy day.", "sad"), "");
    }

    #[test]
    fn cloze_handles_irregular_forms() {
        assert_eq!(cloze_example("All the windows broke.", "break"), "All the windows ____.");
        assert_eq!(cloze_example("She ran to school.", "run"), "She ____ to school.");
    }

    #[test]
    fn cloze_multi_word_phrase() {
        let out = cloze_example("They decided to break away from the group.", "break away");
        assert_eq!(out, "They decided to ____ ____ from the group.");
    }

    #[test]
    fn initials() {
        assert_eq!(initials_mask("break away"), "B---- A---");
        assert_eq!(initials_mask("a"), "A");
    }

    #[test]
    fn first_letter() {
        assert_eq!(first_letter_mask("apple"), "A----（5个字母）");
    }

    #[test]
    fn norm_ignores_stress() {
        assert_eq!(norm_phrase("give sb a ˈbreak"), norm_phrase("give sb a break"));
    }

    #[test]
    fn plainify_strips_markup() {
        assert_eq!(plainify("<div>a<br>b</div>"), "a\nb");
        assert_eq!(plainify("x&nbsp;y"), "x y");
    }
}
