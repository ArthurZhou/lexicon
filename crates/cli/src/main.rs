//! lexicon-cli — command line tool for the lexicon engine.
//! Subcommands:
//!   import <file.mdx> <out.sqlite>   Parse an MDX dict into SQLite.
//!   lookup <db> <word>                Print a stored entry.
//!   stats <db>                       Print table counts.
//!   review <db> [--range <start> <end>] [words...]
//!        Add words as cards, then grade due cards interactively.
//!        --range limits the session to headwords in [start, end).
//!   scope <db> <wordlist-file>       Add every word in a file as cards.

mod mcp;

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
                    println!("
review session done, {n} cards processed");
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
            let db = need(&args, 2, "wordlist <db> <name> <wordlist-file> [--type word|phrase|pattern]");
            let name = need(&args, 3, "wordlist <db> <name> <wordlist-file> [--type word|phrase|pattern]");
            let file = need(&args, 4, "wordlist <db> <name> <wordlist-file> [--type word|phrase|pattern]");
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
                match extract_all_phrases(db) {
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
            match serve(db, port) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("serve failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("mcp") => {
            // mcp <db>  —  serve the lexicon as an MCP tool server over stdio.
            let db = need(&args, 2, "mcp <db>");
            match Db::open(db) {
                Ok(d) => match mcp::run(&d) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("mcp failed: {e}");
                        ExitCode::FAILURE
                    }
                },
                Err(e) => {
                    eprintln!("open {db} failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: lexicon-cli <import|lookup|stats|review|scope|wordlist|phrases|config|serve|mcp> ...");
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
        for (key, val) in mdx.items() {
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
            let def = d.lookup_resolved(&word)?.unwrap_or_default();
            let first_line = strip_html(&def).chars().take(120).collect::<String>();
            println!("
=== {word} ===
{first_line}...");
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

/// Parse a wordlist file where each line may be:
///   headword
///   headword	word|phrase|pattern
///   word|phrase|pattern:headword[:phrase-text]
/// and create cards with the right CardType. Skips empty lines and
/// lines whose headword is not in the imported dictionary.
fn import_wordlist(db: &str, name: &str, file: &str, default_type: &str) -> anyhow::Result<usize> {
    use lexicon_core::model::{CardType, Difficulty};
    let d = Db::open(db)?;
    let wl_id = d.create_wordlist(name)?;
    let content = std::fs::read_to_string(file)?;
    let mut count = 0usize;
    let mut headwords: Vec<String> = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Supported line forms:
        //   headword            -> default type
        //   headword<TAB>类型    -> explicit type (word|phrase|pattern)
        //   类型:headword|短语   -> typed card with explicit phrase text
        let (ctype, rest) = match line.split_once('\t') {
            Some((h, t)) if ["word", "phrase", "pattern"].contains(&t) => (t.to_string(), h),
            _ => match line.split_once(':') {
                Some((t, r)) if ["word", "phrase", "pattern"].contains(&t) => (t.to_string(), r),
                _ => (default_type.to_string(), line),
            },
        };
        let (head, phrase) = match rest.split_once('|') {
            Some((h, p)) => (h.trim(), Some(p.trim().to_string())),
            None => (rest.trim(), None),
        };
        let card_type = match ctype.as_str() {
            "phrase" => CardType::Phrase,
            "pattern" => CardType::Pattern,
            _ => CardType::Word,
        };
        let id = d.add_new_card_full(
            head,
            card_type,
            Difficulty::Easy,
            head,
            phrase.as_deref().unwrap_or(head),
        )?;
        if id > 0 {
            count += 1;
            headwords.push(head.to_string());
            if let Some(p) = phrase {
                headwords.push(p);
            }
        }
    }
    d.add_wordlist_words(wl_id, &headwords)?;
    Ok(count)
}

/// Extract every phrase/idiom from all entries into the phrase bank.
fn extract_all_phrases(db: &str) -> anyhow::Result<usize> {
    use lexicon_core::definition::parse_definition;
    let mut d = Db::open(db)?;
    d.conn().execute_batch("DELETE FROM phrases")?;
    let sql = "SELECT headword, definition FROM entries";
    let mut n = 0usize;
    d.transaction(|db| {
        let mut stmt = db.conn().prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (head, def) = r?;
            let parsed = parse_definition(&head, &def);
            for p in parsed.phrases {
                db.insert_phrase(&head, &p.kind, &p.text, &p.def_en, &p.def_zh, &p.example_en, &p.example_zh)?;
                n += 1;
            }
        }
        Ok(n)
    })?;
    Ok(n)
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
                d.set_setting("new_per_day", args[i + 1].as_str())?;
                i += 2;
            }
            "--review" if i + 1 < args.len() => {
                d.set_setting("review_per_day", args[i + 1].as_str())?;
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

/// Serve the built-in single-page UI over HTTP.
/// Listens on 0.0.0.0 so phones/tablets on the same LAN can reach it.
fn serve(db: &str, port: u16) -> anyhow::Result<()> {
    let server = tiny_http::Server::http(("0.0.0.0", port)).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let d = Db::open(db)?;
    println!("lexicon UI listening on http://0.0.0.0:{port}");
    for ip in local_ips() {
        println!("  mobile (same Wi-Fi): http://{ip}:{port}");
    }
    println!("  this computer:       http://127.0.0.1:{port}");
    println!("  sqlite db:           {db}");
    for mut req in server.incoming_requests() {
        let resp = handle(&d, &mut req);
        let _ = req.respond(resp);
    }
    Ok(())
}

/// Best-effort detection of this host's LAN IPv4 addresses.
fn local_ips() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        // UDP "connect" does not send packets; it only picks a route,
        // which is enough to learn the local address for that route.
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                if !addr.ip().is_loopback() {
                    out.push(addr.ip().to_string());
                }
            }
        }
    }
    out
}

fn json_response(status: u16, body: String) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    text_response(status, "application/json; charset=utf-8", body)
}

fn html_response(status: u16, body: String) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    text_response(status, "text/html; charset=utf-8", body)
}

fn text_response(
    status: u16,
    content_type: &'static str,
    body: String,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    use std::io::Cursor;
    let headers = vec![
        tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
            .expect("valid header"),
        tiny_http::Header::from_bytes(&b"Cache-Control"[..], b"no-cache").expect("valid header"),
        tiny_http::Header::from_bytes(&b"X-Content-Type-Options"[..], b"nosniff")
            .expect("valid header"),
    ];
    let bytes = body.into_bytes();
    let len = bytes.len();
    tiny_http::Response::new(
        tiny_http::StatusCode(status),
        headers,
        Cursor::new(bytes),
        Some(len),
        None,
    )
}

fn handle(d: &Db, req: &mut tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let (path, query) = match req.url().split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (req.url(), None),
    };
    match (req.method(), path) {
        (&tiny_http::Method::Get, "/") => html_response(200, INDEX_HTML.to_string()),
        (&tiny_http::Method::Get, "/manifest.json") => {
            text_response(200, "application/manifest+json; charset=utf-8", MANIFEST_JSON.to_string())
        }
        (&tiny_http::Method::Get, "/sw.js") => {
            text_response(200, "text/javascript; charset=utf-8", SW_JS.to_string())
        }
        (&tiny_http::Method::Get, "/icon.svg") => {
            text_response(200, "image/svg+xml", ICON_SVG.to_string())
        }
        (&tiny_http::Method::Get, "/api/due") => {
            let now = chrono::Local::now().naive_local();
            let limit: i64 = query
                .and_then(|q| q.split('&').next())
                .and_then(|q| q.split('=').nth(1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            let wl: Option<i64> = query
                .and_then(|q| {
                    q.split("&").find_map(|kv| {
                        let (k, v) = kv.split_once("=")?;
                        (k == "list").then(|| v.parse::<i64>().ok()).flatten()
                    })
                })
                .or_else(|| {
                    d.get_setting("active_wordlist")
                        .ok()
                        .flatten()
                        .and_then(|v| v.parse::<i64>().ok())
                });
            match d.daily_due_queue(&now, limit, wl) {
                Ok((cards, new_left, review_left)) => {
                    let arr: Vec<serde_json::Value> = cards
                        .into_iter()
                        .map(|(id, headword, ctype, diff, phrase)| {
                            let prompt = build_card_prompt(&d, id, &headword, &ctype, &diff, &phrase);
                            serde_json::json!({
                                "id": id,
                                "headword": headword,
                                "card_type": ctype,
                                "difficulty": diff,
                                "phrase": phrase,
                                "prompt": prompt,
                            })
                        })
                        .collect();
                    json_response(
                        200,
                        serde_json::json!({
                            "cards": arr,
                            "new_left": new_left,
                            "review_left": review_left,
                        })
                        .to_string(),
                    )
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/settings") => {
            let (new_pd, rev_pd) = d.daily_limits().unwrap_or((20, 100));
            let (new_done, rev_done) = d.today_progress().unwrap_or((0, 0));
            let active = d.get_setting("active_wordlist").unwrap_or_default().and_then(|v| v.parse::<i64>().ok());
            json_response(
                200,
                serde_json::json!({
                    "new_per_day": new_pd,
                    "review_per_day": rev_pd,
                    "new_done": new_done,
                    "review_done": rev_done,
                    "active_wordlist": active,
                })
                .to_string(),
            )
        }
        (&tiny_http::Method::Post, "/api/settings") => {
            let body = read_body(req);
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    if let Some(n) = v["new_per_day"].as_u64() {
                        let _ = d.set_setting("new_per_day", &n.to_string());
                    }
                    if let Some(r) = v["review_per_day"].as_u64() {
                        let _ = d.set_setting("review_per_day", &r.to_string());
                    }
                    if let Some(wl) = v["active_wordlist"].as_str() {
                        let _ = d.set_setting("active_wordlist", wl);
                    }
                    json_response(200, "{\"ok\":true}".into())
                }
                Err(e) => json_response(400, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/wordlists") => {
            match d.wordlists() {
                Ok(lists) => {
                    let arr: Vec<serde_json::Value> = lists
                        .iter()
                        .map(|w| serde_json::json!({"id": w.id, "name": w.name, "size": w.size}))
                        .collect();
                    json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/cards") => {
            let limit: i64 = query
                .and_then(|q| q.split('&').next())
                .and_then(|q| q.split('=').nth(1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(200);
            match d.card_records(limit) {
                Ok(rows) => {
                    let arr: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|(id, headword, ctype, diff, status, reps, lapses, due, last, _created)| {
                            serde_json::json!({
                                "id": id,
                                "headword": headword,
                                "card_type": ctype,
                                "difficulty": diff,
                                "status": status,
                                "reps": reps,
                                "lapses": lapses,
                                "due": due,
                                "last_review": last,
                            })
                        })
                        .collect();
                    json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Post, "/api/random") => {
            match d.random_word() {
                Ok(Some(w)) => {
                    match d.add_new_card(&w) {
                        Ok(id) if id > 0 => {
                            json_response(200, format!("{{\"ok\":true,\"word\":\"{w}\",\"id\":{id}}}"))
                        }
                        _ => json_response(200, "{\"ok\":false}".into()),
                    }
                }
                _ => json_response(200, "{\"ok\":false}".into()),
            }
        }
        (&tiny_http::Method::Get, "/api/phrase_count") => {
            let n = d.phrase_count().unwrap_or(0);
            json_response(200, format!("{{\"count\":{n}}}"))
        }
        (&tiny_http::Method::Post, "/api/card/difficulty") => {
            let body = read_body(req);
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    let id = v["id"].as_i64().unwrap_or(0);
                    let diff = match v["difficulty"].as_str().unwrap_or("easy") {
                        "hard" => lexicon_core::model::Difficulty::Hard,
                        _ => lexicon_core::model::Difficulty::Easy,
                    };
                    match d.set_card_difficulty(id, diff) {
                        Ok(()) => json_response(200, "{\"ok\":true}".into()),
                        Err(e) => json_response(200, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
                    }
                }
                Err(e) => json_response(400, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/card/history") => {
            let id: i64 = query
                .and_then(|q| q.split('=').nth(1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            match d.card_history(id) {
                Ok(log) => {
                    let arr: Vec<serde_json::Value> = log
                        .iter()
                        .map(|l| {
                            serde_json::json!({
                                "at": l.reviewed_at.format("%m-%d %H:%M").to_string(),
                                "grade": l.grade,
                                "is_new": l.is_new,
                            })
                        })
                        .collect();
                    json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/search") => {
            let q = percent_decode(query.and_then(|s| s.strip_prefix("q=")).unwrap_or(""));
            match d.search(&q, 50) {
                Ok(rows) => {
                    let arr: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|(headword, def)| {
                            let text = resolve_def(&d, &headword, def.as_deref().unwrap_or(""));
                            serde_json::json!({"headword": headword, "text": plainify(&text)})
                        })
                        .collect();
                    json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/lookup") => {
            let w = percent_decode(query.and_then(|s| s.strip_prefix("w=")).unwrap_or(""));
            match d.lookup_resolved(&w) {
                Ok(Some(def)) => json_response(
                    200,
                    serde_json::json!({"headword": w, "text": plainify(&def), "found": true}).to_string(),
                ),
                Ok(None) => json_response(
                    200,
                    serde_json::json!({"headword": w, "text": null, "found": false}).to_string(),
                ),
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Post, "/api/add") => {
            let body = read_body(req);
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    let word = v["word"].as_str().unwrap_or("").to_string();
                    match d.add_new_card(&word) {
                        Ok(id) if id > 0 => {
                            json_response(200, format!("{{\"ok\":true,\"id\":{id}}}"))
                        }
                        Ok(_) => json_response(
                            404,
                            format!("{{\"ok\":false,\"error\":\"word not in dictionary: {word}\"}}"),
                        ),
                        Err(e) => json_response(500, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
                    }
                }
                Err(e) => json_response(400, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Post, "/api/grade") => {
            let body = read_body(req);
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    let id = v["id"].as_i64().unwrap_or(0);
                    let grade = v["grade"].as_u64().unwrap_or(2) as u8;
                    match d.grade_card(id, grade) {
                        Ok(()) => json_response(200, "{\"ok\":true}".into()),
                        Err(e) => {
                            json_response(200, format!("{{\"ok\":false,\"error\":\"{e}\"}}"))
                        }
                    }
                }
                Err(e) => json_response(400, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/stats") => {
            let now = chrono::Local::now().naive_local();
            let stats = d.stats().unwrap_or((0, 0));
            let due = d.due_count(&now).unwrap_or(0);
            json_response(
                200,
                serde_json::json!({"words": stats.0, "cards": stats.1, "due": due}).to_string(),
            )
        }
        _ => json_response(404, "{\"error\": \"not found\"}".into()),
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

/// Decode percent-encoded query values ("a%20b" -> "a b", "+" -> " ").
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Resolve an @@@LINK= alias to its real definition, falling back to the
/// stored definition when the target is missing (defensive).
fn resolve_def(d: &Db, headword: &str, def: &str) -> String {
    match d.lookup_resolved(headword) {
        Ok(Some(text)) => text,
        _ => def.to_string(),
    }
}

/// Build the review prompt for one card according to its study mode:
///  - word (difficulty easy): the example sentence + its Chinese
///    translation as a hint; the answer is the headword.
///  - word (difficulty hard): first letter + a few Chinese senses.
///  - phrase: the Chinese gloss + Chinese example; answer is the phrase.
///  - pattern: the usage note (Chinese) + example; answer is the pattern.
/// Returns a JSON object: {kind, question, hint, answer, extra}.
/// Normalize a phrase text for matching: drop stress/separator glyphs
/// (ˈ ˌ ↔ | / ) and trim, so "give sb a ˈbreak" matches "give sb a break".
fn norm_phrase(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{02C8}' | '\u{02CC}' | '\u{2194}' | '\u{007C}' | '\u{002F}'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn build_card_prompt(
    d: &Db,
    _card_id: i64,
    headword: &str,
    ctype: &str,
    diff: &str,
    phrase: &str,
) -> serde_json::Value {
    use lexicon_core::definition::parse_definition;

    let def = d.lookup_resolved(headword).ok().flatten().unwrap_or_default();
    let parsed = parse_definition(headword, &def);

    match ctype {
        "phrase" => {
            // Fixed phrase / collocation: prefer the phrase bank (already
            // extracted, deduped) so the gloss matches THIS phrase, falling
            // back to whatever the headword definition parses.
            let (zh, ex_en, ex_zh) = d
                .phrases_for(headword)
                .unwrap_or_default()
                .into_iter()
                .find(|(_, _pt, text, _de, _dz, _xe, _xz)| norm_phrase(text) == norm_phrase(phrase))
                .map(|(_, _pt, _text, de, dz, xe, xz)| {
                    let z = if dz.is_empty() { de.clone() } else { dz.clone() };
                    (z, xe.clone(), xz.clone())
                })
                .or_else(|| {
                    parsed.phrases.iter().find(|p| p.text.contains(phrase)).map(|p| (
                        if p.def_zh.is_empty() { p.def_en.clone() } else { p.def_zh.clone() },
                        p.example_en.clone(),
                        p.example_zh.clone(),
                    ))
                })
                .unwrap_or_default();
            let question = if !zh.is_empty() { zh } else { plainify(&def).chars().take(60).collect() };
            let hint = if !ex_en.is_empty() { format!("{ex_en}\n{ex_zh}") } else { String::new() };
            serde_json::json!({
                "kind": "phrase",
                "question": question,
                "hint": hint,
                "answer": phrase,
                "extra": {
                    "pos": parsed.pos,
                    "phonetic": parsed.phonetic,
                    "source": headword,
                    "example": ex_en,
                    "example_zh": ex_zh,
                }
            })
        }
        "pattern" => {
            // Sentence pattern / special usage: also prefer the phrase bank
            // (idioms/sayings carry the usage gloss); else first id/sd block.
            let (zh, ex_en, ex_zh) = d
                .phrases_for(headword)
                .unwrap_or_default()
                .into_iter()
                .find(|(_, pt, text, _de, _dz, _xe, _xz)| {
                    (pt == "id" || pt == "sd") && norm_phrase(text) == norm_phrase(phrase)
                })
                .map(|(_, _pt, _text, de, dz, xe, xz)| {
                    let z = if dz.is_empty() { de.clone() } else { dz.clone() };
                    (z, xe.clone(), xz.clone())
                })
                .or_else(|| {
                    parsed.phrases.iter().find(|p| p.kind == "id" || p.kind == "sd").map(|p| (
                        if p.def_zh.is_empty() { p.def_en.clone() } else { p.def_zh.clone() },
                        p.example_en.clone(),
                        p.example_zh.clone(),
                    ))
                })
                .unwrap_or_default();
            let question = if !zh.is_empty() { zh } else { plainify(&def).chars().take(60).collect() };
            serde_json::json!({
                "kind": "pattern",
                "question": question,
                "hint": ex_zh,
                "answer": phrase,
                "extra": {
                    "source": headword,
                    "example": ex_en,
                    "example_zh": ex_zh,
                }
            })
        }
        _ => {
            // Word: Chinese->English. Easy = example + Chinese hint;
            // hard = initial letter + a couple of Chinese senses.
            let zh = if parsed.senses_zh.is_empty() {
                plainify(&def).chars().take(60).collect::<String>()
            } else {
                parsed.senses_zh[..parsed.senses_zh.len().min(2)].join("；")
            };
            let (ex_en, ex_zh) = parsed
                .examples
                .first()
                .map(|(e, z)| (e.clone(), z.clone()))
                .unwrap_or_default();
            let hard_example = format!("{ex_en}
{ex_zh}");
            let (question, hint) = if diff == "hard" {
                // Hard: first letter mask + Chinese senses.
                let initial = headword
                    .chars()
                    .take(1)
                    .collect::<String>()
                    .to_uppercase();
                (
                    format!("{initial}___  {zh}"),
                    String::new(),
                )
            } else {
                // Easy: example sentence + Chinese as the hint.
                (
                    if zh.is_empty() { ex_en.clone() } else { zh },
                    hard_example,
                )
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

/// Convert an MDX definition (HTML with markup, styles and media links)
/// into readable plain text. This is what kills the "raw source code" look:
/// scripts, styles, images, audio and sound:// refs are dropped, block
/// boundaries become line breaks, entities are decoded.
fn plainify(html: &str) -> String {
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
    let re_void = regex::Regex::new(r"(?is)<(?:link|img|source|meta|input|area|base|track|iframe|object|embed)\b[^>]*/?>").unwrap();
    s = re_void.replace_all(&s, "").to_string();

    // <br> becomes a newline
    let re_br = regex::Regex::new(r"(?i)<br\s*/?>").unwrap();
    s = re_br.replace_all(&s, "\n").to_string();

    // block containers become newlines
    let re_close = regex::Regex::new(r"(?i)</(?:div|p|li|tr|h[1-6]|section|ul|ol|table|dl|dt|dd)>").unwrap();
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


const ICON_SVG: &str = r####"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128"><rect width="128" height="128" rx="26" fill="#2563eb"/><rect x="20" y="22" width="54" height="84" rx="8" fill="#ffffff" opacity="0.92"/><path d="M74 28h24a10 10 0 0 1 10 10v58h-34z" fill="#93c5fd"/><rect x="30" y="36" width="34" height="6" rx="3" fill="#1e3a8a"/><rect x="30" y="50" width="34" height="4" rx="2" fill="#3b82f6"/><rect x="30" y="60" width="28" height="4" rx="2" fill="#3b82f6"/><rect x="30" y="70" width="30" height="4" rx="2" fill="#3b82f6"/><circle cx="96" cy="30" r="10" fill="#fbbf24" stroke="#2563eb" stroke-width="3"/></svg>"####;

const MANIFEST_JSON: &str = r####"{
  "name": "Lexicon — MDX 词典 / 背单词",
  "short_name": "Lexicon",
  "start_url": "/",
  "scope": "/",
  "display": "standalone",
  "orientation": "portrait",
  "background_color": "#f8fafc",
  "theme_color": "#2563eb",
  "description": "本地 MDX 词典查询与间隔重复背单词",
  "icons": [
    { "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" }
  ]
}"####;

const SW_JS: &str = r####"var CACHE = 'lexicon-v1';
var SHELL = ['/', '/manifest.json', '/icon.svg'];
self.addEventListener('install', function (e) {
  e.waitUntil(caches.open(CACHE).then(function (c) { return c.addAll(SHELL); }).then(function () { return self.skipWaiting(); }));
});
self.addEventListener('activate', function (e) {
  e.waitUntil(caches.keys().then(function (ks) {
    return Promise.all(ks.filter(function (k) { return k !== CACHE; }).map(function (k) { return caches.delete(k); }));
  }).then(function () { return self.clients.claim(); }));
});
self.addEventListener('fetch', function (e) {
  var url = new URL(e.request.url);
  if (e.request.method !== 'GET' || url.pathname.indexOf('/api/') === 0) { return; }
  e.respondWith(
    caches.match(e.request).then(function (hit) {
      return hit || fetch(e.request).then(function (res) {
        var copy = res.clone();
        caches.open(CACHE).then(function (c) { return c.put(e.request, copy); });
        return res;
      });
    })
  );
});"####;

const INDEX_HTML: &str = r####"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="theme-color" content="#2563eb">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<meta name="apple-mobile-web-app-title" content="Lexicon">
<link rel="manifest" href="/manifest.json">
<link rel="icon" href="/icon.svg" type="image/svg+xml">
<title>Lexicon 词典 · 背单词</title>
<style>
  :root {
    --bg: #f1f5f9; --card: #ffffff; --ink: #0f172a; --muted: #64748b;
    --brand: #2563eb; --brand-ink: #ffffff; --line: #e2e8f0;
    --again: #dc2626; --hard: #f59e0b; --good: #16a34a; --easy: #0891b2;
    --safe-b: env(safe-area-inset-bottom, 0px); --safe-t: env(safe-area-inset-top, 0px);
  }
  * { box-sizing: border-box; -webkit-tap-highlight-color: transparent; }
  html, body { margin: 0; padding: 0; height: 100%; }
  body {
    background: var(--bg); color: var(--ink);
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
    overscroll-behavior-y: none;
  }
  #app { display: flex; flex-direction: column; height: 100%; }
  header {
    background: linear-gradient(135deg, #1d4ed8, #2563eb, #3b82f6);
    color: #fff; padding: calc(12px + var(--safe-t)) 16px 26px 16px;
  }
  header .toprow { display: flex; align-items: center; justify-content: space-between; }
  header h1 { margin: 0; font-size: 1.3rem; font-weight: 800; }
  header .progress { margin-top: 10px; display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  header .pill {
    background: rgba(255,255,255,.16); border-radius: 10px; padding: 8px 12px;
    font-size: .82rem; font-weight: 600; display: flex; align-items: center; gap: 8px;
  }
  header .pill b { font-size: 1.05rem; }
  header .mini { font-size: .72rem; opacity: .85; margin-left: auto; }
  main { flex: 1; overflow-y: auto; padding: 0 14px calc(86px + var(--safe-b)); }
  .cardbox {
    background: var(--card); border-radius: 18px; margin: 12px auto; max-width: 640px;
    box-shadow: 0 1px 3px rgba(15,23,42,.08), 0 10px 30px rgba(15,23,42,.08);
    border: 1px solid var(--line); overflow: hidden;
  }
  .tag { display: inline-block; font-size: .72rem; font-weight: 700; padding: 3px 10px; border-radius: 999px; letter-spacing: .04em; }
  .tag.word { background: #dbeafe; color: #1d4ed8; }
  .tag.phrase { background: #dcfce7; color: #15803d; }
  .tag.pattern { background: #fef3c7; color: #b45309; }
  .tag.diff-hard { background: #fee2e2; color: #b91c1c; }
  .tag.diff-easy { background: #e0f2fe; color: #0369a1; }
  .prompt {
    padding: 22px 20px 14px; font-size: 1.35rem; font-weight: 700; line-height: 1.5;
    word-break: break-word; min-height: 96px;
  }
  .hint { padding: 0 20px 8px; color: var(--muted); font-size: .95rem; line-height: 1.6; word-break: break-word; }
  .answer-box { padding: 10px 20px 18px; display: none; }
  .answer-box .aw {
    background: #f0fdf4; border: 1px solid #bbf7d0; border-radius: 12px;
    padding: 14px 16px; font-size: 1.5rem; font-weight: 800; color: #14532d;
    word-break: break-word; text-align: center;
  }
  .answer-box .meta { margin-top: 10px; font-size: .9rem; color: #334155; line-height: 1.7; white-space: pre-wrap; word-break: break-word; }
  .button-row { padding: 0 20px 18px; display: flex; gap: 10px; }
  button {
    border: 0; border-radius: 12px; font-weight: 700; cursor: pointer;
    touch-action: manipulation; font-size: .98rem; color: #fff;
  }
  .btn {
    flex: 1; padding: 15px 8px; font-size: 1.02rem; font-weight: 800;
  }
  .btn.again { background: var(--again); }
  .btn.hard { background: var(--hard); }
  .btn.good { background: var(--good); }
  .btn.easy { background: var(--easy); }
  .reveal {
    width: 100%; display: block; margin: 0; padding: 16px;
    background: var(--brand); color: var(--brand-ink); font-size: 1.05rem; border-radius: 0;
  }
  .toolbar { display: flex; gap: 8px; padding: 12px 14px; align-items: center; flex-wrap: wrap; }
  .toolbar select, .toolbar input[type=number] {
    border: 1px solid var(--line); border-radius: 10px; padding: 9px 10px; font-size: .9rem; background: #fff; color: var(--ink);
  }
  .toolbar label { font-size: .82rem; color: var(--muted); font-weight: 600; }
  .ghost {
    background: #eef2ff; color: #3730a3; border: 1px solid #c7d2fe;
    padding: 9px 14px; border-radius: 10px; font-size: .86rem; font-weight: 700;
  }
  .ghost.warn { background: #fef2f2; color: #991b1b; border-color: #fecaca; }
  .section-title { font-size: .8rem; font-weight: 800; color: var(--muted); letter-spacing: .1em; margin: 18px 6px 8px; }
  .look-row {
    background: #fff; border: 1px solid var(--line); border-radius: 12px; padding: 10px 12px;
    margin-bottom: 8px; display: flex; align-items: center; gap: 10px; cursor: pointer;
  }
  .look-row .hw { font-weight: 700; font-size: 1.02rem; flex: 1; word-break: break-word; }
  .look-row .add { background: var(--brand); color: #fff; border: 0; border-radius: 8px; padding: 8px 12px; font-size: .8rem; font-weight: 700; cursor: pointer; }
  .searchbox { display: flex; gap: 8px; padding: 4px 0 14px; }
  .searchbox input {
    flex: 1; border: 1px solid var(--line); border-radius: 12px; padding: 12px 14px;
    font-size: 1rem; background: #fff;
  }
  .setting-row {
    background: #fff; border: 1px solid var(--line); border-radius: 12px; padding: 12px 14px;
    margin-bottom: 10px; display: flex; align-items: center; gap: 12px;
  }
  .setting-row .lbl { flex: 1; font-weight: 600; font-size: .92rem; }
  .setting-row .sub { color: var(--muted); font-size: .78rem; margin-top: 2px; }
  .toggle { position: relative; width: 46px; height: 26px; flex-shrink: 0; }
  .toggle input { opacity: 0; width: 0; height: 0; }
  .toggle .slider {
    position: absolute; inset: 0; background: #cbd5e1; border-radius: 999px; transition: .2s;
  }
  .toggle .slider::before {
    content: ""; position: absolute; width: 20px; height: 20px; border-radius: 50%;
    background: #fff; top: 3px; left: 3px; transition: .2s;
  }
  .toggle input:checked + .slider { background: var(--brand); }
  .toggle input:checked + .slider::before { transform: translateX(20px); }
  .empty { text-align: center; color: var(--muted); padding: 40px 20px; font-size: .95rem; }
  .empty .big { font-size: 2.2rem; margin-bottom: 10px; }
  nav {
    position: fixed; bottom: 0; left: 0; right: 0; z-index: 20;
    background: rgba(255,255,255,.96); backdrop-filter: blur(10px);
    border-top: 1px solid var(--line);
    display: grid; grid-template-columns: 1fr 1fr 1fr; padding-bottom: var(--safe-b);
  }
  nav button {
    background: none; color: var(--muted); padding: 10px 4px 8px;
    display: flex; flex-direction: column; align-items: center; gap: 3px;
    font-size: .68rem; font-weight: 700;
  }
  nav button .ico { font-size: 1.3rem; }
  nav button.on { color: var(--brand); }
  .row { display: flex; align-items: center; gap: 10px; }
  .stat-line { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; margin: 10px 0; }
  .stat-cell { background: #fff; border: 1px solid var(--line); border-radius: 12px; padding: 10px; text-align: center; }
  .stat-cell .n { font-size: 1.2rem; font-weight: 800; }
  .stat-cell .t { font-size: .72rem; color: var(--muted); font-weight: 600; margin-top: 2px; }
  .rec-row {
    background: #fff; border: 1px solid var(--line); border-radius: 12px; padding: 12px 14px;
    margin-bottom: 8px;
  }
  .rec-row .top { display: flex; align-items: center; gap: 8px; }
  .rec-row .hw { font-weight: 700; flex: 1; word-break: break-word; }
  .rec-row .sub { font-size: .78rem; color: var(--muted); margin-top: 6px; line-height: 1.6; }
  .thin-progress { height: 6px; background: #e2e8f0; border-radius: 999px; overflow: hidden; margin-top: 8px; }
  .thin-progress .fill { height: 100%; background: var(--brand); border-radius: 999px; transition: width .4s; }
  .toast {
    position: fixed; left: 50%; bottom: calc(90px + var(--safe-b)); transform: translateX(-50%);
    background: #0f172a; color: #fff; padding: 10px 18px; border-radius: 999px;
    font-size: .85rem; opacity: 0; transition: opacity .25s; pointer-events: none; z-index: 50;
  }
  .toast.show { opacity: .95; }
</style>
</head>
<body>
<div id="app">
  <header>
    <div class="toprow"><h1>Lexicon</h1><span class="mini" id="hdrList">全部词库</span></div>
    <div class="progress">
      <div class="pill">🆕 新学 <b id="pNew">0</b><span class="mini" id="pNewLim">/20</span></div>
      <div class="pill">🔁 复习 <b id="pRev">0</b><span class="mini" id="pRevLim">/100</span></div>
    </div>
  </header>
  <main id="main"></main>
  <nav>
    <button id="tabStudy" class="on"><span class="ico">🎴</span>学习</button>
    <button id="tabLook"><span class="ico">🔎</span>查词</button>
    <button id="tabStats"><span class="ico">📊</span>记录</button>
  </nav>
  <div class="toast" id="toast"></div>
</div>
<script>
"use strict";
var state = { tab: "study", queue: [], idx: 0, settings: null, lists: [], activeList: 0, typing: "" };

function $(id) { return document.getElementById(id); }
function esc(s) { return String(s == null ? "" : s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); }
function toast(msg) { var t = $("toast"); t.textContent = msg; t.classList.add("show"); setTimeout(function () { t.classList.remove("show"); }, 1600); }
function post(url, data, cb) {
  fetch(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(data) })
    .then(function (r) { return r.json(); })
    .then(function (j) { cb && cb(j); })
    .catch(function () { toast("网络错误"); });
}

function tagHtml(ctype, diff) {
  var c = ctype || "word";
  var d = diff || "easy";
  return '<span class="tag ' + esc(c) + '">' + (c === "word" ? "单词" : c === "phrase" ? "词组" : "句式") + '</span> ' +
         '<span class="tag diff-' + esc(d) + '">' + (d === "hard" ? "困难" : "简单") + '</span>';
}

// ---- header progress ----
function refreshHeader() {
  fetch("/api/settings").then(function (r) { return r.json(); }).then(function (s) {
    state.settings = s;
    $("pNew").textContent = s.new_done; $("pNewLim").textContent = "/" + s.new_per_day;
    $("pRev").textContent = s.review_done; $("pRevLim").textContent = "/" + s.review_per_day;
  }).catch(function () {});
  fetch("/api/wordlists").then(function (r) { return r.json(); }).then(function (ls) {
    state.lists = ls;
    if (ls.length === 0) { $("hdrList").textContent = "全部词库"; return; }
    var cur = null;
    ls.forEach(function (l) { if (l.id === state.activeList) cur = l; });
    $("hdrList").textContent = cur ? cur.name : "全部词库";
  }).catch(function () {});
}

// ---- study tab ----
function loadQueue() {
  var url = "/api/due?limit=60";
  if (state.activeList > 0) url = url + "&list=" + state.activeList;
  fetch(url).then(function (r) { return r.json(); }).then(function (j) {
    state.queue = j.cards || [];
    state.idx = 0;
    if (state.queue.length === 0) renderStudyEmpty();
    else renderCard();
    refreshHeader();
  }).catch(function () { renderStudyEmpty(); });
}

function renderStudyEmpty() {
  var main = $("main");
  main.innerHTML =
    '<div class="cardbox"><div class="empty"><div class="big">🎉</div>' +
    '今天的任务完成啦！<br><br>' +
    '<span class="ghost" onclick="addRandom()">抽一张生词看看</span></div></div>' +
    '<div class="cardbox" style="padding:14px 16px">' +
    '<div class="section-title">复习设置</div>\
    <div class="setting-row"><div class="lbl">每日新学词数<div class="sub">每天最多新认识的单词（含词组/句式）</div></div>' +
    '<input type="number" min="1" max="200" value="' + (state.settings ? state.settings.new_per_day : 20) + '" id="inNew" style="width:70px">' +
    '</div>' +
    '<div class="setting-row"><div class="lbl">每日复习上限<div class="sub">每天处理到期卡片的上限</div></div>' +
    '<input type="number" min="5" max="1000" value="' + (state.settings ? state.settings.review_per_day : 100) + '" id="inRev" style="width:70px">' +
    '</div>' +
    '<button class="reveal" onclick="saveLimits()">保存设定</button>' +
    '<div class="section-title">所在词表</div>' +
    wordlistSelector() +
    '</div>';
}

function wordlistSelector() {
  var opts = '<option value="0">全部词表</option>';
  (state.lists || []).forEach(function (l) {
    opts += '<option value="' + l.id + '" ' + (l.id === state.activeList ? "selected" : "") + '>' + esc(l.name) + "（" + l.size + "词）</option>";
  });
  return '<div class="toolbar"><select id="selList" onchange="changeList(this.value)">' + opts + "</select></div>";
}

function changeList(v) {
  state.activeList = parseInt(v, 10) || 0;
  loadQueue();
}

function saveLimits() {
  var n = parseInt($("inNew").value, 10) || 20;
  var r = parseInt($("inRev").value, 10) || 100;
  post("/api/settings", { new_per_day: n, review_per_day: r }, function (j) {
    toast(j.ok ? "已保存" : "保存失败");
    refreshHeader();
  });
}

function addRandom() {
  post("/api/random", {}, function (j) { if (j && j.ok) { toast("已加入: " + j.word); loadQueue(); } });
}

function renderCard() {
  var c = state.queue[state.idx];
  if (!c) { renderStudyEmpty(); return; }
  var p = c.prompt || {};
  var main = $("main");
  var meta = p.extra || {};
  var metaHtml = "";
  if (meta.phonetic) metaHtml += esc(meta.phonetic) + " ";
  if (meta.pos) metaHtml += "(" + esc(meta.pos) + ") ";
  if (meta.senses_zh && meta.senses_zh.length) metaHtml += "\n" + esc(meta.senses_zh.join("；"));
  if (meta.example) metaHtml += "\n例: " + esc(meta.example) + (meta.example_zh ? " " + esc(meta.example_zh) : "");
  if (meta.source && meta.source !== c.headword) metaHtml += "\n源自: " + esc(meta.source);

  main.innerHTML =
    '<div class="cardbox">' +
    '<div class="toolbar">' + tagHtml(c.card_type, c.difficulty) +
    '<span class="ghost" onclick="flipDiff(' + c.id + ')">切换难度</span>' +
    '<span class="ghost warn" onclick="showHistory(' + c.id + ')">📜 记录</span></div>' +
    '<div class="prompt">' + esc(p.question || c.headword) + '</div>' +
    '<div class="hint">' + (p.hint ? "💡 " + esc(p.hint) : "") + '</div>' +
    '<div id="ansBox" class="answer-box">' +
      '<div class="aw">' + esc(p.answer || c.headword) + '</div>' +
      '<div class="meta">' + metaHtml + '</div>' +
    '</div>' +
    '<button class="reveal" id="btnReveal" onclick="reveal(' + c.id + ')">显示答案</button>' +
    '<div class="button-row" id="gradeRow" style="display:none">' +
      '<button class="btn again" onclick="grade(' + c.id + ',0)">😵 忘记</button>' +
      '<button class="btn hard" onclick="grade(' + c.id + ',1)">🤔 困难</button>' +
      '<button class="btn good" onclick="grade(' + c.id + ',2)">😊 认识</button>' +
      '<button class="btn easy" onclick="grade(' + c.id + ',3)">😎 简单</button>' +
    '</div></div>' +
    '<div class="cardbox" style="padding:10px 16px"><span style="font-size:.8rem;color:var(--muted)">' +
    (state.idx + 1) + " / " + state.queue.length + " · 点「记录」查看该词全部学习历史与难度</span></div>";
  // input-mode optional: typing is skipped, reveal+grade is the flow
}

function reveal() {
  $("ansBox").style.display = "block";
  $("btnReveal").style.display = "none";
  $("gradeRow").style.display = "flex";
}

function grade(id, g) {
  post("/api/grade", { id: id, grade: g }, function (j) {
    state.idx++;
    if (state.idx >= state.queue.length) loadQueue();
    else renderCard();
    refreshHeader();
  });
}

function flipDiff(id) {
  var cur = state.queue.find(function (c) { return c.id === id; });
  var next = (cur && cur.difficulty === "hard") ? "easy" : "hard";
  post("/api/card/difficulty", { id: id, difficulty: next }, function () {
    toast("难度已切换为" + (next === "hard" ? "困难" : "简单"));
    loadQueue();
  });
}

function showHistory(id) {
  fetch("/api/card/history?id=" + id).then(function (r) { return r.json(); }).then(function (log) {
    var h = "该卡复习历史（共 " + log.length + " 次）:\n";
    log.forEach(function (l) {
      var g = ["忘记","困难","认识","简单"][l.grade] || l.grade;
      h += (l.is_new ? "🆕" : " ") + " " + l.at + "  " + g + "\n";
    });
    alert(h);
  }).catch(function () { toast("暂无记录"); });
}

// ---- look up tab ----
function switchTab(tab) {
  state.tab = tab;
  $("tabStudy").className = tab === "study" ? "on" : "";
  $("tabLook").className = tab === "look" ? "on" : "";
  $("tabStats").className = tab === "stats" ? "on" : "";
  if (tab === "study") loadQueue();
  if (tab === "look") renderLook();
  if (tab === "stats") renderStats();
}

function renderLook() {
  var main = $("main");
  main.innerHTML =
    '<div class="cardbox" style="padding:14px 16px">' +
    '<div class="searchbox"><input id="q" placeholder="输入英文或中文搜索，回车查看…" onkeydown="if(event.key===\'Enter\')doSearch()"></div>' +
    '<button class="reveal" style="border-radius:12px" onclick="doSearch()">搜索</button>' +
    '<div class="section-title">最近查过的词典结果</div><div id="res"></div></div>' +
    '<div class="cardbox" style="padding:14px 16px"><div class="section-title" style="margin-top:0">💡 学单词三步走</div>' +
    '<p style="font-size:.9rem;color:var(--muted);line-height:1.7;margin:0">查词 → 加入复习 → 按"中译英"考察。\n简单模式给例句+中文回忆单词；困难模式只给首字母+中文意思。词组与句式也会出现在复习队列里。</p></div>';
}

function doSearch() {
  var q = $("q").value.trim();
  if (!q) return;
  fetch("/api/search?q=" + encodeURIComponent(q)).then(function (r) { return r.json(); }).then(function (rows) {
    var box = $("res");
    if (!rows || rows.length === 0) { box.innerHTML = '<div class="empty">没有匹配的词条</div>'; return; }
    var html = "";
    rows.slice(0, 30).forEach(function (row) {
      html += '<div class="look-row"><div class="hw">' + esc(row.headword) + '</div>' +
              '<button class="add" onclick="addWord(\'' + esc(row.headword).replace(/'/g, "\\'") + '\')">加入</button></div>';
    });
    box.innerHTML = html;
  }).catch(function () { toast("搜索失败"); });
}

function addWord(w) {
  post("/api/add", { word: w }, function (j) {
    toast(j && j.ok ? "已加入复习" + " · 再去学习页查看" : (j && j.error ? j.error : "失败"));
  });
}

// ---- stats tab ----
function renderStats() {
  var main = $("main");
  main.innerHTML =
    '<div class="cardbox" style="padding:14px 16px"><div class="section-title" style="margin-top:0">今日进度</div>' +
    '<div class="stat-line">' +
      '<div class="stat-cell"><div class="n" id="stNew">0/0</div><div class="t">新学</div></div>' +
      '<div class="stat-cell"><div class="n" id="stRev">0/0</div><div class="t">复习</div></div>' +
      '<div class="stat-cell"><div class="n" id="stDue">?</div><div class="t">词数</div></div>' +
    '</div>' +
    '<div class="section-title">词库概况</div>' +
    '<div class="stat-line">' +
      '<div class="stat-cell"><div class="n" id="stAll">?</div><div class="t">总词条</div></div>' +
      '<div class="stat-cell"><div class="n" id="stCards">?</div><div class="t">复习卡</div></div>' +
      '<div class="stat-cell"><div class="n" id="stPhrases">?</div><div class="t">词组/句式库</div></div>' +
    '</div></div>' +
    '<div class="cardbox" style="padding:14px 16px">' +
    '<div class="section-title" style="margin-top:0">📖 学习记录（每词难度与历史）</div>' +
    '<div id="recList"></div></div>' +
    '<div class="cardbox" style="padding:14px 16px"><div class="section-title" style="margin-top:0">词表管理</div><div id="wlList"></div></div>';

  refreshHeader();
  fetch("/api/stats").then(function (r) { return r.json(); }).then(function (s) {
    $("stAll").textContent = s.words; $("stCards").textContent = s.cards; $("stDue").textContent = s.due;
  }).catch(function () {});
  fetch("/api/phrase_count").then(function (r) { return r.json(); }).then(function (j) {
    $("stPhrases").textContent = j.count;
  }).catch(function () { $("stPhrases").textContent = "-"; });

  if (state.settings) {
    $("stNew").textContent = state.settings.new_done + "/" + state.settings.new_per_day;
    $("stRev").textContent = state.settings.review_done + "/" + state.settings.review_per_day;
  }
  loadRecords();
  loadWlList();
}

function loadRecords() {
  var box = $("recList");
  if (!box) return;
  fetch("/api/cards?limit=200").then(function (r) { return r.json(); }).then(function (cards) {
    if (!cards || cards.length === 0) { box.innerHTML = '<div class="empty">还没有学习记录，先去学习页加几张卡吧</div>'; return; }
    var html = "";
    cards.forEach(function (c) {
      var status = { New: "新词", Learning: "学习中", Review: "已入期", Relearning: "重学" }[c.status] || c.status;
      var last = c.last_review ? ("上次 " + c.last_review) : "未复习";
      var miss = c.lapses > 0 ? "忘记" + c.lapses + "次" : "无遗忘";
      html += '<div class="rec-row"><div class="top">' + tagHtml(c.card_type, c.difficulty) +
              '<span class="hw">' + esc(c.headword) + '</span></div>' +
              '<div class="sub">' + status + " · 已学 " + c.reps + " 次 · " + miss + " · " + last + " · 到期 " + (c.due || "—") +
              ' <span class="ghost warn" style="padding:3px 8px;font-size:.72rem;cursor:pointer" onclick="showHistory(' + c.id + ')">历史</span>' +
              '</div><div class="thin-progress"><div class="fill" style="width:' + Math.min(100, c.reps * 12) + '%"></div></div></div>';
    });
    box.innerHTML = html;
  }).catch(function () { box.innerHTML = '<div class="empty">读取失败</div>'; });
}

function loadWlList() {
  var box = $("wlList");
  if (!box) return;
  fetch("/api/wordlists").then(function (r) { return r.json(); }).then(function (ls) {
    if (!ls || ls.length === 0) { box.innerHTML = '<div class="empty">暂无词表。用命令行导入：lexicon-cli wordlist oxford.db 高考 <词表文件></div>'; return; }
    var html = "";
    ls.forEach(function (l) {
      html += '<div class="setting-row"><div class="lbl">' + esc(l.name) + '<div class="sub">' + l.size + " 个词</div></div>" +
              '<button class="ghost" onclick="activateList(' + l.id + ')">' + (l.id === state.activeList ? "✓ 当前" : "设为范围") + '</button></div>';
    });
    box.innerHTML = html;
  }).catch(function () {});
}

function activateList(id) {
  post("/api/settings", { active_wordlist: String(id) }, function () {
    state.activeList = id;
    toast("学习范围已切换");
    loadWlList();
    refreshHeader();
  });
}

// boot
$("tabStudy").onclick = function () { switchTab("study"); };
$("tabLook").onclick = function () { switchTab("look"); };
$("tabStats").onclick = function () { switchTab("stats"); };
refreshHeader();
loadQueue();

if ('serviceWorker' in navigator) {
  navigator.serviceWorker.register('/sw.js').catch(function () {});
}
</script>
</body>
</html>"####;
