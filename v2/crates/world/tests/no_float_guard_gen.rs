//! Recursive GLOB no-float guard for the world-gen pipeline (plan F16/F17). Unlike `sim-core`'s
//! `no_float_guard.rs` (a hardcoded per-file allow-list), this guard scans EVERY `.rs` file under
//! **`world/src/gen/` (recursively, glob root + pattern documented here)** for `f32`/`f64` tokens
//! — a new stage file (or a new sub-module under `gen/`) needs NO registration to be covered; the
//! integer discipline is enforced by directory placement alone (fails closed).
//!
//! Scan root: `<world crate>/src/gen/`. Pattern: every file at any depth ending in `.rs`
//! (equivalent to the glob `world/src/gen/**/*.rs`). The legacy `NoiseWorld` in `world/src/lib.rs`
//! (still `f64 sin`) sits OUTSIDE `gen/` and is deliberately NOT scanned — it is deleted at W-6.
//!
//! Comment lines and trailing line-comments are ignored (prose may discuss "f64"); this guard
//! itself lives in `tests/`, outside the scanned `src/gen/` root.

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

/// Scan one source string (already comment-stripped per line by the caller) for banned tokens.
fn scan_source(src: &str, banned: &[&str], label: &str, hits: &mut Vec<String>) {
    for (i, raw) in src.lines().enumerate() {
        if raw.trim_start().starts_with("//") {
            continue;
        }
        let code = raw.split("//").next().unwrap_or("");
        for &b in banned {
            if contains_token(code, b) {
                hits.push(format!("{}:{}: banned `{}` in `{}`", label, i + 1, b, code.trim()));
            }
        }
    }
}

/// Recursively glob every `.rs` file under `dir` and scan it.
fn scan_dir_recursive(dir: &Path, banned: &[&str], hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan_dir_recursive(&path, banned, hits);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        scan_source(&src, banned, &path.display().to_string(), hits);
    }
}

/// Critic F7 — negative control WITHOUT polluting a real file: prove the detector itself has
/// teeth by running `scan_source` against synthetic in-memory strings, never a committed file.
/// A string containing a real `f64` token must be flagged; a clean all-integer string must not.
#[test]
fn detector_flags_synthetic_float_and_passes_synthetic_integer() {
    let dirty = "fn bad(x: f64) -> f64 { x * 2.0 }\n";
    let mut dirty_hits = Vec::new();
    scan_source(dirty, &["f32", "f64"], "<synthetic-dirty>", &mut dirty_hits);
    assert!(!dirty_hits.is_empty(), "detector must flag a synthetic `f64` sample — guard would be vacuous otherwise");

    let dirty32 = "let y: f32 = 1.0;\n";
    let mut dirty32_hits = Vec::new();
    scan_source(dirty32, &["f32", "f64"], "<synthetic-dirty32>", &mut dirty32_hits);
    assert!(!dirty32_hits.is_empty(), "detector must flag a synthetic `f32` sample");

    let clean = "fn good(x: i64) -> i64 { x.wrapping_mul(2) }\n// mentions f64 only in a comment\n";
    let mut clean_hits = Vec::new();
    scan_source(clean, &["f32", "f64"], "<synthetic-clean>", &mut clean_hits);
    assert!(clean_hits.is_empty(), "detector must NOT flag a clean integer sample or a comment-only mention: {:?}", clean_hits);

    // Word-boundary sanity: an identifier merely CONTAINING "f64" as a substring (not the bare
    // token) must not false-positive.
    let substr = "let f64ish_name_not_the_type = 1i64;\n";
    let mut substr_hits = Vec::new();
    scan_source(substr, &["f32", "f64"], "<synthetic-substr>", &mut substr_hits);
    assert!(substr_hits.is_empty(), "detector must not false-positive on an identifier merely containing `f64` as a substring: {:?}", substr_hits);
}

/// The real guard: every `.rs` under `world/src/gen/` (recursively) must be float-free.
#[test]
fn world_gen_tree_is_integer_only() {
    let gen_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("gen");
    assert!(gen_root.is_dir(), "expected the world-gen module home at {}", gen_root.display());

    let mut hits = Vec::new();
    scan_dir_recursive(&gen_root, &["f32", "f64"], &mut hits);
    assert!(
        hits.is_empty(),
        "world/src/gen/ must be pure integer/fixed-point (no f32/f64 anywhere under the pipeline stage home):\n{}",
        hits.join("\n")
    );
}
