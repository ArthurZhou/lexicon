//! MCP (Model Context Protocol) server for lexicon.
//!
//! Speaks JSON-RPC 2.0 over newline-delimited stdio — the same protocol the
//! official MCP SDKs use for local servers — so any MCP client (Claude
//! Desktop, Cursor, Claude Code, etc.) can drive the dictionary and the
//! spaced-repetition study queue as AI-callable tools.
//!
//! Run:  lexicon-cli mcp <db>

use lexicon_core::storage::Db;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

pub fn run(d: &Db) -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(resp) = handle(&line, d) {
            writeln!(out, "{resp}")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// Process one inbound JSON-RPC line. Returns the response to write (if any).
fn handle(line: &str, d: &Db) -> Option<String> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Some(error_response(Value::Null, -32700, "parse error")),
    };
    let id = msg.get("id").cloned();
    let method = msg
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    // Notifications carry no id and expect no response.
    let Some(id) = id else { return None; };

    let resp = match method.as_str() {
        "initialize" => json_response(id, initialize_result(&params)),
        "tools/list" => json_response(id, tools_list()),
        "tools/call" => tools_call(id, &params, d),
        "ping" => json_response(id, json!({})),
        m => error_response(id, -32601, format!("method not found: {m}")),
    };
    Some(resp)
}

fn initialize_result(params: &Value) -> Value {
    let proto = params
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("2025-03-26")
        .to_string();
    json!({
        "protocolVersion": proto,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "lexicon", "version": "0.1.0" },
    })
}

fn tools_call(id: Value, params: &Value, d: &Db) -> String {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let (text, is_error) = dispatch_tool(&name, &args, d);
    json_response(
        id,
        json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error }),
    )
}

fn json_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i64, message: impl AsRef<str>) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.as_ref() },
    })
    .to_string()
}

/// JSON Schema helper for a tool whose args are a flat object of strings/numbers.
fn str_prop(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}
fn num_prop(desc: &str) -> Value {
    json!({ "type": "number", "description": desc })
}
fn obj(properties: Value, required: Vec<&str>) -> Value {
    let req: Vec<String> = required.iter().map(|s| s.to_string()).collect();
    json!({ "type": "object", "properties": properties, "required": req, "additionalProperties": false })
}

fn tools_list() -> Value {
    let tools = vec![
        json!({
            "name": "lookup",
            "description": "查词典：返回某词的音标、词性、中文释义、例句与固定搭配（中英）。词典来自本机牛津 MDX。",
            "inputSchema": obj(json!({ "word": str_prop("要查询的单词或词组（如 apple / give sb a break）") }), vec!["word"]),
        }),
        json!({
            "name": "search",
            "description": "前缀搜索词条，用于找词/补全（如搜 \"ab\" 得到 ability, able 等）。",
            "inputSchema": obj(json!({ "prefix": str_prop("搜索前缀，可含 % 通配"), "limit": num_prop("返回条数上限，默认 20") }), vec!["prefix"]),
        }),
        json!({
            "name": "stats",
            "description": "词典规模统计：总词条数、已加入复习的卡片数、今日待复习数。",
            "inputSchema": obj(json!({}), vec![]),
        }),
        json!({
            "name": "add_card",
            "description": "把一个单词（或词组/句式）加入复习队列。card_type: word|phrase|pattern；difficulty: easy|hard；phrase 为词组/句式的题干文本。",
            "inputSchema": obj(json!({
                "word": str_prop("主词条，必须在词典中存在"),
                "card_type": str_prop("word|phrase|pattern，默认 word"),
                "difficulty": str_prop("easy|hard，默认 easy"),
                "phrase": str_prop("词组/句式文本（仅 phrase/pattern 需要）"),
            }), vec!["word"]),
        }),
        json!({
            "name": "due_queue",
            "description": "取今日复习队列：先到期的旧卡，再补新卡，受每日新学/复习上限约束。返回每张卡的题面（含难度区分）、答案与学习提示。wordlist_id 可选限定词表范围（0=全部）。",
            "inputSchema": obj(json!({
                "limit": num_prop("一次返回卡数上限，默认 20"),
                "wordlist_id": num_prop("限定词表 id，0 或省略为全部"),
            }), vec![]),
        }),
        json!({
            "name": "grade_card",
            "description": "为一张卡打分并更新其间隔重复排程。grade: 0=忘记/重学, 1=困难, 2=认识, 3=简单。",
            "inputSchema": obj(json!({
                "id": num_prop("卡片 id（来自 due_queue 或 add_card）"),
                "grade": num_prop("0|1|2|3"),
            }), vec!["id", "grade"]),
        }),
        json!({
            "name": "card_history",
            "description": "查看一张卡的全部复习历史（时间、评分、是否首学）与该词当前难度。",
            "inputSchema": obj(json!({ "id": num_prop("卡片 id") }), vec!["id"]),
        }),
        json!({
            "name": "card_records",
            "description": "列出学习记录：每张卡的状态、已学次数、遗忘次数、上次复习时间、到期时间与难度。",
            "inputSchema": obj(json!({ "limit": num_prop("返回条数上限，默认 50") }), vec![]),
        }),
        json!({
            "name": "set_difficulty",
            "description": "标记某张卡的难度（easy=简单/hard=困难），影响日后题面（困难只给首字母+中文义项）。",
            "inputSchema": obj(json!({
                "id": num_prop("卡片 id"),
                "difficulty": str_prop("easy|hard"),
            }), vec!["id", "difficulty"]),
        }),
        json!({
            "name": "wordlists",
            "description": "列出所有词表（如高考、雅思）及其词数，供限定学习范围。",
            "inputSchema": obj(json!({}), vec![]),
        }),
        json!({
            "name": "settings_get",
            "description": "读取学习设置：每日新学数、每日复习上限、今日已完成新学/复习数、当前生效词表。",
            "inputSchema": obj(json!({}), vec![]),
        }),
        json!({
            "name": "settings_set",
            "description": "修改学习设置：new_per_day 每日新学数、review_per_day 每日复习上限、active_wordlist 当前词表 id（0=全部）。",
            "inputSchema": obj(json!({
                "new_per_day": num_prop("每日新学数"),
                "review_per_day": num_prop("每日复习上限"),
                "active_wordlist": num_prop("词表 id，0=全部"),
            }), vec![]),
        }),
        json!({
            "name": "random_word",
            "description": "随机抽一个词典中的词加入复习（用于不知道学什么的场景）。",
            "inputSchema": obj(json!({}), vec![]),
        }),
        json!({
            "name": "phrase_count",
            "description": "已提取的词组/习语/句式库规模。",
            "inputSchema": obj(json!({}), vec![]),
        }),
        json!({
            "name": "phrases_for",
            "description": "查看某词条下提取出的词组/习语/句式（含中文释义与例句）。",
            "inputSchema": obj(json!({ "word": str_prop("源词条") }), vec!["word"]),
        }),
    ];
    json!({ "tools": tools })
}

// ---- arg helpers ----
fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}
fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}
fn arg_u8(args: &Value, key: &str) -> Option<u8> {
    args.get(key).and_then(|v| v.as_u64()).map(|n| n.min(4) as u8)
}

/// Route a tools/call to the right handler. Returns (text, is_error).
fn dispatch_tool(name: &str, args: &Value, d: &Db) -> (String, bool) {
    let r = match name {
        "lookup" => tool_lookup(args, d),
        "search" => tool_search(args, d),
        "stats" => tool_stats(d),
        "add_card" => tool_add_card(args, d),
        "due_queue" => tool_due_queue(args, d),
        "grade_card" => tool_grade_card(args, d),
        "card_history" => tool_card_history(args, d),
        "card_records" => tool_card_records(args, d),
        "set_difficulty" => tool_set_difficulty(args, d),
        "wordlists" => tool_wordlists(d),
        "settings_get" => tool_settings_get(d),
        "settings_set" => tool_settings_set(args, d),
        "random_word" => tool_random_word(d),
        "phrase_count" => tool_phrase_count(d),
        "phrases_for" => tool_phrases_for(args, d),
        other => Err(anyhow::anyhow!("unknown tool: {other}")),
    };
    match r {
        Ok(text) => (text, false),
        Err(e) => (format!("error: {e}"), true),
    }
}

fn tool_lookup(args: &Value, d: &Db) -> anyhow::Result<String> {
    use lexicon_core::definition::parse_definition;
    let word = arg_str(args, "word").unwrap_or_default();
    if word.is_empty() {
        return Err(anyhow::anyhow!("word 不能为空"));
    }
    let def = d.lookup_resolved(&word)?.unwrap_or_default();
    if def.is_empty() {
        return Ok(format!("词典中未找到：{word}"));
    }
    let p = parse_definition(&word, &def);
    let mut out = String::new();
    out.push_str(&format!("## {}\n", p.headword));
    if !p.phonetic.is_empty() {
        out.push_str(&format!("音标：{}\n", p.phonetic));
    }
    if !p.pos.is_empty() {
        out.push_str(&format!("词性：{}\n", p.pos));
    }
    if !p.senses_zh.is_empty() {
        out.push_str("释义：\n");
        for (i, s) in p.senses_zh.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, s));
        }
    }
    if !p.examples.is_empty() {
        out.push_str("例句：\n");
        for (en, zh) in p.examples.iter().take(3) {
            out.push_str(&format!("- {en}\n  {zh}\n"));
        }
    }
    if !p.phrases.is_empty() {
        out.push_str("固定搭配 / 词组 / 句式：\n");
        for ph in p.phrases.iter().take(6) {
            let zh = if ph.def_zh.is_empty() { ph.def_en.as_str() } else { ph.def_zh.as_str() };
            out.push_str(&format!("- [{}] {} — {}\n", ph.kind, ph.text, zh));
        }
    }
    Ok(out)
}

fn tool_search(args: &Value, d: &Db) -> anyhow::Result<String> {
    let prefix = arg_str(args, "prefix").unwrap_or_default();
    let limit = arg_i64(args, "limit").unwrap_or(20).max(1).min(200);
    let rows = d.search(&prefix, limit)?;
    if rows.is_empty() {
        return Ok(format!("没有匹配「{prefix}」的词条"));
    }
    let mut out = String::from("匹配词条：\n");
    for (hw, _) in rows {
        out.push_str(&format!("- {hw}\n"));
    }
    Ok(out)
}

fn tool_stats(d: &Db) -> anyhow::Result<String> {
    let (words, cards) = d.stats()?;
    let now = chrono::Local::now().naive_local();
    let due = d.due_count(&now)?;
    let phrases = d.phrase_count()?;
    Ok(format!(
        "词典词条：{words}\n复习卡片：{cards}\n今日到期：{due}\n词组/句式库：{phrases}"
    ))
}

fn tool_add_card(args: &Value, d: &Db) -> anyhow::Result<String> {
    use lexicon_core::model::{CardType, Difficulty};
    let word = arg_str(args, "word").unwrap_or_default();
    if word.is_empty() {
        return Err(anyhow::anyhow!("word 不能为空"));
    }
    let ctype = match arg_str(args, "card_type").as_deref() {
        Some("phrase") => CardType::Phrase,
        Some("pattern") => CardType::Pattern,
        _ => CardType::Word,
    };
    let diff = match arg_str(args, "difficulty").as_deref() {
        Some("hard") => Difficulty::Hard,
        _ => Difficulty::Easy,
    };
    let phrase = arg_str(args, "phrase").unwrap_or_else(|| word.clone());
    let id = d.add_new_card_full(&word, ctype, diff, &word, &phrase)?;
    if id <= 0 {
        return Err(anyhow::anyhow!("{word} 不在词典中，无法加入"));
    }
    Ok(format!("已加入复习队列，卡片 id={id}（类型 {ctype:?}，难度 {diff:?}）"))
}

fn tool_due_queue(args: &Value, d: &Db) -> anyhow::Result<String> {
    let now = chrono::Local::now().naive_local();
    let limit = arg_i64(args, "limit").unwrap_or(20).max(1).min(100);
    let wl: Option<i64> = match arg_i64(args, "wordlist_id") {
        Some(0) | None => None,
        Some(id) => Some(id),
    };
    let (cards, new_left, review_left) = d.daily_due_queue(&now, limit, wl)?;
    if cards.is_empty() {
        return Ok(format!(
            "今日没有待复习/新学卡片（新学剩余 {new_left}，复习剩余 {review_left}）。可用 add_card 加词，或 random_word 随机抽词。"
        ));
    }
    let mut out = String::new();
    out.push_str(&format!(
        "今日复习队列（新学剩余 {new_left}，复习剩余 {review_left}）：\n"
    ));
    for (i, (id, hw, ctype, diff, phrase)) in cards.iter().enumerate() {
        let prompt = crate::build_card_prompt(d, *id, hw, ctype, diff, phrase);
        out.push_str(&format!(
            "--- 卡 {i} id={id} 【{ctype}·{diff}】 {hw} ---\n题面：{}\n",
            prompt["question"].as_str().unwrap_or("")
        ));
        if let Some(h) = prompt["hint"].as_str() {
            if !h.is_empty() {
                out.push_str(&format!("提示：{h}\n"));
            }
        }
        out.push_str(&format!("答案：{}\n", prompt["answer"].as_str().unwrap_or("")));
        if let Some(src) = prompt["extra"]["source"].as_str() {
            if src != hw {
                out.push_str(&format!("源自词条：{src}\n"));
            }
        }
    }
    Ok(out)
}

fn tool_grade_card(args: &Value, d: &Db) -> anyhow::Result<String> {
    let id = arg_i64(args, "id").unwrap_or(0);
    let grade = arg_u8(args, "grade").unwrap_or(2);
    if id <= 0 {
        return Err(anyhow::anyhow!("id 无效"));
    }
    d.grade_card(id, grade)?;
    // report the new due state
    let due = d
        .load_card(id)?
        .and_then(|c| c.due)
        .map(|t| t.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".to_string());
    let grade_names = ["忘记/重学", "困难", "认识", "简单"];
    let gname = grade_names.get(grade as usize).copied().unwrap_or("?");
    Ok(format!(
        "已评分：{gname}（grade={grade}）。下次到期：{due}"
    ))
}

fn tool_card_history(args: &Value, d: &Db) -> anyhow::Result<String> {
    let id = arg_i64(args, "id").unwrap_or(0);
    if id <= 0 {
        return Err(anyhow::anyhow!("id 无效"));
    }
    let log = d.card_history(id)?;
    if log.is_empty() {
        return Ok(format!("卡片 {id} 尚无复习记录"));
    }
    let grade_names = ["忘记", "困难", "认识", "简单"];
    let mut out = format!("卡片 {id} 复习历史（{} 次）：\n", log.len());
    for l in log.iter().rev() {
        let g = grade_names.get(l.grade as usize).copied().unwrap_or("?");
        out.push_str(&format!(
            "{}  {}{}\n",
            l.reviewed_at.format("%m-%d %H:%M"),
            if l.is_new { "🆕 " } else { "" },
            g
        ));
    }
    Ok(out)
}

fn tool_card_records(args: &Value, d: &Db) -> anyhow::Result<String> {
    let limit = arg_i64(args, "limit").unwrap_or(50).max(1).min(500);
    let rows = d.card_records(limit)?;
    if rows.is_empty() {
        return Ok("还没有学习记录。用 add_card 或 random_word 开始。".to_string());
    }
    let mut out = String::from("学习记录：\n");
    for (id, hw, ctype, diff, status, reps, lapses, due, last, _) in rows {
        let due_s = due.unwrap_or_else(|| "—".to_string());
        let last_s = last.unwrap_or_else(|| "未复习".to_string());
        out.push_str(&format!(
            "- id={id} {hw} [{ctype}/{diff}] {status} 已学{reps}次 遗忘{lapses}次 上次{last_s} 到期{due_s}\n"
        ));
    }
    Ok(out)
}

fn tool_set_difficulty(args: &Value, d: &Db) -> anyhow::Result<String> {
    use lexicon_core::model::Difficulty;
    let id = arg_i64(args, "id").unwrap_or(0);
    let diff = match arg_str(args, "difficulty").as_deref() {
        Some("hard") => Difficulty::Hard,
        _ => Difficulty::Easy,
    };
    if id <= 0 {
        return Err(anyhow::anyhow!("id 无效"));
    }
    d.set_card_difficulty(id, diff)?;
    Ok(format!("卡片 {id} 难度已设为 {diff:?}"))
}

fn tool_wordlists(d: &Db) -> anyhow::Result<String> {
    let lists = d.wordlists()?;
    if lists.is_empty() {
        return Ok("暂无词表。可用命令行导入：lexicon-cli wordlist <db> <名称> <词表文件>".to_string());
    }
    let mut out = String::from("词表：\n");
    for l in lists {
        out.push_str(&format!("- id={} {}（{}词）\n", l.id, l.name, l.size));
    }
    Ok(out)
}

fn tool_settings_get(d: &Db) -> anyhow::Result<String> {
    let (new_pd, rev_pd) = d.daily_limits()?;
    let (new_done, rev_done) = d.today_progress()?;
    let active = d
        .get_setting("active_wordlist")?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    Ok(format!(
        "每日新学：{new_done}/{new_pd}\n每日复习：{rev_done}/{rev_pd}\n当前词表 id：{active}（0=全部）"
    ))
}

fn tool_settings_set(args: &Value, d: &Db) -> anyhow::Result<String> {
    if let Some(n) = arg_i64(args, "new_per_day") {
        d.set_setting("new_per_day", &n.to_string())?;
    }
    if let Some(r) = arg_i64(args, "review_per_day") {
        d.set_setting("review_per_day", &r.to_string())?;
    }
    if let Some(wl) = arg_i64(args, "active_wordlist") {
        d.set_setting("active_wordlist", &wl.to_string())?;
    }
    Ok("设置已更新".to_string())
}

fn tool_random_word(d: &Db) -> anyhow::Result<String> {
    match d.random_word()? {
        Some(w) => {
            let id = d.add_new_card(&w)?;
            if id > 0 {
                Ok(format!("已随机加入：{w}（id={id}）"))
            } else {
                Ok(format!("随机选中：{w}，但加入复习失败（可能已存在）"))
            }
        }
        None => Err(anyhow::anyhow!("词典为空")),
    }
}

fn tool_phrase_count(d: &Db) -> anyhow::Result<String> {
    Ok(format!("词组/习语/句式库：{} 条", d.phrase_count()?))
}

fn tool_phrases_for(args: &Value, d: &Db) -> anyhow::Result<String> {
    let word = arg_str(args, "word").unwrap_or_default();
    let items = d.phrases_for(&word)?;
    if items.is_empty() {
        return Ok(format!("「{word}」下没有提取到词组/句式"));
    }
    let mut out = String::new();
    for (_, ptype, text, de, dz, xe, xz) in items {
        out.push_str(&format!("[{ptype}] {text}\n"));
        if !dz.is_empty() {
            out.push_str(&format!("  中文：{dz}\n"));
        } else if !de.is_empty() {
            out.push_str(&format!("  {de}\n"));
        }
        if !xe.is_empty() {
            out.push_str(&format!("  例：{xe}\n  {xz}\n"));
        }
    }
    Ok(out)
}
