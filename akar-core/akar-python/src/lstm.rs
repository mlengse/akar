//! PyO3 bindings for LSTM model (akar-ml).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// LSTM cell output for one timestep.
#[pyclass(skip_from_py_object)]
#[derive(Clone)]
pub struct LstmCell {
    pub input_gate: Vec<f64>,
    pub forget_gate: Vec<f64>,
    pub candidate: Vec<f64>,
    pub output_gate: Vec<f64>,
    pub cell_state: Vec<f64>,
    pub hidden_state: Vec<f64>,
}

#[pymethods]
impl LstmCell {
    fn input_gate(&self) -> Vec<f64> {
        self.input_gate.clone()
    }
    fn forget_gate(&self) -> Vec<f64> {
        self.forget_gate.clone()
    }
    fn candidate(&self) -> Vec<f64> {
        self.candidate.clone()
    }
    fn output_gate(&self) -> Vec<f64> {
        self.output_gate.clone()
    }
    fn cell_state(&self) -> Vec<f64> {
        self.cell_state.clone()
    }
    fn hidden_state(&self) -> Vec<f64> {
        self.hidden_state.clone()
    }
}

/// Training result returned by `LstmModel.train()`.
#[pyclass(skip_from_py_object)]
#[derive(Clone)]
pub struct TrainingResult {
    pub final_loss: f64,
    pub epochs: usize,
    pub loss_history: Vec<f64>,
}

#[pymethods]
impl TrainingResult {
    fn final_loss(&self) -> f64 {
        self.final_loss
    }
    fn epochs(&self) -> usize {
        self.epochs
    }
    fn loss_history(&self) -> Vec<f64> {
        self.loss_history.clone()
    }
}

/// LSTM model for sequence prediction.
#[pyclass]
pub struct LstmModel {
    inner: akar_ml::lstm::LstmModel,
}

#[pymethods]
impl LstmModel {
    /// Create a new LSTM model with Xavier-initialized weights.
    #[new]
    #[pyo3(signature = (input_size, hidden_size, output_size, num_layers = 1))]
    fn new(input_size: usize, hidden_size: usize, output_size: usize, num_layers: usize) -> Self {
        Self {
            inner: akar_ml::lstm::LstmModel::new(akar_ml::lstm::LstmConfig {
                input_size,
                hidden_size,
                output_size,
                num_layers,
            }),
        }
    }

    /// Forward pass through the LSTM cell for one timestep.
    fn forward_cell(&self, x: Vec<f64>, h_prev: Vec<f64>, c_prev: Vec<f64>) -> PyResult<LstmCell> {
        let cell = self.inner.forward_cell(&x, &h_prev, &c_prev);
        Ok(LstmCell {
            input_gate: cell.input_gate,
            forget_gate: cell.forget_gate,
            candidate: cell.candidate,
            output_gate: cell.output_gate,
            cell_state: cell.cell_state,
            hidden_state: cell.hidden_state,
        })
    }

    /// Forward pass through a sequence of inputs. Returns the final output vector.
    fn forward_sequence(&self, sequence: Vec<Vec<f64>>) -> PyResult<Vec<f64>> {
        let (_cells, output) = self.inner.forward_sequence(&sequence);
        Ok(output)
    }

    /// Forward pass through a sequence of inputs, returning the projected
    /// output **and** the last layer's raw hidden state at every timestep
    /// (each of length `hidden_size`, not `output_size`).
    fn forward_sequence_hidden(&self, sequence: Vec<Vec<f64>>) -> PyResult<(Vec<f64>, Vec<Vec<f64>>)> {
        let (output, hidden_states) = self.inner.forward_sequence_hidden(&sequence);
        Ok((output, hidden_states))
    }

    /// Online single-pair training step: one forward + one backward BPTT pass
    /// that updates the model weights **in place** (all `num_layers`).
    ///
    /// Returns `(mse_loss, final_hidden_state)`, where `mse_loss` is a scalar
    /// (like the C++ `LSTMPredictor::train_step`) and `final_hidden_state` has
    /// length `hidden_size`.
    fn train_pair(&mut self, input: Vec<Vec<f64>>, target: Vec<f64>, lr: f64) -> PyResult<(f64, Vec<f64>)> {
        if input.is_empty() {
            return Err(PyValueError::new_err("train_pair: input sequence must not be empty"));
        }
        if target.len() != self.inner.config.output_size {
            return Err(PyValueError::new_err(format!(
                "train_pair: target length {} must equal output_size {}",
                target.len(),
                self.inner.config.output_size,
            )));
        }
        Ok(self.inner.train_pair(&input, &target, lr))
    }

    /// Train the model on a batch of sequences and targets.
    #[staticmethod]
    #[pyo3(signature = (input_size, hidden_size, output_size, inputs, targets, epochs, lr, num_layers = 1))]
    fn train(
        input_size: usize,
        hidden_size: usize,
        output_size: usize,
        inputs: Vec<Vec<Vec<f64>>>,
        targets: Vec<Vec<f64>>,
        epochs: usize,
        lr: f64,
        num_layers: usize,
    ) -> PyResult<TrainingResult> {
        let mut model = akar_ml::lstm::LstmModel::new(akar_ml::lstm::LstmConfig {
            input_size,
            hidden_size,
            output_size,
            num_layers,
        });

        let result = akar_ml::lstm::train(&mut model, &inputs, &targets, epochs, lr);

        Ok(TrainingResult {
            final_loss: result.final_loss,
            epochs: result.epochs,
            loss_history: result.loss_history,
        })
    }

    /// Save model weights to a JSON file.
    fn save(&self, path: &str) -> PyResult<()> {
        akar_ml::lstm::save_model(&self.inner, path).map_err(PyValueError::new_err)
    }

    /// Load model weights from a JSON file.
    #[staticmethod]
    fn load(path: &str) -> PyResult<Self> {
        let inner = akar_ml::lstm::load_model(path).map_err(PyValueError::new_err)?;
        Ok(Self { inner })
    }

    /// Save model weights to a compact binary file (P117.1).
    fn save_bin(&self, path: &str) -> PyResult<()> {
        akar_ml::lstm::save_bin(&self.inner, path).map_err(PyValueError::new_err)
    }

    /// Load model weights from a binary file written by `save_bin`.
    #[staticmethod]
    fn load_bin(path: &str) -> PyResult<Self> {
        let inner = akar_ml::lstm::load_bin(path).map_err(PyValueError::new_err)?;
        Ok(Self { inner })
    }

    fn __repr__(&self) -> String {
        format!(
            "<akar.LstmModel input={} hidden={} output={} num_layers={}>",
            self.inner.config.input_size,
            self.inner.config.hidden_size,
            self.inner.config.output_size,
            self.inner.config.num_layers,
        )
    }
}

/// Register this submodule on the parent `akar` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "lstm")?;
    sub.add_class::<LstmModel>()?;
    sub.add_class::<LstmCell>()?;
    sub.add_class::<TrainingResult>()?;
    m.add_submodule(&sub)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lstm_new_and_repr() {
        let m = LstmModel::new(3, 4, 2, 1);
        let r = m.__repr__();
        assert!(r.contains("input=3"));
        assert!(r.contains("hidden=4"));
        assert!(r.contains("output=2"));
        assert!(r.contains("num_layers=1"));
    }

    #[test]
    fn test_lstm_two_layer_forward_cell() {
        let m = LstmModel::new(2, 3, 1, 2);
        let x = vec![0.5, -0.3];
        let h = vec![0.0; 3];
        let c = vec![0.0; 3];
        let cell = m.forward_cell(x, h, c).unwrap();
        assert_eq!(cell.input_gate().len(), 3);
        assert_eq!(cell.hidden_state().len(), 3);
        assert_eq!(m.inner.extra_layers.len(), 1);
    }

    #[test]
    fn test_lstm_forward_cell() {
        let m = LstmModel::new(2, 3, 1, 1);
        let x = vec![0.5, -0.3];
        let h = vec![0.0; 3];
        let c = vec![0.0; 3];
        let cell = m.forward_cell(x, h, c).unwrap();
        assert_eq!(cell.input_gate().len(), 3);
        assert_eq!(cell.hidden_state().len(), 3);
    }

    #[test]
    fn test_lstm_forward_sequence() {
        let m = LstmModel::new(2, 3, 1, 1);
        let seq = vec![vec![1.0, 0.5], vec![0.3, -0.2]];
        let output = m.forward_sequence(seq).unwrap();
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn test_lstm_forward_sequence_hidden() {
        let m = LstmModel::new(2, 3, 1, 1);
        let seq = vec![vec![1.0, 0.5], vec![0.3, -0.2], vec![0.1, 0.9]];
        let (output, hidden) = m.forward_sequence_hidden(seq).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(hidden.len(), 3, "hidden states len = seq_len");
        for h in &hidden {
            assert_eq!(h.len(), 3, "hidden dim = hidden_size");
        }
    }

    #[test]
    fn test_lstm_forward_sequence_hidden_consistent_with_forward_cell() {
        let m = LstmModel::new(2, 3, 1, 1);
        let seq = vec![vec![1.0, 0.5], vec![0.3, -0.2]];
        let (output, hidden) = m.forward_sequence_hidden(seq.clone()).unwrap();

        let mut h_prev = vec![0.0; 3];
        let mut c_prev = vec![0.0; 3];
        for (i, x) in seq.iter().enumerate() {
            let cell = m.forward_cell(x.clone(), h_prev.clone(), c_prev.clone()).unwrap();
            for (a, b) in hidden[i].iter().zip(cell.hidden_state().iter()) {
                assert!((a - b).abs() < 1e-9, "hidden mismatch at step {i}: {a} vs {b}");
            }
            h_prev = cell.hidden_state();
            c_prev = cell.cell_state();
        }

        let projected: Vec<f64> = (0..1)
            .map(|j| {
                let mut val = m.inner.b_ho[j];
                for (k, h) in h_prev.iter().enumerate() {
                    val += m.inner.w_ho[j][k] * h;
                }
                val
            })
            .collect();
        assert!((projected[0] - output[0]).abs() < 1e-9);
    }

    #[test]
    fn test_lstm_two_layer_forward_sequence() {
        let m = LstmModel::new(2, 3, 2, 2);
        let seq = vec![vec![0.5, -0.3], vec![0.1, 0.7]];
        let output = m.forward_sequence(seq).unwrap();
        assert_eq!(output.len(), 2, "2-layer sequence output must equal output_size");
        assert_eq!(m.inner.extra_layers.len(), 1);
    }

    #[test]
    fn test_lstm_train() {
        let inputs = vec![vec![vec![0.0, 0.0]], vec![vec![1.0, 1.0]]];
        let targets = vec![vec![0.0], vec![1.0]];
        let result = LstmModel::train(2, 4, 1, inputs, targets, 10, 0.01, 1).unwrap();
        assert_eq!(result.epochs(), 10);
        assert!(result.final_loss() > 0.0);
    }

    #[test]
    fn test_lstm_train_pair_shape() {
        let mut m = LstmModel::new(2, 4, 1, 1);
        let input = vec![vec![0.0, 1.0], vec![0.5, -0.5]];
        let (loss, hidden) = m.train_pair(input, vec![1.0], 0.05).unwrap();
        assert!(loss.is_finite() && loss >= 0.0);
        assert_eq!(hidden.len(), 4, "hidden length must equal hidden_size");
    }

    #[test]
    fn test_lstm_train_pair_reduces_loss_over_calls() {
        let mut m = LstmModel::new(2, 4, 1, 1);
        let input = vec![vec![0.0, 0.0], vec![1.0, 1.0]];
        let (first, _) = m.train_pair(input.clone(), vec![1.0], 0.05).unwrap();
        let mut last = first;
        for _ in 0..60 {
            let (loss, _) = m.train_pair(input.clone(), vec![1.0], 0.05).unwrap();
            last = loss;
        }
        assert!(last < first, "online loss should decrease: {first} -> {last}");
    }

    #[test]
    fn test_lstm_train_pair_two_layer_updates_extra_layers() {
        let mut m = LstmModel::new(2, 3, 1, 2);
        let before = m.inner.extra_layers[0].w_ih[0][0];
        let _ = m
            .train_pair(vec![vec![0.5, -0.3], vec![0.1, 0.7]], vec![1.0], 0.05)
            .unwrap();
        let after = m.inner.extra_layers[0].w_ih[0][0];
        assert_ne!(before, after, "extra layer weights must be updated in place");
    }

    #[test]
    fn test_lstm_train_pair_rejects_bad_target() {
        let mut m = LstmModel::new(2, 3, 1, 1);
        assert!(m.train_pair(vec![vec![0.0, 0.0]], vec![1.0, 2.0], 0.05).is_err());
        let mut m = LstmModel::new(2, 3, 1, 1);
        assert!(m.train_pair(vec![], vec![1.0], 0.05).is_err());
    }

    #[test]
    fn test_lstm_save_bin_load_bin_roundtrip() {
        // P117.1: binary roundtrip through the PyO3 surface — config and a
        // reference weight must survive.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        let m = LstmModel::new(2, 4, 1, 2);
        let w_ref = m.inner.w_ih[0][0];
        m.save_bin(path.to_str().unwrap()).unwrap();
        let loaded = LstmModel::load_bin(path.to_str().unwrap()).unwrap();

        assert_eq!(loaded.inner.config.input_size, 2);
        assert_eq!(loaded.inner.config.hidden_size, 4);
        assert_eq!(loaded.inner.config.output_size, 1);
        assert_eq!(loaded.inner.config.num_layers, 2);
        assert_eq!(loaded.inner.extra_layers.len(), 1);
        assert_eq!(loaded.inner.w_ih[0][0], w_ref);
        assert_eq!(loaded.inner.w_ih, m.inner.w_ih);
        assert_eq!(loaded.inner.w_ho, m.inner.w_ho);
    }

    #[test]
    fn test_lstm_save_bin_roundtrip_forward_identical() {
        // P117.1: forward output after binary roundtrip must be bit-identical.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        let m = LstmModel::new(3, 4, 2, 2);
        let seq = vec![vec![0.5, -0.3, 0.8], vec![0.1, 0.7, -0.2]];
        let before = m.forward_sequence(seq.clone()).unwrap();
        m.save_bin(path.to_str().unwrap()).unwrap();
        let loaded = LstmModel::load_bin(path.to_str().unwrap()).unwrap();
        let after = loaded.forward_sequence(seq).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn test_lstm_save_bin_json_still_works() {
        // P117.1 backward-compat: JSON save/load path must remain available
        // alongside the new binary one.
        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("m.bin");
        let json_path = dir.path().join("m.json");
        let m = LstmModel::new(2, 3, 1, 1);

        m.save_bin(bin_path.to_str().unwrap()).unwrap();
        let loaded_bin = LstmModel::load_bin(bin_path.to_str().unwrap()).unwrap();
        loaded_bin.save(json_path.to_str().unwrap()).unwrap();
        let loaded_json = LstmModel::load(json_path.to_str().unwrap()).unwrap();

        assert_eq!(loaded_json.inner.config.num_layers, 1);
        // Binary is bit-exact; JSON may be off by ~1 ulp, so compare with tolerance.
        for (a_row, b_row) in loaded_json.inner.w_ih.iter().zip(loaded_bin.inner.w_ih.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert!((a - b).abs() < 1e-10, "bin→json w_ih mismatch: {a} vs {b}");
            }
        }
        for (a_row, b_row) in loaded_json.inner.w_ho.iter().zip(loaded_bin.inner.w_ho.iter()) {
            for (a, b) in a_row.iter().zip(b_row.iter()) {
                assert!((a - b).abs() < 1e-10, "bin→json w_ho mismatch: {a} vs {b}");
            }
        }
    }

    #[test]
    fn test_lstm_load_bin_rejects_garbage() {
        // P117.1: garbage binary → ValueError, not a panic.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.bin");
        std::fs::write(&path, b"not-an-lstm").unwrap();
        assert!(LstmModel::load_bin(path.to_str().unwrap()).is_err());
    }
}
