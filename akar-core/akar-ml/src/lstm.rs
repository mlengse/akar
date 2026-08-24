//! 1-layer LSTM implementation.
//!
//! Architecture:
//! ```text
//!   input (x_t) ──→ [LSTM Cell] ──→ hidden (h_t)
//!                        │
//!                   cell state (c_t)
//!
//!   Gates (concatenated):
//!     [i, f, g, o] = W_ih * x_t + W_hh * h_{t-1} + b
//!     i = sigmoid(input gate)
//!     f = sigmoid(forget gate)
//!     g = tanh(candidate)
//!     o = sigmoid(output gate)
//!     c_t = f ⊙ c_{t-1} + i ⊙ g
//!     h_t = o ⊙ tanh(c_t)
//! ```

use serde::{Deserialize, Serialize};

/// Configuration for building an LSTM model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LstmConfig {
    /// Dimension of input features.
    pub input_size: usize,
    /// Dimension of hidden state.
    pub hidden_size: usize,
    /// Dimension of output (1 for regression/binary, N for classification).
    pub output_size: usize,
}

/// 1-layer LSTM model with trained weights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LstmModel {
    pub config: LstmConfig,
    /// Input-to-hidden weights: (4*hidden, input).
    pub w_ih: Vec<Vec<f64>>,
    /// Hidden-to-hidden weights: (4*hidden, hidden).
    pub w_hh: Vec<Vec<f64>>,
    /// Input-to-hidden bias: (4*hidden,).
    pub b_ih: Vec<f64>,
    /// Hidden-to-hidden bias: (4*hidden,).
    pub b_hh: Vec<f64>,
    /// Output projection weights: (output, hidden).
    pub w_ho: Vec<Vec<f64>>,
    /// Output projection bias: (output,).
    pub b_ho: Vec<f64>,
}

/// Intermediate state from a single forward pass (for backprop).
#[derive(Debug, Clone)]
pub struct LstmCell {
    // Gate activations
    pub input_gate: Vec<f64>,
    pub forget_gate: Vec<f64>,
    pub candidate: Vec<f64>,
    pub output_gate: Vec<f64>,
    // States
    pub cell_state: Vec<f64>,
    pub hidden_state: Vec<f64>,
    // Inputs (for backprop)
    pub x: Vec<f64>,
    pub h_prev: Vec<f64>,
    pub c_prev: Vec<f64>,
}

/// Result of training.
#[derive(Debug, Clone)]
pub struct TrainingResult {
    pub final_loss: f64,
    pub epochs: usize,
    pub loss_history: Vec<f64>,
}

// ─────────────────────── Helper math ───────────────────────

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

fn sigmoid_derivative(s: f64) -> f64 {
    s * (1.0 - s)
}

fn tanh_derivative(t: f64) -> f64 {
    1.0 - t * t
}

/// Element-wise multiply.
fn hadamard(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).collect()
}

/// Vector addition.
fn vec_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
}

// ─────────────────────── LSTM Cell ───────────────────────

impl LstmModel {
    /// Create a new model with Xavier-initialized weights.
    pub fn new(config: LstmConfig) -> Self {
        use rand::RngExt;
        let mut rng = rand::rng();
        let h = config.hidden_size;
        let i = config.input_size;
        let o = config.output_size;

        let mut xavier = |rows: usize, cols: usize| -> Vec<Vec<f64>> {
            let limit = (6.0 / (rows + cols) as f64).sqrt();
            (0..rows)
                .map(|_| (0..cols).map(|_| rng.random_range(-limit..limit)).collect())
                .collect()
        };

        Self {
            config,
            w_ih: xavier(4 * h, i),
            w_hh: xavier(4 * h, h),
            b_ih: vec![0.0; 4 * h],
            b_hh: vec![0.0; 4 * h],
            w_ho: xavier(o, h),
            b_ho: vec![0.0; o],
        }
    }

    /// Forward pass through the LSTM cell for one timestep.
    ///
    /// `x` — input vector (input_size)
    /// `h_prev` — previous hidden state (hidden_size)
    /// `c_prev` — previous cell state (hidden_size)
    pub fn forward_cell(&self, x: &[f64], h_prev: &[f64], c_prev: &[f64]) -> LstmCell {
        let h = self.config.hidden_size;
        let combined = 4 * h;

        // Compute gate pre-activations
        let mut gates = vec![0.0; combined];
        for j in 0..combined {
            gates[j] = self.b_ih[j] + self.b_hh[j];
            for k in 0..x.len() {
                gates[j] += self.w_ih[j][k] * x[k];
            }
            for k in 0..h_prev.len() {
                gates[j] += self.w_hh[j][k] * h_prev[k];
            }
        }

        // Split into gates
        let input_gate: Vec<f64> = gates[0..h].iter().map(|&v| sigmoid(v)).collect();
        let forget_gate: Vec<f64> = gates[h..2 * h].iter().map(|&v| sigmoid(v)).collect();
        let candidate: Vec<f64> = gates[2 * h..3 * h].iter().map(|&v| v.tanh()).collect();
        let output_gate: Vec<f64> = gates[3 * h..4 * h].iter().map(|&v| sigmoid(v)).collect();

        // Cell state update: c_t = f ⊙ c_prev + i ⊙ g
        let cell_state = {
            let fg = hadamard(&forget_gate, c_prev);
            let ig = hadamard(&input_gate, &candidate);
            vec_add(&fg, &ig)
        };

        // Hidden state: h_t = o ⊙ tanh(c_t)
        let tanh_c: Vec<f64> = cell_state.iter().map(|&v| v.tanh()).collect();
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

    /// Run forward pass over a sequence, return all cell states and final output.
    ///
    /// `sequence` — list of input vectors, one per timestep
    /// Returns: (all cells, output projection at final step)
    pub fn forward_sequence(&self, sequence: &[Vec<f64>]) -> (Vec<LstmCell>, Vec<f64>) {
        let h = self.config.hidden_size;
        let o = self.config.output_size;
        let mut h_prev = vec![0.0; h];
        let mut c_prev = vec![0.0; h];
        let mut cells = Vec::with_capacity(sequence.len());

        for x in sequence {
            let cell = self.forward_cell(x, &h_prev, &c_prev);
            h_prev = cell.hidden_state.clone();
            c_prev = cell.cell_state.clone();
            cells.push(cell);
        }

        // Output projection
        let final_h = &cells.last().unwrap().hidden_state;
        let output: Vec<f64> = (0..o)
            .map(|j| {
                let mut val = self.b_ho[j];
                for k in 0..final_h.len() {
                    val += self.w_ho[j][k] * final_h[k];
                }
                val
            })
            .collect();

        (cells, output)
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
pub fn train(
    model: &mut LstmModel,
    inputs: &[Vec<Vec<f64>>],
    targets: &[Vec<f64>],
    epochs: usize,
    lr: f64,
) -> TrainingResult {
    assert_eq!(inputs.len(), targets.len(), "inputs and targets must have same length");
    let n = inputs.len();
    let h = model.config.hidden_size;
    let i_sz = model.config.input_size;
    let o_sz = model.config.output_size;
    let mut loss_history = Vec::with_capacity(epochs);

    for epoch in 0..epochs {
        let mut epoch_loss = 0.0;

        for (seq, target) in inputs.iter().zip(targets.iter()) {
            let t_len = seq.len();

            // ── Forward pass: store all cells ──
            let (cells, output) = model.forward_sequence(seq);

            // ── MSE loss ──
            let loss: f64 = output
                .iter()
                .zip(target.iter())
                .map(|(o, t)| (o - t).powi(2))
                .sum::<f64>()
                / output.len() as f64;
            epoch_loss += loss;

            // ── Output layer gradient ──
            let o_len = output.len();
            let d_output: Vec<f64> = output
                .iter()
                .zip(target.iter())
                .map(|(o, t)| 2.0 * (o - t) / o_len as f64)
                .collect();

            // Accumulate parameter gradients across all timesteps
            let mut dw_ih = vec![vec![0.0; i_sz]; 4 * h];
            let mut dw_hh = vec![vec![0.0; h]; 4 * h];
            let mut db_ih = vec![0.0; 4 * h];
            let mut db_hh = vec![0.0; 4 * h];
            let mut dw_ho = vec![vec![0.0; h]; o_sz];
            let mut db_ho = vec![0.0; o_sz];

            // Output projection gradients (from final hidden state)
            let final_h = &cells[t_len - 1].hidden_state;
            for j in 0..o_sz {
                for k in 0..h {
                    dw_ho[j][k] += d_output[j] * final_h[k];
                }
                db_ho[j] += d_output[j];
            }

            // Gradient flowing into final hidden state from output layer
            let mut d_h: Vec<f64> = vec![0.0; h];
            for k in 0..h {
                for j in 0..o_sz {
                    d_h[k] += model.w_ho[j][k] * d_output[j];
                }
            }
            let mut d_c: Vec<f64> = vec![0.0; h];

            // ── BPTT: walk backward through all cells ──
            for t in (0..t_len).rev() {
                let cell = &cells[t];

                // tanh(c_t) — cached from forward
                let tanh_c: Vec<f64> = cell.cell_state.iter().map(|&v| v.tanh()).collect();

                // d_o = d_h ⊙ tanh(c_t)
                let d_o: Vec<f64> = d_h.iter().zip(tanh_c.iter()).map(|(dh, tc)| dh * tc).collect();

                // d_c += d_h ⊙ o ⊙ (1 - tanh²(c_t))  (accumulate with carry from future)
                let d_c_local: Vec<f64> = d_h
                    .iter()
                    .zip(cell.output_gate.iter())
                    .zip(tanh_c.iter())
                    .map(|((dh, o), tc)| dh * o * (1.0 - tc * tc))
                    .collect();
                for k in 0..h {
                    d_c[k] += d_c_local[k];
                }

                // d_f = d_c ⊙ c_prev
                let d_f: Vec<f64> = d_c.iter().zip(cell.c_prev.iter()).map(|(dc, cp)| dc * cp).collect();

                // d_i = d_c ⊙ g
                let d_i: Vec<f64> = d_c.iter().zip(cell.candidate.iter()).map(|(dc, g)| dc * g).collect();

                // d_g = d_c ⊙ i ⊙ (1 - g²)
                let d_g: Vec<f64> = d_c
                    .iter()
                    .zip(cell.input_gate.iter())
                    .zip(cell.candidate.iter())
                    .map(|((dc, ig), g)| dc * ig * (1.0 - g * g))
                    .collect();

                // Gate pre-activation gradients
                let mut d_gates = vec![0.0; 4 * h];
                for j in 0..h {
                    d_gates[j] = d_i[j] * sigmoid_derivative(cell.input_gate[j]);
                    d_gates[h + j] = d_f[j] * sigmoid_derivative(cell.forget_gate[j]);
                    d_gates[2 * h + j] = d_g[j] * tanh_derivative(cell.candidate[j]);
                    d_gates[3 * h + j] = d_o[j] * sigmoid_derivative(cell.output_gate[j]);
                }

                // Accumulate W_ih, W_hh, b_ih, b_hh
                for j in 0..4 * h {
                    for k in 0..cell.x.len() {
                        dw_ih[j][k] += d_gates[j] * cell.x[k];
                    }
                    for k in 0..cell.h_prev.len() {
                        dw_hh[j][k] += d_gates[j] * cell.h_prev[k];
                    }
                    db_ih[j] += d_gates[j];
                    db_hh[j] += d_gates[j];
                }

                // Propagate d_c and d_h to previous cell
                if t > 0 {
                    // d_c_prev = d_c ⊙ f  (gradient through cell state carry)
                    let d_c_prev: Vec<f64> = d_c.iter().zip(cell.forget_gate.iter()).map(|(dc, f)| dc * f).collect();

                    // d_h_prev = W_hh^T * d_gates
                    let mut d_h_prev = vec![0.0; h];
                    for k in 0..h {
                        for j in 0..4 * h {
                            d_h_prev[k] += model.w_hh[j][k] * d_gates[j];
                        }
                    }

                    d_c = d_c_prev;
                    d_h = d_h_prev;
                }
            }

            // ── Apply accumulated gradients ──
            for j in 0..o_sz {
                for k in 0..h {
                    model.w_ho[j][k] -= lr * dw_ho[j][k];
                }
                model.b_ho[j] -= lr * db_ho[j];
            }
            for j in 0..4 * h {
                for k in 0..i_sz {
                    model.w_ih[j][k] -= lr * dw_ih[j][k];
                }
                for k in 0..h {
                    model.w_hh[j][k] -= lr * dw_hh[j][k];
                }
                model.b_ih[j] -= lr * db_ih[j];
                model.b_hh[j] -= lr * db_hh[j];
            }
        }

        let avg_loss = epoch_loss / n as f64;
        loss_history.push(avg_loss);

        if epoch % 100 == 0 || epoch == epochs - 1 {
            tracing::info!("epoch {epoch}: loss = {avg_loss:.6}");
        }
    }

    TrainingResult {
        final_loss: *loss_history.last().unwrap_or(&0.0),
        epochs,
        loss_history,
    }
}

// ─────────────────────── Save / Load ───────────────────────

/// Save model weights to a JSON file.
pub fn save_model(model: &LstmModel, path: &str) -> Result<(), String> {
    let json = serde_json::to_string_pretty(model).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write: {e}"))
}

/// Load model weights from a JSON file.
pub fn load_model(path: &str) -> Result<LstmModel, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("deserialize: {e}"))
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
        let model = LstmModel::new(LstmConfig {
            input_size: 2,
            hidden_size: 4,
            output_size: 1,
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
        let model = LstmModel::new(LstmConfig {
            input_size: 5,
            hidden_size: 10,
            output_size: 3,
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
}
