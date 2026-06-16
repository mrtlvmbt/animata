//! Ancestry log: records every creature's birth (id, parent, tick, lineage) and
//! death tick — including the dead — so the full family tree can be walked or
//! exported. Bounded by a record cap so a long session can't grow without limit.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;

/// Safety cap on stored records (normally kept far below this by pruning).
const MAX_RECORDS: usize = 5_000_000;

#[derive(Clone, Copy)]
struct Record {
    id: u64,
    parent: Option<u64>,
    birth: u64,
    death: Option<u64>,
    lineage: u32,
}

#[derive(Default)]
pub struct Ancestry {
    records: Vec<Record>,
    index: HashMap<u64, usize>,
}

impl Ancestry {
    pub fn new() -> Self {
        Ancestry {
            records: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub fn record_birth(&mut self, id: u64, parent: Option<u64>, birth: u64, lineage: u32) {
        if self.records.len() >= MAX_RECORDS || self.index.contains_key(&id) {
            return;
        }
        self.index.insert(id, self.records.len());
        self.records.push(Record {
            id,
            parent,
            birth,
            death: None,
            lineage,
        });
    }

    pub fn record_death(&mut self, id: u64, tick: u64) {
        if let Some(&i) = self.index.get(&id) {
            self.records[i].death = Some(tick);
        }
    }

    /// Chain of ancestor ids from the given creature up toward its founder
    /// (nearest parent first), capped at `limit` entries.
    pub fn ancestors(&self, id: u64, limit: usize) -> Vec<u64> {
        let mut chain = Vec::new();
        let mut cur = id;
        while chain.len() < limit {
            let Some(&i) = self.index.get(&cur) else { break };
            let Some(parent) = self.records[i].parent else { break };
            chain.push(parent);
            cur = parent;
        }
        chain
    }

    /// Depth of a creature's full ancestor chain (how many ancestors are logged).
    pub fn depth(&self, id: u64) -> usize {
        self.ancestors(id, usize::MAX).len()
    }

    /// Rebuild an ancestry log from a CSV previously written by [`export_csv`]
    /// (`id,parent,birth,death,lineage`). Used to persist the tree across saves.
    pub fn import_csv(path: &str) -> std::io::Result<Ancestry> {
        let text = std::fs::read_to_string(path)?;
        let mut a = Ancestry::new();
        for line in text.lines().skip(1) {
            if line.is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(',').collect();
            if f.len() != 5 {
                continue;
            }
            let parse_u64 = |s: &str| s.parse::<u64>().ok();
            let (Some(id), Some(birth), Some(lineage)) =
                (parse_u64(f[0]), parse_u64(f[2]), f[4].parse::<u32>().ok())
            else {
                continue;
            };
            let parent = if f[1].is_empty() { None } else { parse_u64(f[1]) };
            let death = if f[3].is_empty() { None } else { parse_u64(f[3]) };
            a.index.insert(id, a.records.len());
            a.records.push(Record { id, parent, birth, death, lineage });
        }
        Ok(a)
    }

    /// Write the whole tree as an edge list CSV for external rendering.
    pub fn export_csv(&self, path: &str) -> std::io::Result<()> {
        let mut s = String::from("id,parent,birth,death,lineage\n");
        for r in &self.records {
            let parent = r.parent.map(|p| p.to_string()).unwrap_or_default();
            let death = r.death.map(|d| d.to_string()).unwrap_or_default();
            s.push_str(&format!("{},{},{},{},{}\n", r.id, parent, r.birth, death, r.lineage));
        }
        let mut f = std::fs::File::create(path)?;
        f.write_all(s.as_bytes())
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Garbage-collect: keep only the ancestors of the given living creatures
    /// (full chains to their founders), dropping extinct dead-end branches. This
    /// bounds the log to the relevant ancestry so the tree always reaches roots.
    pub fn prune(&mut self, living: &[u64]) {
        let mut keep: HashSet<u64> = HashSet::new();
        for &l in living {
            let mut cur = l;
            while keep.insert(cur) {
                let Some(&i) = self.index.get(&cur) else { break };
                match self.records[i].parent {
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }
        self.records.retain(|r| keep.contains(&r.id));
        self.index.clear();
        for (i, r) in self.records.iter().enumerate() {
            self.index.insert(r.id, i);
        }
    }

    /// The coalescent tree of a set of (living) creatures: the deduplicated union
    /// of their ancestor paths back to founders. Shared ancestors appear once, so
    /// the result is the genealogy of that population converging on its roots.
    pub fn coalescent(&self, living: &[u64]) -> Vec<TreeNode> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for &start in living {
            let mut cur = start;
            loop {
                if !seen.insert(cur) {
                    break; // this node (and its ancestors) already collected
                }
                let Some(&i) = self.index.get(&cur) else { break };
                let r = self.records[i];
                out.push(TreeNode {
                    id: r.id,
                    parent: r.parent,
                    birth: r.birth,
                    lineage: r.lineage,
                });
                match r.parent {
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }
        out
    }
}

/// A node in a coalescent tree (for rendering).
pub struct TreeNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub birth: u64,
    pub lineage: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_chain_walks_to_founder() {
        let mut a = Ancestry::new();
        a.record_birth(0, None, 0, 0); // founder
        a.record_birth(1, Some(0), 5, 0);
        a.record_birth(2, Some(1), 10, 0);
        assert_eq!(a.ancestors(2, 10), vec![1, 0]);
        assert_eq!(a.depth(2), 2);
        assert_eq!(a.depth(0), 0, "founder has no ancestors");
        assert_eq!(a.ancestors(2, 1), vec![1], "chain respects the limit");
        a.record_death(1, 20); // must not panic or affect the chain
        assert_eq!(a.depth(2), 2);
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn prune_keeps_living_ancestry_drops_dead_ends() {
        let mut a = Ancestry::new();
        a.record_birth(0, None, 0, 0); // founder
        a.record_birth(1, Some(0), 1, 0);
        a.record_birth(2, Some(1), 2, 0); // living lineage tip
        a.record_birth(3, Some(0), 1, 0); // extinct dead-end branch
        a.prune(&[2]);
        // Living tip's full chain to the founder survives...
        assert_eq!(a.ancestors(2, 10), vec![1, 0]);
        // ...and reaches a real root (founder, parent None).
        assert_eq!(a.depth(0), 0);
        // The dead-end branch is gone.
        assert_eq!(a.len(), 3);
        assert!(a.ancestors(3, 10).is_empty());
    }
}
