//! The two-level determinism gate (F5) — mandatory from M0, both levels in the `cli` harness, both
//! in CI:
//!
//!   (a) two-run-same-seed  — `run(seed)` twice, `hash_a[t] == hash_b[t] ∀ t`. Catches run-to-run
//!       NON-DETERMINISM (a forgotten natural-order reduction, a random hasher) that a golden-only
//!       diff would misdiagnose as "golden broke" (both non-deterministic runs would drift from the
//!       pin). Arch-independent (integer) → would run on every arch for free.
//!   (b) drift-vs-golden    — `hash_a[t] == GOLDEN[t] ∀ t`. Catches a trajectory drift.
//!
//! The M0 golden is INTEGER ⇒ identical on x86 and arm64 AND identical in debug and release (proven
//! by `match debug==release` at pin time). So M0 runs on the single x86 job; the float-arch-bound
//! `v2_golden_*` namespace + arm64 job arrive with M1.
//!
//! Re-pin procedure (single-writer, agent A): if `drift_vs_golden` fails, read the new value from the
//! assert's `left:`/`right:`, confirm it is an INTENDED trajectory change, and update `GOLDEN`. Never
//! re-pin to silence an unexplained drift.

use cli::run;

const GOLDEN_SEED: u64 = 0x0000_0000_a11a_2a11;
const GOLDEN_ENTITIES: u64 = 32;

/// Per-tick state hash, ticks 0..64, pinned on agent A's machine; debug==release verified at pin.
const GOLDEN: [u64; 64] = [
    0xcc0c53d54e440ebe, 0x7d07dd5d69270f57, 0x63834c194cd7caf7, 0x094c1ede159e3a21,
    0xf2adbe2d65f07fe9, 0x3cbcee7c30fdeb58, 0xfa6372cbb86ecda9, 0x7315489cb5704095,
    0xe315f40514d0520e, 0xd1e8b7d6065ebfc5, 0x204fc93c20391268, 0xe95b72579165be5c,
    0x529a52d8474f3234, 0x2b3426b1325b337c, 0x14cc180692f52a9b, 0xb2f8fa8b040b7891,
    0x82ad492f7456f120, 0x2b7f12a67363603b, 0x1c71935318d8c083, 0xd87d03240b88e308,
    0x9321f5a6b71c401f, 0x19f47dd5f37d279a, 0xf5d9d3d7cf5c7663, 0x5e4737b992075f97,
    0xce035f98ee361ecb, 0xa35757cb1c0d39f7, 0x8600e065d69b1089, 0x01fff97a5e6a8667,
    0x309c4c1170f75b16, 0x0679b5015f1c5e84, 0x6e7a773f74ccb41b, 0xa4c2706e321a4b7c,
    0xa9f16d70a19492d8, 0xa4f31a6012bf4cc3, 0xfc2f18d3f3553b6a, 0x8cb3c47ebe668748,
    0xb55953d6430d94f0, 0x8af573e9094b362a, 0x0bc15e19fd5bbb07, 0x1c2b6aba34fe0aa7,
    0xe4b2914dd9681530, 0x0c72120dc0604c0b, 0x53e9c5757a9fa3f0, 0xfe4e00332e547501,
    0xd6ca4cdf6e3ad805, 0x4880a8732cb931e1, 0xcc3cd5b1819bcff5, 0x6fe92a77c1e3acbb,
    0x6d8e43ed06c668d7, 0x4c6361cf60aac6d7, 0x838d9f48d2f66cb7, 0x39c5814ce3e2ee02,
    0xe152836754b50a11, 0x53de4ccc09c95234, 0x013925d76ee2d9c9, 0x670a8c9fd6d3eecf,
    0x5046df5a23126259, 0xe4473ca2f804a8ba, 0x5cfcc82246ae5548, 0x71efe00aaf2d6868,
    0xf85d5f02b6775a34, 0x033293ed1547f350, 0x6d8ec9c60a68a04a, 0x3476f34a85dd7791,
];

/// (a) two-run-same-seed — run-to-run determinism. Lives OUTSIDE any `v2_golden_*` namespace so that
/// from M1 it runs on both arches; in M0 it runs on the single x86 job.
#[test]
fn v2_two_run_same_seed() {
    let a = run(GOLDEN_SEED, GOLDEN_ENTITIES, GOLDEN.len() as u64);
    let b = run(GOLDEN_SEED, GOLDEN_ENTITIES, GOLDEN.len() as u64);
    for t in 0..GOLDEN.len() {
        assert_eq!(a[t], b[t], "run-to-run non-determinism at tick {t}");
    }
}

/// (b) drift-vs-golden — bit-for-bit replay against the pin (R19).
#[test]
fn v2_golden_drift() {
    let a = run(GOLDEN_SEED, GOLDEN_ENTITIES, GOLDEN.len() as u64);
    for t in 0..GOLDEN.len() {
        assert_eq!(a[t], GOLDEN[t], "golden drift at tick {t} (left=run, right=GOLDEN)");
    }
}
