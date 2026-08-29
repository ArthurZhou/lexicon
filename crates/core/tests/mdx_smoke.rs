//! Smoke test: open the downloaded ODE 3e / WordNet .mdx with our parser.
//! Set env LEXICON_MDX to the path of an .mdx file; skipped otherwise.

#[test]
fn opens_mdx_and_looks_up_apple() {
    let path = std::env::var("LEXICON_MDX").map(std::path::PathBuf::from);
    let path = match path {
        Ok(p) if p.exists() => p,
        _ => return, // skip when not provided
    };

    let mdx = lexicon_core::mdx_parser::Mdx::open(&path).expect("open mdx");
    let n = mdx.len();
    eprintln!("total entries: {n}");
    assert!(n > 0, "expected >0 entries, got {n}");

    let mut apple: Option<String> = None;
    for (key, value) in mdx.items() {
        let k = String::from_utf8_lossy(&key).to_string();
        if k.eq_ignore_ascii_case("apple") {
            apple = Some(String::from_utf8_lossy(&value).to_string());
            break;
        }
    }

    let d = apple.expect("apple not found");
    eprintln!("apple definition:\n{d}");
}
