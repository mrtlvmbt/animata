//! R-3 interactive isometric camera — pan (WASD/arrows + mouse drag), zoom (mouse wheel),
//! rotate (yaw about world-Y). Pure render-local state (f32), never feeds back into the sim.
//! Frustum planes are extracted from the view-projection matrix for culling.

use macroquad::prelude::*;

/// Iso pitch angle (radians) — fixed at a canonical isometric angle (~35.26°).
/// tan(pitch) = 0.8660 ≈ √3 / 2 → pitch ≈ 40.9° (or close isometric approximation).
const ISO_PITCH: f32 = std::f32::consts::PI * 40.9 / 180.0;

/// Minimum ortho span (zoom limit — view too close).
const ORTHO_SPAN_MIN: f32 = 5.0;
/// Maximum ortho span (zoom limit — view too far).
const ORTHO_SPAN_MAX: f32 = 200.0;

/// Zoom rate per scroll tick (0.1 = 10% change).
const ZOOM_RATE: f32 = 0.1;

/// Pan speed (world units per second) — keyboard-driven.
const PAN_SPEED: f32 = 20.0;

/// Mouse drag sensitivity (world units per pixel).
const MOUSE_DRAG_SENSITIVITY: f32 = 0.2;

/// Yaw rotation step (radians) — Q/E keys rotate in fixed increments.
const YAW_STEP: f32 = std::f32::consts::PI / 3.0; // 60°

/// Frustum plane: normal and distance from origin.
#[derive(Clone, Copy, Debug)]
pub struct FrustumPlane {
    pub normal: Vec3,
    pub d: f32,
}

impl FrustumPlane {
    /// Check if a point is on the positive side of the plane (inside the frustum for this plane).
    pub fn point_in_front(&self, p: Vec3) -> bool {
        self.normal.dot(p) + self.d >= 0.0
    }

    /// Check if an AABB is inside (or intersecting) this plane.
    /// CONSERVATIVE: returns true if there's any chance the AABB is visible.
    pub fn aabb_intersects(&self, min: Vec3, max: Vec3) -> bool {
        let p = Vec3::new(
            if self.normal.x > 0.0 { max.x } else { min.x },
            if self.normal.y > 0.0 { max.y } else { min.y },
            if self.normal.z > 0.0 { max.z } else { min.z },
        );
        p.dot(self.normal) + self.d >= 0.0
    }
}

/// Interactive isometric camera.
pub struct IsoCam {
    /// World-space focus point (the center of the view).
    pub focus: Vec3,
    /// Yaw angle (rotation about world-Y, radians).
    pub yaw: f32,
    /// Orthographic span (half-width of the view in world units).
    pub ortho_span: f32,
    /// Last mouse position (for drag detection).
    last_mouse_pos: (f32, f32),
}

impl IsoCam {
    /// Create a camera with a given focus, yaw, and initial ortho span.
    pub fn new(focus: Vec3, yaw: f32, ortho_span: f32) -> Self {
        IsoCam { focus, yaw, ortho_span: ortho_span.clamp(ORTHO_SPAN_MIN, ORTHO_SPAN_MAX), last_mouse_pos: mouse_position() }
    }

    /// Update the camera state based on input this frame.
    pub fn update(&mut self) {
        self.update_pan();
        self.update_zoom();
        self.update_rotate();
    }

    /// Update pan (WASD / arrows + mouse drag).
    fn update_pan(&mut self) {
        let dt = get_frame_time();

        // Keyboard pan: WASD and arrow keys.
        let mut pan_delta = Vec3::ZERO;
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            pan_delta.z -= PAN_SPEED * dt;
        }
        if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
            pan_delta.z += PAN_SPEED * dt;
        }
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            pan_delta.x -= PAN_SPEED * dt;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            pan_delta.x += PAN_SPEED * dt;
        }

        // Apply keyboard pan in the camera's local frame (rotated by yaw).
        let cos_yaw = self.yaw.cos();
        let sin_yaw = self.yaw.sin();
        let local_x = Vec3::new(cos_yaw, 0.0, sin_yaw);
        let local_z = Vec3::new(-sin_yaw, 0.0, cos_yaw);
        self.focus += local_x * pan_delta.x + local_z * pan_delta.z;

        // Mouse drag pan (middle or right button).
        let mouse_pos = mouse_position();
        if is_mouse_button_down(MouseButton::Middle) || is_mouse_button_down(MouseButton::Right) {
            let delta = (mouse_pos.0 - self.last_mouse_pos.0, mouse_pos.1 - self.last_mouse_pos.1);
            let world_delta_x = -delta.0 * MOUSE_DRAG_SENSITIVITY * self.ortho_span / 200.0;
            let world_delta_z = delta.1 * MOUSE_DRAG_SENSITIVITY * self.ortho_span / 200.0;
            self.focus += local_x * world_delta_x + local_z * world_delta_z;
        }
        self.last_mouse_pos = mouse_pos;
    }

    /// Update zoom (mouse wheel).
    fn update_zoom(&mut self) {
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            // Positive wheel = zoom in (decrease span), negative = zoom out (increase span).
            let zoom_factor = (1.0 - ZOOM_RATE * wheel).max(0.1);
            self.ortho_span = (self.ortho_span * zoom_factor).clamp(ORTHO_SPAN_MIN, ORTHO_SPAN_MAX);
        }
    }

    /// Update rotation (Q/E keys or comma/period).
    fn update_rotate(&mut self) {
        if is_key_pressed(KeyCode::Q) || is_key_pressed(KeyCode::Comma) {
            self.yaw -= YAW_STEP;
        }
        if is_key_pressed(KeyCode::E) || is_key_pressed(KeyCode::Period) {
            self.yaw += YAW_STEP;
        }
        // Wrap yaw to [0, 2π).
        while self.yaw < 0.0 {
            self.yaw += std::f32::consts::TAU;
        }
        while self.yaw >= std::f32::consts::TAU {
            self.yaw -= std::f32::consts::TAU;
        }
    }

    /// Generate a Camera3D for this frame.
    pub fn to_camera3d(&self) -> Camera3D {
        // Position the camera at an isometric angle relative to the focus.
        // Distance from focus is proportional to the view span.
        let distance = self.ortho_span * 1.4;
        let cos_yaw = self.yaw.cos();
        let sin_yaw = self.yaw.sin();
        let cos_pitch = ISO_PITCH.cos();
        let sin_pitch = ISO_PITCH.sin();

        let cam_x = cos_yaw * cos_pitch * distance;
        let cam_y = sin_pitch * distance;
        let cam_z = sin_yaw * cos_pitch * distance;

        let position = self.focus + Vec3::new(cam_x, cam_y, cam_z);
        Camera3D {
            position,
            target: self.focus,
            up: Vec3::new(0.0, 1.0, 0.0),
            projection: Projection::Orthographics,
            fovy: self.ortho_span,
            ..Default::default()
        }
    }

    /// Extract the 6 frustum planes from the camera's view-projection matrix (Gribb-Hartmann).
    /// macroquad's Mat4 uses column vectors (x_axis, y_axis, z_axis, w_axis as Vec4).
    /// We extract rows by indexing into each axis.
    pub fn frustum_planes(&self) -> [FrustumPlane; 6] {
        let cam = self.to_camera3d();
        let vp = cam.matrix();

        // Extract matrix elements from column vectors (macroquad's representation).
        // Row i = [x_axis[i], y_axis[i], z_axis[i], w_axis[i]]
        let row0 = [vp.x_axis[0], vp.y_axis[0], vp.z_axis[0], vp.w_axis[0]];
        let row1 = [vp.x_axis[1], vp.y_axis[1], vp.z_axis[1], vp.w_axis[1]];
        let row2 = [vp.x_axis[2], vp.y_axis[2], vp.z_axis[2], vp.w_axis[2]];
        let row3 = [vp.x_axis[3], vp.y_axis[3], vp.z_axis[3], vp.w_axis[3]];

        // Gribb-Hartmann plane extraction. Planes are ordered: right, left, bottom, top, far, near.
        let mut planes = [
            // Right plane: (row3 - row0)
            FrustumPlane {
                normal: Vec3::new(row3[0] - row0[0], row3[1] - row0[1], row3[2] - row0[2]),
                d: row3[3] - row0[3],
            },
            // Left plane: (row3 + row0)
            FrustumPlane {
                normal: Vec3::new(row3[0] + row0[0], row3[1] + row0[1], row3[2] + row0[2]),
                d: row3[3] + row0[3],
            },
            // Bottom plane: (row3 + row1)
            FrustumPlane {
                normal: Vec3::new(row3[0] + row1[0], row3[1] + row1[1], row3[2] + row1[2]),
                d: row3[3] + row1[3],
            },
            // Top plane: (row3 - row1)
            FrustumPlane {
                normal: Vec3::new(row3[0] - row1[0], row3[1] - row1[1], row3[2] - row1[2]),
                d: row3[3] - row1[3],
            },
            // Far plane: (row3 - row2)
            FrustumPlane {
                normal: Vec3::new(row3[0] - row2[0], row3[1] - row2[1], row3[2] - row2[2]),
                d: row3[3] - row2[3],
            },
            // Near plane: (row3 + row2)
            FrustumPlane {
                normal: Vec3::new(row3[0] + row2[0], row3[1] + row2[1], row3[2] + row2[2]),
                d: row3[3] + row2[3],
            },
        ];

        // Normalize planes. Assert non-degenerate matrices (macroquad's Camera3D should never
        // produce zero-length normals from orthographic projection).
        for plane in &mut planes {
            let len = plane.normal.length();
            assert!(len > 1e-6, "frustum plane normal too small (degenerate matrix)");
            plane.normal /= len;
            plane.d /= len;
        }

        planes
    }

    /// Check if a point is visible in the frustum.
    pub fn point_in_frustum(&self, p: Vec3) -> bool {
        let planes = self.frustum_planes();
        planes.iter().all(|plane| plane.point_in_front(p))
    }

    /// Check if an AABB is visible (or intersecting) the frustum.
    pub fn aabb_in_frustum(&self, min: Vec3, max: Vec3) -> bool {
        let planes = self.frustum_planes();
        planes.iter().all(|plane| plane.aabb_intersects(min, max))
    }

    /// Get the current zoom level as a value in [0, 1] for LOD purposes.
    /// Returns 0 when zoomed FAR (ortho_span=200), 1 when zoomed CLOSE (ortho_span=5).
    /// For RnD R21: LOD is a pure function of zoom, deterministic, never per-creature distance.
    pub fn zoom_lod_factor(&self) -> f32 {
        // Invert the span-to-factor mapping: 1 at small ortho_span (close zoom), 0 at large (far zoom).
        (1.0 - (self.ortho_span - ORTHO_SPAN_MIN) / (ORTHO_SPAN_MAX - ORTHO_SPAN_MIN)).clamp(0.0, 1.0)
    }
}
