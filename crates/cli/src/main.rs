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
            match serve(db, port) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("serve failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: lexicon-cli <import|lookup|stats|review|scope|serve> ...");
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

/// Serve the built-in single-page UI over HTTP.
fn serve(db: &str, port: u16) -> anyhow::Result<()> {
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let d = Db::open(db)?;
    println!("lexicon UI at http://127.0.0.1:{port}");
    for mut req in server.incoming_requests() {
        let (status, body) = handle(&d, &mut req);
        let resp = tiny_http::Response::from_string(body).with_status_code(status);
        let _ = req.respond(resp);
    }
    Ok(())
}

fn handle(d: &Db, req: &mut tiny_http::Request) -> (u16, String) {
    let (path, query) = match req.url().split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (req.url(), None),
    };
    match (req.method(), path) {
        (&tiny_http::Method::Get, "/") => (200, INDEX_HTML.to_string()),
        (&tiny_http::Method::Get, "/api/due") => {
            let now = chrono::Local::now().naive_local();
            let limit: i64 = query.and_then(|q| q.split('=').nth(1)).and_then(|v| v.parse().ok()).unwrap_or(100);
            match d.due_card_details(&now, None, limit) {
                Ok(cards) => {
                    let arr: Vec<serde_json::Value> = cards
                        .into_iter()
                        .map(|(id, headword, def)| serde_json::json!({"id": id, "headword": headword, "definition": def}))
                        .collect();
                    (200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => (500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Post, "/api/grade") => {
            let _ = query;
            let body = read_body(req);
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    let id = v["id"].as_i64().unwrap_or(0);
                    let grade = v["grade"].as_u64().unwrap_or(2) as u8;
                    match d.grade_card(id, grade) {
                        Ok(()) => (200, "{\"ok\":true}".into()),
                        Err(e) => (200, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
                    }
                }
                Err(e) => (400, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
            }
        }
        _ => (404, "not found".into()),
    }
}

fn read_body(req: &mut tiny_http::Request) -> String {
    let mut tmp = Vec::new();
    if req.as_reader().read_to_end(&mut tmp).is_ok() {
        String::from_utf8_lossy(&tmp).to_string()
    } else {
        String::new()
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lexicon</title>
<style>
  body { font-family: system-ui, sans-serif; max-width: 640px; margin: 2rem auto; padding: 0 1rem; color: #222; }
  .card { border: 1px solid #ddd; border-radius: 10px; padding: 1.5rem; margin-bottom: 1rem; }
  .head { font-size: 1.6rem; font-weight: 700; }
  .def { margin-top: 0.8rem; white-space: pre-wrap; color: #444; }
  .btn { padding: 0.6rem 1.2rem; border: none; border-radius: 8px; font-size: 1rem; cursor: pointer; margin-right: 0.5rem; }
  .again { background:#e5484d; color:#fff; }
  .good { background:#30a46c; color:#fff; }
  .easy { background:#0091ff; color:#fff; }
  .empty { color:#888; text-align:center; padding: 3rem 0; }
</style>
</head>
<body>
<h1>Lexicon</h1>
<div id="app"><p class="empty">Loading…</p></div>
<script>
const app = document.getElementById('app');
function strip(html){
  const d = document.createElement('div'); d.innerHTML = html;
  return d.textContent || '';
}
async function load(){
  const r = await fetch('/api/due');
  const cards = await r.json();
  if(!cards.length){ app.innerHTML = '<p class="empty">No cards due 🎉</p>'; return; }
  const c = cards[0];
  app.innerHTML = '';
  const card = document.createElement('div'); card.className='card';
  card.innerHTML = '<div class="head">'+c.headword+'</div><div class="def"></div>';
  card.querySelector('.def').textContent = strip(c.definition);
  const row = document.createElement('div'); row.style.marginTop='1rem';
  ['again','good','easy'].forEach(k=>{
    const b = document.createElement('button'); b.className='btn '+k; b.textContent=k;
    b.onclick = async ()=>{ await fetch('/api/grade',{method:'POST',body:JSON.stringify({id:c.id,grade:k==='again'?0:k==='good'?2:4})}); load(); };
    row.appendChild(b);
  });
  card.appendChild(row); app.appendChild(card);
}
load();
</script>
</body>
</html>
"#;
