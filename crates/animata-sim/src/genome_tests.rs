use super::*;
use crate::rng::Rng;

// ============================================================================
// PR-D0 SPIKE (throwaway, cfg(test) only — NEVER ships): does a position-anchored
// morphogen SOURCE + lattice DIFFUSION build a monotone axis gradient on ≤32 cells
// over DEV_STEPS, and does thresholding that gradient yield a COHESIVE organ (so
// there is a real organ_power fitness channel)? Gate A + Gate B of the plan.
// Reuses the real dev primitives (regulate / place_cell / is_adjacent).
// ============================================================================

/// Prototype dev loop WITH a one-channel morphogen. Per step: reaction (`regulate`) → diffusion
/// (synchronous snapshot, serial fixed index order) → DECAY (degradation) → source re-injection at
/// the origin cell (a position-anchored boundary pinned to 1.0) → division (as in `grow`). The DECAY
/// is what the spike discovered is essential: source + diffusion ALONE homogenise to the source value
/// (flat field, no gradient); a degradation term gives the screened-Poisson steady state c(r) ∝
/// exp(−r/λ), λ = √(D/k) — a real monotone gradient. `extra_settle` runs additional diffuse+decay
/// steps after growth stops, so the gradient can relax on the finished body (a tuning knob D0 reports
/// to D1: how many DEV_STEPS / settle steps the gradient needs). Returns lattice + concentration.
#[cfg(test)]
fn grow_spike(g: &Genome, diff_rate: f32, decay: f32, extra_settle: usize) -> (Vec<(i16, i16)>, Vec<f32>) {
    let mut seed = [0.0f32; G];
    seed[0] = 1.0;
    let mut states: Vec<[f32; G]> = vec![seed];
    let mut pos: Vec<(i16, i16)> = vec![(0, 0)];
    let mut morph: Vec<f32> = vec![1.0]; // origin starts as the source
    let mut grew = true;
    for step in 0..(DEV_STEPS + extra_settle) {
        let cur = states.len();
        for s in states.iter_mut().take(cur) {
            *s = g.regulate(s, &[0.0; N_MORPH]); // spike carries its own morphogen field separately
        }
        // DIFFUSION: each cell relaxes toward the mean of its 4-neighbours (snapshot → order-free).
        let snap = morph.clone();
        let n = states.len();
        for a in 0..n {
            let (mut sum, mut cnt) = (0.0f32, 0u32);
            for b in 0..n {
                if is_adjacent(pos[a], pos[b]) {
                    sum += snap[b];
                    cnt += 1;
                }
            }
            if cnt > 0 {
                morph[a] += diff_rate * (sum / cnt as f32 - morph[a]);
            }
        }
        // DECAY: uniform degradation — the term that makes a gradient instead of a flat field.
        for m in morph.iter_mut() {
            *m *= 1.0 - decay;
        }
        // SOURCE: re-pin the origin cell to 1.0 (position-anchored boundary, survives growth + decay).
        for i in 0..n {
            if pos[i] == (0, 0) {
                morph[i] = 1.0;
            }
        }
        // DIVISION (mirrors `grow`) — only while still in the growth phase.
        if step < DEV_STEPS && states.len() < MAX_CELLS {
            let (mut nb, mut nb_pos, mut nb_m) = (Vec::new(), Vec::new(), Vec::new());
            for i in 0..cur {
                let ns = states[i];
                if ns[GENE_DIVIDE] > DIVIDE_THETA && cur + nb.len() < MAX_CELLS {
                    let mut child = ns;
                    child[GENE_POLARITY] = -child[GENE_POLARITY];
                    let p = place_cell(pos[i], ns[GENE_POLARITY], &pos, &nb_pos);
                    nb.push(child);
                    nb_pos.push(p);
                    nb_m.push(morph[i]);
                }
            }
            grew = !nb.is_empty();
            states.extend(nb);
            pos.extend(nb_pos);
            morph.extend(nb_m);
        } else if !grew && extra_settle == 0 {
            break;
        }
    }
    (pos, morph)
}

/// Largest 4-connected component of the cells flagged `true` (flood fill on the lattice).
#[cfg(test)]
fn largest_true_cluster(pos: &[(i16, i16)], mask: &[bool]) -> u32 {
    let n = pos.len();
    let mut visited = vec![false; n];
    let mut best = 0u32;
    for start in 0..n {
        if !mask[start] || visited[start] {
            continue;
        }
        let mut stack = vec![start];
        visited[start] = true;
        let mut size = 0u32;
        while let Some(a) = stack.pop() {
            size += 1;
            for b in 0..n {
                if !visited[b] && mask[b] && is_adjacent(pos[a], pos[b]) {
                    visited[b] = true;
                    stack.push(b);
                }
            }
        }
        best = best.max(size);
    }
    best
}

/// A genome forced to grow a full body (so the gradient has a real lattice to form on): empty GRN
/// except a strong divide bias ⇒ tanh(b) > DIVIDE_THETA every step ⇒ grows to MAX_CELLS.
#[cfg(test)]
fn growing_genome() -> Genome {
    let mut g = Genome::founder(&mut Rng::new(1));
    g.grn_b[GENE_DIVIDE] = 1.0;
    g
}

/// GATE A — a position-anchored source + diffusion builds a MONOTONE axis gradient: morphogen
/// concentration falls with lattice distance from the origin source. Measured by a Kendall-style
/// concordance — the fraction of cell pairs where the nearer-to-origin cell holds MORE morphogen.
/// If this is not strongly > 0.5, pure diffusion is homogenising and PR-D is dead here.
#[test]
fn spike_gate_a_source_diffusion_builds_monotone_gradient() {
    let (pos, morph) = grow_spike(&growing_genome(), 0.5, 0.3, 8);
    let n = pos.len();
    assert!(n >= 16, "spike body too small to judge a gradient: {n} cells");
    let dist = |p: (i16, i16)| (p.0.abs() + p.1.abs()) as i32;

    // Concordance over all pairs at DIFFERENT distances: nearer ⇒ more morphogen.
    let (mut concordant, mut total) = (0u32, 0u32);
    for i in 0..n {
        for j in (i + 1)..n {
            let (di, dj) = (dist(pos[i]), dist(pos[j]));
            if di == dj {
                continue;
            }
            total += 1;
            let nearer_has_more = if di < dj { morph[i] > morph[j] } else { morph[j] > morph[i] };
            if nearer_has_more {
                concordant += 1;
            }
        }
    }
    let frac = concordant as f32 / total as f32;
    // Profile by distance band, for the log.
    let maxd = pos.iter().map(|&p| dist(p)).max().unwrap_or(0);
    let mut by_band: Vec<(i32, f32, u32)> = Vec::new();
    for d in 0..=maxd {
        let vals: Vec<f32> = (0..n).filter(|&i| dist(pos[i]) == d).map(|i| morph[i]).collect();
        if !vals.is_empty() {
            by_band.push((d, vals.iter().sum::<f32>() / vals.len() as f32, vals.len() as u32));
        }
    }
    eprintln!("GATE A: {n} cells, concordance(near⇒more) {:.3}", frac);
    for (d, mean, k) in &by_band {
        eprintln!("  dist {d}: mean morph {:.3}  ({k} cells)", mean);
    }
    assert!(frac > 0.7, "no monotone gradient — diffusion homogenised (concordance {frac:.3})");
}

/// GATE B — segregating cell TYPE by the gradient yields a COHESIVE organ: thresholding the morphogen
/// (high near the source) labels a spatially contiguous region, so its largest connected cluster
/// beats a RANDOM labelling of the same count. This is the selective pull — `organ_power` rewards the
/// largest connected same-type cluster, so a gradient-segregated body develops bigger organs than
/// chance, giving evolution a fitness channel to climb toward axial body plans.
#[test]
fn spike_gate_b_gradient_segregation_beats_random_cohesion() {
    let (pos, morph) = grow_spike(&growing_genome(), 0.5, 0.3, 8);
    let n = pos.len();
    // Threshold at the median morphogen so ~half the cells are "high" (the would-be organ).
    let mut sorted = morph.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[n / 2];
    let grad_mask: Vec<bool> = morph.iter().map(|&m| m >= median).collect();
    let high = grad_mask.iter().filter(|&&b| b).count();
    let grad_cluster = largest_true_cluster(&pos, &grad_mask);

    // Control: the SAME number of "high" labels scattered at random (averaged over seeds).
    let mut rng = Rng::new(777);
    let trials = 64;
    let mut rand_total = 0u32;
    let mut rand_best = 0u32;
    for _ in 0..trials {
        // Fisher–Yates pick `high` indices.
        let mut idx: Vec<usize> = (0..n).collect();
        for k in 0..high {
            let r = k + (rng.unit() * (n - k) as f32) as usize % (n - k).max(1);
            idx.swap(k, r.min(n - 1));
        }
        let mut mask = vec![false; n];
        for &i in idx.iter().take(high) {
            mask[i] = true;
        }
        let c = largest_true_cluster(&pos, &mask);
        rand_total += c;
        rand_best = rand_best.max(c);
    }
    let rand_mean = rand_total as f32 / trials as f32;
    eprintln!(
        "GATE B: {n} cells, {high} high. gradient organ {grad_cluster} vs random mean {:.1} (max {rand_best})",
        rand_mean
    );
    assert!(
        grad_cluster as f32 > rand_mean * 1.3,
        "gradient segregation gave no cohesion edge: organ {grad_cluster} vs random mean {rand_mean:.1}"
    );
}

/// The continuity keystone (autotroph-base): an empty-GRN founder — save the single `FOUNDER_PHOTO_BIAS`
/// constant — develops to EXACTLY one PHOTO cell (a sedentary ancestral "plant cell"): biomass 1, the
/// only specialisation photosynthesis, no division. The divide gene stays `tanh(0)=0 < DIVIDE_THETA`.
#[test]
fn founder_develops_to_one_photo_cell() {
    let mut rng = Rng::new(1);
    let g = Genome::founder(&mut rng);
    let p = g.develop();
    // PR-D-zones: a one-cell body is a single uniform zone (`zones = 1`), so the exact phenotype carries it.
    assert_eq!(p, Phenotype { n_cells: 1, photo: 1, organ: [0, 0, 0, 0, 0, 0, 1], zones: 1, ..Default::default() });
    assert_eq!(p.complexity(), 0); // one cell ⇒ no multicellular complexity
    // A single photo cell registers as a size-1 photo organ but carries NO effector organ, so the
    // effector organ_power is still the bare (zero) count — body-driven stats unchanged by morphogenesis.
    assert_eq!(p.organ_power(0), 0.0, "founder effector power must be the bare count (no organ bonus)");
    // PR-D1: a single cell has no spatial axis to speak of.
    assert_eq!(p.axis_order, 0, "a single-cell founder must have no axial order");
}

/// PR-D1 machinery: the morphogen READ path is correct and READY — a genome whose `morph_w` couples a
/// function gene to the (armed, position-anchored) morphogen gradient develops a body whose TYPE varies
/// with radial position, so `axis_order` rises well above the founder's 0. (In PR-D1 the evolutionary
/// coupling is held INERT — `mutate` freezes `morph_w` at 0 — so this constructs the coupling by hand
/// to prove the mechanism is sound for PR-D2 to switch on. Founder rates are armed, so only `morph_w`
/// is set here.)
#[test]
fn morphogen_read_makes_type_track_position() {
    // Grow a full body, and make the EFFECTOR gene respond strongly to morphogen channel 0: cells in
    // the high-morphogen core (near the origin source) specialise; the low-morphogen rim stays
    // structural ⇒ type segregates along the radial axis.
    let mut g = Genome::founder(&mut Rng::new(1));
    g.grn_b[GENE_DIVIDE] = 1.0; // force growth to MAX_CELLS so there is a body to pattern
    g.morph_w[GENE_EFFECTOR * N_MORPH] = 6.0; // effector gene reads the axis morphogen
    let p = g.develop();
    eprintln!("axis_order {} (n_cells {}, effector {})", p.axis_order, p.n_cells, p.effector);
    assert!(p.n_cells >= 16, "need a real body to judge an axis: {} cells", p.n_cells);
    assert!(p.effector > 0 && p.effector < p.n_cells, "morphogen should specialise SOME (not all) cells");
    assert!(p.axis_order > 20, "type did not track radial position via the morphogen ({})", p.axis_order);

    // And it is determined purely by the genome (re-developing is identical) — the dev path stays pure.
    assert_eq!(p, g.develop(), "morphogen development must be deterministic");
}

/// PR-D1 inertness anchor: with the founder's `morph_w = 0`, the morphogen is NOT read, so a grown
/// body's cell types — hence its `axis_order` — are whatever the pre-morphogen development produced.
/// A growing genome with NO morphogen coupling specialises NOTHING from the gradient, so its
/// single-type body has no axial order. (Guards that the inert path really is inert.)
#[test]
fn morphogen_inert_when_unread() {
    let mut g = Genome::founder(&mut Rng::new(1));
    g.grn_b[GENE_DIVIDE] = 1.0; // grows, but morph_w stays 0 ⇒ gradient unread
    let p = g.develop();
    assert_eq!(p.effector, 0, "no morphogen coupling ⇒ the gradient specialises no cells");
    assert_eq!(p.axis_order, 0, "an unread gradient must leave the body axis-less");
}

/// PR-D-zones: REGIONALISATION counts distinct contiguous type-regions along the radial axis. A founder
/// (one uniform cell) is a single zone; a body that reads the morphogen to segregate type by radius
/// carves ≥2 zones — and `zones` is deterministic. (The ≥3 emergence-under-selection is the 8000-tick
/// acceptance; here we lock the mechanism + the founder floor.)
#[test]
fn radial_zones_counts_regionalisation() {
    let f = Genome::founder(&mut Rng::new(1)).develop();
    assert_eq!(f.zones, 1, "a one-cell founder is a single uniform zone, got {}", f.zones);

    let mut g = Genome::founder(&mut Rng::new(1));
    g.grn_b[GENE_DIVIDE] = 1.0; // force a real body to regionalise
    g.morph_w[GENE_EFFECTOR * N_MORPH] = 6.0; // effector gene reads the axis morphogen
    let p = g.develop();
    eprintln!("zones {} (n_cells {}, axis_order {})", p.zones, p.n_cells, p.axis_order);
    assert!(p.n_cells >= 16, "need a real body to judge zones: {} cells", p.n_cells);
    assert!(p.zones >= 2, "morphogen regionalisation must carve ≥2 zones, got {}", p.zones);
    assert_eq!(p, g.develop(), "zones must be deterministic");
}

/// `organ_power` is monotone in BOTH cell count and organ coherence: adding a cell of the type, or
/// growing its largest cluster, never LOWERS the type's power. This is the no-fitness-valley
/// guarantee — the climb from scattered cells to a coherent organ is a smooth selective gradient,
/// never a cliff a lineage must leap.
#[test]
fn organ_power_is_monotone() {
    let pheno = |count: u32, organ0: u8| Phenotype { effector: count, organ: [organ0, 0, 0, 0, 0, 0, 0], ..Default::default() };
    // Monotone in count (organ fixed).
    for o in 0u8..=12 {
        for c in 0u32..24 {
            assert!(
                pheno(c + 1, o).organ_power(0) >= pheno(c, o).organ_power(0),
                "power dropped when count rose (count {c}, organ {o})"
            );
        }
    }
    // Monotone in organ coherence (count fixed).
    for c in 1u32..24 {
        for o in 0u8..12 {
            assert!(
                pheno(c, o + 1).organ_power(0) >= pheno(c, o).organ_power(0),
                "power dropped when the organ grew (count {c}, organ {o})"
            );
        }
    }
    // A coherent organ strictly beats the same cells scattered.
    assert!(pheno(4, 4).organ_power(0) > pheno(4, 1).organ_power(0), "a coherent organ must beat scattered cells");
}

/// Development is bounded and deterministic for any genome (cost + replay).
#[test]
fn development_is_bounded_and_deterministic() {
    for seed in 0..200u64 {
        let mut rng = Rng::new(seed);
        let mut mrng = Rng::new(seed ^ 0xD2);
        let mut grng = Rng::new(seed ^ 0x6A5);
        // A mutated genome (non-empty GRN) — may grow a body.
        let g = Genome::founder(&mut rng).mutate(&mut rng, &mut mrng, &mut grng, 0.3, 0.8);
        let p1 = g.develop();
        let p2 = g.develop();
        assert_eq!(p1, p2, "development not deterministic");
        assert!(p1.n_cells >= 1 && p1.n_cells as usize <= MAX_CELLS, "cell count out of range: {}", p1.n_cells);
        let typed = p1.effector + p1.storage + p1.sensor + p1.predator + p1.flight + p1.burrow + p1.photo;
        assert_eq!(typed + p1.structural, p1.n_cells);
    }
}

/// Mutation can grow multicellular AND specialised bodies (the mechanism isn't stuck at 1
/// cell) — over many random GRNs we see >1-cell bodies and ≥2-type complex ones.
#[test]
fn mutation_can_grow_complex_bodies() {
    let (mut multi, mut complex, mut maxn) = (0, 0, 0u32);
    for seed in 0..2000u64 {
        let mut rng = Rng::new(seed ^ 0xABCD);
        let mut mrng = Rng::new(seed ^ 0xABCD ^ 0xD2);
        let mut grng = Rng::new(seed ^ 0xABCD ^ 0x6A5);
        // Several mutation steps so the GRN drifts well away from empty.
        let mut g = Genome::founder(&mut rng);
        for _ in 0..5 {
            g = g.mutate(&mut rng, &mut mrng, &mut grng, 0.3, 0.9);
        }
        let p = g.develop();
        if p.n_cells > 1 {
            multi += 1;
        }
        if p.complexity() == 2 {
            complex += 1;
        }
        maxn = maxn.max(p.n_cells);
    }
    eprintln!("of 2000 drifted GRNs: {multi} multicellular, {complex} complex, max cells {maxn}");
    assert!(multi > 50, "almost no multicellular bodies emerge: {multi}");
    assert!(complex > 5, "no complex (multi-type) bodies emerge: {complex}");
}

/// PR-B: the spatial body layout (positions + the differential-adhesion sort) is deterministic
/// WITHIN a profile, every cell occupies a UNIQUE lattice slot, and the sort PRESERVES the cell
/// multiset — its `(structural + per-type)` tally equals `develop()`'s counts (so the sort can never
/// shift the golden). Checked across many drifted multicellular GRNs.
#[test]
fn body_layout_is_deterministic_and_preserves_counts() {
    let mut checked = 0;
    for seed in 0..1500u64 {
        let mut rng = Rng::new(seed ^ 0x5151);
        let mut mrng = Rng::new(seed ^ 0x5151 ^ 0xD2);
        let mut grng = Rng::new(seed ^ 0x5151 ^ 0x6A5);
        let mut g = Genome::founder(&mut rng);
        for _ in 0..5 {
            g = g.mutate(&mut rng, &mut mrng, &mut grng, 0.3, 0.9);
        }
        let p = g.develop();
        if p.n_cells <= 1 {
            continue; // exercise real multicellular bodies
        }
        checked += 1;

        // Deterministic within a profile: two layouts of the same genome are byte-identical.
        let layout = g.body_layout();
        assert_eq!(layout, g.body_layout(), "body_layout must be deterministic for a fixed genome");
        assert_eq!(layout.len() as u32, p.n_cells, "layout cell count must equal n_cells");

        // Every cell occupies a unique lattice slot.
        let mut coords: Vec<(i16, i16)> = layout.iter().map(|&(x, y, _)| (x, y)).collect();
        coords.sort_unstable();
        coords.dedup();
        assert_eq!(coords.len() as u32, p.n_cells, "two cells share a lattice slot");

        // The sort preserves the multiset: per-type tally over the layout == develop() counts.
        let mut tally = [0u32; 8]; // 0 structural, 1..=7 functions
        for &(_, _, ty) in &layout {
            tally[ty as usize] += 1;
        }
        assert_eq!(tally[0], p.structural, "structural count drifted");
        assert_eq!(tally[1], p.effector, "effector count drifted");
        assert_eq!(tally[2], p.storage, "storage count drifted");
        assert_eq!(tally[3], p.sensor, "sensor count drifted");
        assert_eq!(tally[4], p.predator, "predator count drifted");
        assert_eq!(tally[5], p.flight, "flight count drifted");
        assert_eq!(tally[6], p.burrow, "burrow count drifted");
        assert_eq!(tally[7], p.photo, "photo count drifted");
    }
    eprintln!("body_layout: checked {checked} multicellular layouts — deterministic, unique slots, counts preserved");
    assert!(checked > 50, "too few multicellular layouts exercised: {checked}");
}

// ============================================================================
// PR-D-segments SPIKE (throwaway, #[ignore], exploratory — NEVER ships, no assert
// thresholds yet). Resolves the plan's two gates by MEASURING genome space:
//   Gate A — what axial structure is REACHABLE from the existing linear read?
//     * ZONES (regionalisation): distinct contiguous TYPE-regions along the radius
//       — monotone-achievable (k thresholds → k+1 zones). The CHEAP rung.
//     * SEGMENTS (metamerism): one type repeating along the radius (A/gap/A) —
//       needs a NON-monotone M→type map. The HARD rung (a single linear morph read
//       + tanh is monotone in M(r) ∝ exp(−r/λ), so a SINGLE type should give 1 run;
//       any segments≥2 means the DEV_STEPS recurrence / polarity flip bent it).
//   Gate B — does a segmented/zoned body keep an organ_power channel, or does
//     splitting a type into bands strictly shrink its largest cluster (organ_power
//     genome.rs:112 rewards ONE big cluster) ⇒ selection opposes it ⇒ need B1.
// Uses the REAL grow path (regulate FED the morphogen), PRE-adhesion_sort (F6).
// ============================================================================

/// The real `grow` dev loop, returning the PRE-`adhesion_sort` `(states, pos)` (what the production
/// metric will see, F6). A faithful copy of `Genome::grow` (genome.rs:365-411) minus the sort.
#[cfg(test)]
fn grow_presort(g: &Genome) -> (Vec<[f32; G]>, Vec<(i16, i16)>) {
    let mut seed = [0.0f32; G];
    seed[0] = 1.0;
    let mut states: Vec<[f32; G]> = vec![seed];
    let mut pos: Vec<(i16, i16)> = vec![(0, 0)];
    let mut morph: Vec<[f32; N_MORPH]> = vec![[0.0; N_MORPH]];
    let reads_morph = g.morph_w.iter().any(|&w| w != 0.0);
    for _ in 0..DEV_STEPS {
        let cur = states.len();
        let (mut nb, mut nb_pos, mut nb_m) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..cur {
            let ns = g.regulate(&states[i], &morph[i]);
            states[i] = ns;
            if ns[GENE_DIVIDE] > DIVIDE_THETA && cur + nb.len() < MAX_CELLS {
                let mut child = ns;
                child[GENE_POLARITY] = -child[GENE_POLARITY];
                let p = place_cell(pos[i], ns[GENE_POLARITY], &pos, &nb_pos);
                nb.push(child);
                nb_pos.push(p);
                nb_m.push(morph[i]);
            }
        }
        let settled = nb.is_empty();
        states.extend(nb);
        pos.extend(nb_pos);
        morph.extend(nb_m);
        if reads_morph {
            diffuse_morphogen(&mut morph, &pos, &g.diff_rate, &g.decay_rate);
        }
        if settled || states.len() >= MAX_CELLS {
            break;
        }
    }
    (states, pos)
}

/// METAMERISM: for each function type, presence runs along the radius — the count of SEPARATED bands of
/// that one type (A present / absent / present = 2). Returns the (best type, its run count). A run needs
/// ≥1 cell at a radius to be "present"; segments≥2 for one type ⇒ that type recurs at separated radii ⇒
/// the M→type map was NON-monotone (the hard rung). Min run cell-count guards scattered singletons.
#[cfg(test)]
fn type_metamerism(states: &[[f32; G]], pos: &[(i16, i16)]) -> (u8, u8) {
    let maxd = pos.iter().map(|&(x, y)| (x.abs() + y.abs()) as i32).max().unwrap_or(0);
    let (mut best_t, mut best_runs) = (0u8, 0u8);
    for t in 1u8..=7 {
        let mut present: Vec<bool> = Vec::new();
        let mut run_cells: Vec<u32> = Vec::new(); // cells of t at each radius
        for r in 0..=maxd {
            let c = (0..pos.len())
                .filter(|&i| (pos[i].0.abs() + pos[i].1.abs()) as i32 == r && cell_type(&states[i]) == t)
                .count() as u32;
            present.push(c >= 1);
            run_cells.push(c);
        }
        // Count runs whose TOTAL cells ≥ ORGAN_MIN (kills scattered singletons, per-run not per-radius).
        let (mut runs, mut k) = (0u8, 0usize);
        while k < present.len() {
            if present[k] {
                let (start, mut total) = (k, 0u32);
                while k < present.len() && present[k] {
                    total += run_cells[k];
                    k += 1;
                }
                let _ = start;
                if total >= crate::config::ORGAN_MIN as u32 {
                    runs = runs.saturating_add(1);
                }
            } else {
                k += 1;
            }
        }
        if runs > best_runs {
            best_runs = runs;
            best_t = t;
        }
    }
    (best_t, best_runs)
}

/// SPIKE: sweep genome space from a growing genome (random grn_w + morph_w perturbations) and tally what
/// axial structure is reachable. Prints the ZONES and SEGMENTS distributions + the Gate-B organ tradeoff.
#[test]
#[ignore = "exploratory PR-D-segments spike — run with --ignored, read the log, decide go/no-go"]
fn spike_segments_reachability() {
    let mut rng = Rng::new(20260623);
    let mut zones_hist = [0u32; 12];
    let mut seg_hist = [0u32; 12];
    let mut bodies = 0u32;
    // Gate B: dominant-type largest cluster (POST-sort, as organ_power reads it) bucketed by segments.
    let (mut org_seg1, mut n_seg1, mut org_seg2, mut n_seg2) = (0u64, 0u32, 0u64, 0u32);
    let mut best_seg_example = (0u8, 0u8, 0u32); // (segments, type, n_cells)
    let mut best_zone_example = (0u8, 0u32);
    // Decorrelation: corr(zones, n_cells) — is `zones` a real plan descriptor or just body size?
    let (mut sz, mut sn, mut szz, mut snn, mut szn, mut cnt) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);

    for std_w in [0.4f32, 0.8, 1.4, 2.0] {
        for std_m in [2.0f32, 4.0, 7.0] {
            for _ in 0..600 {
                let mut g = growing_genome();
                for w in g.grn_w.iter_mut() {
                    *w += std_w * rng.signed();
                }
                g.grn_b[GENE_DIVIDE] = 1.0; // keep growth alive after the perturbation
                for c in 0..G {
                    g.morph_w[c * N_MORPH] = std_m * rng.signed(); // every gene may read the axis morphogen
                }
                let (states, pos) = grow_presort(&g);
                if states.len() < 12 {
                    continue; // need a real body to judge an axis
                }
                bodies += 1;
                let z = radial_zones(&states, &pos) as usize;
                let (seg_t, seg) = type_metamerism(&states, &pos);
                zones_hist[z.min(11)] += 1;
                seg_hist[(seg as usize).min(11)] += 1;
                if seg >= best_seg_example.0 {
                    best_seg_example = (seg, seg_t, states.len() as u32);
                }
                if (z as u8) >= best_zone_example.0 {
                    best_zone_example = (z as u8, states.len() as u32);
                }
                let (zf, nf) = (z as f64, states.len() as f64);
                sz += zf;
                sn += nf;
                szz += zf * zf;
                snn += nf * nf;
                szn += zf * nf;
                cnt += 1.0;
                // Gate B: organ = largest connected cluster of the dominant function type, POST-sort.
                let mut ss = states.clone();
                adhesion_sort(&mut ss, &pos);
                let organ = largest_organs(&ss, &pos);
                let dom = (1u8..=7).max_by_key(|&t| organ[(t - 1) as usize]).unwrap();
                let dom_org = organ[(dom - 1) as usize] as u64;
                if seg >= 2 {
                    org_seg2 += dom_org;
                    n_seg2 += 1;
                } else {
                    org_seg1 += dom_org;
                    n_seg1 += 1;
                }
            }
        }
    }
    eprintln!("=== PR-D-segments SPIKE: {bodies} bodies (≥12 cells) ===");
    eprintln!("ZONES (regionalisation, monotone-OK) distribution:");
    for (z, &c) in zones_hist.iter().enumerate() {
        if c > 0 {
            eprintln!("  {z} zones: {c}");
        }
    }
    eprintln!("  best: {} zones at {} cells", best_zone_example.0, best_zone_example.1);
    eprintln!("SEGMENTS (metamerism, needs non-monotone) distribution:");
    for (s, &c) in seg_hist.iter().enumerate() {
        if c > 0 {
            eprintln!("  {s} segments: {c}");
        }
    }
    eprintln!(
        "  best: {} segments of type {} at {} cells",
        best_seg_example.0, best_seg_example.1, best_seg_example.2
    );
    let m1 = if n_seg1 > 0 { org_seg1 as f32 / n_seg1 as f32 } else { 0.0 };
    let m2 = if n_seg2 > 0 { org_seg2 as f32 / n_seg2 as f32 } else { 0.0 };
    eprintln!(
        "GATE B: mean dominant-organ size — segments≤1: {m1:.2} (n={n_seg1})  vs  segments≥2: {m2:.2} (n={n_seg2})"
    );
    eprintln!("  (if segments≥2 organ << segments≤1 organ ⇒ banding shrinks organs ⇒ selection opposes ⇒ B1 needed)");
    let corr = (cnt * szn - sz * sn) / (((cnt * szz - sz * sz) * (cnt * snn - sn * sn)).sqrt());
    eprintln!("DECORRELATION: corr(zones, n_cells) = {corr:.3}  (axis_order's bar is < 0.6)");
}
