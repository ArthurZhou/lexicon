//! lexicon-cli — command line tool for the lexicon engine.
//! Subcommands:
//!   import <file.mdx> <out.sqlite>   Parse an MDX dict into SQLite.
//!   lookup <db> <word>                Print a stored entry.
//!   stats <db>                       Print table counts.

use lexicon_core::storage::Db;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("import") => {
            let file = need(&args, 2, "import <file.mdx> <out.sqlite>");
            let db = need(&args, 3, "import <file.mdx> <out.sqlite>");
            match import(file, db) {
                Ok(n) => {
                    println!("imported {n} entries into {db}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("import failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("lookup") => {
            let db = need(&args, 2, "lookup <db> <word>");
            let word = need(&args, 3, "lookup <db> <word>");
            match lookup(db, word) {
                Ok(Some(s)) => {
                    println!("{s}");
                    ExitCode::SUCCESS
                }
                Ok(None) => {
                    println!("not found: {word}");
                    ExitCode::FAILURE
                }
                Err(e) => {
                    eprintln!("lookup failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("stats") => {
            let db = need(&args, 2, "stats <db>");
            match Db::open(db) {
                Ok(d) => match d.stats() {
                    Ok((w, c)) => {
                        println!("words: {w}, cards: {c}");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("stats failed: {e}");
                        ExitCode::FAILURE
                    }
                },
                Err(e) => {
                    eprintln!("open failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("review") => {
            // review <db> [<word>...]  —  add given words as cards then run due loop
            let db = need(&args, 2, "review <db> [words...]");
            match review(db, &args[3..]) {
                Ok(n) => {
                    println!("review session done, {n} cards processed");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("review failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: lexicon-cli <import|lookup|stats|review> ...");
            ExitCode::FAILURE
        }
    }
}

fn need<'a>(args: &'a [String], i: usize, usage: &'a str) -> &'a str {
    match args.get(i) {
        Some(s) => s.as_str(),
        None => {
            eprintln!("usage: {usage}");
            std::process::exit(2);
        }
    }
}

fn import(file: &str, db: &str) -> anyhow::Result<usize> {
    let _ = std::fs::remove_file(db); // fresh db
    let mut d = Db::open(db)?;
    let mdx = lexicon_core::mdx_parser::Mdx::open(file)?;
    let mut n = 0usize;
    for (key, val) in mdx.items() {
        let headword = String::from_utf8_lossy(&key).to_string();
        let def = String::from_utf8_lossy(&val).to_string();
        d.insert_entry(&headword, &def)?;
        n += 1;
    }
    Ok(n)
}

fn lookup(db: &str, word: &str) -> anyhow::Result<Option<String>> {
    let d = Db::open(db)?;
    Ok(d.lookup(word)?)
}

/// Add words as cards, then process the due queue once.
fn review(db: &str, words: &[String]) -> anyhow::Result<usize> {
    let d = Db::open(db)?;
    for w in words {
        d.add_new_card(w)?;
    }
    let now = chrono::Local::now().naive_local();
    let due = d.due_cards(&now, 100)?;
    let mut processed = 0usize;
    for (id, word) in due {
        // fetch definition to show
        let def = d.lookup(&word)?.unwrap_or_default();
        let first_line = strip_html(&def).chars().take(120).collect::<String>();
        println!("\n=== {word} ===\n{first_line}...");
        if let Some(mut card) = d.load_card(id)? {
            // simulate: grade 2 (Good)
            lexicon_core::sm2::apply_review(&mut card, lexicon_core::sm2::Rating::new(2, now));
            d.save_card(&card)?;
            processed += 1;
        }
    }
    Ok(processed)
}

fn strip_html(s: &str) -> String {
    let re = regex::Regex::new(r#"<[^>]*>"#).unwrap();
    re.replace_all(s, " ").to_string()
}
