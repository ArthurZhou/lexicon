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
        _ => {
            eprintln!("usage: lexicon-cli <import|lookup|stats> ...");
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
