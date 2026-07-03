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
//! itself lives in `tests/`, outside the scanned `src/gen/` root. Comment/string stripping stays
//! fail-closed as in W-1 (only `//` comments are stripped, never string literals — a float literal
//! embedded in a string would still be flagged, which is conservative/safe, not a bug).
//!
//! **W-2 extension (W-1 cold-review F1/F7):** the original guard caught only the `f32`/`f64`
//! TOKENS, not bare float literals — `let x = 1.0;` infers `f64` without ever writing the token,
//! slipping through. [`contains_float_literal`] closes that hole: it detects the float-literal
//! SHAPES `\d+\.\d*` (e.g. `1.0`, `3.14`), `\.\d+` (e.g. `.5`), and scientific notation
//! `\d+[eE][+-]?\d+` / the combined decimal+exponent form (e.g. `1e6`, `1.5e-2`), while explicitly
//! NOT flagging tuple/field access (`a.0`, `x.1`), ranges (`0..10`, `0..=9`), or digits embedded in
//! identifiers (`layer2`, `rate1e2`) — see its doc comment for the token-boundary reasoning.

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

/// Detect a bare float-literal SHAPE in one line of code (`\d+\.\d*`, `\.\d+`, or scientific
/// notation) without a regex dependency — a small hand-rolled scanner that respects Rust token
/// boundaries (critic F1):
///
/// - **Identifiers are consumed WHOLE first** (a maximal run of letters/digits/underscores
///   starting on a letter/underscore). This is what keeps `layer2` and `rate1e2` safe: the digits
///   are never visited standalone, because the whole identifier is swallowed as one token before
///   any digit-run/exponent check can fire on its embedded digits.
/// - **A digit run only starts a numeric-literal check at a token boundary** (Rust identifiers can
///   never START with a digit, so any digit NOT already consumed as part of an identifier is
///   unambiguously the start of a number). Digit-separator underscores (`2_000_000`) are consumed
///   as part of the same run.
/// - **`\d+\.` immediately flags** UNLESS the following char is also `.` (the `..`/`..=` range
///   operator — critic negative control `0..10`, `0..=9`): in that case the digit run was an
///   integer range bound, not a float, and the loop resumes at the first dot to correctly no-op
///   through the range operator's own two dots.
/// - **A lone `.` (not preceded by a digit-run, another dot, an identifier char, or `]`/`)`, and
///   not followed by another dot) immediately followed by a digit flags** `\.\d+` (e.g. `.5`). The
///   "not preceded by an identifier char" guard is what keeps `a.0`/`x.1` (tuple/field access) safe
///   — `a`/`x` are consumed as identifiers, then the dot's preceding char is a letter, not a float.
///   The `]`/`)` guard extends this to field/tuple access on an expression RESULT (`points[0].0`,
///   `f().0`) — Rust has no syntax where a float literal directly follows a closing bracket/paren.
/// - **`\d+[eE][+-]?\d+` flags** scientific notation (`1e6`, `1e-3`) when no `.` follows the digit
///   run (the combined decimal+exponent form, e.g. `1.5e-2`, is already caught by the `\d+\.`
///   check above, which fires before the exponent is even inspected).
fn contains_float_literal(code: &str) -> bool {
    let b = code.as_bytes();
    let n = b.len();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0usize;
    while i < n {
        let c = b[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            // Whole identifier (may embed digits, e.g. `layer2`, `rate1e2`) — never a literal.
            i += 1;
            while i < n && is_ident(b[i]) {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_digit() {
            // Token-boundary digit run (underscores are the Rust digit-separator convention).
            while i < n && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
            if i < n && b[i] == b'.' {
                if i + 1 < n && b[i + 1] == b'.' {
                    continue; // `..`/`..=` range — the digit run was an integer bound, not a float
                }
                return true; // `\d+\.\d*`
            }
            if i < n && (b[i] == b'e' || b[i] == b'E') {
                let mut k = i + 1;
                if k < n && (b[k] == b'+' || b[k] == b'-') {
                    k += 1;
                }
                if k < n && b[k].is_ascii_digit() {
                    return true; // `\d+[eE][+-]?\d+`
                }
            }
            continue;
        }
        if c == b'.' {
            let prev = if i > 0 { Some(b[i - 1]) } else { None };
            let preceded_by_dot = prev == Some(b'.');
            let preceded_by_ident = prev.map(is_ident).unwrap_or(false);
            // `expr[0].0` / `f().0`: a `]`/`)` right before the dot means this is field/tuple
            // access on an expression RESULT, not a fresh leading-dot float literal — Rust has no
            // syntax where a float literal directly follows `]`/`)`.
            let preceded_by_expr_end = matches!(prev, Some(b']') | Some(b')'));
            let followed_by_dot = i + 1 < n && b[i + 1] == b'.';
            if !preceded_by_dot
                && !preceded_by_ident
                && !preceded_by_expr_end
                && !followed_by_dot
                && i + 1 < n
                && b[i + 1].is_ascii_digit()
            {
                return true; // `\.\d+`
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    false
}

/// Scan one source string (already comment-stripped per line by the caller) for banned tokens AND
/// bare float literals (W-2 extension).
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
        if contains_float_literal(code) {
            hits.push(format!("{}:{}: banned float literal in `{}`", label, i + 1, code.trim()));
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

/// W-2 (W-1 cold-review F1/F7) — two-sided negative control for [`contains_float_literal`]: it
/// MUST detect every real float-literal shape (plain decimal, leading-dot, scientific notation,
/// combined decimal+exponent) AND MUST NOT trip on tuple/field access, ranges, digit-embedding
/// identifiers, digit-separator integers, or clean integer source.
#[test]
fn float_literal_scanner_detects_real_literals_and_ignores_false_positives() {
    let positives = [
        "let x: f64 = 1.0;",
        "let y = .5;",
        "const PI: f64 = 3.14;",
        "let e = 1e6;",
        "let e2 = 1.5e-2;",
        "let neg = -1E-3;",
    ];
    for lit in positives {
        assert!(contains_float_literal(lit), "must detect a real float literal in: {lit}");
    }

    let negatives = [
        "a.0",
        "x.1",
        "let t = pair.0 + pair.1;",
        "let z = points[0].0;",
        "let w = f().1;",
        "0..10",
        "0..=9",
        "for i in 0..n { }",
        "let layer2 = 3;",
        "let rate1e2 = 7i64;",
        "let big = 2_000_000;",
        "fn good(x: i64) -> i64 { x.wrapping_mul(2) }",
    ];
    for clean in negatives {
        assert!(!contains_float_literal(clean), "must NOT flag a false positive: {clean}");
    }
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
