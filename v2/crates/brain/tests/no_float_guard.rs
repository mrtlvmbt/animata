//! Zero-float shield for the `brain` crate (F11) — the SAME grep guard M0 put on `sim-core`, now
//! extended to inference. The brain hot path must be PURE INTEGER end-to-end: `f32`/`f64` are banned
//! anywhere in `brain/src/` (including the generated LUT — the float tanh lives only in the OFFLINE
//! generator `v2/tools/gen_brain_lut.py`, never in this crate). Without this, "zero-float in brain"
//! would rest on discipline, not CI, and a float dequant/LUT injection could slip past the gate.
//!
//! Comment lines and trailing line-comments are ignored (prose may discuss "float"); this test file is
//! not scanned. The whole crate is integer, so the ban is total (unlike `sim-core`, where the f32
//! signal path is allowed and only the conserved modules are scanned).

use std::fs;
use std::path::Path;

fn is_word_at(hay: &str, idx: usize, needle: &str) -> bool {
    let before = hay[..idx].chars().next_back();
    let after = hay[idx + needle.len()..].chars().next();
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    before.is_none_or(|c| !ident(c)) && after.is_none_or(|c| !ident(c))
}

fn contains_token(code: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = code[from..].find(needle) {
        let idx = from + rel;
        if is_word_at(code, idx, needle) {
            return true;
        }
        from = idx + needle.len();
    }
    false
}

fn scan(dir: &Path, banned: &[&str], hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, banned, hits);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        for (i, raw) in src.lines().enumerate() {
            if raw.trim_start().starts_with("//") {
                continue;
            }
            let code = raw.split("//").next().unwrap_or("");
            for &b in banned {
                if contains_token(code, b) {
                    hits.push(format!("{}:{}: banned `{}` in `{}`", path.display(), i + 1, b, code.trim()));
                }
            }
        }
    }
}

/// The whole inference crate is integer — NO `f32`/`f64`, and no random-hasher std map.
#[test]
fn brain_src_is_integer_only() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    let mut float_hits = Vec::new();
    scan(&src, &["f32", "f64"], &mut float_hits);
    assert!(float_hits.is_empty(), "brain inference must be pure integer (no f32/f64 in src/):\n{}", float_hits.join("\n"));

    let mut map_hits = Vec::new();
    scan(&src, &["HashMap", "HashSet"], &mut map_hits);
    assert!(map_hits.is_empty(), "no bare std HashMap/HashSet in brain:\n{}", map_hits.join("\n"));
}
