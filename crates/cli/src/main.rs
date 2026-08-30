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
                .and_then(|q| q.split('=').nth(1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(100);
            match d.due_card_details(&now, None, limit) {
                Ok(cards) => {
                    let arr: Vec<serde_json::Value> = cards
                        .into_iter()
                        .map(|(id, headword, def)| {
                            let text = resolve_def(&d, &headword, &def);
                            serde_json::json!({"id": id, "headword": headword, "text": plainify(&text)})
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
<link rel="apple-touch-icon" href="/icon.svg">
<title>Lexicon</title>
<style>
  :root {
    --bg: #f8fafc; --card: #ffffff; --ink: #0f172a; --muted: #64748b;
    --brand: #2563eb; --brand-ink: #ffffff;
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
    color: #fff; padding: calc(14px + var(--safe-t)) 18px 30px 18px;
    display: flex; align-items: baseline; justify-content: space-between;
  }
  header h1 { margin: 0; font-size: 1.35rem; font-weight: 800; letter-spacing: 0.02em; }
  header .stat { font-size: 0.85rem; opacity: 0.92; font-weight: 600; }
  main { flex: 1; overflow-y: auto; padding: 0 14px 90px; }
  .card {
    background: var(--card); border-radius: 16px;
    box-shadow: 0 1px 3px rgba(15,23,42,.08), 0 6px 18px rgba(15,23,42,.06);
    margin: -14px auto 14px; max-width: 640px; overflow: hidden;
    border: 1px solid #e2e8f0;
  }
  .headword {
    padding: 20px 20px 10px; font-size: 1.9rem; font-weight: 800; color: var(--ink);
    line-height: 1.15; word-break: break-word;
  }
  .reveal {
    margin: 4px 20px 20px; padding: 14px; width: calc(100% - 40px);
    border: 0; border-radius: 12px; background: var(--brand); color: var(--brand-ink);
    font-size: 1.05rem; font-weight: 700; cursor: pointer; touch-action: manipulation;
  }
  .def {
    padding: 0 20px 16px; color: #1e293b; font-size: 1rem; line-height: 1.7;
    white-space: pre-wrap; word-break: break-word; max-height: 45vh; overflow-y: auto;
  }
  .grades { display: grid; grid-template-columns: 1fr 1fr 1fr 1fr; gap: 8px; padding: 0 20px 20px; }
  .grades button {
    border: 0; border-radius: 12px; padding: 14px 4px; font-size: 0.98rem; font-weight: 800;
    color: #fff; cursor: pointer; touch-action: manipulation; min-height: 52px;
  }
  .g-again { background: var(--again); } .g-hard { background: var(--hard); }
  .g-good { background: var(--good); } .g-easy { background: var(--easy); }
  .empty { padding: 48px 24px; text-align: center; color: var(--muted); font-size: 1rem; }
  .empty .big { font-size: 2.2rem; margin-bottom: 8px; }
  .searchbox { display: flex; gap: 8px; padding: 14px; }
  .searchbox input {
    flex: 1; border: 1px solid #cbd5e1; border-radius: 12px; padding: 12px 14px;
    font-size: 1.05rem; background: #fff; color: var(--ink); outline: none;
  }
  .searchbox input:focus { border-color: var(--brand); box-shadow: 0 0 0 3px rgba(37,99,235,.15); }
  .results { list-style: none; margin: 0; padding: 0 14px; }
  .results li {
    background: #fff; border: 1px solid #e2e8f0; border-radius: 12px;
    padding: 14px 16px; margin-bottom: 10px; cursor: pointer;
  }
  .results li b { font-size: 1.15rem; display: block; margin-bottom: 4px; }
  .results li p { margin: 0; color: var(--muted); font-size: 0.9rem; line-height: 1.5;
    display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .detail { background: #fff; border: 1px solid #e2e8f0; border-radius: 16px; padding: 16px; margin: 0 14px 14px; }
  .detail h2 { margin: 0 0 8px; font-size: 1.5rem; }
  .detail .def { padding: 0; max-height: 40vh; }
  .addbtn {
    width: 100%; border: 0; border-radius: 12px; padding: 14px; margin-top: 12px;
    background: var(--brand); color: var(--brand-ink); font-size: 1.05rem; font-weight: 700; cursor: pointer;
  }
  .prog {
    padding: 10px 20px 0; font-size: 0.85rem; color: var(--brand); font-weight: 700;
  }
  nav {
    position: fixed; left: 0; right: 0; bottom: 0; z-index: 50;
    background: rgba(255,255,255,.92); backdrop-filter: blur(12px);
    border-top: 1px solid #e2e8f0; display: flex; padding-bottom: var(--safe-b);
  }
  nav button {
    flex: 1; border: 0; background: none; padding: 12px 0; font-size: 0.95rem; font-weight: 700;
    color: var(--muted); cursor: pointer; position: relative;
  }
  nav button.active { color: var(--brand); }
  nav button.active::after {
    content: ""; position: absolute; left: 50%; transform: translateX(-50%); bottom: 4px;
    width: 32px; height: 3px; border-radius: 2px; background: var(--brand);
  }
  .toast {
    position: fixed; left: 50%; bottom: calc(76px + var(--safe-b)); transform: translateX(-50%);
    background: #0f172a; color: #fff; padding: 10px 18px; border-radius: 10px;
    font-size: 0.9rem; opacity: 0; pointer-events: none; transition: opacity .25s; z-index: 99;
  }
  .toast.show { opacity: .95; }
</style>
</head>
<body>
<div id="app">
  <header>
    <h1>Lexicon</h1>
    <span class="stat" id="stat">…</span>
  </header>
  <main id="main"></main>
  <nav>
    <button id="tab-review" class="active">复习</button>
    <button id="tab-lookup">查词</button>
  </nav>
  <div class="toast" id="toast"></div>
</div>
<script>
(function () {
  var main = document.getElementById('main');
  var toast = document.getElementById('toast');
  var statEl = document.getElementById('stat');
  var current = null;  // card under review: {id, headword, text}
  var revealed = false;

  function showToast(msg) {
    toast.textContent = msg;
    toast.classList.add('show');
    clearTimeout(showToast._t);
    showToast._t = setTimeout(function () { toast.classList.remove('show'); }, 1800);
  }
  function el(tag, cls, text) {
    var d = document.createElement(tag);
    if (cls) d.className = cls;
    if (text !== undefined) d.textContent = text;
    return d;
  }

  /* ---------- stats ---------- */
  function refreshStat() {
    fetch('/api/stats').then(function (r) { return r.json(); }).then(function (s) {
      statEl.textContent = '待复习 ' + (s.due || 0) + ' · ' + (s.cards || 0) + ' 卡';
    }).catch(function () { statEl.textContent = '复习中'; });
  }

  /* ---------- review flow ---------- */
  function loadReview() {
    fetch('/api/due?limit=1').then(function (r) { return r.json(); }).then(function (cards) {
      refreshStat();
      if (!cards.length) {
        main.innerHTML = '';
        var card = el('div', 'card');
        var e = el('div', 'empty');
        var big = el('div', 'big'); big.textContent = '🎉';
        e.appendChild(big);
        e.appendChild(document.createTextNode('暂无到期卡片'));
        card.appendChild(e);
        var again = el('button', 'reveal', '抽一张生词看看');
        again.onclick = addRandom;
        card.appendChild(again);
        main.appendChild(card);
        current = null; revealed = false;
        return;
      }
      current = cards[0]; revealed = false;
      renderCard();
    }).catch(function () { main.innerHTML = '<div class="empty">无法连接服务</div>'; });
  }

  function renderCard() {
    main.innerHTML = '';
    var card = el('div', 'card');
    var prog = el('div', 'prog');
    prog.textContent = '点击下方按钮显示释义';
    card.appendChild(prog);
    var hw = el('div', 'headword'); hw.textContent = current.headword;
    card.appendChild(hw);
    if (revealed) {
      var def = el('div', 'def');
      def.textContent = (current.text && current.text.trim()) ? current.text : '（词典中未找到释义）';
      card.appendChild(def);
      var grades = el('div', 'grades');
      var opts = [['再认一次', 0, 'g-again'], ['困难', 1, 'g-hard'], ['认识', 2, 'g-good'], ['简单', 3, 'g-easy']];
      opts.forEach(function (o) {
        var b = el('button', o[2], o[0]);
        b.onclick = function () { grade(o[1]); };
        grades.appendChild(b);
      });
      card.appendChild(grades);
    } else {
      var b = el('button', 'reveal', '显示释义');
      b.onclick = function () { revealed = true; renderCard(); };
      card.appendChild(b);
    }
    main.appendChild(card);
  }

  function grade(g) {
    if (!current) return;
    var id = current.id;
    fetch('/api/grade', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: id, grade: g })
    }).then(function (r) { return r.json(); }).then(function (d) {
      loadReview();
      main.scrollTop = 0;
    }).catch(loadReview);
  }

  /* ---------- add a random unseen word (when nothing is due) ---------- */
  function addRandom() {
    fetch('/api/stats').then(function (r) { return r.json(); }).then(function (s) {
      if (!s.words) { showToast('词库为空'); return; }
      var letters = 'abcdefghijklmnopqrstuvwxyz';
      var l = letters.charAt(Math.floor(Math.random() * letters.length));
      return fetch('/api/search?q=' + l).then(function (r) { return r.json(); }).then(function (list) {
        if (!list.length) { showToast('没有可添加的词'); return; }
        var item = list[Math.floor(Math.random() * list.length)];
        return fetch('/api/add', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ word: item.headword })
        }).then(function () {
          showToast('已加入: ' + item.headword);
          loadReview();
        });
      });
    }).catch(function () { showToast('操作失败'); });
  }

  /* ---------- lookup tab ---------- */
  var input, results, detail;
  function buildLookup() {
    main.innerHTML = '';
    var box = el('div', 'searchbox');
    input = el('input');
    input.type = 'search'; input.placeholder = '输入单词或前缀，如 apple';
    box.appendChild(input);
    results = el('ul', 'results');
    detail = el('div', 'detail');
    detail.style.display = 'none';
    main.appendChild(box); main.appendChild(results); main.appendChild(detail);
    var t = null;
    input.addEventListener('input', function () {
      clearTimeout(t);
      var q = input.value.trim();
      if (!q) { results.innerHTML = ''; return; }
      t = setTimeout(function () { doSearch(q); }, 200);
    });
    input.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { e.preventDefault(); doSearch(input.value.trim()); }
    });
    setTimeout(function () { input.focus(); }, 100);
  }
  function doSearch(q) {
    fetch('/api/search?q=' + encodeURIComponent(q)).then(function (r) { return r.json(); }).then(function (list) {
      results.innerHTML = '';
      detail.style.display = 'none';
      if (!list.length) {
        var li = el('li'); li.style.cursor = 'default';
        li.appendChild(document.createTextNode('无匹配结果'));
        results.appendChild(li);
        return;
      }
      list.forEach(function (item) {
        var li = el('li');
        var b = el('b'); b.textContent = item.headword; li.appendChild(b);
        var p = el('p');
        p.textContent = (item.text && item.text.trim()) ? item.text.slice(0, 140) : '（无释义）';
        li.appendChild(p);
        li.onclick = function () { showDetail(item.headword); };
        results.appendChild(li);
      });
    }).catch(function () { results.innerHTML = '<li>搜索失败</li>'; });
  }
  function showDetail(w) {
    fetch('/api/lookup?w=' + encodeURIComponent(w)).then(function (r) { return r.json(); }).then(function (d) {
      results.innerHTML = '';
      detail.innerHTML = '';
      detail.style.display = '';
      var h = el('h2'); h.textContent = d.headword || w; detail.appendChild(h);
      var def = el('div', 'def');
      def.textContent = (d.text && d.text.trim()) ? d.text : '（无释义）';
      detail.appendChild(def);
      var add = el('button', 'addbtn', '加入复习');
      add.onclick = function () {
        fetch('/api/add', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ word: d.headword || w })
        }).then(function (r) { return r.json(); }).then(function (x) {
          if (x && x.ok) { showToast('已加入复习'); }
          else { showToast((x && x.error) || '无法加入'); }
        });
      };
      detail.appendChild(add);
      main.scrollTop = 0;
    });
  }

  /* ---------- tabs ---------- */
  function setTab(t) {
    document.getElementById('tab-review').classList.toggle('active', t === 'review');
    document.getElementById('tab-lookup').classList.toggle('active', t === 'lookup');
    if (t === 'review') loadReview();
    else buildLookup();
  }
  document.getElementById('tab-review').onclick = function () { setTab('review'); };
  document.getElementById('tab-lookup').onclick = function () { setTab('lookup'); };

  /* ---------- pwa ---------- */
  if ('serviceWorker' in navigator) {
    window.addEventListener('load', function () {
      navigator.serviceWorker.register('/sw.js').catch(function () {});
    });
  }

  /* ---------- boot ---------- */
  loadReview();
})();
</script>
</body>
</html>
"####;
