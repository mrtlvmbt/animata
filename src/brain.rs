//! Fixed-topology recurrent neural network (leaky-integrator Elman / CTRNN-style).
//!
//! Topology: NN_INPUTS + recurrent NN_HIDDEN -> NN_HIDDEN (tanh) -> NN_OUTPUTS.
//! The hidden layer reads its own previous activation, and the new state is a
//! leaky blend `(1-γ)·old + γ·tanh(...)` (γ = the `leak` gene): with low γ the
//! state changes slowly and carries memory over many steps. Weights are assembled
//! from the genome's marker-decoded [`Synapse`] list into dense matrices (see
//! [`Brain::from_synapses`]), so the forward pass stays a plain matmul.

use crate::config::*;
use crate::genome::Synapse;

pub struct Brain {
    /// input->hidden weights, length NN_INPUTS * n_hidden.
    w_ih: Vec<f32>,
    /// recurrent hidden->hidden weights, length n_hidden * n_hidden.
    w_hh: Vec<f32>,
    /// hidden->output weights, length n_hidden * n_outputs.
    w_ho: Vec<f32>,
    /// Output-port count (base controls + one per appendage; variable per body).
    n_outputs: usize,
    /// Reused output buffer (avoids per-step allocation), length n_outputs.
    out: Vec<f32>,
    /// Leaky-integrator rate γ (LEAK_RANGE): the fraction of the new candidate
    /// activation blended in each step. γ=1 -> plain Elman; low γ -> slow memory.
    leak: f32,
    /// Hidden-layer width (evolvable per creature via neuron records).
    n_hidden: usize,
    /// Previous hidden activations (the memory); starts at zero, length n_hidden.
    state: Vec<f32>,
    /// Reused scratch for the next hidden state (avoids per-step allocation).
    scratch: Vec<f32>,
    /// Smoothed share of hidden activation coming from the recurrent (memory)
    /// term vs the current inputs, 0..1. A running gauge of how much this brain
    /// actually *uses* its memory while behaving (not just its weight capacity).
    pub mem_use: f32,
}

impl Brain {
    /// Assemble the brain from a tag-based synapse list into dense weight
    /// matrices (sized to this creature's `n_hidden`), so the forward pass stays a
    /// plain matmul. Each record routes by its (src,dst) port kinds; multiple
    /// records to the same slot accumulate, and unconnected slots stay zero.
    /// Synapse tags are already resolved against `n_hidden` at decode time:
    /// `src < NN_INPUTS` is an input else hidden `src-NN_INPUTS`; `dst < n_hidden`
    /// is hidden else output `dst-n_hidden`.
    pub fn from_synapses(synapses: &[Synapse], leak: f32, n_hidden: usize, n_outputs: usize) -> Self {
        let nh = n_hidden.max(1);
        let no = n_outputs.max(1);
        let mut w_ih = vec![0.0f32; NN_INPUTS * nh];
        let mut w_hh = vec![0.0f32; nh * nh];
        let mut w_ho = vec![0.0f32; nh * no];
        for s in synapses {
            let (src, dst) = (s.src as usize, s.dst as usize);
            if src < NN_INPUTS {
                if dst < nh {
                    w_ih[dst * NN_INPUTS + src] += s.w; // input -> hidden
                }
            } else {
                let p = src - NN_INPUTS; // hidden source unit
                if p >= nh {
                    continue;
                }
                if dst < nh {
                    w_hh[dst * nh + p] += s.w; // hidden -> hidden (recurrent)
                } else {
                    w_ho[(dst - nh) * nh + p] += s.w; // hidden -> output
                }
            }
        }
        Brain {
            w_ih,
            w_hh,
            w_ho,
            n_outputs: no,
            out: vec![0.0; no],
            leak,
            n_hidden: nh,
            state: vec![0.0; nh],
            scratch: vec![0.0; nh],
            mem_use: 0.0,
        }
    }

    /// Run one recurrent forward pass, updating the memory. `inputs` must be
    /// length NN_INPUTS. Returns the `n_outputs` output activations (each in
    /// `-1..=1`): the base controls first, then one drive per appendage port.
    pub fn forward(&mut self, inputs: &[f32; NN_INPUTS]) -> &[f32] {
        let nh = self.n_hidden;
        let mut in_abs = 0.0f32; // total |input contribution| across hidden units
        let mut rec_abs = 0.0f32; // total |recurrent contribution|
        for h in 0..nh {
            let mut sum = 0.0;
            for i in 0..NN_INPUTS {
                sum += inputs[i] * self.w_ih[h * NN_INPUTS + i];
            }
            let in_part = sum; // input contribution before the recurrent term
            // Recurrent term: previous hidden state feeds back in.
            for p in 0..nh {
                sum += self.state[p] * self.w_hh[h * nh + p];
            }
            in_abs += in_part.abs();
            rec_abs += (sum - in_part).abs();
            // Leaky integrator: blend the new candidate with the carried-over old
            // state (both use the old `self.state`). Low γ -> slow memory.
            self.scratch[h] = (1.0 - self.leak) * self.state[h] + self.leak * sum.tanh();
        }
        std::mem::swap(&mut self.state, &mut self.scratch);
        // Realized memory reliance this step, EMA-smoothed (warms over ~25 steps).
        let frac = rec_abs / (in_abs + rec_abs + 1e-6);
        self.mem_use += 0.04 * (frac - self.mem_use);

        for o in 0..self.n_outputs {
            let mut sum = 0.0;
            for h in 0..nh {
                sum += self.state[h] * self.w_ho[o * nh + h];
            }
            self.out[o] = sum.tanh();
        }
        &self.out
    }
}
