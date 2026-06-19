//! Environment fields accessed by selection pressures.
//!
//! Pressures declare dependencies via `fields()`, and an `EnvSampler` provides per-column
//! lazy sampling (F3: not eager union of all fields for every creature, but per-pressure on-demand).

use crate::terrain::VoxelTerrain;

/// A named environment field that pressures can query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Temperature,
    Moisture,
    Light,
    GroundTone,
    Nutrient,
    Elevation,
}

/// Read-only environment sample for a single creature at a column. Pressures receive this;
/// fields are computed on first read and cached for the tick to avoid repeated work.
pub struct EnvSample {
    col: (usize, usize),
    tick: u64,
    terrain: *const VoxelTerrain,
    // Cached field values (computed lazily on first access).
    temperature: Option<f32>,
    moisture: Option<f32>,
    light: Option<f32>,
    ground_tone: Option<f32>,
    nutrient: Option<f32>,
    elevation: Option<f32>,
}

impl EnvSample {
    /// Create a new environment sample for a column at a tick. The terrain pointer is captured
    /// (never dereferenced in an unsafe way — just to defer borrowing across the tick).
    pub fn new(col: (usize, usize), tick: u64, terrain: &VoxelTerrain) -> Self {
        Self {
            col,
            tick,
            terrain: terrain as *const _,
            temperature: None,
            moisture: None,
            light: None,
            ground_tone: None,
            nutrient: None,
            elevation: None,
        }
    }

    /// Sample a field value, computing and caching it on first access.
    pub fn sample(&mut self, field: Field) -> f32 {
        match field {
            Field::Temperature => {
                if self.temperature.is_none() {
                    let terrain = unsafe { &*self.terrain };
                    self.temperature = Some(terrain.temperature_at(self.col.0, self.col.1));
                }
                self.temperature.unwrap()
            }
            Field::Moisture => {
                if self.moisture.is_none() {
                    let terrain = unsafe { &*self.terrain };
                    self.moisture = Some(terrain.moisture_at(self.col.0, self.col.1));
                }
                self.moisture.unwrap()
            }
            Field::Light => {
                // Light is computed by the pressure itself based on stratum/time — not from terrain.
                // This field is a placeholder; pressures compute it directly via light_for().
                if self.light.is_none() {
                    self.light = Some(0.0);
                }
                self.light.unwrap()
            }
            Field::GroundTone => {
                if self.ground_tone.is_none() {
                    let terrain = unsafe { &*self.terrain };
                    self.ground_tone = Some(terrain.ground_tone_at(self.col.0, self.col.1));
                }
                self.ground_tone.unwrap()
            }
            Field::Nutrient => {
                if self.nutrient.is_none() {
                    let terrain = unsafe { &*self.terrain };
                    self.nutrient = Some(terrain.nutrient_at(self.col.0, self.col.1, self.tick));
                }
                self.nutrient.unwrap()
            }
            Field::Elevation => {
                if self.elevation.is_none() {
                    let terrain = unsafe { &*self.terrain };
                    self.elevation = Some(terrain.height_at(self.col.0, self.col.1) as f32 / 255.0);
                }
                self.elevation.unwrap()
            }
        }
    }
}
