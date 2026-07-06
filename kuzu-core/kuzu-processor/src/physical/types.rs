//! Core types and traits for physical operators.

use kuzu_common::types::Value;
use kuzu_common::vector::DataChunk;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Result of executing a physical operator.
pub type OperatorResult = Result<Vec<DataChunk>, String>;

pub type HashJoinBucket = Vec<(Value, Vec<(usize, usize)>)>;
pub type HashJoinTable = HashMap<u64, HashJoinBucket>;

/// A semi-mask tracks which node offsets match a join condition (SIP).
#[derive(Debug, Clone)]
pub struct NodeSemiMask {
    pub masked_offsets: Arc<Mutex<HashSet<u64>>>,
    pub table_id: u64,
    pub initialized: Arc<std::sync::atomic::AtomicBool>,
}

impl NodeSemiMask {
    pub fn new(table_id: u64) -> Self {
        Self {
            masked_offsets: Arc::new(Mutex::new(HashSet::new())),
            table_id,
            initialized: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    pub fn mask(&self, offset: u64) {
        if let Ok(mut guard) = self.masked_offsets.lock() { guard.insert(offset); }
    }
    pub fn is_masked(&self, offset: u64) -> bool {
        if !self.initialized.load(std::sync::atomic::Ordering::SeqCst) { return true; }
        self.masked_offsets.lock().map(|guard| guard.contains(&offset)).unwrap_or(true)
    }
    pub fn finalize(&self) {
        self.initialized.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

pub trait PhysicalOperatorExec {
    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult;
    fn operator_type(&self) -> &str;
}

pub struct PhysicalSemiMasker {
    pub key_column: usize,
    pub mask: NodeSemiMask,
}

impl PhysicalOperatorExec for PhysicalSemiMasker {
    fn operator_type(&self) -> &str { "semi_masker" }
    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() || input[0].fields.is_empty() { return Ok(input); }
        let chunk = &input[0];
        if self.key_column >= chunk.fields.len() {
            return Err(format!("SemiMasker: key_column {} out of bounds ({} fields)", self.key_column, chunk.fields.len()));
        }
        let field = &chunk.fields[self.key_column];
        for i in 0..chunk.size {
            if !field.is_null(i) {
                let offset = u64::from_le_bytes(field.data()[i * 8..i * 8 + 8].try_into().unwrap_or([0u8; 8]));
                self.mask.mask(offset);
            }
        }
        self.mask.finalize();
        Ok(input)
    }
}
