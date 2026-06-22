// GPU erosion kernels — a port of `erosion.rs`'s hydraulic droplet + thermal talus passes.
// Geomorphically equivalent to, but NOT bit-identical to, the CPU reference (accepted: worldgen
// geometry is not in the determinism checksum). Edits accumulate in a fixed-point atomic<i32>
// buffer (WGSL has no atomic<f32>); integer adds are associative, so the per-batch/per-pass sum is
// order-independent. `resolve` folds the fixed-point edits back into `elev` and zeroes the buffer.

struct Params {
    cols: u32,
    rows: u32,
    max_lifetime: u32,
    brush_len: u32,

    inertia: f32,
    sediment_capacity: f32,
    min_capacity: f32,
    erode_speed: f32,

    deposit_speed: f32,
    evaporate: f32,
    gravity: f32,
    start_water: f32,

    start_speed: f32,
    scale: f32,        // fixed-point factor (Δheight * scale → i32)
    talus: f32,
    thermal_rate: f32,
};

struct Batch {
    start: u32,        // global index of this batch's first droplet
    count: u32,        // droplets in this batch
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<storage, read_write> elev: array<f32>;
@group(0) @binding(1) var<storage, read_write> edit: array<atomic<i32>>;
@group(0) @binding(2) var<storage, read> spawns: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read> brush: array<vec4<f32>>; // (bx, by, weight, 0)
@group(0) @binding(4) var<uniform> P: Params;
@group(0) @binding(5) var<uniform> B: Batch;
@group(0) @binding(6) var<storage, read_write> overflow: array<atomic<u32>>;

// Bilinear height + gradient at a float position (mirrors `height_grad` in erosion.rs).
fn height_grad(px: f32, py: f32) -> vec3<f32> {
    let cx = u32(floor(px));
    let cy = u32(floor(py));
    let fx = px - f32(cx);
    let fy = py - f32(cy);
    let i = cy * P.cols + cx;
    let h00 = elev[i];
    let h10 = elev[i + 1u];
    let h01 = elev[i + P.cols];
    let h11 = elev[i + P.cols + 1u];
    let gx = (h10 - h00) * (1.0 - fy) + (h11 - h01) * fy;
    let gy = (h01 - h00) * (1.0 - fx) + (h11 - h10) * fx;
    let height = h00 * (1.0 - fx) * (1.0 - fy)
        + h10 * fx * (1.0 - fy)
        + h01 * (1.0 - fx) * fy
        + h11 * fx * fy;
    return vec3<f32>(height, gx, gy);
}

fn add_edit(idx: u32, dz: f32) {
    atomicAdd(&edit[idx], i32(round(dz * P.scale)));
}

// --- Hydraulic droplet: one thread per droplet. Reads the frozen `elev` snapshot, scatters
//     (index, Δ) edits into the fixed-point buffer. Port of `simulate_droplet`. ---
@compute @workgroup_size(64)
fn droplet(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= B.count) { return; }
    let s = spawns[B.start + t];
    var px = s.x;
    var py = s.y;
    var dx = 0.0;
    var dy = 0.0;
    var speed = P.start_speed;
    var water = P.start_water;
    var sediment = 0.0;

    for (var life = 0u; life < P.max_lifetime; life = life + 1u) {
        let cx = i32(floor(px));
        let cy = i32(floor(py));
        let node = u32(cy) * P.cols + u32(cx);
        let hg = height_grad(px, py);
        let h = hg.x;

        dx = dx * P.inertia - hg.y * (1.0 - P.inertia);
        dy = dy * P.inertia - hg.z * (1.0 - P.inertia);
        let len = sqrt(dx * dx + dy * dy);
        if (len < 1e-6) { break; }
        dx = dx / len;
        dy = dy / len;
        let npx = px + dx;
        let npy = py + dy;
        if (npx < 0.0 || npy < 0.0 || npx >= f32(P.cols - 1u) || npy >= f32(P.rows - 1u)) { break; }

        let nh = height_grad(npx, npy).x;
        let dh = nh - h;
        let capacity = max(-dh, P.min_capacity) * speed * water * P.sediment_capacity;

        if (sediment > capacity || dh > 0.0) {
            var drop = 0.0;
            if (dh > 0.0) {
                drop = min(sediment, dh);
            } else {
                drop = (sediment - capacity) * P.deposit_speed;
            }
            sediment = sediment - drop;
            let fx = px - f32(cx);
            let fy = py - f32(cy);
            add_edit(node, drop * (1.0 - fx) * (1.0 - fy));
            add_edit(node + 1u, drop * fx * (1.0 - fy));
            add_edit(node + P.cols, drop * (1.0 - fx) * fy);
            add_edit(node + P.cols + 1u, drop * fx * fy);
        } else {
            let amount = min((capacity - sediment) * P.erode_speed, -dh);
            for (var b = 0u; b < P.brush_len; b = b + 1u) {
                let off = brush[b];
                let ex = cx + i32(off.x);
                let ey = cy + i32(off.y);
                if (ex < 0 || ey < 0 || ex >= i32(P.cols) || ey >= i32(P.rows)) { continue; }
                let e = u32(ey) * P.cols + u32(ex);
                let taken = min(amount * off.z, elev[e]);
                add_edit(e, -taken);
                sediment = sediment + taken;
            }
        }

        speed = sqrt(max(speed * speed + dh * P.gravity, 0.0));
        water = water * (1.0 - P.evaporate);
        if (water < 1e-3) { break; }
        px = npx;
        py = npy;
    }
}

// --- Thermal/talus: one thread per cell. Same scatter as the CPU `thermal` pass (shed the
//     steepest excess to lower neighbours), into the fixed-point edit buffer. ---
@compute @workgroup_size(64)
fn thermal(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = P.cols * P.rows;
    if (i >= n) { return; }
    let x = i32(i % P.cols);
    let y = i32(i / P.cols);
    let h = elev[i];

    var offx = array<i32, 8>(1, -1, 0, 0, 1, 1, -1, -1);
    var offy = array<i32, 8>(0, 0, 1, -1, 1, -1, 1, -1);
    let INV = 0.7071067811865476;
    var inv = array<f32, 8>(1.0, 1.0, 1.0, 1.0, INV, INV, INV, INV);

    var lowers_j = array<u32, 8>();
    var lowers_s = array<f32, 8>();
    var k = 0u;
    var total = 0.0;
    var smax = 0.0;
    for (var m = 0u; m < 8u; m = m + 1u) {
        let nx = x + offx[m];
        let ny = y + offy[m];
        if (nx < 0 || ny < 0 || nx >= i32(P.cols) || ny >= i32(P.rows)) { continue; }
        let j = u32(ny) * P.cols + u32(nx);
        let sl = (h - elev[j]) * inv[m];
        if (sl > P.talus) {
            lowers_j[k] = j;
            lowers_s[k] = sl;
            k = k + 1u;
            total = total + sl;
            if (sl > smax) { smax = sl; }
        }
    }
    if (k > 0u) {
        let move_amt = (smax - P.talus) * 0.5 * P.thermal_rate;
        add_edit(i, -move_amt);
        for (var q = 0u; q < k; q = q + 1u) {
            add_edit(lowers_j[q], move_amt * (lowers_s[q] / total));
        }
    }
}

// --- Resolve: fold the fixed-point edits back into `elev`, zero the buffer, flag near-overflow. ---
@compute @workgroup_size(64)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = P.cols * P.rows;
    if (i >= n) { return; }
    let e = atomicExchange(&edit[i], 0);
    if (abs(e) > 1073741823) { atomicStore(&overflow[0], 1u); } // > i32::MAX/2 → near saturation
    elev[i] = elev[i] + f32(e) / P.scale;
}

// --- Final clamp to [0,1] (mirrors the tail of `erode`). ---
@compute @workgroup_size(64)
fn clamp_field(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = P.cols * P.rows;
    if (i >= n) { return; }
    elev[i] = clamp(elev[i], 0.0, 1.0);
}
