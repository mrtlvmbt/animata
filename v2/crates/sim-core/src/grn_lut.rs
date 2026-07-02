//! ⚠️ GENERATED + provenance-pinned — do NOT hand-edit the table. Regenerate ONLY by re-running the
//! OFFLINE tool `v2/tools/gen_grn_lut.py` (logistic sigma evaluated in float ON THE HOST, never in
//! CI/build) and pasting its stdout over this file. The float generator does NOT run in CI; CI
//! verifies the table by the committed [`SIGMA_LUT_SHA256`] integer checksum (a pure SHA-256 over
//! the LE-i16 bytes — arch-identical, ~0 cost). Re-pinning is a deliberate act, like the golden.
//!
//! **Not a copy of `brain::TANH_LUT`** (critic F3): the GRN gene state is a non-negative EXPRESSION
//! LEVEL, not a signed concentration, so the activation is a LOGISTIC σ ∈ **[0, EXPR_MAX]** — reusing
//! `tanh`'s [−256, 256] range would silently recode repression as negative "mass" and change the
//! attractor arithmetic. The domain/format otherwise mirrors `brain/src/lut.rs`'s provenance
//! discipline (Q8.8 preact, same `PREACT_MIN`/`LUT_BIN` shape) for consistency.
//!
//! Format (must match the generator): activation σ = logistic. The table maps a pre-activation in
//! Q8.8 to **σ·256 (Q8.8 expression level, range [0, 256])**. Domain is `[PREACT_MIN, PREACT_MAX]`
//! (real [-8.0, +7.97]) in `LUT_BIN`-sized Q8.8 bins; out-of-range inputs CLAMP (never wrap).
//!
//! These domain/clamp constants are COMMITTED `const` beside the table under the same checksum
//! provenance: NOT calibrated from a run's statistics at runtime (hidden per-run state → replay
//! divergence). Calibration, if ever changed, happens offline at re-pin.
// Guard: no float arithmetic (this file is pure integer const data, but the lint is kept consistent
// with grn.rs/morphogen.rs — belt-and-braces alongside the no_float_guard.rs token scan).
#![deny(clippy::float_arithmetic)]

/// Q8.8 pre-activation clamp lower bound (real −8.0). `SIGMA_LUT[0]`.
pub const PREACT_MIN: i64 = -2048;
/// Q8.8 pre-activation clamp upper bound (real +7.96875). `SIGMA_LUT[511]`.
pub const PREACT_MAX: i64 = 2040;
/// Q8.8 width of one LUT bin (real 0.03125). `(PREACT_MAX - PREACT_MIN) / LUT_BIN == 511`.
pub const LUT_BIN: i64 = 8;
/// Committed non-negative expression-level ceiling — `SIGMA_LUT` values are in `[0, EXPR_MAX]`.
pub const EXPR_MAX: i32 = 256;

pub const SIGMA_LUT: [i16; 512] = [
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 6, 6,
    6, 6, 6, 6, 7, 7, 7, 7,
    8, 8, 8, 8, 8, 9, 9, 9,
    10, 10, 10, 10, 11, 11, 11, 12,
    12, 13, 13, 13, 14, 14, 15, 15,
    15, 16, 16, 17, 17, 18, 18, 19,
    19, 20, 21, 21, 22, 22, 23, 24,
    24, 25, 26, 27, 27, 28, 29, 30,
    31, 31, 32, 33, 34, 35, 36, 37,
    38, 39, 40, 41, 42, 43, 44, 46,
    47, 48, 49, 50, 52, 53, 54, 56,
    57, 58, 60, 61, 63, 64, 66, 67,
    69, 70, 72, 74, 75, 77, 79, 80,
    82, 84, 86, 87, 89, 91, 93, 95,
    97, 99, 100, 102, 104, 106, 108, 110,
    112, 114, 116, 118, 120, 122, 124, 126,
    128, 130, 132, 134, 136, 138, 140, 142,
    144, 146, 148, 150, 152, 154, 156, 157,
    159, 161, 163, 165, 167, 169, 170, 172,
    174, 176, 177, 179, 181, 182, 184, 186,
    187, 189, 190, 192, 193, 195, 196, 198,
    199, 200, 202, 203, 204, 206, 207, 208,
    209, 210, 212, 213, 214, 215, 216, 217,
    218, 219, 220, 221, 222, 223, 224, 225,
    225, 226, 227, 228, 229, 229, 230, 231,
    232, 232, 233, 234, 234, 235, 235, 236,
    237, 237, 238, 238, 239, 239, 240, 240,
    241, 241, 241, 242, 242, 243, 243, 243,
    244, 244, 245, 245, 245, 246, 246, 246,
    246, 247, 247, 247, 248, 248, 248, 248,
    248, 249, 249, 249, 249, 250, 250, 250,
    250, 250, 250, 251, 251, 251, 251, 251,
    251, 252, 252, 252, 252, 252, 252, 252,
    252, 253, 253, 253, 253, 253, 253, 253,
    253, 253, 253, 253, 254, 254, 254, 254,
    254, 254, 254, 254, 254, 254, 254, 254,
    254, 254, 254, 254, 254, 255, 255, 255,
    255, 255, 255, 255, 255, 255, 255, 255,
    255, 255, 255, 255, 255, 255, 255, 255,
    255, 255, 255, 255, 255, 255, 255, 255,
    255, 255, 255, 255, 255, 255, 255, 255,
    256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256,
];
pub const SIGMA_LUT_SHA256: &str = "b7211c32cb650e93782830dc56e401653a92fd77bbe02c048a41050e8ef0de78";
