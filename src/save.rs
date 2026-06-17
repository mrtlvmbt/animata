//! Save / load the whole world to a compact text file (no extra dependencies).
//!
//! Format (whitespace-separated):
//! ```text
//! animata-save v1
//! behavior n
//! biome 314159
//! tick 12345
//! food 512
//! <x> <y>            (one line per pellet)
//! ...
//! creatures 240
//! <n|r> <id> <parent|-> <lineage> <x> <y> <heading> <energy> <age> <gen> <ACGT>
//! ...
//! ```
//! Phenotype and brain are re-derived from the genome on load, so only genome +
//! mutable state are stored.

use crate::behavior::BehaviorKind;
use crate::biome::BiomeMap;
use crate::config::Params;
use crate::creature::Creature;
use crate::phylo::Ancestry;
use crate::genome::Genome;
use crate::stats::Stats;
use crate::world::{Profile, World};
use macroquad::math::Vec2;
use std::fmt::Write as _;
use std::io::{Error, ErrorKind, Result};

pub fn save(world: &World, path: &str) -> Result<()> {
    let mut s = String::new();
    let _ = writeln!(s, "animata-save v1");
    let _ = writeln!(s, "behavior {}", world.behavior.code());
    let _ = writeln!(s, "biome {}", world.biome_seed);
    let _ = writeln!(s, "tick {}", world.tick);
    let _ = writeln!(s, "food {}", world.food.len());
    for ((f, fl), layer) in world.food.iter().zip(&world.flavor).zip(&world.food_layer) {
        let _ = writeln!(s, "{} {} {} {}", f.x, f.y, fl, layer);
    }
    let _ = writeln!(s, "creatures {}", world.creatures.len());
    for c in &world.creatures {
        let parent = c.parent_id.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
        let _ = writeln!(
            s,
            "{} {} {} {} {} {} {} {} {} {} {}",
            c.kind.code(),
            c.id,
            parent,
            c.lineage,
            c.pos.x,
            c.pos.y,
            c.heading,
            c.energy,
            c.age,
            c.generation,
            c.genome.to_string(),
        );
    }
    std::fs::write(path, s)?;
    // Persist the ancestry tree alongside the save so the phylogeny survives load.
    world.ancestry.export_csv(&tree_path(path))
}

/// Companion file holding the ancestry tree for a given save path.
fn tree_path(path: &str) -> String {
    format!("{path}.tree")
}

pub fn load(path: &str) -> Result<World> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();

    let header = lines.next().unwrap_or_default();
    // Accept the legacy "life-save" magic too, so pre-rename saves still load.
    if !(header.starts_with("animata-save") || header.starts_with("life-save")) {
        return Err(bad("not an animata-save file"));
    }

    let behavior_code = tagged(lines.next(), "behavior")?;
    let behavior = behavior_code
        .chars()
        .next()
        .and_then(BehaviorKind::from_code)
        .ok_or_else(|| bad("unknown behavior code"))?;

    let biome_seed: u64 =
        tagged(lines.next(), "biome")?.parse().map_err(|_| bad("bad biome seed"))?;
    let tick: u64 = tagged(lines.next(), "tick")?.parse().map_err(|_| bad("bad tick"))?;

    let food_n: usize = tagged(lines.next(), "food")?.parse().map_err(|_| bad("bad food count"))?;
    let mut food = Vec::with_capacity(food_n);
    let mut flavor = Vec::with_capacity(food_n);
    let mut food_layer = Vec::with_capacity(food_n);
    for _ in 0..food_n {
        let line = lines.next().ok_or_else(|| bad("truncated food"))?;
        let mut t = line.split_whitespace();
        let x = next_f32(&mut t)?;
        let y = next_f32(&mut t)?;
        // Flavor + layer columns are optional (older saves omit them).
        let fl = t.next().and_then(|s| s.parse().ok()).unwrap_or(0.5);
        let layer = t.next().and_then(|s| s.parse().ok()).unwrap_or(crate::config::LAYER_SURFACE);
        food.push(Vec2::new(x, y));
        flavor.push(fl);
        food_layer.push(layer);
    }

    let cre_n: usize =
        tagged(lines.next(), "creatures")?.parse().map_err(|_| bad("bad creature count"))?;
    let mut creatures = Vec::with_capacity(cre_n);
    for _ in 0..cre_n {
        let line = lines.next().ok_or_else(|| bad("truncated creatures"))?;
        let mut t = line.split_whitespace();
        let kind = t
            .next()
            .and_then(|s| s.chars().next())
            .and_then(BehaviorKind::from_code)
            .ok_or_else(|| bad("bad kind"))?;
        let id: u64 = t.next().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad id"))?;
        let parent_id = match t.next() {
            Some("-") => None,
            Some(s) => Some(s.parse().map_err(|_| bad("bad parent id"))?),
            None => return Err(bad("missing parent id")),
        };
        let lineage: u32 = t.next().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad lineage"))?;
        let x = next_f32(&mut t)?;
        let y = next_f32(&mut t)?;
        let heading = next_f32(&mut t)?;
        let energy = next_f32(&mut t)?;
        let age: u32 = t.next().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad age"))?;
        let generation: u32 =
            t.next().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad generation"))?;
        let genome = Genome::from_acgt(t.next().ok_or_else(|| bad("missing genome"))?);
        creatures.push(Creature::restore(
            id,
            parent_id,
            lineage,
            genome,
            Vec2::new(x, y),
            heading,
            energy,
            age,
            generation,
            kind,
        ));
    }

    let next_id = creatures.iter().map(|c| c.id).max().map_or(0, |m| m + 1);
    // Restore the full ancestry tree from its companion file if present; otherwise
    // (e.g. an old save) treat the loaded population as founders of a fresh tree.
    let ancestry = Ancestry::import_csv(&tree_path(path)).unwrap_or_else(|_| {
        let mut a = Ancestry::new();
        for c in &creatures {
            a.record_birth(c.id, None, tick, c.lineage);
        }
        a
    });
    Ok(World {
        creatures,
        food,
        flavor,
        food_layer,
        tick,
        stats: Stats::new(),
        behavior,
        next_id,
        params: Params::default(),
        biome: BiomeMap::new(biome_seed),
        biome_seed,
        ancestry,
        speciation: crate::speciation::Speciation::new(),
        drought_until: 0,
        circulating_strain: macroquad::rand::gen_range(0.0f32, 1.0),
        profile: Profile::default(),
        morpho: crate::world::MorphoCohort::default(),
        markers: crate::marker::MarkerField::new(
            crate::config::WORLD_W,
            crate::config::WORLD_H,
            crate::config::MARKER_CELL,
        ),
        g_food: Default::default(),
        g_cre: Default::default(),
        buf_cpos: Vec::new(),
        buf_carns: Vec::new(),
        buf_targets: Vec::new(),
    })
}

fn bad(msg: &str) -> Error {
    Error::new(ErrorKind::InvalidData, msg.to_string())
}

/// Expect a `"<tag> <value>"` line and return the value part.
fn tagged(line: Option<&str>, tag: &str) -> Result<String> {
    let line = line.ok_or_else(|| bad("unexpected end of file"))?;
    let rest = line
        .strip_prefix(tag)
        .ok_or_else(|| bad("missing expected tag"))?;
    Ok(rest.trim().to_string())
}

fn next_f32<'a>(t: &mut impl Iterator<Item = &'a str>) -> Result<f32> {
    t.next().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad number"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_round_trip_preserves_world() {
        let mut w = World::new(5, BehaviorKind::Neural);
        for _ in 0..200 {
            w.step();
        }
        let path = std::env::temp_dir().join("life_test_save.txt");
        let path = path.to_str().unwrap();

        save(&w, path).unwrap();
        let r = load(path).unwrap();

        assert_eq!(r.tick, w.tick);
        assert_eq!(r.creatures.len(), w.creatures.len());
        assert_eq!(r.food.len(), w.food.len());

        // Spot-check the first creature: genome and mutable state survive, and
        // the phenotype was correctly re-derived from the genome.
        let a = &w.creatures[0];
        let b = &r.creatures[0];
        assert_eq!(a.genome.nt, b.genome.nt);
        assert_eq!(a.generation, b.generation);
        assert_eq!(a.pos, b.pos);
        // Phenotype (incl. the marker-decoded brain) re-derives from the genome.
        assert_eq!(a.pheno.synapses.len(), b.pheno.synapses.len());

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(tree_path(path));
    }

    #[test]
    fn save_load_preserves_ancestry_tree() {
        let mut w = World::new(9, BehaviorKind::Neural);
        for _ in 0..1200 {
            w.step();
        }
        let path = std::env::temp_dir().join("life_test_tree.txt");
        let path = path.to_str().unwrap();
        save(&w, path).unwrap();
        let r = load(path).unwrap();

        // The whole ancestry log survives the round trip...
        assert_eq!(w.ancestry.len(), r.ancestry.len(), "ancestry size changed");
        // ...and the deepest living creature's ancestor chain is identical.
        let id = w
            .creatures
            .iter()
            .max_by_key(|c| w.ancestry.depth(c.id))
            .map(|c| c.id)
            .unwrap();
        assert!(w.ancestry.depth(id) > 0, "expected at least one creature with ancestors");
        assert_eq!(w.ancestry.ancestors(id, 100), r.ancestry.ancestors(id, 100));

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(tree_path(path));
    }
}
