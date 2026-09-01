//! Parse MDX definition HTML into structured study content.
//!
//! Two Oxford layouts are supported:
//!
//! 1. oalecd9 (tag-based, verified against the 9th-edition mdx):
//!    `<phon>breɪk</phon>` IPA · `<pos>verb</pos>` part of speech ·
//!    `<def>english <chn>中文</chn></def>` sense + Chinese gloss ·
//!    `<x>example <chn>翻译</chn></x>` example + translation ·
//!    `<pv-g><pv>ˌbreak aˈway</pv>…</pv-g>` phrasal verb block ·
//!    `<idm-g><idm>…</idm>…</idm-g>` idiom block
//!
//! 2. oalecd8 (class-based, kept as fallback):
//!    `.phon-gb/.phon-us, .pos, .d + .chn, .x + .tx, .pv-g > .pv, .id-g > .id`
//!
//! This module extracts exactly the pieces the study modes need:
//! word (zh->en with tiered hints), phrase/collocation, and sentence pattern.

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

/// Parse a definition into structured study content.
pub fn parse_definition(headword: &str, html: &str) -> Definition {
    let mut def = Definition {
        headword: headword.to_string(),
        ..Default::default()
    };

    // ---- tag-based layout (oalecd9) ----
    def.phonetic = tag_blocks(html, "phon")
        .first()
        .map(|s| strip_tags(s))
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    def.pos = tag_blocks(html, "pos")
        .first()
        .map(|s| strip_tags(s))
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    // phrasal verbs / idioms: kind "pv" and "id" (idioms -> "id" so the
    // pattern prompt can prefer them)
    for (tag, kind) in [("pv-g", "pv"), ("idm-g", "id")] {
        for block in tag_blocks(html, tag) {
            let Some(text) = tag_blocks(&block, if tag == "pv-g" { "pv" } else { "idm" })
                .first()
                .map(|s| strip_tags(s).trim().to_string())
            else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let def_el = tag_blocks(&block, "def").first().cloned().unwrap_or_default();
            let def_zh = tag_blocks(&def_el, "chn")
                .first()
                .map(|s| strip_tags(s))
                .unwrap_or_default();
            let def_en = {
                let mut e = def_el.clone();
                if let Some(c) = tag_blocks(&def_el, "chn").first() {
                    e = e.replace(c.as_str(), "");
                }
                strip_tags(&e)
            };
            // example inside the phrase block: <x>english<chn>翻译</chn></x>
            let (example_en, example_zh) = tag_blocks(&block, "x")
                .first()
                .map(|x| split_example(x))
                .unwrap_or_default();
            def.phrases.push(PhraseItem {
                kind: kind.to_string(),
                text,
                def_en: def_en.trim().trim_start_matches(';').trim().to_string(),
                def_zh,
                example_en,
                example_zh,
            });
        }
    }

    // senses + examples: only from the head part, before the phrase blocks,
    // so phrase senses don't pollute the word prompt
    let head_end = ["<pv-g", "<idm-g", "<pv-gs-blk", "<idm-gs-blk"]
        .iter()
        .filter_map(|m| html.find(m))
        .min()
        .unwrap_or(html.len());
    let head = &html[..head_end];
    for d in tag_blocks(head, "def") {
        if let Some(zh) = tag_blocks(&d, "chn").first() {
            let zh = strip_tags(zh);
            if !zh.is_empty() {
                def.senses_zh.push(zh);
            }
        }
    }
    for x in tag_blocks(head, "x").into_iter().take(12) {
        let (en, zh) = split_example(&x);
        if !en.is_empty() {
            def.examples.push((en, zh));
        }
    }

    // ---- class-based fallback (oalecd8) ----
    if def.phonetic.is_empty() {
        def.phonetic = class_text(html, "phon-gb")
            .or_else(|| class_text(html, "phon"))
            .unwrap_or_default();
    }
    if def.pos.is_empty() {
        def.pos = class_text(html, "pos").unwrap_or_default();
    }
    if def.senses_zh.is_empty() {
        def.senses_zh = class_texts(html, "chn");
        if def.senses_zh.is_empty() {
            if let Some(first) = class_text(html, "d") {
                def.senses_zh.push(first);
            }
        }
    }
    if def.examples.is_empty() {
        def.examples = extract_examples(html);
    }
    if def.phrases.is_empty() {
        for kind in ["pv", "id", "sd"] {
            extract_phrase_blocks(html, kind, &mut def.phrases);
        }
    }
    def
}

/// Split `<x>` content into (english, chinese translation): the English part
/// is everything minus the inner `<chn>…</chn>`, the Chinese part is that
/// inner content.
fn split_example(x_html: &str) -> (String, String) {
    let zh = tag_blocks(x_html, "chn")
        .first()
        .map(|s| strip_tags(s))
        .unwrap_or_default();
    let mut en = x_html.to_string();
    if let Some(c) = tag_blocks(x_html, "chn").first() {
        let full = format!("<chn>{c}</chn>");
        en = en.replace(&full, "");
    }
    (strip_tags(&en).trim().to_string(), zh)
}

/// Contents of every `<tag …>…</tag>` block (non-greedy, no nested same-tag
/// handling — sufficient for these flat dictionary elements).
fn tag_blocks(html: &str, tag: &str) -> Vec<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let is_word_boundary = |c: u8| c == b'>' || c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = html[from..].find(&open) {
        let start = from + rel;
        // avoid matching prefixes like <xhtml: for tag "x"
        match html.as_bytes().get(start + open.len()) {
            Some(&c) if is_word_boundary(c) => {}
            _ => {
                from = start + open.len();
                continue;
            }
        }
        let Some(close_rel) = html[start..].find(&close) else { break };
        let content_start = match html[start..].find('>') {
            Some(g) => start + g + 1,
            None => {
                from = start + open.len();
                continue;
            }
        };
        let content_end = start + close_rel;
        if content_end > content_start {
            out.push(html[content_start..content_end].to_string());
        }
        from = content_end + close.len();
    }
    out
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

    #[test]
    fn parses_oalecd9_tag_layout() {
        let html = concat!(
            r#"<top-g><h>break</h><pron-g><phon-blk>/<phon>breɪk</phon>/</phon-blk></pron-g>"#,
            r#"<pos-g><pos-blk><pos>verb</pos></pos-blk></pos-g></top-g>"#,
            r#"<sn-gs><sn-g><def>to be damaged<chnsep> </chnsep><chn>（使）破，裂，碎</chn></def>"#,
            r#"<x-gs><x-g-blk><x>to <xhtml:a href="x:break">break</xhtml:a> a cup<chn>打破杯子</chn></x></x-g-blk></x-gs></sn-g></sn-gs>"#,
            r#"<pv-gs-blk><pv-gs><pv-g><top-g><pv-blk><pv>ˌbreak aˈway (from sb/sth)</pv></pv-blk></top-g>"#,
            r#"<sn-gs><sn-g><def>to escape<chnsep> </chnsep><chn>挣脱</chn></def>"#,
            r#"<x-gs><x-g-blk><x>The prisoner broke away.<chn>囚犯挣脱了。</chn></x></x-g-blk></x-gs></sn-g></sn-gs></pv-g></pv-gs></pv-gs-blk>"#,
            r#"<idm-gs-blk><idm-gs><idm-g><idm-blk><idm>break one's heart</idm></idm-blk>"#,
            r#"<sn-gs><sn-g><def>to make sb very sad<chn>使心碎</chn></def></sn-g></sn-gs></idm-g></idm-gs></idm-gs-blk>"#,
        );
        let d = parse_definition("break", html);
        assert_eq!(d.phonetic, "breɪk");
        assert_eq!(d.pos, "verb");
        assert_eq!(d.senses_zh, vec!["（使）破，裂，碎".to_string()]);
        assert_eq!(d.examples.len(), 1);
        assert_eq!(d.examples[0].0, "to break a cup");
        assert_eq!(d.examples[0].1, "打破杯子");
        // phrase blocks: 1 pv + 1 idiom, and their senses must NOT leak into senses_zh
        assert_eq!(d.phrases.len(), 2);
        assert_eq!(d.phrases[0].kind, "pv");
        assert_eq!(d.phrases[0].text, "ˌbreak aˈway (from sb/sth)");
        assert_eq!(d.phrases[0].def_zh, "挣脱");
        assert_eq!(d.phrases[0].example_en, "The prisoner broke away.");
        assert_eq!(d.phrases[1].kind, "id");
        assert_eq!(d.phrases[1].text, "break one's heart");
        assert_eq!(d.phrases[1].def_zh, "使心碎");
    }
}
