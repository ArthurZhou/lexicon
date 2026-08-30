//! Parse MDX definition HTML into structured study content.
//!
//! The Oxford HTML uses these class hooks (verified against oxford.mdx):
//!   .phon-gb / .phon-us      IPA
//!   .pos                     part of speech
//!   .d + .chn                English sense + Chinese gloss
//!   .x + .tx                 example sentence + its Chinese translation
//!   .pv-g > .pv              phrasal verb head + block
//!   .id-g > .id              idiom head + block
//! This module extracts exactly the pieces the three study modes need:
//! word (zh->en with hints), phrase/collocation, and sentence pattern.

/// A phrasal verb or idiom extracted from a definition.
#[derive(Debug, Clone, Default)]
pub struct PhraseItem {
    /// "pv" (phrasal verb) or "id" (idiom) or "sd" (saying)
    pub kind: String,
    pub text: String,
    pub def_en: String,
    pub def_zh: String,
    pub example_en: String,
    pub example_zh: String,
}

/// Structured content for one headword.
#[derive(Debug, Clone, Default)]
pub struct Definition {
    pub headword: String,
    pub phonetic: String,
    pub pos: String,
    /// Chinese senses (from .chn)
    pub senses_zh: Vec<String>,
    /// (english example, chinese translation) pairs from .x/.tx
    pub examples: Vec<(String, String)>,
    /// phrasal verbs and idioms found in the definition
    pub phrases: Vec<PhraseItem>,
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    let mut out = s.to_string();
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
        ("&rsquo;", "'"),
        ("&lsquo;", "'"),
        ("&ldquo;", "\""),
        ("&rdquo;", "\""),
    ] {
        out = out.replace(e, c);
    }
    out.trim().to_string()
}

/// Extract the first text value for an element class.
fn class_text(html: &str, class: &str) -> Option<String> {
    let pat = format!("class=\"{}\"", class);
    let mut search_from = 0usize;
    while let Some(pos) = html[search_from..].find(&pat) {
        let abs = search_from + pos;
        let after = &html[abs + pat.len()..];
        if let Some(gt) = after.find('>') {
            let text_start = gt + 1;
            let text_end = after[text_start..].find('<').map(|e| text_start + e).unwrap_or(after.len());
            let t = strip_tags(&after[text_start..text_end]);
            if !t.is_empty() {
                return Some(t);
            }
        }
        search_from = abs + pat.len();
    }
    None
}

/// Collect every text value for an element class (e.g. all .chn glosses).
fn class_texts(html: &str, class: &str) -> Vec<String> {
    let pat = format!("class=\"{}\"", class);
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(pos) = html[search_from..].find(&pat) {
        let abs = search_from + pos;
        let after = &html[abs + pat.len()..];
        if let Some(gt) = after.find('>') {
            let text_start = gt + 1;
            let text_end = after[text_start..].find('<').map(|e| text_start + e).unwrap_or(after.len());
            let t = strip_tags(&after[text_start..text_end]);
            if !t.is_empty() {
                out.push(t);
            }
        }
        search_from = abs + pat.len();
    }
    out
}

/// Position of every class attribute occurrence.
fn find_all_class(html: &str, class: &str) -> Vec<usize> {
    let pat = format!("class=\"{}\"", class);
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(p) = html[from..].find(&pat) {
        let abs = from + p;
        out.push(abs);
        from = abs + pat.len();
    }
    out
}

/// Parse a definition into structured study content.
pub fn parse_definition(headword: &str, html: &str) -> Definition {
    let mut def = Definition {
        headword: headword.to_string(),
        ..Default::default()
    };
    def.phonetic = class_text(html, "phon-gb")
        .or_else(|| class_text(html, "phon"))
        .unwrap_or_default();
    def.pos = class_text(html, "pos").unwrap_or_default();
    def.senses_zh = class_texts(html, "chn");
    if def.senses_zh.is_empty() {
        if let Some(first) = class_text(html, "d") {
            def.senses_zh.push(first);
        }
    }
    def.examples = extract_examples(html);

    for kind in ["pv", "id", "sd"] {
        extract_phrase_blocks(html, kind, &mut def.phrases);
    }
    def
}

fn extract_examples(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while from < html.len() {
        let Some(rel) = html[from..].find("class=\"x-g\"") else { break };
        let start = from + rel;
        let rest = &html[start + 10..];
        let next = rest.find("class=\"x-g\"").map(|p| start + 10 + p).unwrap_or(html.len());
        let block = &html[start..next];
        let en = class_text(block, "x").unwrap_or_default();
        let zh = class_text(block, "tx").unwrap_or_default();
        if !en.is_empty() {
            out.push((en, zh));
        }
        from = next;
        if out.len() >= 12 {
            break;
        }
    }
    out
}

/// Extract .pv / .id / .sd heads together with their definitions.
fn extract_phrase_blocks(html: &str, kind: &str, out: &mut Vec<PhraseItem>) {
    let container = format!("{kind}-g");
    let mut from = 0usize;
    while from < html.len() {
        let Some(rel) = html[from..].find(&format!("class=\"{}\"", container)) else { break };
        let start = from + rel;
        let rest = &html[start + container.len() + 7..];
        let next = rest.find(&format!("class=\"{}\"", container)).map(|p| start + container.len() + 7 + p).unwrap_or(html.len());
        let block = &html[start..next.max(start)];
        let head = class_text(block, kind).unwrap_or_default();
        if head.is_empty() {
            from = next;
            continue;
        }
        let def_en = class_text(block, "d").unwrap_or_default();
        let def_zh = class_text(block, "chn").unwrap_or_default();
        let example_en = class_text(block, "x").unwrap_or_default();
        let example_zh = class_text(block, "tx").unwrap_or_default();
        out.push(PhraseItem {
            kind: kind.to_string(),
            text: head,
            def_en,
            def_zh,
            example_en,
            example_zh,
        });
        from = next;
    }
}

/// True when an entry has phrasal verbs / idioms worth exposing.
pub fn has_phrases(html: &str) -> bool {
    html.contains("class=\"pv-g\"") || html.contains("class=\"id-g\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apple_definition() {
        let html = r#"<link rel="stylesheet" href="oalecd8e.css"><span class="entry"><span class="h">apple</span><span class="phon-gb">ˈæpl</span><span class="pos">noun</span><span class="def-g"><span class="d">a round fruit <span class="chn">苹果</span></span></span><span id="apple_xg_1" class="x-g"><span class="x">I ate an apple.</span><span class="tx">我吃了个苹果。</span></span></span>"#;
        let d = parse_definition("apple", html);
        assert_eq!(d.phonetic, "ˈæpl");
        assert_eq!(d.pos, "noun");
        assert!(d.senses_zh.iter().any(|s| s.contains("苹果")));
        assert!(d.examples.iter().any(|(en, _)| en.contains("apple")));
    }

    #[test]
    fn extracts_phrasal_verbs() {
        let html = r#"<span class="h">break</span><span class="pv-g"><span class="pv">ˌbreak aˈway</span><span class="def-g"><span class="d">to escape <span class="chn">挣脱</span></span></span><span class="x-g"><span class="x">The prisoner broke away.</span><span class="tx">囚犯挣脱了。</span></span></span><span class="pv-g"><span class="pv">break ˈdown</span><span class="def-g"><span class="d">stop working <span class="chn">坏掉</span></span></span></span>"#;
        let mut items = Vec::new();
        extract_phrase_blocks(html, "pv", &mut items);
        assert!(items.len() >= 2);
        assert!(items[0].text.contains("break"));
        assert_eq!(items[0].def_zh, "挣脱");
    }

    #[test]
    fn entity_decode() {
        assert_eq!(decode_entities("a &apos;quote&apos; &amp; more"), "a 'quote' & more");
    }
}
