//! lexicon-cli — command line tool for the lexicon engine.
//! Subcommands:
//!   import <file.mdx> <out.sqlite>   Parse an MDX dict into SQLite.
//!   lookup <db> <word>                Print a stored entry.
//!   stats <db>                       Print table counts.
//!   review <db> [--range <start> <end>] [words...]
//!        Add words as cards, then grade due cards interactively.
//!        --range limits the session to headwords in [start, end).
//!   scope <db> <wordlist-file>       Add every word in a file as cards.

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
            // review <db> [<word>...]  —  add given words as cards then run due loop interactively
            let db = need(&args, 2, "review <db> [--range <start> <end>] [words...]");
            let mut range: Option<(String, String)> = None;
            let mut words = Vec::new();
            let mut i = 3;
            while i < args.len() {
                if args[i] == "--range" && i + 2 < args.len() {
                    range = Some((args[i + 1].clone(), args[i + 2].clone()));
                    i += 3;
                } else {
                    words.push(args[i].clone());
                    i += 1;
                }
            }
            match review(db, &words, range.as_ref()) {
                Ok(n) => {
                    println!("\nreview session done, {n} cards processed");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("review failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("scope") => {
            // scope <db> <wordlist-file>  —  add every word in a file as cards
            let db = need(&args, 2, "scope <db> <wordlist-file>");
            let file = need(&args, 3, "scope <db> <wordlist-file>");
            match load_scope(db, file) {
                Ok(n) => {
                    println!("scope loaded, {n} cards");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("scope failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: lexicon-cli <import|lookup|stats|review|scope> ...");
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
fn review(db: &str, words: &[String], range: Option<&(String, String)>) -> anyhow::Result<usize> {
    let d = Db::open(db)?;
    for w in words {
        d.add_new_card(w)?;
    }
    // Loop until every due card has been answered correctly (lapses requeue to end).
    let mut processed = 0usize;
    loop {
        let now = chrono::Local::now().naive_local();
        let due = match range {
            Some((s, e)) => d.due_cards_in_range(&now, s, e, 100)?,
            None => d.due_cards(&now, 100)?,
        };
        if due.is_empty() {
            break;
        }
        for (id, word) in due {
            let def = d.lookup(&word)?.unwrap_or_default();
            let first_line = strip_html(&def).chars().take(120).collect::<String>();
            println!("\n=== {word} ===\n{first_line}...");
            if let Some(mut card) = d.load_card(id)? {
                let grade = prompt_grade() as u8;
                let now = chrono::Local::now().naive_local();
                lexicon_core::sm2::apply_review(&mut card, lexicon_core::sm2::Rating::new(grade, now));
                d.save_card(&card)?;
                processed += 1;
            }
        }
    }
    Ok(processed)
}

/// Interactive grade prompt: 0/1=again, 2=good, 3/4=easy.
fn prompt_grade() -> u32 {
    use std::io::Write;
    loop {
        print!("grade [0-2-4] (0=again, 2=good, 4=easy): ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return 2; // EOF fallback
        }
        match line.trim().parse::<u32>() {
            Ok(0) => return 0,
            Ok(1) => return 1,
            Ok(2) => return 2,
            Ok(3) => return 3,
            Ok(4) => return 4,
            _ => continue,
        }
    }
}

fn strip_html(s: &str) -> String {
    let re = regex::Regex::new(r#"<[^>]*>"#).unwrap();
    re.replace_all(s, " ").to_string()
}

/// Load a wordlist file, adding each word as a card.
fn load_scope(db: &str, file: &str) -> anyhow::Result<usize> {
    let d = Db::open(db)?;
    let content = std::fs::read_to_string(file)?;
    let mut n = 0usize;
    for line in content.lines() {
        let w = line.trim();
        if w.is_empty() {
            continue;
        }
        if d.add_new_card(w)? > 0 {
            n += 1;
        }
    }
    Ok(n)
}
