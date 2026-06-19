/// Chunk and region mesh building from voxel geometry.
use macroquad::prelude::*;
use animata_sim::config::*;
use animata_sim::terrain::{self, VoxelTerrain, cell_biome, cell_height, feature_unit, BiomeKind};

#[derive(Clone, Copy, PartialEq)]
pub enum TreeKind {
    None,
    Broadleaf,
    Conifer,
}

#[derive(Clone, Copy)]
struct BiomeDef {
    surface: (f32, f32, f32),
    tree_density: f32,
    tree: TreeKind,
}

const fn def(surface: (f32, f32, f32), tree_density: f32, tree: TreeKind) -> BiomeDef {
    BiomeDef {
        surface,
        tree_density,
        tree,
    }
}

/// Indexed by `BiomeKind::id()` (0..12 used, 12..16 padded). Order matches the enum.
static BIOME_DEFS: [BiomeDef; 16] = [
    def((0.13, 0.32, 0.55), 0.0, TreeKind::None), // 0 Ocean
    def((0.84, 0.78, 0.54), 0.0, TreeKind::None), // 1 Beach
    def((0.42, 0.62, 0.30), 0.04, TreeKind::Broadleaf), // 2 Plains
    def((0.20, 0.46, 0.24), 0.30, TreeKind::Broadleaf), // 3 Forest
    def((0.80, 0.70, 0.44), 0.0, TreeKind::None), // 4 Desert
    def((0.48, 0.46, 0.45), 0.0, TreeKind::None), // 5 Mountain
    def((0.93, 0.95, 0.98), 0.02, TreeKind::Conifer), // 6 Snow
    def((0.17, 0.38, 0.29), 0.30, TreeKind::Conifer), // 7 Taiga
    def((0.62, 0.64, 0.56), 0.0, TreeKind::None), // 8 Tundra
    def((0.70, 0.66, 0.34), 0.03, TreeKind::Broadleaf), // 9 Savanna
    def((0.31, 0.40, 0.25), 0.14, TreeKind::Broadleaf), // 10 Swamp
    def((0.12, 0.43, 0.17), 0.50, TreeKind::Broadleaf), // 11 Jungle
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None), // 12-15 padding
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None),
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None),
    def((0.42, 0.62, 0.30), 0.0, TreeKind::None),
];

fn biome_def(biome: BiomeKind) -> &'static BiomeDef {
    &BIOME_DEFS[biome.id() as usize]
}

/// Surface (top-face) base colour per biome.
fn top_rgb(biome: BiomeKind) -> (f32, f32, f32) {
    biome_def(biome).surface
}

/// Side-wall colour for the exposed level `gz` of a column of height `h`: a thin lip of
/// the surface colour `top` just under the surface, then (unless `rocky`) topsoil, then
/// stone. `rocky` biomes (mountain/snow/seabed) skip the brown topsoil band.
fn strata_rgb(gz: u8, h: u8, top: (f32, f32, f32), rocky: bool) -> (f32, f32, f32) {
    if gz + 1 == h {
        (top.0 * 0.85, top.1 * 0.85, top.2 * 0.85)
    } else if !rocky && gz + 3 >= h {
        (0.42, 0.31, 0.20) // topsoil
    } else {
        (0.40, 0.38, 0.36) // stone
    }
}

// Baked directional face shading (fixed "sun"), so volume reads without lighting.
const SHADE_TOP: f32 = 1.0;
const SHADE_PX: f32 = 0.86;
const SHADE_NX: f32 = 0.62;
const SHADE_PZ: f32 = 0.74;
const SHADE_NZ: f32 = 0.54;

pub fn shaded(rgb: (f32, f32, f32), s: f32) -> Color {
    Color::new(rgb.0 * s, rgb.1 * s, rgb.2 * s, 1.0)
}

/// A built mesh plus its world-space AABB, so the renderer can frustum-cull it: with a
/// big map most chunks are off-screen, and macroquad re-batches every drawn mesh's
/// vertices each frame, so skipping off-screen ones keeps per-frame cost ∝ what's
/// visible, not ∝ the whole map.
pub struct Batch {
    pub mesh: Mesh,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// macroquad's `draw_mesh` pushes through the immediate batch buffer, which **clamps**
/// (silently dropping geometry) at `>= 10000` vertices or `>= 5000` indices per call.
/// Indices bind first (6 per quad vs 4 verts), so we split meshes on the index count,
/// keeping a margin for the largest single-column burst (top + 4 cliff sides + a tree).
const MAX_MESH_INDICES: usize = 4800;
/// Worst-case indices a single column/LOD-block can add at once: a block can emit four
/// full-relief side faces (≈ `4 × MAX_H` strata quads) at a tall LOD step, plus the top.
const COLUMN_INDEX_BURST: usize = 1200;

/// Build the meshes for ONE chunk `(cx, cy)` at `lod` — the unit the detail tier streams.
/// Returns `(opaque, water)`.
pub fn build_chunk_mesh(t: &VoxelTerrain, cx: usize, cy: usize, lod: u32) -> (Vec<Batch>, Vec<Batch>) {
    let x1 = (cx * CHUNK + CHUNK).min(COLS);
    let y1 = (cy * CHUNK + CHUNK).min(ROWS);
    build_region_mesh(t, cx * CHUNK, cy * CHUNK, x1, y1, lod)
}

/// Build the opaque + water meshes for an arbitrary column rectangle `[x0,x1) × [y0,y1)`
/// at `lod`, merged into as few batches as the per-draw limit allows. A single chunk uses
/// this for the streamed detail tier; a whole super-tile uses it for the coarse overview
/// tier (many chunks → a handful of buffers, so the whole map is a few hundred draws).
///
/// At LOD>0 columns are read on a `stride` grid (blocks aligned globally because `x0/y0`
/// are stride multiples) and each block emits one `stride×stride` footprint sampled from
/// its origin column, with neighbour heights read a stride away. Trees are full-detail
/// only. Returns `(opaque, water)`: the opaque seabed/land/trees, plus a translucent
/// water surface (one quad per submerged column + connective faces for river steps),
/// drawn in a second, animated pass.
pub fn build_region_mesh(
    t: &VoxelTerrain,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    lod: u32,
) -> (Vec<Batch>, Vec<Batch>) {
    let stride = 1usize << lod;
    let si = stride as i32;
    let mut opaque = Vec::new();
    let mut verts: Vec<Vertex> = Vec::new();
    let mut idx: Vec<u16> = Vec::new();
    let mut water = Vec::new();
    let mut wv: Vec<Vertex> = Vec::new();
    let mut wi: Vec<u16> = Vec::new();
    // Trees are voxelised into a set first, then meshed with EXPOSED faces only (like the
    // terrain) — so overlapping canopies and adjacent cubes don't leave coincident,
    // differently-shaded faces that z-fight into dashed seams.
    let mut tvox: VoxMap = std::collections::HashMap::new();
    let mut gyc = y0;
    while gyc < y1 {
        let mut gxc = x0;
        while gxc < x1 {
            let (gx, gy) = (gxc, gyc);
            gxc += stride;
            if gx >= COLS || gy >= ROWS {
                continue; // outside the world
            }
            let cell = t.cell(gx as i32, gy as i32);
            let h = cell_height(cell);
            if h == 0 {
                continue; // air
            }
            // Split before macroquad's per-drawcall batch limit (see consts). The burst
            // margin must cover a tall LOD step's 4 full-height side faces.
            if idx.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
                flush_mesh(&mut verts, &mut idx, &mut opaque);
            }
            let biome = cell_biome(cell);
            let (ix, iy) = (gx as i32, gy as i32);
            let wl = t.water_level(ix, iy);
            // Submerged columns get a sand/rock SEABED top (by depth), not the blue Ocean
            // biome colour — the floor reads as a bottom under the blue water surface.
            // Submerged columns are a sand/rock seabed (by depth); land uses its biome.
            // Either way the SIDE faces are drawn (the bed reads as 3D terrain like land,
            // and culling them left sky showing through the steps as blue edges).
            let submerged = wl > h;
            let top_col = if submerged {
                seabed_rgb(wl - h)
            } else if matches!(biome, BiomeKind::Mountain) {
                rock_rgb(gx, gy, t.seed)
            } else {
                top_rgb(biome)
            };
            let rocky = submerged || matches!(biome, BiomeKind::Mountain | BiomeKind::Snow);
            let nb = [
                (t.height(ix + si, iy), t.water_level(ix + si, iy), Face::Px),
                (t.height(ix - si, iy), t.water_level(ix - si, iy), Face::Nx),
                (t.height(ix, iy + si), t.water_level(ix, iy + si), Face::Pz),
                (t.height(ix, iy - si), t.water_level(ix, iy - si), Face::Nz),
            ];
            // A face that fronts a LOWER neighbour is a step edge; the top's rim verts on that
            // edge get darkened (fake AO), so a 1-cell dark band traces every height boundary —
            // otherwise two same-biome plateaus at different heights read as one flat tone.
            let drops = [nb[0].0 < h, nb[1].0 < h, nb[2].0 < h, nb[3].0 < h]; // [Px,Nx,Pz,Nz]
            push_top(&mut verts, &mut idx, gx, gy, stride, h, top_col, drops);
            for (nh, nwl, face) in nb {
                if nh < h {
                    // `nwl` = the neighbour's water surface: levels of this face below it are
                    // underwater (this neighbour is the water body fronting the face), so the
                    // mesher colours them as seabed rather than land strata.
                    push_side(
                        &mut verts,
                        &mut idx,
                        (gx, gy),
                        stride,
                        h,
                        nh,
                        face,
                        top_col,
                        rocky,
                        nwl,
                        lod == 0,
                    );
                }
            }

            // Translucent water surface over this column: a quad at the water level, depth
            // (`wl - h`) carried per vertex for the shader's depth shading. Connective side
            // faces ONLY toward a slightly LOWER water neighbour (a river step ≤ WATER_STEP_MAX)
            // so a descending river reads continuous; a BIG drop is two separate bodies (e.g.
            // mountain lake beside the sea) — no face there, which avoids the old "water walls".
            if submerged {
                let depth = wl - h;
                if wi.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
                    flush_mesh(&mut wv, &mut wi, &mut water);
                }
                push_water_top(&mut wv, &mut wi, gx, gy, stride, wl, depth);
                for (nx, ny, face) in [
                    (ix + si, iy, Face::Px),
                    (ix - si, iy, Face::Nx),
                    (ix, iy + si, Face::Pz),
                    (ix, iy - si, Face::Nz),
                ] {
                    let nwl = t.water_level(nx, ny);
                    if nwl > 0 && nwl < wl && wl - nwl <= WATER_STEP_MAX {
                        push_water_side(&mut wv, &mut wi, (gx, gy), stride, wl, nwl, depth, face);
                    }
                }
            }

            // Trees on dry land (through LOD1, one per block, so the canopy fades a ring
            // out instead of a hard edge). Water itself is a separate translucent pass.
            if !submerged && lod <= 1 {
                let bd = biome_def(biome);
                if bd.tree != TreeKind::None && feature_unit(t.seed, gx, gy, 101) < bd.tree_density
                {
                    collect_tree(&mut tvox, t, gx, gy, h, bd.tree);
                }
            }
        }
        gyc += stride;
    }
    mesh_tree_voxels(&mut verts, &mut idx, &mut opaque, &tvox);
    flush_mesh(&mut verts, &mut idx, &mut opaque);
    flush_mesh(&mut wv, &mut wi, &mut water);
    (opaque, water)
}

/// A sparse voxel set (position → colour) the trees are rasterised into before meshing.
type VoxMap = std::collections::HashMap<(i32, i32, u8), (f32, f32, f32)>;

/// Voxelise a tree on column `(gx, gy)` standing on surface height `h` into `vox`.
/// **Broadleaf**: short brown trunk under a 3×3 leaf canopy + cap (rounded, deciduous).
/// **Conifer**: taller trunk with a narrow tapering spire (1-cell tip over a + of leaves).
/// Per-column hashes keep it deterministic; canopy blocks overhanging the world / water are
/// skipped. Writing into a SET de-duplicates overlapping canopies (no coincident faces).
fn collect_tree(vox: &mut VoxMap, t: &VoxelTerrain, gx: usize, gy: usize, h: u8, kind: TreeKind) {
    let seed = t.seed;
    let trunk = (0.36, 0.26, 0.16);
    let leaf = if kind == TreeKind::Conifer {
        (0.09, 0.24, 0.16)
    } else {
        (0.16, 0.42, 0.20)
    };
    let (gxi, gyi) = (gx as i32, gy as i32);
    // Leaves are skipped over water / off-map; the trunk sits on the tree's own (valid) column.
    let leaf_at = |vox: &mut VoxMap, lx: i32, ly: i32, lz: u8| {
        if (0..COLS as i32).contains(&lx)
            && (0..ROWS as i32).contains(&ly)
            && t.water_level(lx, ly) == 0
        {
            vox.insert((lx, ly, lz), leaf);
        }
    };
    if kind == TreeKind::Conifer {
        let th = 3 + (feature_unit(seed, gx, gy, 202) * 2.0) as u8; // 3 or 4
        for gz in h..h + th {
            vox.insert((gxi, gyi, gz), trunk);
        }
        for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            leaf_at(vox, gxi + dx, gyi + dy, h + th);
        }
        leaf_at(vox, gxi, gyi, h + th + 1);
        leaf_at(vox, gxi, gyi, h + th + 2);
    } else {
        let th = 2 + (feature_unit(seed, gx, gy, 202) * 2.0) as u8; // 2 or 3
        for gz in h..h + th {
            vox.insert((gxi, gyi, gz), trunk);
        }
        let top = h + th;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                leaf_at(vox, gxi + dx, gyi + dy, top);
            }
        }
        leaf_at(vox, gxi, gyi, top + 1);
    }
}

/// Mesh the tree voxel set, emitting only EXPOSED faces (a face is drawn only where the
/// neighbour voxel is absent) — exactly like the terrain mesher, so there are no interior
/// or coincident faces to z-fight. Bottom faces are omitted (unseen from the iso top-down
/// view), matching the terrain. Side faces bias their top edge back (the column's own top
/// wins the rim, no dark speckle).
fn mesh_tree_voxels(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    opaque: &mut Vec<Batch>,
    vox: &VoxMap,
) {
    for (&(gx, gy, gz), &rgb) in vox {
        if idx.len() + COLUMN_INDEX_BURST > MAX_MESH_INDICES {
            flush_mesh(verts, idx, opaque);
        }
        let (x0, x1) = (gx as f32 * VOX, (gx + 1) as f32 * VOX);
        let (z0, z1) = (gy as f32 * VOX, (gy + 1) as f32 * VOX);
        let (y0, y1) = (gz as f32 * VOX, (gz + 1) as f32 * VOX);
        // Side verts are bottom (0,1) then top (2,3). Bias the TOP edge back (the voxel's
        // own top wins the rim) AND the BOTTOM edge forward (toward camera): a tree is a
        // SEPARATE mesh sitting on the terrain, so its base edge ties the ground's top and
        // would otherwise be eaten ("saw") — the forward nudge makes the trunk win there.
        let top_back = [-1.0, -1.0, 1.0, 1.0];
        if !vox.contains_key(&(gx, gy, gz + 1)) {
            push_quad(
                verts,
                idx,
                [
                    vec3(x0, y1, z0),
                    vec3(x1, y1, z0),
                    vec3(x1, y1, z1),
                    vec3(x0, y1, z1),
                ],
                shaded(rgb, SHADE_TOP),
                0.0,
            );
        }
        if !vox.contains_key(&(gx + 1, gy, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x1, y0, z0),
                    vec3(x1, y0, z1),
                    vec3(x1, y1, z1),
                    vec3(x1, y1, z0),
                ],
                shaded(rgb, SHADE_PX),
                top_back,
            );
        }
        if !vox.contains_key(&(gx - 1, gy, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x0, y0, z1),
                    vec3(x0, y0, z0),
                    vec3(x0, y1, z0),
                    vec3(x0, y1, z1),
                ],
                shaded(rgb, SHADE_NX),
                top_back,
            );
        }
        if !vox.contains_key(&(gx, gy + 1, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x1, y0, z1),
                    vec3(x0, y0, z1),
                    vec3(x0, y1, z1),
                    vec3(x1, y1, z1),
                ],
                shaded(rgb, SHADE_PZ),
                top_back,
            );
        }
        if !vox.contains_key(&(gx, gy - 1, gz)) {
            push_quad_v(
                verts,
                idx,
                [
                    vec3(x0, y0, z0),
                    vec3(x1, y0, z0),
                    vec3(x1, y1, z0),
                    vec3(x0, y1, z0),
                ],
                shaded(rgb, SHADE_NZ),
                top_back,
            );
        }
    }
}

#[derive(Clone, Copy)]
enum Face {
    Px,
    Nx,
    Pz,
    Nz,
}

pub fn flush_mesh(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, out: &mut Vec<Batch>) {
    if verts.is_empty() {
        return;
    }
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for v in verts.iter() {
        lo = lo.min(v.position);
        hi = hi.max(v.position);
    }
    out.push(Batch {
        mesh: Mesh {
            vertices: std::mem::take(verts),
            indices: std::mem::take(idx),
            texture: None,
        },
        lo,
        hi,
    });
}

/// Per-vertex `back` flags (0/1) → uv.x; the shader nudges back=1 verts a hair toward the
/// far plane. Only the TOP edge of a side wall is flagged: that edge is shared with the
/// column's own top face (which must win the rim → no dark z-fight speckle), while the
/// wall's BOTTOM edge stays unbiased so it isn't eaten by the lower neighbour's top face.
fn push_quad_v(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    q: [Vec3; 4],
    col: Color,
    backs: [f32; 4],
) {
    push_quad_c(verts, idx, q, [col; 4], backs);
}

/// Quad with a PER-VERTEX colour (`cols`) and per-vertex `back` flag — used by the top face
/// to bake rim AO into individual corners.
fn push_quad_c(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    q: [Vec3; 4],
    cols: [Color; 4],
    backs: [f32; 4],
) {
    let base = verts.len() as u16;
    for ((p, b), c) in q.into_iter().zip(backs).zip(cols) {
        verts.push(Vertex::new(p.x, p.y, p.z, b, 0.0, c));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Quad with a uniform `back` flag on all four verts.
fn push_quad(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], col: Color, back: f32) {
    push_quad_v(verts, idx, q, col, [back; 4]);
}

/// Max level drop across which a connective water SIDE face is drawn (a river step). A
/// bigger drop means two separate water bodies (e.g. a mountain lake beside the sea), where
/// a face would stand as a tall spurious "water wall" — so it's capped.
const WATER_STEP_MAX: u8 = 2;

/// A water-surface quad covering column `(gx, gy)`'s `stride×stride` footprint at level
/// `wl`. `depth` (= `wl - terrain_h`, voxel levels) goes in every vertex's `uv.y` for the
/// water shader's depth shading; vertex colour is unused by the shader (placeholder WHITE).
fn push_water_top(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    gx: usize,
    gy: usize,
    s: usize,
    wl: u8,
    depth: u8,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let y = wl as f32 * VOX;
    let q = [
        vec3(x0, y, z0),
        vec3(x1, y, z0),
        vec3(x1, y, z1),
        vec3(x0, y, z1),
    ];
    push_water_quad(verts, idx, q, depth);
}

/// A water side face on one edge, from the lower neighbour surface `lo` up to this water
/// level `hi` — fills the vertical gap at a river step so the ribbon reads continuous.
#[allow(clippy::too_many_arguments)]
fn push_water_side(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    (gx, gy): (usize, usize),
    s: usize,
    hi: u8,
    lo: u8,
    depth: u8,
    face: Face,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let (y0, y1) = (lo as f32 * VOX, hi as f32 * VOX);
    let q = match face {
        Face::Px => [
            vec3(x1, y0, z0),
            vec3(x1, y0, z1),
            vec3(x1, y1, z1),
            vec3(x1, y1, z0),
        ],
        Face::Nx => [
            vec3(x0, y0, z1),
            vec3(x0, y0, z0),
            vec3(x0, y1, z0),
            vec3(x0, y1, z1),
        ],
        Face::Pz => [
            vec3(x1, y0, z1),
            vec3(x0, y0, z1),
            vec3(x0, y1, z1),
            vec3(x1, y1, z1),
        ],
        Face::Nz => [
            vec3(x0, y0, z0),
            vec3(x1, y0, z0),
            vec3(x1, y1, z0),
            vec3(x0, y1, z0),
        ],
    };
    push_water_quad(verts, idx, q, depth);
}

/// Emit a water quad: `uv.x = 0` (no terrain depth-nudge), `uv.y = depth`, colour WHITE
/// (the water shader computes colour/alpha from depth, ignoring vertex colour).
fn push_water_quad(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], depth: u8) {
    let base = verts.len() as u16;
    let d = depth as f32;
    for p in q {
        verts.push(Vertex::new(p.x, p.y, p.z, 0.0, d, WHITE));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Sea/lake BED colour by water depth: a sandy shoal in the shallows grading to bare rock
/// in the deeps — submerged columns render this instead of a (removed) water surface.
fn seabed_rgb(depth: u8) -> (f32, f32, f32) {
    let t = (depth as f32 / 5.0).clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    (lerp(0.80, 0.40), lerp(0.72, 0.39), lerp(0.52, 0.37)) // sand → rock
}

/// Varied mountain rock: a coherent (low-frequency) brightness field over the bare grey,
/// plus greenish mossy patches — so a massif isn't a flat slab of one stone colour.
fn rock_rgb(gx: usize, gy: usize, seed: u64) -> (f32, f32, f32) {
    let v = terrain::fbm(seed, gx as f32 / 22.0, gy as f32 / 22.0, 303, 3); // brightness [0,1]
    let m = terrain::fbm(seed, gx as f32 / 34.0, gy as f32 / 34.0, 305, 2); // moss mask
    let g = 0.36 + 0.22 * v;
    let mut c = (g, g * 0.98, g * 0.93); // slightly warm, brightness-varied grey
    if m > 0.60 {
        let k = ((m - 0.60) / 0.40).min(1.0) * 0.5;
        c = (
            c.0 + (0.33 - c.0) * k,
            c.1 + (0.45 - c.1) * k,
            c.2 + (0.29 - c.2) * k,
        ); // → moss
    }
    c
}

/// Contour line: width of the dark rim strip overlaid along a terrace edge, as a fraction
/// of ONE voxel (kept constant in world space, not scaled by LOD stride, so the line stays
/// thin on coarse far tiles). And how much it darkens the top colour.
const RIM_LINE_W: f32 = 0.02;
const RIM_LINE_SHADE: f32 = 0.6;

/// `drops` = [Px, Nx, Pz, Nz]: whether each neighbour edge steps DOWN from this column. A
/// thin dark strip is overlaid along every dropping edge (nudged toward the camera via
/// `uv.x=-1` so it wins the top plane without z-fight). Only step edges get it and the
/// strips run the full edge, so adjacent rim cells join into ONE continuous contour around
/// the terrace — interior cube joins (same height, no drop) stay unmarked.
#[allow(clippy::too_many_arguments)]
fn push_top(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    gx: usize,
    gy: usize,
    s: usize,
    h: u8,
    rgb: (f32, f32, f32),
    drops: [bool; 4],
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    let y = h as f32 * VOX;
    push_quad(
        verts,
        idx,
        [
            vec3(x0, y, z0),
            vec3(x1, y, z0),
            vec3(x1, y, z1),
            vec3(x0, y, z1),
        ],
        shaded(rgb, SHADE_TOP),
        0.0,
    );
    let [dpx, dnx, dpz, dnz] = drops;
    if dpx || dnx || dpz || dnz {
        let line = shaded(rgb, RIM_LINE_SHADE);
        let w = RIM_LINE_W * VOX;
        // Each strip keeps the top winding [(-,-),(+,-),(+,+),(-,+)]; `back=-1` nudges it
        // toward the camera so it deterministically wins the shared top plane.
        let mut strip = |ax0: f32, ax1: f32, az0: f32, az1: f32| {
            push_rim(
                verts,
                idx,
                [
                    vec3(ax0, y, az0),
                    vec3(ax1, y, az0),
                    vec3(ax1, y, az1),
                    vec3(ax0, y, az1),
                ],
                line,
            );
        };
        if dpx {
            strip(x1 - w, x1, z0, z1);
        }
        if dnx {
            strip(x0, x0 + w, z0, z1);
        }
        if dpz {
            strip(x0, x1, z1 - w, z1);
        }
        if dnz {
            strip(x0, x1, z0, z0 + w);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_side(
    verts: &mut Vec<Vertex>,
    idx: &mut Vec<u16>,
    (gx, gy): (usize, usize),
    s: usize,
    h: u8,
    nh: u8,
    face: Face,
    top: (f32, f32, f32),
    rocky: bool,
    nwl: u8,
    rim: bool,
) {
    let (x0, x1) = (gx as f32 * VOX, (gx + s) as f32 * VOX);
    let (z0, z1) = (gy as f32 * VOX, (gy + s) as f32 * VOX);
    // Face quad for a vertical [y0,y1] band, winding outward per face (shared by the strata
    // quads and the rim strip below).
    let wall = |y0: f32, y1: f32| match face {
        Face::Px => [
            vec3(x1, y0, z0),
            vec3(x1, y0, z1),
            vec3(x1, y1, z1),
            vec3(x1, y1, z0),
        ],
        Face::Nx => [
            vec3(x0, y0, z1),
            vec3(x0, y0, z0),
            vec3(x0, y1, z0),
            vec3(x0, y1, z1),
        ],
        Face::Pz => [
            vec3(x1, y0, z1),
            vec3(x0, y0, z1),
            vec3(x0, y1, z1),
            vec3(x1, y1, z1),
        ],
        Face::Nz => [
            vec3(x0, y0, z0),
            vec3(x1, y0, z0),
            vec3(x1, y1, z0),
            vec3(x0, y1, z0),
        ],
    };
    let shade = match face {
        Face::Px => SHADE_PX,
        Face::Nx => SHADE_NX,
        Face::Pz => SHADE_PZ,
        Face::Nz => SHADE_NZ,
    };
    for gz in nh..h {
        let (y0, y1) = (gz as f32 * VOX, (gz + 1) as f32 * VOX);
        // Levels below the fronting water's surface (`nwl`) are seabed, not land strata: a dry
        // shore column dropping into a lake/sea exposes a side face whose underwater part would
        // otherwise show the land (grass/dirt) colour and, through the translucent water, read
        // as "water drawn over land" (and only from the angle the face points at the camera —
        // hence view-dependent). `nwl` is the NEIGHBOUR's level, so it works for high lakes too,
        // not just the global sea. Colour by depth below that surface, matching submerged tops.
        let col = if gz < nwl {
            shaded(seabed_rgb(nwl - gz), shade)
        } else {
            shaded(strata_rgb(gz, h, top, rocky), shade)
        };
        let q = wall(y0, y1);
        // Bias only the wall's TOP edge (the topmost level quad's top verts 2,3) back, so
        // the column's top face wins that rim; the rest of the wall is unbiased.
        let backs = if gz + 1 == h {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [0.0; 4]
        };
        push_quad_v(verts, idx, q, col, backs);
    }
    // Vertical leg of the contour: a thin dark strip down the TOP of the wall from the rim,
    // overlaid (nudged toward the camera) so it wins the strata face. Together with the top
    // strip it wraps the edge in an L, so the step reads from the side too. LOD0 only.
    if rim {
        let yt = h as f32 * VOX;
        let yb = (yt - RIM_LINE_W * VOX).max(nh as f32 * VOX);
        push_rim(verts, idx, wall(yb, yt), shaded(top, RIM_LINE_SHADE));
    }
}

/// Emit a contour-overlay quad: `uv.x = -1` nudges it toward the camera so it wins the face
/// it sits on (no z-fight), `uv.y = 1` flags it so the fragment shader can hide the whole
/// outline when toggled off (key `O`), baring the face underneath.
fn push_rim(verts: &mut Vec<Vertex>, idx: &mut Vec<u16>, q: [Vec3; 4], col: Color) {
    let base = verts.len() as u16;
    for p in q {
        verts.push(Vertex::new(p.x, p.y, p.z, -1.0, 1.0, col));
    }
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}
