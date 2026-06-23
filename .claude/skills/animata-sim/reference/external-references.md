# External references — the library/numerical facts behind the invariants

**Read this when** you need the external fact behind a sim invariant (FMA, rayon ordering, bincode,
glam, fast-approx, splitmix). These are *durable* (they change on a toolchain/dep bump, not on our every
spike) — see the volatility split in `README.md`.

## The anchor-or-version-or-drop rule (how to trust an entry here)

A wrong determinism fact is worse than no KB. So every claim below is either:
- **(a) Anchored** to an in-production, golden-locked SKILL invariant (it merely *explains* a behaviour
  the live golden already proves), OR
- **(b) Version-stamped** against the real `Cargo.lock` dep, so you can re-verify against the exact crate.

A claim that anchored to neither was **dropped**, not transcribed (the web pass surfaced a few — noted at
the end). URL-resolve + a `checked:` date is the floor, never sufficient on its own. **Versions are from
`Cargo.lock` at the time of writing — re-read `Cargo.lock` before trusting a version-stamped claim.**

---

## 1. FMA contraction — why debug ≠ release

> **Anchor:** SKILL §0 "debug ≠ release" — the golden is pinned PER PROFILE (`sim.rs:1560`, two
> `cfg!(debug_assertions)` arms) *because the two builds observably diverge over thousands of ticks*.
> That divergence is the in-production fact; the entries below explain the mechanism, they do not
> override it. (The web pass muddied this — "contraction is one statement-level flag" is true at the
> Clang level but does NOT contradict our observed per-profile divergence; trust the golden.)

- LLVM/Clang default `-ffp-contract=on` contracts `a*b+c` into a fused multiply-add *within a statement*;
  what actually reaches an `fma` depends on opt level, inlining, and target ISA — so an optimised release
  build and an unoptimised debug build need not produce the same rounding. — <https://clang.llvm.org/docs/UsersManual.html> · checked 2026-06
- Rust `f32::mul_add` is "optional contraction": it lowers to a hardware `fma` only when the target
  supports it and it's profitable; it is NOT guaranteed bit-identical to a separate `mul`+`add`. —
  <https://github.com/rust-lang/rust/issues/40406> · checked 2026-06
- **Takeaway for us:** never assume debug and release agree bit-for-bit; verify in `--release` (canonical)
  and keep both goldens pinned.

## 2. rayon ordering & float non-associativity

> **Anchor:** SKILL §0 corollaries — "parallel only the read-only decide phase, collect in index order;
> never float-add into a determinism-critical aggregate." The entries below are why.

- `IndexedParallelIterator::collect` **preserves index order**; `reduce`/`sum` do NOT fix the combine
  order. rayon's own docs warn: if `+` is not truly associative (floating point), the results are not
  fully deterministic. — <https://docs.rs/rayon/1.12.0/rayon/iter/trait.ParallelIterator.html> ·
  rayon **1.12.0** · checked 2026-06
- Floating-point addition is non-associative — `(a+b)+c ≠ a+(b+c)` under round-off — the classical
  reference. — Goldberg, *What Every Computer Scientist Should Know About Floating-Point Arithmetic*,
  <https://dl.acm.org/doi/10.1145/103162.103163> · checked 2026-06
- **Takeaway for us:** our index-order `decisions[i]` collect + serial fixed-order mutation is exactly the
  pattern that makes the parallel phases replay-exact; an integer/`to_bits` fold replaces any float reduce.

## 3. bincode positional format & the serde wire-proxy

> **Anchor:** SKILL §7 ("`bincode` is positional — a new field shifts every following byte") + the
> wire-proxy in `terrain.rs:391` (`determinism.md` §persist).

- bincode is a **positional, schema-less** encoding: layout is the struct field order; adding/reordering a
  field shifts all following bytes; there is no built-in versioning (hence our `MAGIC`). — bincode
  **1.3.3** (`Cargo.lock`), spec discussion <https://github.com/bincode-org/bincode/issues/517> ·
  checked 2026-06
- serde container attributes `#[serde(into = "Wire", from = "Wire")]` route (de)serialization through a
  stable intermediate type, so an in-memory relayout leaves the on-disk bytes unchanged (only the
  Wire↔struct conversion moves). This is the Tier-3 trick. —
  <https://serde.rs/container-attrs.html> · serde **1.0.228** · checked 2026-06
- **NB (version-specific):** we are on bincode **1.3.3**; the 2.x "schema fingerprint / SchemaHashMismatch"
  feature does NOT apply to us — our only guard against a stale decode is the `MAGIC` prefix
  (`persist.rs:31`). Don't assume bincode self-detects a layout mismatch.

## 4. glam determinism

- glam targets **bit-for-bit identical results across platforms by default**; enabling its `fast-math`
  feature *forfeits* that (allows FMA/SIMD reordering). We rely on the default. — glam **0.27.0**
  (`Cargo.lock`), <https://github.com/bitshifter/glam-rs> · checked 2026-06
- `Vec2` is scalar; `Vec3`/`Vec4` may use SIMD but without implicit fast-math; `Vec3A` is the explicitly
  SIMD-aligned variant. For strict determinism prefer the non-`A` types / default features. —
  <https://docs.rs/glam/0.27.0/glam/> · glam **0.27.0** · checked 2026-06
- **Takeaway for us:** keep glam on default features (no `fast-math`); geo/movement math stays reproducible.

## 5. fast-approx tanh / exp (anchors `fastmath.rs`)

> **Anchor:** `fastmath::tanh`/`exp` (`fastmath.rs:25`/`:38`) are pure-`f32`, deterministic, and
> bit-stable — the golden was re-pinned ONCE for the tanh approx (`sim.rs:1561`/`:1563`) and replays
> exactly in parallel since. The references below are the method, not a license to swap the impl.

- Schraudolph's fast `exp` exploits the IEEE-754 layout (the exponent field *is* a `2^x`), computing
  `e^x` via `2^(log2(e)·x)` — roughly LUT-with-interpolation speed, far cheaper than libm. —
  <https://nic.schraudolph.org/pubs/Schraudolph99.pdf> · checked 2026-06
- `tanh` on a bounded interval is well-approximated by a minimax polynomial (Remez / Chebyshev), trading a
  known max error for speed. — <https://mathworld.wolfram.com/RemezAlgorithm.html> · checked 2026-06
- **Bit-stability caveat (important):** a deterministic, RNG-free approximation is bit-identical *only if
  called in the same order with the same inputs*. Our parallel decide preserves per-index inputs, so the
  approx stays replay-exact — but any change to call order or accumulation can perturb bits
  (`determinism.md`). — non-associativity ref as §2 · checked 2026-06

## 6. splitmix64 / seed mixing (anchors `rng.rs`)

> **Anchor:** `splitmix64` (`rng.rs:8`), `seed_fold` (`rng.rs:18`) — our only sanctioned RNG primitives.

- SplitMix64 (Steele, Lea, Flood, OOPSLA 2014) is a fast 64-bit mixer for seeding; the standard mix is
  two `xor-shift-multiply` rounds (`0xbf58476d1ce4e5b9`, `0x94d049bb133111eb`) then a final xor-shift,
  giving strong avalanche (a one-bit input change flips ~half the output bits). —
  <https://gee.cs.oswego.edu/dl/papers/oopsla14.pdf>, reference impl
  <https://docs.rs/rand_xoshiro/latest/src/rand_xoshiro/splitmix64.rs.html> · checked 2026-06
- It is fast but NOT cryptographic; XOR-mixing distinct salts before the mix yields well-separated streams
  (the basis of our distinct-`SALT` per draw site). — same sources · checked 2026-06
- **Takeaway for us:** distinct `SALT` constants XOR'd into splitmix give independent streams; this is why
  toggling one feature's stream must not perturb another's (`determinism.md` §RNG).

## 7. Broad context — Rust profiling, SoA/SIMD, packing

- Profiling: `cargo-flamegraph` (Linux `perf`, macOS DTrace w/ SIP caveats), or `cargo instruments`
  (Apple Instruments) on macOS. Sampling shows proportional CPU time — good for hot paths; micro-bench
  needs `criterion`/`perf stat`. — <https://github.com/flamegraph-rs/flamegraph>,
  <https://nnethercote.github.io/perf-book/profiling.html> · checked 2026-06
- LLVM autovectorisation needs regular loops (SoA), no data-dependent branches, predictable access — which
  is *why* our flat-brain SoA spike couldn't be cheaply won (see `performance.md` dead levers). Cache line
  ≈ 64 B; AoS-packing co-read fields (our Tier-3 `BioCell`/`GeoCell`) cuts scattered lines; a LUT trades
  compute for memory-bandwidth/cache pressure (not always a win — Kleiber LUT washed). —
  <https://nnethercote.github.io/perf-book/build-configuration.html> · checked 2026-06

---

## Dropped (failed anchor-or-version-or-drop)

- "bincode debug/release same flag ⇒ no divergence" — contradicts our golden-locked §0 per-profile pin;
  NOT transcribed.
- bincode 2.x schema-fingerprint / "bincode 3.0 unmaintained" — wrong dep version (we are on **1.3.3**);
  irrelevant to this codebase, dropped to avoid a version-mismatch fact.
