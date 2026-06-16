//! Fixed-topology recurrent neural network (leaky-integrator Elman / CTRNN-style).
//!
//! Topology: NN_INPUTS + recurrent NN_HIDDEN -> NN_HIDDEN (tanh) -> NN_OUTPUTS.
//! The hidden layer reads its own previous activation, and the new state is a
//! leaky blend `(1-γ)·old + γ·tanh(...)` (γ = the `leak` gene): with low γ the
//! state changes slowly and carries memory over many steps. Weights come from
//! the decoded genome in order: input->hidden, hidden->hidden, hidden->output.

use crate::config::*;

pub struct Brain {
    /// input->hidden weights, length NN_INPUTS * NN_HIDDEN.
    w_ih: Vec<f32>,
    /// recurrent hidden->hidden weights, length NN_HIDDEN * NN_HIDDEN.
    w_hh: Vec<f32>,
    /// hidden->output weights, length NN_HIDDEN * NN_OUTPUTS.
    w_ho: Vec<f32>,
    /// Leaky-integrator rate γ (LEAK_RANGE): the fraction of the new candidate
    /// activation blended in each step. γ=1 -> plain Elman; low γ -> slow memory.
    leak: f32,
    /// Previous hidden activations (the memory); starts at zero.
    state: [f32; NN_HIDDEN],
    /// Smoothed share of hidden activation coming from the recurrent (memory)
    /// term vs the current inputs, 0..1. A running gauge of how much this brain
    /// actually *uses* its memory while behaving (not just its weight capacity).
    pub mem_use: f32,
}

impl Brain {
    pub fn from_weights(weights: &[f32], leak: f32) -> Self {
        debug_assert_eq!(weights.len(), NN_WEIGHTS);
        let ih = NN_INPUTS * NN_HIDDEN;
        let hh = NN_HIDDEN * NN_HIDDEN;
        Brain {
            w_ih: weights[..ih].to_vec(),
            w_hh: weights[ih..ih + hh].to_vec(),
            w_ho: weights[ih + hh..].to_vec(),
            leak,
            state: [0.0; NN_HIDDEN],
            mem_use: 0.0,
        }
    }

    /// Run one recurrent forward pass, updating the memory. `inputs` must be
    /// length NN_INPUTS. Returns `[throttle, turn]`, each in `-1..=1`.
    pub fn forward(&mut self, inputs: &[f32; NN_INPUTS]) -> [f32; NN_OUTPUTS] {
        let mut hidden = [0.0f32; NN_HIDDEN];
        let mut in_abs = 0.0f32; // total |input contribution| across hidden units
        let mut rec_abs = 0.0f32; // total |recurrent contribution|
        for h in 0..NN_HIDDEN {
            let mut sum = 0.0;
            for i in 0..NN_INPUTS {
                sum += inputs[i] * self.w_ih[h * NN_INPUTS + i];
            }
            let in_part = sum; // input contribution before the recurrent term
            // Recurrent term: previous hidden state feeds back in. Accumulated
            // into the same `sum` (same order as before) to keep numerics exact.
            for p in 0..NN_HIDDEN {
                sum += self.state[p] * self.w_hh[h * NN_HIDDEN + p];
            }
            in_abs += in_part.abs();
            rec_abs += (sum - in_part).abs();
            // Leaky integrator: blend the new candidate with the carried-over old
            // state (both terms use the old `self.state`). Low γ -> slow memory.
            let cand = sum.tanh();
            hidden[h] = (1.0 - self.leak) * self.state[h] + self.leak * cand;
        }
        self.state = hidden;
        // Realized memory reliance this step, EMA-smoothed (warms over ~25 steps).
        let frac = rec_abs / (in_abs + rec_abs + 1e-6);
        self.mem_use += 0.04 * (frac - self.mem_use);

        let mut out = [0.0f32; NN_OUTPUTS];
        for o in 0..NN_OUTPUTS {
            let mut sum = 0.0;
            for h in 0..NN_HIDDEN {
                sum += hidden[h] * self.w_ho[o * NN_HIDDEN + h];
            }
            out[o] = sum.tanh();
        }
        out
    }
}
