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

/// F2: Camera input snapshot for testable gating. Extracted from macroquad input state
/// so unit tests can inject synthetic input without relying on macroquad state.
#[derive(Clone, Copy, Debug)]
pub struct CamInput {
    /// Mouse wheel y delta (positive = zoom in, negative = zoom out).
    pub wheel_y: f32,
    /// Mouse movement delta in screen pixels (None if not dragging).
    pub mouse_delta: Option<(f32, f32)>,
    /// Keyboard pan direction: (x, z) components (normalized or raw).
    pub pan_dir: (f32, f32),
    /// Yaw rotation step: -1 (Q), 0 (no key), or +1 (E).
    pub yaw_step: i8,
    /// Current mouse position in screen pixels (for tracking drag state). Pure input to apply_cam_input.
    pub current_mouse_pos: (f32, f32),
}

impl CamInput {
    /// Collect current frame input from macroquad state.
    pub fn collect() -> Self {
        let dt = get_frame_time();
        let wheel = mouse_wheel().1;

        // Keyboard pan
        let mut pan_x = 0.0f32;
        let mut pan_z = 0.0f32;
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            pan_z -= PAN_SPEED * dt;
        }
        if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
            pan_z += PAN_SPEED * dt;
        }
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            pan_x -= PAN_SPEED * dt;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            pan_x += PAN_SPEED * dt;
        }

        // Mouse drag (middle or right button)
        let current_mouse_pos = mouse_position();
        let mouse_delta = if is_mouse_button_down(MouseButton::Middle) || is_mouse_button_down(MouseButton::Right) {
            Some(current_mouse_pos)
        } else {
            None
        };

        // Yaw rotation
        let mut yaw_step = 0i8;
        if is_key_pressed(KeyCode::Q) || is_key_pressed(KeyCode::Comma) {
            yaw_step = -1;
        } else if is_key_pressed(KeyCode::E) || is_key_pressed(KeyCode::Period) {
            yaw_step = 1;
        }

        CamInput {
            wheel_y: wheel,
            mouse_delta,
            pan_dir: (pan_x, pan_z),
            yaw_step,
            current_mouse_pos,
        }
    }
}

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
        IsoCam { focus, yaw, ortho_span: ortho_span.clamp(ORTHO_SPAN_MIN, ORTHO_SPAN_MAX), last_mouse_pos: (0.0, 0.0) }
    }

    /// Update the camera state based on input this frame (no gating — for screenshot/bench paths).
    /// F3: Consolidation — ungated update just delegates to gated with all input accepted.
    pub fn update(&mut self) {
        self.update_gated(false, false);
    }

    /// Update the camera state with UI gating.
    /// When UI wants pointer input, mouse-driven camera controls are skipped.
    /// When UI wants keyboard input, keyboard-driven camera controls are skipped.
    /// F2: Refactored to collect input and apply it testably.
    pub fn update_gated(&mut self, wants_pointer: bool, wants_keyboard: bool) {
        let input = CamInput::collect();
        self.apply_cam_input(&input, wants_pointer, wants_keyboard);
    }

    /// Apply camera input with gating (F2: testable core).
    /// Test can inject synthetic CamInput and verify that gating actually blocks changes.
    pub fn apply_cam_input(&mut self, input: &CamInput, wants_pointer: bool, wants_keyboard: bool) {
        // Keyboard pan
        if !wants_keyboard && (input.pan_dir.0 != 0.0 || input.pan_dir.1 != 0.0) {
            let cos_yaw = self.yaw.cos();
            let sin_yaw = self.yaw.sin();
            let local_x = Vec3::new(cos_yaw, 0.0, sin_yaw);
            let local_z = Vec3::new(-sin_yaw, 0.0, cos_yaw);
            let pan_delta = Vec3::new(input.pan_dir.0, 0.0, input.pan_dir.1);
            self.focus += local_x * pan_delta.x + local_z * pan_delta.z;
        }

        // Mouse drag pan
        if !wants_pointer {
            if let Some((curr_x, curr_y)) = input.mouse_delta {
                let cos_yaw = self.yaw.cos();
                let sin_yaw = self.yaw.sin();
                let local_x = Vec3::new(cos_yaw, 0.0, sin_yaw);
                let local_z = Vec3::new(-sin_yaw, 0.0, cos_yaw);
                let delta = (curr_x - self.last_mouse_pos.0, curr_y - self.last_mouse_pos.1);
                let world_delta_x = -delta.0 * MOUSE_DRAG_SENSITIVITY * self.ortho_span / 200.0;
                let world_delta_z = delta.1 * MOUSE_DRAG_SENSITIVITY * self.ortho_span / 200.0;
                self.focus += local_x * world_delta_x + local_z * world_delta_z;
            }
        }
        self.last_mouse_pos = input.current_mouse_pos;

        // Zoom
        if !wants_pointer && input.wheel_y != 0.0 {
            let zoom_factor = (1.0 - ZOOM_RATE * input.wheel_y).max(0.1);
            self.ortho_span = (self.ortho_span * zoom_factor).clamp(ORTHO_SPAN_MIN, ORTHO_SPAN_MAX);
        }

        // Yaw rotation
        if !wants_keyboard && input.yaw_step != 0 {
            if input.yaw_step < 0 {
                self.yaw -= YAW_STEP;
            } else if input.yaw_step > 0 {
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

    /// Compute pixels-per-world-unit at the current orthographic zoom (R-4 LOD).
    /// Formula: `screen_height / ortho_span` — the number of screen pixels that span one world unit.
    /// Returns a value proportional to zoom: larger values = zoomed in (closer), smaller = zoomed out (farther).
    /// This is a pure function of zoom and viewport ONLY (RnD R21 determinism) — never per-creature
    /// distance or wall-clock — so the whole creature set shares one LOD tier per frame.
    pub fn px_per_m(&self) -> f32 {
        screen_height() / self.ortho_span
    }
}
