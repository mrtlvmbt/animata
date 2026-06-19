    use super::*;

    /// Every built chunk mesh must stay strictly under macroquad's per-`draw_mesh` batch
    /// limits (`>= 10000` verts / `>= 5000` indices ⇒ silent clamping). Builds chunk by
    /// chunk (the streaming unit) so peak memory is one chunk even at ×16. Guards the
    /// splitter.
    #[test]
    fn meshes_stay_under_macroquad_drawcall_limits() {
        let t = VoxelTerrain::new(1);
        let mut any = false;
        for cy in 0..t.chunks_y {
            for cx in 0..t.chunks_x {
                let (op, wt) = build_chunk_mesh(&t, cx, cy, 0);
                for b in op.iter().chain(wt.iter()) {
                    any = true;
                    assert!(
                        b.mesh.vertices.len() < 10_000,
                        "verts {} at chunk ({cx},{cy})",
                        b.mesh.vertices.len()
                    );
                    assert!(
                        b.mesh.indices.len() < 5_000,
                        "indices {} at chunk ({cx},{cy})",
                        b.mesh.indices.len()
                    );
                }
            }
        }
        assert!(any, "no geometry built");
    }

    /// LOD must (a) stay under the draw-call limits at every level and (b) actually shrink
    /// the geometry as it coarsens (else the far rings buy nothing). Checks a spread of
    /// chunks across the map at LOD 0/1/2.
    #[test]
    fn lod_reduces_geometry_within_limits() {
        let t = VoxelTerrain::new(1);
        let sample = [
            (t.chunks_x / 4, t.chunks_y / 4),
            (t.chunks_x / 2, t.chunks_y / 2),
            (t.chunks_x * 3 / 4, t.chunks_y / 2),
        ];
        for (cx, cy) in sample {
            let mut prev = usize::MAX;
            for lod in 0..3u32 {
                let (op, wt) = build_chunk_mesh(&t, cx, cy, lod);
                let mut verts = 0;
                for b in op.iter().chain(wt.iter()) {
                    verts += b.mesh.vertices.len();
                    assert!(
                        b.mesh.vertices.len() < 10_000,
                        "lod {lod} verts overflow at ({cx},{cy})"
                    );
                    assert!(
                        b.mesh.indices.len() < 5_000,
                        "lod {lod} indices overflow at ({cx},{cy})"
                    );
                }
                assert!(
                    verts <= prev,
                    "lod {lod} not coarser at ({cx},{cy}): {verts} > {prev}"
                );
                prev = verts;
            }
        }
    }

    /// A coarse super-tile (a whole SUPER×SUPER chunk region merged into one mesh stream)
    /// must also stay under the per-draw limits and be far fewer batches than its chunks —
    /// that merge is what lets the whole map render in a few hundred draws.
    #[test]
    fn coarse_super_tiles_merge_within_limits() {
        let t = VoxelTerrain::new(1);
        let span = SUPER as usize * CHUNK;
        for &(sx, sy) in &[(0usize, 0usize), (2, 1)] {
            let (x0, y0) = (sx * span, sy * span);
            if x0 >= COLS || y0 >= ROWS {
                continue;
            }
            let (x1, y1) = ((x0 + span).min(COLS), (y0 + span).min(ROWS));
            let (op, wt) = build_region_mesh(&t, x0, y0, x1, y1, COARSE_LOD);
            let batches = op.len();
            for b in op.iter().chain(wt.iter()) {
                assert!(b.mesh.vertices.len() < 10_000, "coarse verts overflow");
                assert!(b.mesh.indices.len() < 5_000, "coarse indices overflow");
            }
            // A super-tile is SUPER² chunks; the merged coarse mesh must be far fewer
            // buffers than that (else the overview buys no draw-call reduction).
            assert!(
                batches < (SUPER * SUPER) as usize,
                "coarse not merged: {batches} batches"
            );
        }
    }

    /// Report the WHOLE-map mesh size at the current scale (built per chunk + dropped, so
    /// it doesn't hold it all) — this is the number the streaming exists to avoid holding
    /// resident. Informational, `--ignored` (it meshes the whole map).
    #[test]
    #[ignore]
    fn report_mesh_footprint() {
        let t = VoxelTerrain::new(1);
        let (mut verts, mut batches) = (0usize, 0usize);
        for cy in 0..t.chunks_y {
            for cx in 0..t.chunks_x {
                let (op, wt) = build_chunk_mesh(&t, cx, cy, 0);
                for b in op.iter().chain(wt.iter()) {
                    verts += b.mesh.vertices.len();
                    batches += 1;
                }
            }
        }
        let mb = (verts * std::mem::size_of::<Vertex>()) as f64 / (1024.0 * 1024.0);
        eprintln!("MAP_SCALE={MAP_SCALE} SURFACE_RANGE={SURFACE_RANGE}: {verts} verts, {batches} batches, ~{mb:.0} MB if all resident");
    }
