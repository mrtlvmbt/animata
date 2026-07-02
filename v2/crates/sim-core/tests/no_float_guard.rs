//! Static guard (the "grep guard" of the spec). Two invariants, mechanically:
//!
//! * **The CONSERVED layer is INTEGER.** `energy.rs` and `genome.rs` (the energy ledger + the
//!   genetics that price metabolism) must contain NO `f32`/`f64`. This is what keeps energy
//!   conservation exact and arch-independent (R13/R15). From M2 the SIGNAL path legitimately uses
//!   f32 (Sense/Act/Deposit/Telemetry), so the whole-crate float ban of M0/M1 is relaxed to these
//!   conserved-critical modules.
//! * **No bare `std` HashMap anywhere** in core state (random hasher → non-deterministic iteration).
//!   Use `DetMap`/`BTreeMap`/`BTreeSet`.
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
fn conserved_layer_is_integer_and_no_hashmap() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    // (1) The conserved-critical modules must be float-free. `morphogen.rs` (E-2) carries its own
    // `#![deny(clippy::float_arithmetic)]`, which CI does not gate on (nextest, not clippy) — this
    // token scan IS run by `cargo nextest` in CI, so it closes that gap for the plain-token case.
    let mut float_hits = Vec::new();
    for module in ["energy.rs", "genome.rs", "morphogen.rs"] {
        scan_file(&src.join(module), &["f32", "f64"], &mut float_hits);
    }
    assert!(
        float_hits.is_empty(),
        "conserved layer must be integer (no f32/f64 in energy.rs/genome.rs/morphogen.rs):\n{}",
        float_hits.join("\n")
    );

    // (2) No random-hasher std map anywhere in the core.
    let mut map_hits = Vec::new();
    scan(&src, &["HashMap", "HashSet"], &mut map_hits);
    assert!(map_hits.is_empty(), "no bare std HashMap/HashSet in core state:\n{}", map_hits.join("\n"));
}

fn scan_file(path: &Path, banned: &[&str], hits: &mut Vec<String>) {
    let src = fs::read_to_string(path).unwrap();
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
