//! Debug helper: dump the raw definition of a word.
//! Usage: cargo run --example dbg_entry -- <db> <word>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let d = lexicon_core::storage::Db::open(&args[1]).unwrap();
    let def = d.raw_lookup(&args[2]).unwrap().unwrap_or_default();
    let lower = def.to_lowercase();
    let mut count = 0;
    let mut from = 0;
    while let Some(i) = lower[from..].find("<idm-g ") {
        count += 1;
        println!("idm-g #{count} at {}", from + i);
        from += i + 1;
    }
    println!("total idm-g: {count}");
    if let Some(h) = lower.find("heart") {
        if let Some(rel) = lower[..h].rfind("<idm-g ") {
            println!("--- context around heart (inside idm-g at {rel}) ---");
            let end = (rel + 900).min(def.len());
            println!("{}", &def[rel..end]);
        } else {
            println!("'heart' at {h} is NOT inside any idm-g");
            let start = h.saturating_sub(300);
            println!("{}", &def[start..(h + 400).min(def.len())]);
        }
    }
}
