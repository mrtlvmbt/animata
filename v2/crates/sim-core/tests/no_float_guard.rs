//! Static guard (the "grep guard" of the spec): `sim-core/src` must contain NO floating-point types
//! and NO bare `std` HashMap. This is the MECHANISM that justifies the M0 x86-only golden — the core
//! is integer ⇒ cross-arch bit-identical. Floats enter only at M1 (behind a feature, with a matched
//! arm64 golden job); until then this test fails the build if any sneak in.
//!
//! Comment lines and trailing line-comments are ignored (so prose may discuss "float" / "HashMap"),
//! and this guard file is skipped. Code-side occurrences fail.

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
            let trimmed = raw.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            // Drop any trailing line comment, then test the code part only.
            let code = raw.split("//").next().unwrap_or("");
            for &b in banned {
                if contains_token(code, b) {
                    hits.push(format!("{}:{}: banned `{}` in `{}`", path.display(), i + 1, b, code.trim()));
                }
            }
        }
    }
}

#[test]
fn sim_core_is_integer_only_and_deterministic_maps() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Banned in core code: floating-point types and the random-hasher std map. `HashMap` is caught
    // even when written `std::collections::HashMap` (the `HashMap` token still trips). Use `DetMap`.
    let banned = ["f32", "f64", "HashMap", "HashSet"];
    let mut hits = Vec::new();
    scan(&src, &banned, &mut hits);
    assert!(hits.is_empty(), "zero-float / no-HashMap guard tripped:\n{}", hits.join("\n"));
}
