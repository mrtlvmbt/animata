/// Isometric camera with dynamic depth precision and frustum culling.
use macroquad::prelude::*;
use animata_sim::config::*;

/// `zoom` = world-height visible (smaller = closer) and `yaw` rotating in 90° steps.
pub struct IsoCam {
    pub target: Vec3,
    pub zoom: f32,
    pub yaw: f32,
}

impl IsoCam {
    pub fn new() -> Self {
        IsoCam {
            target: vec3(COLS as f32 * VOX * 0.5, 0.0, ROWS as f32 * VOX * 0.5),
            zoom: 170.0, // frames the whole base map
            yaw: 0.0,
        }
    }

    /// Build the macroquad camera. True-isometric elevation (~35.264°); azimuth
    /// 45° + yaw. Orthographic, so distance doesn't change size — `zoom` (fovy) is
    /// the visible world height.
    pub fn camera(&self) -> Camera3D {
        let elev = 35.264_f32.to_radians();
        let azim = 45_f32.to_radians() + self.yaw;
        let dir = vec3(azim.cos() * elev.cos(), elev.sin(), azim.sin() * elev.cos());
        // The camera must sit BEYOND the map along the view direction, else the near half
        // of the ground falls behind the near plane and clips to a triangle when zoomed
        // out. Push back by the whole map extent (+ zoom), with a far plane to match.
        // Sit BEYOND the map along the view dir so all geometry has positive depth (ortho, so
        // distance doesn't affect size).
        let reach = (COLS as f32 + ROWS as f32) * VOX + self.zoom;
        let position = self.target + dir * reach;
        // Depth precision = (z_far - z_near) / depth-buffer-steps, and ortho depth is LINEAR.
        // The OLD range was the whole-map diagonal (~2700 m at ×16), so a single voxel spanned
        // only a handful of depth steps — faces tied and the water pass had to paper over it
        // with `LessOrEqual` + a hand-tuned z-bias (which then bled water over the shore).
        // Instead fit [z_near, z_far] to ONLY the geometry actually on screen: intersect the
        // four screen corners' view rays with the ground (`y=0`) and the tallest possible
        // column, and bracket the resulting depths. This tracks `zoom`, so precision stays
        // per-voxel-fine at any zoom AND any MAP_SCALE — no magic constants.
        let fwd = (self.target - position).normalize();
        let right = fwd.cross(vec3(0.0, 1.0, 0.0)).normalize();
        let up = right.cross(fwd);
        let half_h = self.zoom * 0.5;
        let half_w = half_h * (screen_width() / screen_height().max(1.0));
        // Vertical span of drawable geometry: ground (0) up to the tallest column + a little
        // headroom for tree canopies.
        let top_y = (UNDERGROUND_LEVELS + SEA_LEVEL + 1 + SURFACE_RANGE) as f32 * VOX + 8.0;
        let (mut z_near, mut z_far) = (f32::MAX, f32::MIN);
        for sx in [-half_w, half_w] {
            for sy in [-half_h, half_h] {
                let corner = position + right * sx + up * sy;
                for y in [0.0_f32, top_y] {
                    let t = (y - corner.y) / fwd.y; // distance along the (downward) view ray to y
                    z_near = z_near.min(t);
                    z_far = z_far.max(t);
                }
            }
        }
        Camera3D {
            position,
            target: self.target,
            up: vec3(0.0, 1.0, 0.0),
            fovy: self.zoom,
            aspect: Some(screen_width() / screen_height()),
            projection: Projection::Orthographics,
            render_target: None,
            viewport: None,
            z_near: (z_near - 1.0).max(1.0),
            z_far: z_far + 1.0,
        }
    }
}

/// Offscreen colour+depth target the scene renders into. The depth attachment is the
/// point: `render_target()` makes a colour-only target with no depth buffer, so a
/// depth-testing 3D camera drawing into it loses occlusion (far faces overdraw near).
pub fn new_scene_target(w: u32, h: u32) -> RenderTarget {
    let rt = render_target_ex(
        w,
        h,
        RenderTargetParams {
            depth: true,
            ..Default::default()
        },
    );
    rt.texture.set_filter(FilterMode::Nearest);
    rt
}

/// Conservative frustum cull: project the AABB's 8 corners through the camera's
/// view-projection matrix and keep the mesh unless every corner falls off the same
/// screen edge. Cheap (8 mat·vec per mesh) and yaw-agnostic; only the x/y screen
/// bounds are tested (the ortho z range comfortably covers the world depth).
pub fn aabb_in_view(vp: &Mat4, lo: Vec3, hi: Vec3) -> bool {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let c = *vp * vec4(x, y, z, 1.0);
                let w = c.w.abs().max(1e-6);
                let (nx, ny) = (c.x / w, c.y / w);
                minx = minx.min(nx);
                maxx = maxx.max(nx);
                miny = miny.min(ny);
                maxy = maxy.max(ny);
            }
        }
    }
    const M: f32 = 0.02; // small margin so chunks aren't popped at the very edge
    !(maxx < -1.0 - M || minx > 1.0 + M || maxy < -1.0 - M || miny > 1.0 + M)
}
