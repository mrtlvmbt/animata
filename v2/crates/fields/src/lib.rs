//! `fields` — CPU `FieldStore` backend for the **conserved** resource field (R13).
//!
//! Fixed-point INTEGER end-to-end: every agent↔field exchange is an exact integer add/sub
//! (conservation by construction), regeneration is a bounded integer source, and diffusion is the
//! flux-conserving integer transport of engineering/14 §5.1 (Σ field invariant EXACTLY — no ε). No
//! float anywhere in this crate.

use sim_core::{FieldStore, Vec2Fixed};

/// CPU resource field on a `grid_w × grid_h` cell grid (`M_field` world-voxels per cell).
pub struct CpuResourceField {
    m_field: i64,
    dim: i64,
    grid_w: i64,
    grid_h: i64,
    grid: Vec<i64>,
    staging: Vec<i64>,
    caps: Vec<i64>,
    regen_rate: i64,
    /// Diffusion flux divisor exponent: flux = (a−b) >> diffuse_shift. `>=3` satisfies CFL (≤1/4).
    diffuse_shift: u32,
}

impl CpuResourceField {
    /// `caps[cell]` is the per-cell regeneration cap (from `WorldView::resource`). Cells are filled to
    /// half their cap at start (a gentle initial condition).
    pub fn new(dim: i64, m_field: i64, caps: Vec<i64>, regen_rate: i64, diffuse_shift: u32) -> Self {
        let grid_w = dim / m_field;
        let grid_h = dim / m_field;
        assert_eq!(caps.len(), (grid_w * grid_h) as usize, "caps must cover the grid");
        let grid: Vec<i64> = caps.iter().map(|c| c / 2).collect();
        let staging = vec![0i64; caps.len()];
        CpuResourceField { m_field, dim, grid_w, grid_h, grid, staging, caps, regen_rate, diffuse_shift }
    }

    #[inline]
    fn cell_coords(&self, pos: Vec2Fixed) -> (i64, i64) {
        let x = pos.0.rem_euclid(self.dim) / self.m_field;
        let z = pos.1.rem_euclid(self.dim) / self.m_field;
        (x, z)
    }

    #[inline]
    fn idx(&self, cx: i64, cz: i64) -> usize {
        (cz.rem_euclid(self.grid_h) * self.grid_w + cx.rem_euclid(self.grid_w)) as usize
    }
}

impl FieldStore for CpuResourceField {
    fn m_field(&self) -> i64 {
        self.m_field
    }

    fn cell_index(&self, pos: Vec2Fixed) -> usize {
        let (cx, cz) = self.cell_coords(pos);
        self.idx(cx, cz)
    }

    fn amount_at(&self, pos: Vec2Fixed) -> i64 {
        self.grid[self.cell_index(pos)]
    }

    fn gradient_at(&self, pos: Vec2Fixed, range: i64) -> (i64, i64) {
        let (cx, cz) = self.cell_coords(pos);
        let gx = self.grid[self.idx(cx + range, cz)] - self.grid[self.idx(cx - range, cz)];
        let gz = self.grid[self.idx(cx, cz + range)] - self.grid[self.idx(cx, cz - range)];
        (gx, gz)
    }

    fn take_at(&mut self, pos: Vec2Fixed, amount: i64) -> i64 {
        let i = self.cell_index(pos);
        let got = amount.min(self.grid[i]).max(0);
        self.grid[i] -= got;
        got
    }

    fn scatter_at(&mut self, pos: Vec2Fixed, amount: i64) {
        let i = self.cell_index(pos);
        self.staging[i] += amount;
    }

    fn apply_scatter(&mut self) {
        for (g, s) in self.grid.iter_mut().zip(self.staging.iter_mut()) {
            *g += *s;
            *s = 0;
        }
    }

    fn regenerate(&mut self) -> i64 {
        let mut injected = 0;
        for (g, cap) in self.grid.iter_mut().zip(self.caps.iter()) {
            let inj = self.regen_rate.min((*cap - *g).max(0));
            *g += inj;
            injected += inj;
        }
        injected
    }

    fn diffuse(&mut self) {
        // Flux-conserving integer transport (engineering/14 §5.1). For each cell, exchange with its
        // right and down neighbor (reflective/no-flux boundary — no wrap): flux = (a−b) >> shift,
        // subtract from a, add to b. Pairwise-balanced ⇒ Σ invariant EXACTLY. Row-major canonical
        // order ⇒ bit-identical replay. Max outgoing per cell ≤ a/4 ⇒ never negative.
        for cz in 0..self.grid_h {
            for cx in 0..self.grid_w {
                let a = self.idx(cx, cz);
                if cx + 1 < self.grid_w {
                    let b = self.idx(cx + 1, cz);
                    let flux = (self.grid[a] - self.grid[b]) >> self.diffuse_shift;
                    self.grid[a] -= flux;
                    self.grid[b] += flux;
                }
                if cz + 1 < self.grid_h {
                    let b = self.idx(cx, cz + 1);
                    let flux = (self.grid[a] - self.grid[b]) >> self.diffuse_shift;
                    self.grid[a] -= flux;
                    self.grid[b] += flux;
                }
            }
        }
    }

    fn total(&self) -> i64 {
        self.grid.iter().sum()
    }

    fn check_meta(&self, expected_m_field: i64) -> Result<(), String> {
        if self.m_field == expected_m_field {
            Ok(())
        } else {
            Err(format!("M_field mismatch: field={} expected={}", self.m_field, expected_m_field))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field() -> CpuResourceField {
        CpuResourceField::new(8, 1, vec![100; 64], 5, 3)
    }

    #[test]
    fn diffuse_conserves_exactly() {
        let mut f = field();
        // Perturb one cell.
        f.scatter_at(Vec2Fixed(3, 3), 500);
        f.apply_scatter();
        let before = f.total();
        for _ in 0..100 {
            f.diffuse();
            assert_eq!(f.total(), before, "diffusion must conserve exactly");
        }
        assert!(f.grid.iter().all(|&v| v >= 0), "no negative cells");
    }

    #[test]
    fn take_is_exact() {
        let mut f = field();
        let here = f.amount_at(Vec2Fixed(0, 0));
        assert_eq!(f.take_at(Vec2Fixed(0, 0), 30), 30.min(here));
        assert_eq!(f.take_at(Vec2Fixed(0, 0), 1_000_000), (here - 30).max(0));
        assert_eq!(f.amount_at(Vec2Fixed(0, 0)), 0);
    }

    #[test]
    fn regenerate_respects_cap_and_reports_source() {
        let mut f = field();
        f.take_at(Vec2Fixed(0, 0), 1_000_000); // empty one cell
        let injected = f.regenerate();
        assert!(injected > 0);
        assert!(f.grid.iter().zip(f.caps.iter()).all(|(g, c)| g <= c));
    }
}
