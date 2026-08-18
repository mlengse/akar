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
    fn new(input_size: usize, hidden_size: usize, output_size: usize) -> Self {
        Self {
            inner: akar_ml::lstm::LstmModel::new(akar_ml::lstm::LstmConfig {
                input_size,
                hidden_size,
                output_size,
            }),
        }
    }

    /// Forward pass through the LSTM cell for one timestep.
    fn forward_cell(
        &self,
        x: Vec<f64>,
        h_prev: Vec<f64>,
        c_prev: Vec<f64>,
    ) -> PyResult<LstmCell> {
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

    /// Train the model on a batch of sequences and targets.
    #[staticmethod]
    fn train(
        input_size: usize,
        hidden_size: usize,
        output_size: usize,
        inputs: Vec<Vec<Vec<f64>>>,
        targets: Vec<Vec<f64>>,
        epochs: usize,
        lr: f64,
    ) -> PyResult<TrainingResult> {
        let mut model = akar_ml::lstm::LstmModel::new(akar_ml::lstm::LstmConfig {
            input_size,
            hidden_size,
            output_size,
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
        akar_ml::lstm::save_model(&self.inner, path)
            .map_err(PyValueError::new_err)
    }

    /// Load model weights from a JSON file.
    #[staticmethod]
    fn load(path: &str) -> PyResult<Self> {
        let inner = akar_ml::lstm::load_model(path)
            .map_err(PyValueError::new_err)?;
        Ok(Self { inner })
    }

    fn __repr__(&self) -> String {
        format!(
            "<akar.LstmModel input={} hidden={} output={}>",
            self.inner.config.input_size,
            self.inner.config.hidden_size,
            self.inner.config.output_size,
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
        let m = LstmModel::new(3, 4, 2);
        let r = m.__repr__();
        assert!(r.contains("input=3"));
        assert!(r.contains("hidden=4"));
        assert!(r.contains("output=2"));
    }

    #[test]
    fn test_lstm_forward_cell() {
        let m = LstmModel::new(2, 3, 1);
        let x = vec![0.5, -0.3];
        let h = vec![0.0; 3];
        let c = vec![0.0; 3];
        let cell = m.forward_cell(x, h, c).unwrap();
        assert_eq!(cell.input_gate().len(), 3);
        assert_eq!(cell.hidden_state().len(), 3);
    }

    #[test]
    fn test_lstm_forward_sequence() {
        let m = LstmModel::new(2, 3, 1);
        let seq = vec![vec![1.0, 0.5], vec![0.3, -0.2]];
        let output = m.forward_sequence(seq).unwrap();
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn test_lstm_train() {
        let inputs = vec![vec![vec![0.0, 0.0]], vec![vec![1.0, 1.0]]];
        let targets = vec![vec![0.0], vec![1.0]];
        let result = LstmModel::train(2, 4, 1, inputs, targets, 10, 0.01).unwrap();
        assert_eq!(result.epochs(), 10);
        assert!(result.final_loss() > 0.0);
    }
}
