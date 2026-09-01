//! HTTP serving for the built-in PWA. Interaction layer only: routing,
//! query/body parsing and JSON shaping. All business logic lives in
//! lexicon-core; HTML/JS/CSS live in the assets/ directory (include_str!).

use lexicon_core::model::Difficulty;
use lexicon_core::prompt::{build_card_prompt, plainify, resolve_def};
use lexicon_core::storage::Db;

const INDEX_HTML: &str = include_str!("../assets/index.html");
const MANIFEST_JSON: &str = include_str!("../assets/manifest.json");
const SW_JS: &str = include_str!("../assets/sw.js");
const ICON_SVG: &str = include_str!("../assets/icon.svg");

/// Serve the built-in single-page UI over HTTP.
/// Listens on 0.0.0.0 so phones/tablets on the same LAN can reach it.
pub fn serve(db: &str, port: u16) -> anyhow::Result<()> {
    let server =
        tiny_http::Server::http(("0.0.0.0", port)).map_err(|e| anyhow::anyhow!(e.to_string()))?;
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

type Resp = tiny_http::Response<std::io::Cursor<Vec<u8>>>;

fn json_response(status: u16, body: String) -> Resp {
    text_response(status, "application/json; charset=utf-8", body)
}

fn html_response(status: u16, body: String) -> Resp {
    text_response(status, "text/html; charset=utf-8", body)
}

fn text_response(status: u16, content_type: &'static str, body: String) -> Resp {
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
    tiny_http::Response::new(tiny_http::StatusCode(status), headers, Cursor::new(bytes), Some(len), None)
}

fn handle(d: &Db, req: &mut tiny_http::Request) -> Resp {
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

        // ---- review queue ----
        (&tiny_http::Method::Get, "/api/due") => {
            let now = lexicon_core::sm2::now();
            let limit = query_i64(query, "limit").unwrap_or(50);
            let wl = query_i64(query, "list")
                .or_else(|| {
                    d.get_setting("active_wordlist")
                        .ok()
                        .flatten()
                        .and_then(|v| v.parse::<i64>().ok())
                })
                .or(Some(0));
            match d.daily_due_queue(&now, limit, wl) {
                Ok((cards, new_left, review_left)) => {
                    let arr: Vec<serde_json::Value> = cards
                        .into_iter()
                        .map(|(id, headword, ctype, diff, phrase)| {
                            let prompt =
                                build_card_prompt(d, id, &headword, &ctype, &diff, &phrase);
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

        // ---- grading ----
        (&tiny_http::Method::Post, "/api/grade") => {
            with_json_body(req, |v| {
                let id = v["id"].as_i64().unwrap_or(0);
                let grade = v["grade"].as_u64().unwrap_or(2).min(3) as u8;
                match d.grade_card(id, grade) {
                    Ok(()) => json_response(200, "{\"ok\":true}".into()),
                    Err(e) => json_response(200, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
                }
            })
        }

        // ---- prompt difficulty override (easy/medium/hard) ----
        (&tiny_http::Method::Post, "/api/card/difficulty") => {
            with_json_body(req, |v| {
                let id = v["id"].as_i64().unwrap_or(0);
                let diff = Difficulty::parse(v["difficulty"].as_str().unwrap_or("easy"));
                match d.set_card_difficulty(id, diff) {
                    Ok(()) => json_response(200, "{\"ok\":true}".into()),
                    Err(e) => json_response(200, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
                }
            })
        }

        (&tiny_http::Method::Get, "/api/card/history") => {
            let id = query_i64(query, "id").unwrap_or(0);
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

        // ---- settings ----
        (&tiny_http::Method::Get, "/api/settings") => {
            let (new_pd, rev_pd) = d.daily_limits().unwrap_or((20, 100));
            let (new_done, rev_done) = d.today_progress().unwrap_or((0, 0));
            let active = d
                .get_setting("active_wordlist")
                .unwrap_or_default()
                .and_then(|v| v.parse::<i64>().ok());
            json_response(
                200,
                serde_json::json!({
                    "new_per_day": new_pd,
                    "review_per_day": rev_pd,
                    "new_done": new_done,
                    "review_done": rev_done,
                    "active_wordlist": active.unwrap_or(0),
                })
                .to_string(),
            )
        }
        (&tiny_http::Method::Post, "/api/settings") => {
            with_json_body(req, |v| {
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
            })
        }

        // ---- wordlists ----
        (&tiny_http::Method::Get, "/api/wordlists") => match d.wordlists() {
            Ok(lists) => {
                let arr: Vec<serde_json::Value> = lists
                    .iter()
                    .map(|w| serde_json::json!({"id": w.id, "name": w.name, "size": w.size}))
                    .collect();
                json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
            }
            Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
        },

        // ---- records ----
        (&tiny_http::Method::Get, "/api/cards") => {
            let limit = query_i64(query, "limit").unwrap_or(200);
            match d.card_records(limit) {
                Ok(rows) => {
                    let arr: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|(id, headword, ctype, diff, status, reps, lapses, due, last, _)| {
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

        // ---- dictionary ----
        (&tiny_http::Method::Get, "/api/search") => {
            let q = percent_decode(query_param(query, "q"));
            match d.search(&q, 50) {
                Ok(rows) => {
                    let arr: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|(headword, def)| {
                            let text = resolve_def(d, &headword, def.as_deref().unwrap_or(""));
                            serde_json::json!({"headword": headword, "text": plainify(&text)})
                        })
                        .collect();
                    json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/lookup") => {
            let w = percent_decode(query_param(query, "w"));
            match d.lookup_resolved(&w) {
                Ok(Some(def)) => json_response(
                    200,
                    serde_json::json!({"headword": w, "text": plainify(&def), "found": true})
                        .to_string(),
                ),
                Ok(None) => json_response(
                    200,
                    serde_json::json!({"headword": w, "text": null, "found": false}).to_string(),
                ),
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Post, "/api/add") => {
            with_json_body(req, |v| {
                let word = v["word"].as_str().unwrap_or("").to_string();
                match d.add_new_card(&word) {
                    Ok(id) if id > 0 => json_response(200, format!("{{\"ok\":true,\"id\":{id}}}")),
                    Ok(_) => json_response(
                        404,
                        format!("{{\"ok\":false,\"error\":\"word not in dictionary: {word}\"}}"),
                    ),
                    Err(e) => json_response(500, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
                }
            })
        }
        (&tiny_http::Method::Post, "/api/random") => match d.random_word() {
            Ok(Some(w)) => match d.add_new_card(&w) {
                Ok(id) if id > 0 => {
                    json_response(200, format!("{{\"ok\":true,\"word\":\"{w}\",\"id\":{id}}}"))
                }
                _ => json_response(200, "{\"ok\":false}".into()),
            },
            _ => json_response(200, "{\"ok\":false}".into()),
        },

        // ---- phrase learning ----
        (&tiny_http::Method::Get, "/api/phrase_count") => {
            let n = d.phrase_count().unwrap_or(0);
            json_response(200, format!("{{\"count\":{n}}}"))
        }
        (&tiny_http::Method::Get, "/api/phrases") => {
            let limit = query_i64(query, "limit").unwrap_or(30);
            let offset = query_i64(query, "offset").unwrap_or(0);
            match d.all_phrases(limit, offset) {
                Ok(rows) => {
                    let arr: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|(id, source, ptype, text, def_zh)| {
                            serde_json::json!({
                                "id": id, "source": source, "phrase_type": ptype,
                                "text": text, "def_zh": def_zh,
                            })
                        })
                        .collect();
                    json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        (&tiny_http::Method::Get, "/api/phrases/for") => {
            let w = percent_decode(query_param(query, "word"));
            match d.phrases_for(&w) {
                Ok(rows) => {
                    let arr: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|(_id, ptype, text, de, dz, xe, xz)| {
                            serde_json::json!({
                                "phrase_type": ptype, "text": text,
                                "def_en": de, "def_zh": dz,
                                "example_en": xe, "example_zh": xz,
                            })
                        })
                        .collect();
                    json_response(200, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
                }
                Err(e) => json_response(500, format!("{{\"error\": \"{e}\"}}")),
            }
        }
        // Add a phrase/pattern card: {source, text, type: "phrase"|"pattern"}
        (&tiny_http::Method::Post, "/api/phrase/add") => {
            with_json_body(req, |v| {
                let source = v["source"].as_str().unwrap_or("").trim().to_string();
                let text = v["text"].as_str().unwrap_or("").trim().to_string();
                let ctype = match v["type"].as_str().unwrap_or("phrase") {
                    "pattern" => lexicon_core::model::CardType::Pattern,
                    _ => lexicon_core::model::CardType::Phrase,
                };
                if source.is_empty() || text.is_empty() {
                    return json_response(400, "{\"ok\":false,\"error\":\"source and text required\"}".into());
                }
                match d.add_new_card_full(
                    &source,
                    ctype,
                    lexicon_core::model::Difficulty::Easy,
                    &source,
                    &text,
                ) {
                    Ok(id) if id > 0 => {
                        json_response(200, format!("{{\"ok\":true,\"id\":{id}}}"))
                    }
                    Ok(_) => json_response(
                        200,
                        "{\"ok\":false,\"error\":\"card already exists\"}".into(),
                    ),
                    Err(e) => json_response(500, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
                }
            })
        }
        // Rebuild the whole phrase bank from the dictionary (fast bulk op).
        (&tiny_http::Method::Post, "/api/phrases/rebuild") => {
            match lexicon_core::study::rebuild_phrase_bank(d) {
                Ok(n) => json_response(200, format!("{{\"ok\":true,\"count\":{n}}}")),
                Err(e) => json_response(500, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
            }
        }

        // ---- stats ----
        (&tiny_http::Method::Get, "/api/stats") => {
            let now = lexicon_core::sm2::now();
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

/// Read the request body and answer with `f(body_json)`.
fn with_json_body(req: &mut tiny_http::Request, f: impl FnOnce(&serde_json::Value) -> Resp) -> Resp {
    let body = read_body(req);
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => f(&v),
        Err(e) => json_response(400, format!("{{\"ok\":false,\"error\":\"{e}\"}}")),
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

fn query_param<'a>(query: Option<&'a str>, key: &str) -> Option<&'a str> {
    query.and_then(|q| {
        q.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == key).then_some(v)
        })
    })
}

fn query_i64(query: Option<&str>, key: &str) -> Option<i64> {
    query_param(query, key).and_then(|v| v.parse().ok())
}

/// Decode percent-encoded query values ("a%20b" -> "a b", "+" -> " ").
fn percent_decode(s: Option<&str>) -> String {
    let s = s.unwrap_or("");
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
