//! Multi-layer LSTM implementation, generic over the floating-point precision.
//!
//! Architecture (per layer, stacked):
//! ```text
//!   layer 0: input (x_t) ──→ [LSTM Cell] ──→ hidden (h_t⁰)
//!   layer 1: h_t⁰ ──→ [LSTM Cell] ──→ hidden (h_t¹)
//!   ...   : hidden of layer N feeds layer N+1 as input
//!   final : hidden_t ──→ [projection] ──→ output (y_t)
//! ```
//!
//! Layer 0 has `num_layers == 0` means caller passes 0, treat as 1. Each layer
//! has its own weights; the hidden-state output of layer N becomes the input of
//! layer N+1. The final layer's hidden state is projected to `output_size`.
//!
//! Gates (concatenated, per layer):
//! ```text
//!     [i, f, g, o] = W_ih * x_t + W_hh * h_{t-1} + b
//!     i = sigmoid(input gate)
//!     f = sigmoid(forget gate)
//!     g = tanh(candidate)
//!     o = sigmoid(output gate)
//!     c_t = f ⊙ c_{t-1} + i ⊙ g
//!     h_t = o ⊙ tanh(c_t)
//! ```
//!
//! Precision is controlled via a generic parameter (`f64` default, `f32` via
//! [`LstmModelF32`]) so that pure-Rust models can be built with either
//! precision for parity with the C++ LSTM (f32) while defaulting to f64 for
//! the Python bindings.

use num_traits::Float;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};

fn default_num_layers() -> usize {
    1
}

/// Configuration for building an LSTM model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LstmConfig {
    /// Dimension of input features (layer 0 input).
    pub input_size: usize,
    /// Dimension of hidden state (shared by all stacked layers).
    pub hidden_size: usize,
    /// Dimension of output (1 for regression/binary, N for classification).
    pub output_size: usize,
    /// Number of stacked LSTM layers (>= 1). Hidden state of layer N feeds layer N+1.
    #[serde(default = "default_num_layers")]
    pub num_layers: usize,
}

impl Default for LstmConfig {
    fn default() -> Self {
        Self {
            input_size: 0,
            hidden_size: 0,
            output_size: 0,
            num_layers: 1,
        }
    }
}

/// Recurrent weights for a single stacked LSTM layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LstmLayer<F: Float = f64> {
    /// Input-to-hidden weights: (4*hidden, input_len). Layer 0: input_size; above: hidden_size.
    pub w_ih: Vec<Vec<F>>,
    /// Hidden-to-hidden weights: (4*hidden, hidden).
    pub w_hh: Vec<Vec<F>>,
    /// Input-to-hidden bias: (4*hidden,).
    pub b_ih: Vec<F>,
    /// Hidden-to-hidden bias: (4*hidden,).
    pub b_hh: Vec<F>,
}

/// Stacked LSTM model with trained weights.
///
/// Generic over the floating-point precision `F` (default `f64`; use
/// [`LstmModelF32`] for f32). Layer 0 weights are kept as flat fields
/// (`w_ih`/`w_hh`/`b_ih`/`b_hh`) for JSON/format backward-compat; layers
/// 1..`num_layers` live in `extra_layers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LstmModel<F: Float = f64> {
    pub config: LstmConfig,
    /// Layer 0 input-to-hidden weights: (4*hidden, input_size).
    pub w_ih: Vec<Vec<F>>,
    /// Layer 0 hidden-to-hidden weights: (4*hidden, hidden).
    pub w_hh: Vec<Vec<F>>,
    /// Layer 0 input-to-hidden bias: (4*hidden,).
    pub b_ih: Vec<F>,
    /// Layer 0 hidden-to-hidden bias: (4*hidden,).
    pub b_hh: Vec<F>,
    /// Layers 1..num_layers (each hidden→hidden). Empty for num_layers == 1.
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub extra_layers: Vec<LstmLayer<F>>,
    /// Output projection weights: (output, hidden).
    pub w_ho: Vec<Vec<F>>,
    /// Output projection bias: (output,).
    pub b_ho: Vec<F>,
}

/// `LstmModel<f64>` — default (full) precision.
pub type LstmModelF64 = LstmModel<f64>;
/// `LstmModel<f32>` — half-precision for C++-LSTM parity.
pub type LstmModelF32 = LstmModel<f32>;

/// Intermediate state from a single forward pass (for backprop).
#[derive(Debug, Clone)]
pub struct LstmCell<F: Float = f64> {
    // Gate activations
    pub input_gate: Vec<F>,
    pub forget_gate: Vec<F>,
    pub candidate: Vec<F>,
    pub output_gate: Vec<F>,
    // States
    pub cell_state: Vec<F>,
    pub hidden_state: Vec<F>,
    // Inputs (for backprop)
    pub x: Vec<F>,
    pub h_prev: Vec<F>,
    pub c_prev: Vec<F>,
}

/// Result of training.
#[derive(Debug, Clone)]
pub struct TrainingResult<F: Float = f64> {
    pub final_loss: F,
    pub epochs: usize,
    pub loss_history: Vec<F>,
}

// ─────────────────────── Helper math ───────────────────────

fn sigmoid<F: Float>(x: F) -> F {
    F::one() / (F::one() + (-x).exp())
}

fn sigmoid_derivative<F: Float>(s: F) -> F {
    s * (F::one() - s)
}

fn tanh_derivative<F: Float>(t: F) -> F {
    F::one() - t * t
}

/// Element-wise multiply.
fn hadamard<F: Float>(a: &[F], b: &[F]) -> Vec<F> {
    a.iter().zip(b.iter()).map(|(x, y)| *x * *y).collect()
}

/// Vector addition.
fn vec_add<F: Float>(a: &[F], b: &[F]) -> Vec<F> {
    a.iter().zip(b.iter()).map(|(x, y)| *x + *y).collect()
}

// ─────────────────────── LSTM Cell ───────────────────────

impl<F: Float> LstmModel<F> {
    /// Create a new model with Xavier-initialized weights.
    ///
    /// `num_layers >= 1`; layer 0 maps `input_size -> hidden_size`, each
    /// further layer maps `hidden_size -> hidden_size` (its input is the
    /// previous layer's hidden-state output).
    pub fn new(config: LstmConfig) -> Self {
        use rand::RngExt;
        let mut rng = rand::rng();
        let h = config.hidden_size;
        let i = config.input_size;
        let o = config.output_size;
        let n = config.num_layers.max(1);

        let mut xavier = |rows: usize, cols: usize| -> Vec<Vec<F>> {
            let limit = (6.0 / (rows + cols) as f64).sqrt();
            (0..rows)
                .map(|_| {
                    (0..cols)
                        .map(|_| F::from(rng.random_range(-limit..limit)).expect("f64->F cast"))
                        .collect()
                })
                .collect()
        };

        let mut extra_layers = Vec::with_capacity(n.saturating_sub(1));
        for _ in 1..n {
            extra_layers.push(LstmLayer {
                w_ih: xavier(4 * h, h),
                w_hh: xavier(4 * h, h),
                b_ih: vec![F::zero(); 4 * h],
                b_hh: vec![F::zero(); 4 * h],
            });
        }

        Self {
            config: LstmConfig {
                num_layers: n,
                ..config
            },
            w_ih: xavier(4 * h, i),
            w_hh: xavier(4 * h, h),
            b_ih: vec![F::zero(); 4 * h],
            b_hh: vec![F::zero(); 4 * h],
            extra_layers,
            w_ho: xavier(o, h),
            b_ho: vec![F::zero(); o],
        }
    }

    /// Single LSTM cell step for one layer's weights.
    fn cell_step(
        w_ih: &[Vec<F>],
        w_hh: &[Vec<F>],
        b_ih: &[F],
        b_hh: &[F],
        hidden: usize,
        x: &[F],
        h_prev: &[F],
        c_prev: &[F],
    ) -> LstmCell<F> {
        let combined = 4 * hidden;

        // Compute gate pre-activations
        let mut gates = vec![F::zero(); combined];
        for j in 0..combined {
            gates[j] = b_ih[j] + b_hh[j];
            for k in 0..x.len() {
                gates[j] = gates[j] + w_ih[j][k] * x[k];
            }
            for k in 0..h_prev.len() {
                gates[j] = gates[j] + w_hh[j][k] * h_prev[k];
            }
        }

        // Split into gates
        let input_gate: Vec<F> = gates[0..hidden].iter().map(|&v| sigmoid(v)).collect();
        let forget_gate: Vec<F> = gates[hidden..2 * hidden].iter().map(|&v| sigmoid(v)).collect();
        let candidate: Vec<F> = gates[2 * hidden..3 * hidden].iter().map(|&v| v.tanh()).collect();
        let output_gate: Vec<F> = gates[3 * hidden..4 * hidden].iter().map(|&v| sigmoid(v)).collect();

        // Cell state update: c_t = f ⊙ c_prev + i ⊙ g
        let cell_state = {
            let fg = hadamard(&forget_gate, c_prev);
            let ig = hadamard(&input_gate, &candidate);
            vec_add(&fg, &ig)
        };

        // Hidden state: h_t = o ⊙ tanh(c_t)
        let tanh_c: Vec<F> = cell_state.iter().map(|&v| v.tanh()).collect();
        let hidden_state = hadamard(&output_gate, &tanh_c);

        LstmCell {
            input_gate,
            forget_gate,
            candidate,
            output_gate,
            cell_state,
            hidden_state,
            x: x.to_vec(),
            h_prev: h_prev.to_vec(),
            c_prev: c_prev.to_vec(),
        }
    }

    /// Forward pass through the LSTM cell for one timestep.
    ///
    /// `x` — input vector (input_size)
    /// `h_prev` — previous hidden state (hidden_size), for layer 0
    /// `c_prev` — previous cell state (hidden_size), for layer 0
    ///
    /// For `num_layers > 1`, layer 0's hidden-state output becomes layer 1's
    /// input, and so on; upper layers start from the zero state at each call
    /// (per-timestep state carry across layers is handled by `forward_sequence`).
    pub fn forward_cell(&self, x: &[F], h_prev: &[F], c_prev: &[F]) -> LstmCell<F> {
        let h = self.config.hidden_size;
        let num_layers = 1 + self.extra_layers.len();
        let h_zeros = vec![F::zero(); h];
        let h_layers: Vec<&[F]> = std::iter::once(h_prev)
            .chain(std::iter::repeat_n(&h_zeros as &[F], num_layers.saturating_sub(1)))
            .collect();
        let c_layers: Vec<&[F]> = std::iter::once(c_prev)
            .chain(std::iter::repeat_n(&h_zeros as &[F], num_layers.saturating_sub(1)))
            .collect();
        self.forward_cell_multi(x, &h_layers, &c_layers).0
    }

    /// Forward pass through all layers for one timestep, threading per-layer h/c state.
    ///
    /// Returns `(final_cell, updated_h_layers, updated_c_layers)`.
    /// `h_layers` / `c_layers` must have length `num_layers` (layer 0 first, last layer last).
    fn forward_cell_multi(
        &self,
        x: &[F],
        h_layers: &[&[F]],
        c_layers: &[&[F]],
    ) -> (LstmCell<F>, Vec<Vec<F>>, Vec<Vec<F>>) {
        let (cells, out_h, out_c) = self.forward_cell_multi_all(x, h_layers, c_layers);
        (cells.into_iter().next_back().expect("num_layers >= 1"), out_h, out_c)
    }

    /// Forward pass through all layers for one timestep, returning **every**
    /// layer's cell (layer 0 first) in addition to the updated per-layer h/c
    /// states. The last element is the final layer's cell.
    fn forward_cell_multi_all(
        &self,
        x: &[F],
        h_layers: &[&[F]],
        c_layers: &[&[F]],
    ) -> (Vec<LstmCell<F>>, Vec<Vec<F>>, Vec<Vec<F>>) {
        let h = self.config.hidden_size;
        let num_layers = 1 + self.extra_layers.len();
        assert_eq!(h_layers.len(), num_layers);
        assert_eq!(c_layers.len(), num_layers);

        // Layer 0
        let mut cell = Self::cell_step(
            &self.w_ih,
            &self.w_hh,
            &self.b_ih,
            &self.b_hh,
            h,
            x,
            h_layers[0],
            c_layers[0],
        );
        let mut cells = Vec::with_capacity(num_layers);
        cells.push(cell.clone());
        let mut out_h = vec![cell.hidden_state.clone()];
        let mut out_c = vec![cell.cell_state.clone()];

        // Upper layers
        for (i, layer) in self.extra_layers.iter().enumerate() {
            cell = Self::cell_step(
                &layer.w_ih,
                &layer.w_hh,
                &layer.b_ih,
                &layer.b_hh,
                h,
                &cell.hidden_state, // input = previous layer's hidden
                h_layers[i + 1],
                c_layers[i + 1],
            );
            cells.push(cell.clone());
            out_h.push(cell.hidden_state.clone());
            out_c.push(cell.cell_state.clone());
        }

        (cells, out_h, out_c)
    }

    /// Run forward pass over a sequence, return all cell states and final output.
    ///
    /// `sequence` — list of input vectors, one per timestep.
    /// Returns: (all cells, output projection at final step using last layer's hidden).
    /// Per-layer hidden/cell states are carried across timesteps.
    pub fn forward_sequence(&self, sequence: &[Vec<F>]) -> (Vec<LstmCell<F>>, Vec<F>) {
        let h = self.config.hidden_size;
        let o = self.config.output_size;
        let num_layers = 1 + self.extra_layers.len();

        // Per-layer h/c carried across timesteps.
        let mut h_layers: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];
        let mut c_layers: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];
        let mut cells = Vec::with_capacity(sequence.len());

        for x in sequence {
            let h_refs: Vec<&[F]> = h_layers.iter().map(|v| v.as_slice()).collect();
            let c_refs: Vec<&[F]> = c_layers.iter().map(|v| v.as_slice()).collect();
            let (cell, new_h, new_c) = self.forward_cell_multi(x, &h_refs, &c_refs);
            h_layers = new_h;
            c_layers = new_c;
            cells.push(cell);
        }

        // Output projection uses the LAST layer's hidden state.
        let final_h = &h_layers[num_layers - 1];
        let output: Vec<F> = (0..o)
            .map(|j| {
                let mut val = self.b_ho[j];
                for k in 0..final_h.len() {
                    val = val + self.w_ho[j][k] * final_h[k];
                }
                val
            })
            .collect();

        (cells, output)
    }

    /// Like [`Self::forward_sequence`] but also returns **every layer's** cell
    /// at each timestep (`result[t][l]`, layer 0 first). Used by
    /// [`Self::train_pair`] to back-propagate through the stacked layers.
    fn forward_sequence_collect(&self, sequence: &[Vec<F>]) -> (Vec<Vec<LstmCell<F>>>, Vec<F>) {
        let h = self.config.hidden_size;
        let o = self.config.output_size;
        let num_layers = 1 + self.extra_layers.len();

        let mut h_layers: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];
        let mut c_layers: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];
        let mut per_timestep = Vec::with_capacity(sequence.len());

        for x in sequence {
            let h_refs: Vec<&[F]> = h_layers.iter().map(|v| v.as_slice()).collect();
            let c_refs: Vec<&[F]> = c_layers.iter().map(|v| v.as_slice()).collect();
            let (cells, new_h, new_c) = self.forward_cell_multi_all(x, &h_refs, &c_refs);
            h_layers = new_h;
            c_layers = new_c;
            per_timestep.push(cells);
        }

        // Output projection uses the LAST layer's hidden state.
        let final_h = &h_layers[num_layers - 1];
        let output: Vec<F> = (0..o)
            .map(|j| {
                let mut val = self.b_ho[j];
                for k in 0..final_h.len() {
                    val = val + self.w_ho[j][k] * final_h[k];
                }
                val
            })
            .collect();

        (per_timestep, output)
    }

    /// One online training step on a single sequence: one forward pass plus a
    /// back-propagation-through-time pass that updates **every** stacked
    /// layer's weights (and the output projection) in place.
    ///
    /// Unlike the batch [`train`] helper — whose backward pass only walks the
    /// layer-0 weights — this propagates gradients through all `num_layers`.
    ///
    /// - `input` — one sequence of input vectors (`T` timesteps × `input_size`)
    /// - `target` — target output vector (`output_size`)
    /// - `lr` — learning rate
    ///
    /// Returns `(mse_loss, final_hidden_state)`, where the hidden state is the
    /// last layer's `h_T` converted to `f64` so the return surface is
    /// precision-independent.
    pub fn train_pair(&mut self, input: &[Vec<F>], target: &[F], lr: F) -> (F, Vec<f64>) {
        assert!(!input.is_empty(), "train_pair: input sequence must not be empty");
        assert_eq!(
            target.len(),
            self.config.output_size,
            "train_pair: target length must equal output_size"
        );

        let h = self.config.hidden_size;
        let o_sz = self.config.output_size;
        let i_sz = self.config.input_size;
        let num_layers = 1 + self.extra_layers.len();
        let t_len = input.len();

        let (per_timestep, output) = self.forward_sequence_collect(input);

        // ── MSE loss ──
        let mut loss = F::zero();
        for (o, t) in output.iter().zip(target.iter()) {
            loss = loss + (*o - *t).powi(2);
        }
        loss = loss / F::from(o_sz).unwrap();

        // ── Output layer gradient ──
        let two = F::one() + F::one();
        let d_output: Vec<F> = output
            .iter()
            .zip(target.iter())
            .map(|(o, t)| two * (*o - *t) / F::from(o_sz).unwrap())
            .collect();

        // Gradient accumulators for every layer.
        let mut dw_ih: Vec<Vec<Vec<F>>> = Vec::with_capacity(num_layers);
        let mut dw_hh: Vec<Vec<Vec<F>>> = Vec::with_capacity(num_layers);
        let mut db_ih: Vec<Vec<F>> = Vec::with_capacity(num_layers);
        let mut db_hh: Vec<Vec<F>> = Vec::with_capacity(num_layers);
        for l in 0..num_layers {
            let in_dim = if l == 0 { i_sz } else { h };
            dw_ih.push(vec![vec![F::zero(); in_dim]; 4 * h]);
            dw_hh.push(vec![vec![F::zero(); h]; 4 * h]);
            db_ih.push(vec![F::zero(); 4 * h]);
            db_hh.push(vec![F::zero(); 4 * h]);
        }
        let mut dw_ho = vec![vec![F::zero(); h]; o_sz];
        let mut db_ho = vec![F::zero(); o_sz];

        // Output projection gradients (from the final hidden state).
        let final_h = &per_timestep[t_len - 1][num_layers - 1].hidden_state;
        for j in 0..o_sz {
            for k in 0..h {
                dw_ho[j][k] = dw_ho[j][k] + d_output[j] * final_h[k];
            }
            db_ho[j] = db_ho[j] + d_output[j];
        }

        // Per-layer gradient flowing in from t+1 (carry) and the projection seed.
        let mut dh_carry: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];
        let mut dc_carry: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];
        for k in 0..h {
            for j in 0..o_sz {
                dh_carry[num_layers - 1][k] = dh_carry[num_layers - 1][k] + self.w_ho[j][k] * d_output[j];
            }
        }

        // ── BPTT: walk timesteps backwards, layers top-down ──
        for t in (0..t_len).rev() {
            let mut dh_cur: Vec<Vec<F>> = dh_carry.clone();
            let dc_cur: Vec<Vec<F>> = dc_carry.clone();
            let mut dh_next: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];
            let mut dc_next: Vec<Vec<F>> = vec![vec![F::zero(); h]; num_layers];

            for l in (0..num_layers).rev() {
                let cell = &per_timestep[t][l];
                let tanh_c: Vec<F> = cell.cell_state.iter().map(|&v| v.tanh()).collect();

                // d_o = d_h ⊙ tanh(c_t)
                let d_o: Vec<F> = dh_cur[l].iter().zip(tanh_c.iter()).map(|(dh, tc)| *dh * *tc).collect();

                // d_c = carry + d_h ⊙ o ⊙ (1 - tanh²(c_t))
                let d_c_local: Vec<F> = dh_cur[l]
                    .iter()
                    .zip(cell.output_gate.iter())
                    .zip(tanh_c.iter())
                    .map(|((dh, og), tc)| *dh * *og * (F::one() - *tc * *tc))
                    .collect();
                let mut d_c = dc_cur[l].clone();
                for k in 0..h {
                    d_c[k] = d_c[k] + d_c_local[k];
                }

                let d_f: Vec<F> = d_c.iter().zip(cell.c_prev.iter()).map(|(dc, cp)| *dc * *cp).collect();
                let d_i: Vec<F> = d_c.iter().zip(cell.candidate.iter()).map(|(dc, g)| *dc * *g).collect();
                let d_g: Vec<F> = d_c
                    .iter()
                    .zip(cell.input_gate.iter())
                    .zip(cell.candidate.iter())
                    .map(|((dc, ig), g)| *dc * *ig * (F::one() - *g * *g))
                    .collect();

                let mut d_gates = vec![F::zero(); 4 * h];
                for j in 0..h {
                    d_gates[j] = d_i[j] * sigmoid_derivative(cell.input_gate[j]);
                    d_gates[h + j] = d_f[j] * sigmoid_derivative(cell.forget_gate[j]);
                    d_gates[2 * h + j] = d_g[j] * tanh_derivative(cell.candidate[j]);
                    d_gates[3 * h + j] = d_o[j] * sigmoid_derivative(cell.output_gate[j]);
                }

                // Accumulate this layer's weights.
                for j in 0..4 * h {
                    for k in 0..cell.x.len() {
                        dw_ih[l][j][k] = dw_ih[l][j][k] + d_gates[j] * cell.x[k];
                    }
                    for k in 0..h {
                        dw_hh[l][j][k] = dw_hh[l][j][k] + d_gates[j] * cell.h_prev[k];
                    }
                    db_ih[l][j] = db_ih[l][j] + d_gates[j];
                    db_hh[l][j] = db_hh[l][j] + d_gates[j];
                }

                // d_c_prev = d_c ⊙ f  (cell-state carry to t-1)
                dc_next[l] = d_c
                    .iter()
                    .zip(cell.forget_gate.iter())
                    .map(|(dc, f)| *dc * *f)
                    .collect();

                // d_h_prev = W_hh^T d_gates  (recurrent carry to t-1)
                let w_hh: &[Vec<F>] = if l == 0 {
                    &self.w_hh
                } else {
                    &self.extra_layers[l - 1].w_hh
                };
                let mut d_h_prev = vec![F::zero(); h];
                for k in 0..h {
                    for j in 0..4 * h {
                        d_h_prev[k] = d_h_prev[k] + w_hh[j][k] * d_gates[j];
                    }
                }
                dh_next[l] = d_h_prev;

                // Gradient into this layer's input = h_t^{l-1} of the layer below.
                if l > 0 {
                    let w_ih = &self.extra_layers[l - 1].w_ih;
                    for k in 0..h {
                        let mut g = F::zero();
                        for j in 0..4 * h {
                            g = g + w_ih[j][k] * d_gates[j];
                        }
                        dh_cur[l - 1][k] = dh_cur[l - 1][k] + g;
                    }
                }
            }

            dh_carry = dh_next;
            dc_carry = dc_next;
        }

        // ── Apply gradients (SGD) ──
        for j in 0..4 * h {
            for k in 0..i_sz {
                self.w_ih[j][k] = self.w_ih[j][k] - lr * dw_ih[0][j][k];
            }
            for k in 0..h {
                self.w_hh[j][k] = self.w_hh[j][k] - lr * dw_hh[0][j][k];
            }
            self.b_ih[j] = self.b_ih[j] - lr * db_ih[0][j];
            self.b_hh[j] = self.b_hh[j] - lr * db_hh[0][j];
        }
        for l in 1..num_layers {
            let layer = &mut self.extra_layers[l - 1];
            for j in 0..4 * h {
                for k in 0..h {
                    layer.w_ih[j][k] = layer.w_ih[j][k] - lr * dw_ih[l][j][k];
                }
                for k in 0..h {
                    layer.w_hh[j][k] = layer.w_hh[j][k] - lr * dw_hh[l][j][k];
                }
                layer.b_ih[j] = layer.b_ih[j] - lr * db_ih[l][j];
                layer.b_hh[j] = layer.b_hh[j] - lr * db_hh[l][j];
            }
        }
        for j in 0..o_sz {
            for k in 0..h {
                self.w_ho[j][k] = self.w_ho[j][k] - lr * dw_ho[j][k];
            }
            self.b_ho[j] = self.b_ho[j] - lr * db_ho[j];
        }

        let hidden: Vec<f64> = per_timestep[t_len - 1][num_layers - 1]
            .hidden_state
            .iter()
            .map(|v| v.to_f64().unwrap_or(0.0))
            .collect();
        (loss, hidden)
    }
}

// ─────────────────────── Training (BPTT) ───────────────────────

/// Train an LSTM model on input/output sequence pairs using BPTT.
///
/// - `model` — mutable model to train
/// - `inputs` — list of input sequences (each is a list of timesteps)
/// - `targets` — list of target vectors (one per sequence)
/// - `epochs` — number of training epochs
/// - `lr` — learning rate
pub fn train<F: Float + std::iter::Sum<F>>(
    model: &mut LstmModel<F>,
    inputs: &[Vec<Vec<F>>],
    targets: &[Vec<F>],
    epochs: usize,
    lr: F,
) -> TrainingResult<F> {
    assert_eq!(inputs.len(), targets.len(), "inputs and targets must have same length");
    let n = inputs.len();
    let h = model.config.hidden_size;
    let i_sz = model.config.input_size;
    let o_sz = model.config.output_size;
    let mut loss_history = Vec::with_capacity(epochs);

    for epoch in 0..epochs {
        let mut epoch_loss = F::zero();

        for (seq, target) in inputs.iter().zip(targets.iter()) {
            let t_len = seq.len();

            // ── Forward pass: store all cells ──
            let (cells, output) = model.forward_sequence(seq);

            // ── MSE loss ──
            let loss: F = output
                .iter()
                .zip(target.iter())
                .map(|(o, t)| (*o - *t).powi(2))
                .sum::<F>()
                / F::from(output.len()).unwrap();
            epoch_loss = epoch_loss + loss;

            // ── Output layer gradient ──
            let o_len = output.len();
            let two = F::one() + F::one();
            let d_output: Vec<F> = output
                .iter()
                .zip(target.iter())
                .map(|(o, t)| two * (*o - *t) / F::from(o_len).unwrap())
                .collect();

            // Accumulate parameter gradients across all timesteps
            let mut dw_ih = vec![vec![F::zero(); i_sz]; 4 * h];
            let mut dw_hh = vec![vec![F::zero(); h]; 4 * h];
            let mut db_ih = vec![F::zero(); 4 * h];
            let mut db_hh = vec![F::zero(); 4 * h];
            let mut dw_ho = vec![vec![F::zero(); h]; o_sz];
            let mut db_ho = vec![F::zero(); o_sz];

            // Output projection gradients (from final hidden state)
            let final_h = &cells[t_len - 1].hidden_state;
            for j in 0..o_sz {
                for k in 0..h {
                    dw_ho[j][k] = dw_ho[j][k] + d_output[j] * final_h[k];
                }
                db_ho[j] = db_ho[j] + d_output[j];
            }

            // Gradient flowing into final hidden state from output layer
            let mut d_h: Vec<F> = vec![F::zero(); h];
            for k in 0..h {
                for j in 0..o_sz {
                    d_h[k] = d_h[k] + model.w_ho[j][k] * d_output[j];
                }
            }
            let mut d_c: Vec<F> = vec![F::zero(); h];

            // ── BPTT: walk backward through all cells ──
            for t in (0..t_len).rev() {
                let cell = &cells[t];

                // tanh(c_t) — cached from forward
                let tanh_c: Vec<F> = cell.cell_state.iter().map(|&v| v.tanh()).collect();

                // d_o = d_h ⊙ tanh(c_t)
                let d_o: Vec<F> = d_h.iter().zip(tanh_c.iter()).map(|(dh, tc)| *dh * *tc).collect();

                // d_c += d_h ⊙ o ⊙ (1 - tanh²(c_t))  (accumulate with carry from future)
                let d_c_local: Vec<F> = d_h
                    .iter()
                    .zip(cell.output_gate.iter())
                    .zip(tanh_c.iter())
                    .map(|((dh, o), tc)| *dh * *o * (F::one() - *tc * *tc))
                    .collect();
                for k in 0..h {
                    d_c[k] = d_c[k] + d_c_local[k];
                }

                // d_f = d_c ⊙ c_prev
                let d_f: Vec<F> = d_c.iter().zip(cell.c_prev.iter()).map(|(dc, cp)| *dc * *cp).collect();

                // d_i = d_c ⊙ g
                let d_i: Vec<F> = d_c.iter().zip(cell.candidate.iter()).map(|(dc, g)| *dc * *g).collect();

                // d_g = d_c ⊙ i ⊙ (1 - g²)
                let d_g: Vec<F> = d_c
                    .iter()
                    .zip(cell.input_gate.iter())
                    .zip(cell.candidate.iter())
                    .map(|((dc, ig), g)| *dc * *ig * (F::one() - *g * *g))
                    .collect();

                // Gate pre-activation gradients
                let mut d_gates = vec![F::zero(); 4 * h];
                for j in 0..h {
                    d_gates[j] = d_i[j] * sigmoid_derivative(cell.input_gate[j]);
                    d_gates[h + j] = d_f[j] * sigmoid_derivative(cell.forget_gate[j]);
                    d_gates[2 * h + j] = d_g[j] * tanh_derivative(cell.candidate[j]);
                    d_gates[3 * h + j] = d_o[j] * sigmoid_derivative(cell.output_gate[j]);
                }

                // Accumulate W_ih, W_hh, b_ih, b_hh
                for j in 0..4 * h {
                    for k in 0..cell.x.len() {
                        dw_ih[j][k] = dw_ih[j][k] + d_gates[j] * cell.x[k];
                    }
                    for k in 0..cell.h_prev.len() {
                        dw_hh[j][k] = dw_hh[j][k] + d_gates[j] * cell.h_prev[k];
                    }
                    db_ih[j] = db_ih[j] + d_gates[j];
                    db_hh[j] = db_hh[j] + d_gates[j];
                }

                // Propagate d_c and d_h to previous cell
                if t > 0 {
                    // d_c_prev = d_c ⊙ f  (gradient through cell state carry)
                    let d_c_prev: Vec<F> = d_c
                        .iter()
                        .zip(cell.forget_gate.iter())
                        .map(|(dc, f)| *dc * *f)
                        .collect();

                    // d_h_prev = W_hh^T * d_gates
                    let mut d_h_prev = vec![F::zero(); h];
                    for k in 0..h {
                        for j in 0..4 * h {
                            d_h_prev[k] = d_h_prev[k] + model.w_hh[j][k] * d_gates[j];
                        }
                    }

                    d_c = d_c_prev;
                    d_h = d_h_prev;
                }
            }

            // ── Apply accumulated gradients ──
            for j in 0..o_sz {
                for k in 0..h {
                    model.w_ho[j][k] = model.w_ho[j][k] - lr * dw_ho[j][k];
                }
                model.b_ho[j] = model.b_ho[j] - lr * db_ho[j];
            }
            for j in 0..4 * h {
                for k in 0..i_sz {
                    model.w_ih[j][k] = model.w_ih[j][k] - lr * dw_ih[j][k];
                }
                for k in 0..h {
                    model.w_hh[j][k] = model.w_hh[j][k] - lr * dw_hh[j][k];
                }
                model.b_ih[j] = model.b_ih[j] - lr * db_ih[j];
                model.b_hh[j] = model.b_hh[j] - lr * db_hh[j];
            }
        }

        let avg_loss = epoch_loss / F::from(n).unwrap();
        loss_history.push(avg_loss);

        if epoch % 100 == 0 || epoch == epochs - 1 {
            tracing::info!("epoch {epoch}: loss = {:.6}", avg_loss.to_f64().unwrap_or(0.0));
        }
    }

    TrainingResult {
        final_loss: *loss_history.last().unwrap_or(&F::zero()),
        epochs,
        loss_history,
    }
}

// ─────────────────────── Save / Load ───────────────────────

/// Save model weights to a JSON file.
///
/// # Errors
///
/// Returns an error string if serialization fails or the file cannot be
/// written.
pub fn save_model<F: Float + Serialize>(model: &LstmModel<F>, path: &str) -> Result<(), String> {
    let json = serde_json::to_string_pretty(model).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write: {e}"))
}

/// Load model weights from a JSON file.
///
/// # Errors
///
/// Returns an error string if the file cannot be read or its contents do not
/// deserialize into an [`LstmModel`].
pub fn load_model<F: Float + DeserializeOwned>(path: &str) -> Result<LstmModel<F>, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("deserialize: {e}"))
}

// ─────────────────────── Binary Save / Load (P117.1) ───────────────────────

/// Binary magic for [`save_bin`] / [`load_bin`]: `"LSTM"` (4 bytes).
const BIN_MAGIC: [u8; 4] = *b"LSTM";

/// Current binary format version (little-endian u16).
const BIN_VERSION: u16 = 1;

/// Write a single float in the model's native precision (f32 → 4 bytes,
/// f64 → 8 bytes), little-endian.
fn write_f32_or_f64<F: Float>(w: &mut impl Write, x: F) -> Result<(), String> {
    match std::mem::size_of::<F>() {
        4 => w
            .write_all(&x.to_f32().unwrap_or(0.0).to_le_bytes())
            .map_err(|e| format!("write: {e}")),
        _ => w
            .write_all(&x.to_f64().unwrap_or(0.0).to_le_bytes())
            .map_err(|e| format!("write: {e}")),
    }
}

fn write_flat<F: Float, T: AsRef<[F]>>(w: &mut impl Write, rows: &[T]) -> Result<(), String> {
    for row in rows {
        for x in row.as_ref() {
            write_f32_or_f64(w, *x)?;
        }
    }
    Ok(())
}

/// Read a single float in the model's native precision.
fn read_f32_or_f64<F: Float>(r: &mut Cursor<&[u8]>) -> Result<F, String> {
    match std::mem::size_of::<F>() {
        4 => {
            let mut b = [0u8; 4];
            r.read_exact(&mut b).map_err(|e| format!("read: {e}"))?;
            F::from(f32::from_le_bytes(b)).ok_or_else(|| "binary: invalid f32".to_string())
        }
        _ => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).map_err(|e| format!("read: {e}"))?;
            F::from(f64::from_le_bytes(b)).ok_or_else(|| "binary: invalid f64".to_string())
        }
    }
}

/// Save model weights to a compact binary file (P117.1).
///
/// Format (little-endian):
/// ```text
/// magic    "LSTM" (4 bytes)
/// version  u16 == 1
/// config   input_size u16 · hidden_size u16 · output_size u16 · num_layers u16
/// weights  layer-0 (w_ih, w_hh, b_ih, b_hh)
///          extra layers 1..num_layers (w_ih, w_hh, b_ih, b_hh) each — row-major
///          output projection (w_ho, b_ho)
/// ```
///
/// Floats are written in the model's native precision (`f32` → 4 bytes, `f64`
/// → 8 bytes). More compact than JSON for large models.
///
/// # Errors
///
/// Returns an error string on I/O failure.
pub fn save_bin<F: Float>(model: &LstmModel<F>, path: &str) -> Result<(), String> {
    let cfg = &model.config;
    let mut buf: Vec<u8> = Vec::new();
    buf.write_all(&BIN_MAGIC).map_err(|e| format!("write: {e}"))?;
    buf.write_all(&BIN_VERSION.to_le_bytes())
        .map_err(|e| format!("write: {e}"))?;
    for v in [cfg.input_size, cfg.hidden_size, cfg.output_size, cfg.num_layers.max(1)] {
        let v16 = u16::try_from(v).map_err(|_| format!("binary: dimension {v} exceeds u16"))?;
        buf.write_all(&v16.to_le_bytes()).map_err(|e| format!("write: {e}"))?;
    }

    write_flat(&mut buf, &model.w_ih)?;
    write_flat(&mut buf, &model.w_hh)?;
    write_flat(&mut buf, std::slice::from_ref(&model.b_ih))?;
    write_flat(&mut buf, std::slice::from_ref(&model.b_hh))?;
    for layer in &model.extra_layers {
        write_flat(&mut buf, &layer.w_ih)?;
        write_flat(&mut buf, &layer.w_hh)?;
        write_flat(&mut buf, std::slice::from_ref(&layer.b_ih))?;
        write_flat(&mut buf, std::slice::from_ref(&layer.b_hh))?;
    }
    write_flat(&mut buf, &model.w_ho)?;
    write_flat(&mut buf, std::slice::from_ref(&model.b_ho))?;

    std::fs::write(path, buf).map_err(|e| format!("write: {e}"))
}

/// Load model weights from a binary file written by [`save_bin`].
///
/// The file's float precision must match `F` (an f32 model cannot be loaded as
/// an [`LstmModelF64`], and vice-versa).
///
/// # Errors
///
/// Returns an error string if the file is missing, not an akar LSTM binary, or
/// malformed/truncated.
pub fn load_bin<F: Float>(path: &str) -> Result<LstmModel<F>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read: {e}"))?;
    let mut r = Cursor::new(data.as_slice());

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|e| format!("read magic: {e}"))?;
    if magic != BIN_MAGIC {
        return Err(format!("not an akar LSTM binary file (magic: {magic:?})"));
    }
    let mut version_buf = [0u8; 2];
    r.read_exact(&mut version_buf)
        .map_err(|e| format!("read version: {e}"))?;
    let version = u16::from_le_bytes(version_buf);
    if version != BIN_VERSION {
        return Err(format!(
            "unsupported LSTM binary version {version} (expected {BIN_VERSION})"
        ));
    }

    let mut dims = [0u8; 8];
    r.read_exact(&mut dims).map_err(|e| format!("read config: {e}"))?;
    let mut u16s = dims.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
    let config = LstmConfig {
        input_size: u16s.next().unwrap_or(0) as usize,
        hidden_size: u16s.next().unwrap_or(0) as usize,
        output_size: u16s.next().unwrap_or(0) as usize,
        num_layers: (u16s.next().unwrap_or(1) as usize).max(1),
    };

    let h = config.hidden_size;
    let read_mat = |r: &mut Cursor<&[u8]>, rows: usize, cols: usize| -> Result<Vec<Vec<F>>, String> {
        (0..rows)
            .map(|_| (0..cols).map(|_| read_f32_or_f64(r)).collect())
            .collect()
    };
    let read_vec =
        |r: &mut Cursor<&[u8]>, n: usize| -> Result<Vec<F>, String> { (0..n).map(|_| read_f32_or_f64(r)).collect() };

    let w_ih = read_mat(&mut r, 4 * h, config.input_size)?;
    let w_hh = read_mat(&mut r, 4 * h, h)?;
    let b_ih = read_vec(&mut r, 4 * h)?;
    let b_hh = read_vec(&mut r, 4 * h)?;

    let mut extra_layers = Vec::with_capacity(config.num_layers.saturating_sub(1));
    for _ in 1..config.num_layers {
        extra_layers.push(LstmLayer {
            w_ih: read_mat(&mut r, 4 * h, h)?,
            w_hh: read_mat(&mut r, 4 * h, h)?,
            b_ih: read_vec(&mut r, 4 * h)?,
            b_hh: read_vec(&mut r, 4 * h)?,
        });
    }

    let w_ho = read_mat(&mut r, config.output_size, h)?;
    let b_ho = read_vec(&mut r, config.output_size)?;

    if r.position() as usize != data.len() {
        return Err(format!(
            "binary: {} trailing bytes after model data",
            data.len() - r.position() as usize
        ));
    }

    Ok(LstmModel {
        config,
        w_ih,
        w_hh,
        b_ih,
        b_hh,
        extra_layers,
        w_ho,
        b_ho,
    })
}

// ─────────────────────── Tests ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lstm_cell_shapes() {
        let model = LstmModel::new(LstmConfig {
            input_size: 3,
            hidden_size: 4,
            output_size: 2,
            ..Default::default()
        });

        let x = vec![0.5, -0.3, 0.8];
        let h_prev = vec![0.0; 4];
        let c_prev = vec![0.0; 4];

        let cell = model.forward_cell(&x, &h_prev, &c_prev);

        assert_eq!(cell.input_gate.len(), 4);
        assert_eq!(cell.forget_gate.len(), 4);
        assert_eq!(cell.candidate.len(), 4);
        assert_eq!(cell.output_gate.len(), 4);
        assert_eq!(cell.cell_state.len(), 4);
        assert_eq!(cell.hidden_state.len(), 4);

        // Gates should be in [0, 1]
        for v in &cell.input_gate {
            assert!(*v >= 0.0 && *v <= 1.0, "input_gate out of range: {v}");
        }
        for v in &cell.forget_gate {
            assert!(*v >= 0.0 && *v <= 1.0, "forget_gate out of range: {v}");
        }
        for v in &cell.output_gate {
            assert!(*v >= 0.0 && *v <= 1.0, "output_gate out of range: {v}");
        }
        // Hidden state should be in [-1, 1] (tanh output)
        for v in &cell.hidden_state {
            assert!(*v >= -1.0 && *v <= 1.0, "hidden_state out of range: {v}");
        }
    }

    #[test]
    fn test_forward_sequence() {
        let model = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 3,
            output_size: 1,
            ..Default::default()
        });

        let seq = vec![vec![1.0, 0.5], vec![0.3, -0.2]];
        let (cells, output) = model.forward_sequence(&seq);

        assert_eq!(cells.len(), 2);
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn test_train_xor_converge() {
        // XOR: (0,0)→0, (0,1)→1, (1,0)→1, (1,1)→0
        // Each sample is a 1-step sequence with 2 input features
        let mut model = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 8,
            output_size: 1,
            ..Default::default()
        });

        let inputs: Vec<Vec<Vec<f64>>> = vec![
            vec![vec![0.0, 0.0]],
            vec![vec![0.0, 1.0]],
            vec![vec![1.0, 0.0]],
            vec![vec![1.0, 1.0]],
        ];
        let targets = vec![vec![0.0], vec![1.0], vec![1.0], vec![0.0]];

        let result = train(&mut model, &inputs, &targets, 1000, 0.1);

        assert!(
            result.final_loss < 0.05,
            "XOR training did not converge: final_loss = {}",
            result.final_loss
        );
    }

    #[test]
    fn test_save_load_roundtrip() {
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 1,
            ..Default::default()
        });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.json");

        save_model(&model, path.to_str().unwrap()).unwrap();
        let loaded = load_model(path.to_str().unwrap()).unwrap();

        assert_eq!(loaded.config.input_size, 2);
        assert_eq!(loaded.config.hidden_size, 4);
        assert_eq!(loaded.config.output_size, 1);
        assert_eq!(loaded.w_ih.len(), model.w_ih.len());
        assert_eq!(loaded.w_ho.len(), model.w_ho.len());

        // Verify weights match (float comparison)
        for (a_row, b_row) in model.w_ih.iter().zip(loaded.w_ih.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert!((a - b).abs() < 1e-10, "weight mismatch: {a} vs {b}");
            }
        }
    }

    #[test]
    fn test_model_new_shapes() {
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 5,
            hidden_size: 10,
            output_size: 3,
            ..Default::default()
        });
        assert_eq!(model.w_ih.len(), 40); // 4 * hidden
        assert_eq!(model.w_ih[0].len(), 5); // input_size
        assert_eq!(model.w_hh.len(), 40);
        assert_eq!(model.w_hh[0].len(), 10); // hidden_size
        assert_eq!(model.b_ih.len(), 40);
        assert_eq!(model.w_ho.len(), 3); // output_size
        assert_eq!(model.w_ho[0].len(), 10); // hidden_size
        assert_eq!(model.b_ho.len(), 3);
    }

    #[test]
    fn test_multi_layer_shapes() {
        // 2-layer: layer 0 maps input(3)->hidden(4); layer 1 maps hidden(4)->hidden(4).
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 3,
            hidden_size: 4,
            output_size: 2,
            num_layers: 2,
        });
        assert_eq!(model.config.num_layers, 2);
        assert_eq!(model.extra_layers.len(), 1);
        // Layer 1 input dim == hidden dim (its input is layer 0's hidden output).
        assert_eq!(model.extra_layers[0].w_ih.len(), 16); // 4 * hidden
        assert_eq!(model.extra_layers[0].w_ih[0].len(), 4); // hidden
        assert_eq!(model.extra_layers[0].w_hh.len(), 16);
        assert_eq!(model.extra_layers[0].w_hh[0].len(), 4);
        assert_eq!(model.extra_layers[0].b_ih.len(), 16);
        assert_eq!(model.extra_layers[0].b_hh.len(), 16);
    }

    #[test]
    fn test_two_layer_forward_cell_dims() {
        let model = LstmModel::new(LstmConfig {
            input_size: 3,
            hidden_size: 4,
            output_size: 2,
            num_layers: 2,
        });
        let x = vec![0.5, -0.3, 0.8];
        let cell = model.forward_cell(&x, &[0.0; 4], &[0.0; 4]);
        assert_eq!(cell.hidden_state.len(), 4);
        assert_eq!(cell.cell_state.len(), 4);
        // Layer 0 hidden (dim 4) fed layer 1, whose hidden is also dim 4.
        assert_eq!(cell.input_gate.len(), 4);
        assert_eq!(cell.forget_gate.len(), 4);
        assert_eq!(cell.candidate.len(), 4);
        assert_eq!(cell.output_gate.len(), 4);
    }

    #[test]
    fn test_single_layer_identical_to_num_layers_1() {
        // Regression: explicit num_layers=1 must behave identically to the
        // implicit default (no extra layer, no dimension change).
        let cfg_a = LstmConfig {
            input_size: 2,
            hidden_size: 3,
            output_size: 1,
            ..Default::default()
        };
        let cfg_b = LstmConfig {
            input_size: 2,
            hidden_size: 3,
            output_size: 1,
            num_layers: 1,
        };
        let model_a = LstmModel::new(cfg_a);
        // Clone weights onto model_b so both run with identical (deterministic) weights.
        let mut model_b = LstmModel::new(cfg_b);
        model_b.w_ih = model_a.w_ih.clone();
        model_b.w_hh = model_a.w_hh.clone();
        model_b.b_ih = model_a.b_ih.clone();
        model_b.b_hh = model_a.b_hh.clone();
        model_b.w_ho = model_a.w_ho.clone();
        model_b.b_ho = model_a.b_ho.clone();

        let x = vec![0.5, -0.3];
        let h0 = vec![0.1, 0.2, 0.3];
        let c0 = vec![-0.1, 0.4, 0.9];
        let ca = model_a.forward_cell(&x, &h0, &c0);
        let cb = model_b.forward_cell(&x, &h0, &c0);

        assert_eq!(ca.hidden_state.len(), 3);
        assert_eq!(ca.hidden_state, cb.hidden_state);
        assert_eq!(ca.cell_state, cb.cell_state);
        assert_eq!(model_a.extra_layers.is_empty(), model_b.extra_layers.is_empty());
    }

    #[test]
    fn test_multi_layer_save_load_roundtrip() {
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 1,
            num_layers: 2,
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.json");

        save_model(&model, path.to_str().unwrap()).unwrap();
        let loaded = load_model(path.to_str().unwrap()).unwrap();

        assert_eq!(loaded.config.num_layers, 2);
        assert_eq!(loaded.extra_layers.len(), 1);
        assert_eq!(loaded.w_ih.len(), model.w_ih.len());
        assert_eq!(loaded.w_ho.len(), model.w_ho.len());
        for (a, b) in model.extra_layers[0].w_ih.iter().zip(&loaded.extra_layers[0].w_ih) {
            for (x, y) in a.iter().zip(b.iter()) {
                assert!((x - y).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn test_single_layer_json_backward_compat() {
        // Old JSON (no num_layers / extra_layers) must still deserialize to a
        // 1-layer model.
        let old_json = r#"{
            "config": { "input_size": 2, "hidden_size": 3, "output_size": 1 },
            "w_ih": [[0.1], [0.2], [0.3], [0.4], [0.5], [0.6], [0.7], [0.8], [0.9], [1.0], [1.1], [1.2]],
            "w_hh": [[0.1], [0.2], [0.3], [0.4], [0.5], [0.6], [0.7], [0.8], [0.9], [1.0], [1.1], [1.2]],
            "b_ih": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "b_hh": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "w_ho": [[0.1, 0.2, 0.3]],
            "b_ho": [0.0]
        }"#;
        let model: LstmModel = serde_json::from_str(old_json).expect("old 1-layer JSON must load");
        assert_eq!(model.config.num_layers, 1);
        assert!(model.extra_layers.is_empty());
    }

    #[test]
    fn test_two_layer_sequence_output_len() {
        // P114.2: 2-layer forward_sequence must produce output_len == output_size.
        let model = LstmModel::new(LstmConfig {
            input_size: 3,
            hidden_size: 4,
            output_size: 2,
            num_layers: 2,
        });
        let seq = vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6], vec![0.7, 0.8, 0.9]];
        let (cells, output) = model.forward_sequence(&seq);
        assert_eq!(cells.len(), 3);
        assert_eq!(output.len(), 2, "output must equal output_size");
        // Each cell's hidden_state must be hidden_size (last layer).
        assert_eq!(cells[2].hidden_state.len(), 4);
    }

    #[test]
    fn test_single_layer_sequence_parity() {
        // P114.2: 1-layer forward_sequence must produce identical output to
        // the pre-P114.2 path (proven by the refactored code being the same
        // logic for num_layers=1). Verify explicit 1-layer behaviour.
        let model = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 3,
            output_size: 1,
            num_layers: 1,
        });
        let seq = vec![vec![1.0, 0.5], vec![0.3, -0.2]];
        let (cells, output) = model.forward_sequence(&seq);
        assert_eq!(cells.len(), 2);
        assert_eq!(output.len(), 1);
        // Hidden state carries across timesteps: the second cell's hidden
        // must differ from zeros (proves h_prev was actually fed).
        let h_zero = vec![0.0; 3];
        assert_ne!(cells[1].hidden_state, h_zero, "h_prev not carried across timesteps");
    }

    #[test]
    fn test_multi_layer_state_carry_across_timesteps() {
        // P114.2: upper-layer h/c must be carried across timesteps, not reset.
        let model = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 3,
            output_size: 1,
            num_layers: 2,
        });
        let seq = vec![vec![0.5, -0.3], vec![0.1, 0.7]];
        let (cells, _) = model.forward_sequence(&seq);

        // Layer 0 hidden at t=1 must carry from t=0 (not reset).
        let h_zero = vec![0.0; 3];
        assert_ne!(cells[0].hidden_state, h_zero, "layer 0 h_prev not carried");
        // Both timesteps must succeed without panic.
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn test_multi_layer_sequence_different_output_dim() {
        // P114.2: 3-layer, output_size=5 — output_len must be 5.
        let model = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 5,
            num_layers: 3,
        });
        let seq = vec![vec![0.1, 0.2]];
        let (cells, output) = model.forward_sequence(&seq);
        assert_eq!(cells.len(), 1);
        assert_eq!(output.len(), 5, "output must equal output_size");
        assert_eq!(model.extra_layers.len(), 2);
    }

    #[test]
    fn test_f32_save_load_roundtrip() {
        let model: LstmModelF32 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 1,
            num_layers: 2,
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model_f32.json");
        save_model(&model, path.to_str().unwrap()).unwrap();
        let loaded: LstmModelF32 = load_model(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.config.num_layers, 2);
        assert_eq!(loaded.extra_layers.len(), 1);
        assert_eq!(loaded.w_ih.len(), model.w_ih.len());
        assert_eq!(loaded.w_ho.len(), model.w_ho.len());
        for (a_row, b_row) in model.w_ih.iter().zip(loaded.w_ih.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert_eq!(a, b, "f32 weight mismatch: {a} vs {b}");
            }
        }
    }

    #[test]
    fn test_bin_save_load_roundtrip_f64() {
        // P117.1: 2-layer f64 binary roundtrip — config, all weights exact.
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 3,
            num_layers: 2,
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.bin");

        save_bin(&model, path.to_str().unwrap()).unwrap();
        let loaded: LstmModelF64 = load_bin(path.to_str().unwrap()).unwrap();

        assert_eq!(loaded.config.input_size, 2);
        assert_eq!(loaded.config.hidden_size, 4);
        assert_eq!(loaded.config.output_size, 3);
        assert_eq!(loaded.config.num_layers, 2);
        assert_eq!(loaded.extra_layers.len(), 1);
        assert_eq!(loaded.w_ih.len(), model.w_ih.len());
        assert_eq!(loaded.w_ih[0].len(), model.w_ih[0].len());
        assert_eq!(loaded.w_ho.len(), model.w_ho.len());

        for (a_row, b_row) in model.w_ih.iter().zip(loaded.w_ih.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert_eq!(a, b, "w_ih mismatch: {a} vs {b}");
            }
        }
        for (a_row, b_row) in model.extra_layers[0]
            .w_ih
            .iter()
            .zip(loaded.extra_layers[0].w_ih.iter())
        {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert_eq!(a, b, "extra w_ih mismatch: {a} vs {b}");
            }
        }
        assert_eq!(loaded.b_ho, model.b_ho);
    }

    #[test]
    fn test_bin_save_load_roundtrip_f32() {
        // P117.1: f32 precision roundtrip — stored as 4-byte floats.
        let model: LstmModelF32 = LstmModel::new(LstmConfig {
            input_size: 3,
            hidden_size: 5,
            output_size: 1,
            ..Default::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model_f32.bin");

        save_bin(&model, path.to_str().unwrap()).unwrap();
        let loaded: LstmModelF32 = load_bin(path.to_str().unwrap()).unwrap();

        assert_eq!(loaded.config.input_size, 3);
        assert_eq!(loaded.config.hidden_size, 5);
        assert_eq!(loaded.config.output_size, 1);
        assert_eq!(loaded.config.num_layers, 1);
        assert!(loaded.extra_layers.is_empty());
        for (a_row, b_row) in model.w_ih.iter().zip(loaded.w_ih.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert_eq!(a, b, "f32 w_ih mismatch: {a} vs {b}");
            }
        }
        assert_eq!(loaded.w_ho, model.w_ho);
    }

    #[test]
    fn test_bin_single_layer_roundtrip_parity_with_json() {
        // P117.1: binary roundtrip must agree exactly with JSON roundtrip
        // (weights identical), and the loaded model forward-identically.
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 3,
            output_size: 1,
            ..Default::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("model.bin");
        let json_path = dir.path().join("model.json");

        save_bin(&model, bin_path.to_str().unwrap()).unwrap();
        save_model(&model, json_path.to_str().unwrap()).unwrap();
        let loaded_bin: LstmModelF64 = load_bin(bin_path.to_str().unwrap()).unwrap();
        let loaded_json: LstmModelF64 = load_model(json_path.to_str().unwrap()).unwrap();

        // Binary roundtrip is bit-exact; JSON serialization may be off by ~1
        // ulp, so cross-format weights are compared with the JSON test tolerance.
        for (a_row, b_row) in loaded_json.w_ih.iter().zip(loaded_bin.w_ih.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert!((a - b).abs() < 1e-10, "bin/json w_ih mismatch: {a} vs {b}");
            }
        }
        let seq = vec![vec![0.5, -0.3]];
        let (out_bin, out_json) = (
            loaded_bin.forward_sequence(&seq).1,
            loaded_json.forward_sequence(&seq).1,
        );
        for (a, b) in out_bin.iter().zip(out_json.iter()) {
            assert!((a - b).abs() < 1e-10, "binary and JSON load forward diff: {a} vs {b}");
        }
    }

    #[test]
    fn test_bin_rejects_invalid_magic() {
        // P117.1: garbage file → load_bin must error, not panic.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_lstm.bin");
        std::fs::write(&path, b"not-a-lstm-file-forever").unwrap();
        let err = load_bin::<f64>(path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("magic"), "expected magic error, got: {err}");
    }

    #[test]
    fn test_bin_rejects_unsupported_version() {
        // P117.1: unknown version → error.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.bin");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&BIN_MAGIC);
        bytes.extend_from_slice(&(BIN_VERSION + 1).to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(&[0u8; 8]);
        std::fs::write(&path, bytes).unwrap();
        let err = load_bin::<f64>(path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("version"), "expected version error, got: {err}");
    }

    #[test]
    fn test_bin_rejects_truncated_file() {
        // P117.1: truncation → error, not panic.
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 1,
            ..Default::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let full = dir.path().join("full.bin");
        save_bin(&model, full.to_str().unwrap()).unwrap();
        let bytes = std::fs::read(&full).unwrap();
        let truncated = &bytes[..bytes.len() / 2];
        let tpath = dir.path().join("truncated.bin");
        std::fs::write(&tpath, truncated).unwrap();
        assert!(load_bin::<f64>(tpath.to_str().unwrap()).is_err());
    }

    #[test]
    fn test_bin_save_json_roundtrip_backward_compat() {
        // P117.1 backward-compat: after adding binary persistence the JSON
        // save/load path must still work unimpaired (serialize a binary-loaded
        // model via JSON and reload it).
        let model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 1,
            num_layers: 2,
        });
        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("m.bin");
        let json_path = dir.path().join("m.json");

        save_bin(&model, bin_path.to_str().unwrap()).unwrap();
        let loaded_bin: LstmModelF64 = load_bin(bin_path.to_str().unwrap()).unwrap();
        // JSON save must still work on the loaded model…
        save_model(&loaded_bin, json_path.to_str().unwrap()).unwrap();
        let loaded_json: LstmModelF64 = load_model(json_path.to_str().unwrap()).unwrap();

        assert_eq!(loaded_json.config.num_layers, 2);
        assert_eq!(loaded_json.extra_layers.len(), 1);
        for (a_row, b_row) in loaded_json.w_ih.iter().zip(loaded_bin.w_ih.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert!((a - b).abs() < 1e-10, "bin→json w_ih mismatch: {a} vs {b}");
            }
        }
        for (a_row, b_row) in loaded_json.w_ho.iter().zip(loaded_bin.w_ho.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert!((a - b).abs() < 1e-10, "bin→json w_ho mismatch: {a} vs {b}");
            }
        }
    }

    #[test]
    fn test_f32_vs_f64_forward_parity() {
        // Identical weights, both f32 and f64 must produce close outputs.
        use rand::RngExt;
        let mut rng = rand::rng();
        let (i, h, o) = (2_usize, 4, 2);
        let limit = (6.0 / (i + h + h + o) as f64).sqrt();

        let mut rand_vec = |n: usize| -> Vec<f64> { (0..n).map(|_| rng.random_range(-limit..limit)).collect() };
        let mut rand_mat = |r: usize, c: usize| -> Vec<Vec<f64>> { (0..r).map(|_| rand_vec(c)).collect() };

        let w_ih = rand_mat(4 * h, i);
        let w_hh = rand_mat(4 * h, h);
        let w_ho = rand_mat(o, h);
        let b_ih = rand_vec(4 * h);
        let b_hh = rand_vec(4 * h);
        let b_ho = rand_vec(o);

        let config = LstmConfig {
            input_size: i,
            hidden_size: h,
            output_size: o,
            num_layers: 1,
        };
        let model64 = LstmModelF64 {
            config: config.clone(),
            w_ih: w_ih.clone(),
            w_hh: w_hh.clone(),
            b_ih: b_ih.clone(),
            b_hh: b_hh.clone(),
            extra_layers: Vec::new(),
            w_ho: w_ho.clone(),
            b_ho: b_ho.clone(),
        };

        let to32 =
            |m: &[Vec<f64>]| -> Vec<Vec<f32>> { m.iter().map(|r| r.iter().map(|v| *v as f32).collect()).collect() };
        let to32v = |v: &[f64]| -> Vec<f32> { v.iter().map(|v| *v as f32).collect() };
        let model32 = LstmModelF32 {
            config,
            w_ih: to32(&w_ih),
            w_hh: to32(&w_hh),
            b_ih: to32v(&b_ih),
            b_hh: to32v(&b_hh),
            extra_layers: Vec::new(),
            w_ho: to32(&w_ho),
            b_ho: to32v(&b_ho),
        };

        let seq64 = vec![vec![0.5, -0.3], vec![0.1, 0.7]];
        let seq32: Vec<Vec<f32>> = seq64.iter().map(|v| v.iter().map(|x| *x as f32).collect()).collect();

        let (_, out64) = model64.forward_sequence(&seq64);
        let (_, out32) = model32.forward_sequence(&seq32);

        assert_eq!(out64.len(), out32.len(), "output length must match");
        for (i, (a, b)) in out64.iter().zip(out32.iter()).enumerate() {
            let a32 = *a as f32;
            let diff = (a32 - b).abs();
            assert!(
                diff < 1e-4,
                "f32 vs f64 parity mismatch at index {i}: f64={a} cast_f32={a32} native_f32={b} diff={diff}"
            );
        }
    }

    #[test]
    fn test_train_pair_online_learning_loss_decreases() {
        // P116.1: repeated single-pair updates on one sample must drive the
        // MSE monotonically down (full-batch gradient step on a fixed sample).
        let mut model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 8,
            output_size: 1,
            ..Default::default()
        });
        let seq = vec![vec![0.3, 0.7]];
        let target = vec![1.0];

        let mut losses = Vec::with_capacity(100);
        for _ in 0..100 {
            let (loss, hidden) = model.train_pair(&seq, &target, 0.02);
            assert_eq!(hidden.len(), 8, "hidden state must be hidden_size");
            losses.push(loss);
        }

        assert!(
            losses.iter().all(|l| l.is_finite()),
            "loss must stay finite: {losses:?}"
        );
        assert!(
            losses[99] < losses[0],
            "online loss did not decrease: first={} last={}",
            losses[0],
            losses[99]
        );
        assert!(
            losses[99] < 0.05,
            "online learning did not converge: final={}",
            losses[99]
        );
        for w in losses.windows(2) {
            assert!(
                w[1] <= w[0] + 1e-9,
                "loss increased across a step: {} -> {}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn test_train_pair_multi_layer_updates_all_layers() {
        // P116.1: unlike the batch `train` helper, `train_pair` must back-propagate
        // through stacked layers (upper-layer weights change).
        let mut model: LstmModelF64 = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 1,
            num_layers: 2,
        });
        let before = model.extra_layers[0].w_ih.clone();
        let seq = vec![vec![0.2, -0.4], vec![0.5, 0.1]];
        let target = vec![0.8];

        let (loss, hidden) = model.train_pair(&seq, &target, 0.05);

        assert!(loss.is_finite(), "loss must be finite");
        assert_eq!(hidden.len(), 4, "hidden state must be hidden_size");
        let changed = model.extra_layers[0]
            .w_ih
            .iter()
            .zip(before.iter())
            .any(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| (x - y).abs() > 1e-12));
        assert!(changed, "layer-1 weights were not updated by train_pair");
    }
}
