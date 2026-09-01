//! lexicon-cli — command line tool for the lexicon engine.
//! Only CLI interaction lives here (arg parsing, printing, stdin prompts);
//! all logic lives in the lexicon-core crate, web UI in web.rs + assets/.

mod web;

use lexicon_core::storage::Db;
use std::io::Write;
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
            match load_scope(db, file, None) {
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
        Some("wordlist") => {
            // wordlist <db> <name> <wordlist-file> [--type word|phrase|pattern]
            let db = need(&args, 2, "wordlist <db> <name> <file> [--type word|phrase|pattern]");
            let name = need(&args, 3, "wordlist <db> <name> <file> [--type word|phrase|pattern]");
            let file = need(&args, 4, "wordlist <db> <name> <file> [--type word|phrase|pattern]");
            let mut default_type = "word";
            let mut i = 5;
            while i < args.len() {
                if args[i] == "--type" && i + 1 < args.len() {
                    default_type = args[i + 1].as_str();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            match import_wordlist(db, name, file, default_type) {
                Ok(n) => {
                    println!("wordlist '{name}' ready, {n} cards from {file}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("wordlist failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("phrases") => {
            // phrases <db> <source-headword>  —  print extracted phrases, or
            // phrases <db> --extract  —  build the whole phrase bank from entries
            let db = need(&args, 2, "phrases <db> [--extract | <source-word>]");
            if args.get(3).map(|s| s.as_str()) == Some("--extract") {
                match Db::open(db).and_then(|d| lexicon_core::study::rebuild_phrase_bank(&d)) {
                    Ok(n) => {
                        println!("phrase bank: {n} phrases extracted");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("extract failed: {e}");
                        ExitCode::FAILURE
                    }
                }
            } else {
                let word = need(&args, 3, "phrases <db> <source-word>");
                match list_phrases(db, word) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("phrases failed: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
        }
        Some("config") => {
            // config <db> --new N --review M   (daily limits)
            let db = need(&args, 2, "config <db> [--new N] [--review M]");
            match set_config(db, &args[3..]) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("config failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("serve") => {
            // serve <db> [--port <port>]  —  local web UI over HTTP
            let db = need(&args, 2, "serve <db> [--port <port>]");
            let mut port = 8000u16;
            let mut i = 3;
            while i < args.len() {
                if args[i] == "--port" && i + 1 < args.len() {
                    port = args[i + 1].parse().unwrap_or(8000);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            match web::serve(db, port) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("serve failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: lexicon-cli <import|lookup|stats|review|scope|wordlist|phrases|config|serve> ...");
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
    // Bulk insert in one transaction: per-row autocommit made imports so slow
    // that large dictionaries (11万+ entries) got cut off by timeouts, leaving
    // an incomplete word list behind.
    Ok(d.transaction(|tx| {
        let mut n = 0usize;
        for (key, val) in mdx.items()? {
            let headword = String::from_utf8_lossy(&key).to_string();
            let def = String::from_utf8_lossy(&val).to_string();
            tx.insert_entry(&headword, &def)?;
            n += 1;
        }
        Ok(n)
    })?) // ? converts lexicon_core::Error -> anyhow
}

fn lookup(db: &str, word: &str) -> anyhow::Result<Option<String>> {
    let d = Db::open(db)?;
    Ok(d.lookup_resolved(word)?)
}

/// Interactive terminal review: show the graded prompt, then ask for a grade.
fn review(db: &str, words: &[String], range: Option<&(String, String)>) -> anyhow::Result<usize> {
    use lexicon_core::sm2::{apply_review, Rating};
    let d = Db::open(db)?;
    for w in words {
        d.add_new_card(w)?;
    }
    // Loop until every due card has been answered correctly (lapses requeue to end).
    let mut processed = 0usize;
    loop {
        let now = lexicon_core::sm2::now();
        let due = match range {
            Some((s, e)) => d.due_cards_in_range(&now, s, e, 100)?,
            None => d.due_cards(&now, 100)?,
        };
        if due.is_empty() {
            break;
        }
        for (id, word) in due {
            let diff = d
                .load_card(id)?
                .map(|c| c.difficulty.as_str().to_string())
                .unwrap_or_default();
            let prompt = lexicon_core::prompt::build_card_prompt(&d, id, &word, "word", &diff, "");
            println!("\n=== 复习 ===");
            println!("{}\n---\n提示：{}", prompt["question"], prompt["hint"]);
            if let Some(mut card) = d.load_card(id)? {
                let grade = prompt_grade();
                apply_review(&mut card, Rating::new(grade, lexicon_core::sm2::now()));
                d.save_card(&card)?;
                processed += 1;
                println!("答案：{}", prompt["answer"]);
            }
        }
    }
    Ok(processed)
}

/// Interactive grade prompt: 0=again, 1=hard, 2=good, 3=easy.
fn prompt_grade() -> u8 {
    loop {
        print!("grade [0-3] (0=忘了, 1=困难, 2=认识, 3=简单): ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return 2; // EOF fallback
        }
        match line.trim().parse::<u8>() {
            Ok(g @ 0..=3) => return g,
            _ => continue,
        }
    }
}

/// Load a wordlist file, adding each word as a card.
/// Optionally register the words as a named wordlist for later scoping.
fn load_scope(db: &str, file: &str, wl_name: Option<&str>) -> anyhow::Result<usize> {
    let d = Db::open(db)?;
    let wl_id = match wl_name {
        Some(name) => Some(d.create_wordlist(name)?),
        None => None,
    };
    let content = std::fs::read_to_string(file)?;
    let mut n = 0usize;
    let mut headwords: Vec<String> = Vec::new();
    for line in content.lines() {
        let w = line.trim();
        if w.is_empty() {
            continue;
        }
        if d.add_new_card(w)? > 0 {
            n += 1;
        }
        headwords.push(w.to_string());
    }
    if let Some(id) = wl_id {
        d.add_wordlist_words(id, &headwords)?;
    }
    Ok(n)
}

/// Import a typed wordlist file (see `lexicon_core::study::parse_wordlist_line`).
fn import_wordlist(db: &str, name: &str, file: &str, default_type: &str) -> anyhow::Result<usize> {
    let d = Db::open(db)?;
    let wl_id = d.create_wordlist(name)?;
    let content = std::fs::read_to_string(file)?;
    let mut count = 0usize;
    let mut headwords: Vec<String> = Vec::new();
    for raw in content.lines() {
        if let Some(l) = lexicon_core::study::parse_wordlist_line(raw, default_type) {
            let id = d.add_new_card_full(
                &l.headword,
                l.card_type,
                lexicon_core::study::initial_difficulty(),
                &l.headword,
                &l.phrase,
            )?;
            if id > 0 {
                count += 1;
                if l.phrase != l.headword {
                    headwords.push(l.phrase.clone());
                }
                headwords.push(l.headword);
            }
        }
    }
    d.add_wordlist_words(wl_id, &headwords)?;
    Ok(count)
}

/// Print the phrase bank entries for one source headword.
fn list_phrases(db: &str, word: &str) -> anyhow::Result<()> {
    let d = Db::open(db)?;
    let items = d.phrases_for(word)?;
    if items.is_empty() {
        println!("no phrases found for '{word}'");
        return Ok(());
    }
    for (_, ptype, text, de, dz, xe, xz) in items {
        println!("[{ptype}] {text}");
        if !de.is_empty() {
            println!("  {de}");
        }
        if !dz.is_empty() {
            println!("  zh: {dz}");
        }
        if !xe.is_empty() {
            println!("  ex: {xe} {xz}");
        }
    }
    Ok(())
}

/// Set daily limits (or read them when no args).
fn set_config(db: &str, args: &[String]) -> anyhow::Result<()> {
    let d = Db::open(db)?;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--new" if i + 1 < args.len() => {
                d.set_setting("new_per_day", &args[i + 1])?;
                i += 2;
            }
            "--review" if i + 1 < args.len() => {
                d.set_setting("review_per_day", &args[i + 1])?;
                i += 2;
            }
            _ => i += 1,
        }
    }
    let (new_pd, rev_pd) = d.daily_limits()?;
    let (new_done, rev_done) = d.today_progress()?;
    println!("daily limits: new {new_done}/{new_pd}, review {rev_done}/{rev_pd}");
    Ok(())
}