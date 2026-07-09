use kuzu_common::types::Value;
use kuzu_common::vector::{DataChunk, ValueVector};
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

/// Accumulate — materializes all input into a single contiguous chunk in memory.
///
/// This is required for operations that need random access to all rows,
/// such as hash join build side, correlated subqueries, etc.
pub struct PhysicalAccumulate;

impl PhysicalOperatorExec for PhysicalAccumulate {
    fn operator_type(&self) -> &str {
        "accumulate"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            return Ok(input);
        }

        // Concatenate all chunks into a single chunk
        let num_fields = input[0].num_fields();
        let merged_fields: Vec<ValueVector> = (0..num_fields)
            .map(|i| {
                let first_type = input[0].field(i).physical_type();
                let total_size: usize = input.iter().map(|c| c.field(i).size()).sum();
                let mut merged = ValueVector::new(first_type, total_size.max(1));
                for chunk in &input {
                    merged.append(chunk.field(i));
                }
                merged
            })
            .collect();

        let size = merged_fields.first().map(|f| f.size()).unwrap_or(0);
        let field_names = input[0].field_names.clone();

        Ok(vec![DataChunk {
            fields: merged_fields,
            size,
            field_names,
        }])
    }
}

/// Union — concatenates results from two child pipelines.
///
/// This operator is not currently used in the pipeline (Union is handled inline
/// in QueryProcessor::execute_internal). Kept for API compatibility with C++.
pub struct PhysicalUnion {
    pub all: bool,
}

impl PhysicalOperatorExec for PhysicalUnion {
    fn operator_type(&self) -> &str {
        "union"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        // Not used - Union is handled inline in execute_internal
        Ok(Vec::new())
    }
}

/// ResultCollector — collects all input DataChunks into a single result set.
///
/// This is the final operator in the query pipeline. It consolidates
/// all chunks into a single `Vec<DataChunk>` ready for client return.
/// If there are multiple input chunks, they are merged into one.
pub struct ResultCollector;

impl PhysicalOperatorExec for ResultCollector {
    fn operator_type(&self) -> &str {
        "result_collector"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if input.is_empty() {
            return Ok(input);
        }
        if input.len() == 1 {
            return Ok(input);
        }

        // Merge multiple chunks into one
        let num_fields = input[0].num_fields();
        let merged_fields: Vec<ValueVector> = (0..num_fields)
            .map(|i| {
                let first_type = input[0].field(i).physical_type();
                let total_size: usize = input.iter().map(|c| c.field(i).size()).sum();
                let mut merged = ValueVector::new(first_type, total_size.max(1));
                for chunk in &input {
                    merged.append(chunk.field(i));
                }
                merged
            })
            .collect();

        let size = merged_fields.first().map(|f| f.size()).unwrap_or(0);
        let field_names = input[0].field_names.clone();

        Ok(vec![DataChunk {
            fields: merged_fields,
            size,
            field_names,
        }])
    }
}

/// DummySink — consumes and discards all input.
///
/// Used as a pipeline terminal when results are not needed (e.g.,
/// DDL statements, EXPLAIN without ANALYZE).
pub struct DummySink;

impl PhysicalOperatorExec for DummySink {
    fn operator_type(&self) -> &str {
        "dummy_sink"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        Ok(Vec::new())
    }
}

/// DummySimpleSink — like DummySink but passes through a single empty chunk.
///
/// Some pipeline consumers expect at least one DataChunk to be returned.
pub struct DummySimpleSink;

impl PhysicalOperatorExec for DummySimpleSink {
    fn operator_type(&self) -> &str {
        "dummy_simple_sink"
    }

    fn execute(&self, _input: Vec<DataChunk>) -> OperatorResult {
        Ok(vec![DataChunk::new(Vec::new())])
    }
}

/// Profile — wraps an operator with timing instrumentation.
///
/// Measures wall-clock execution time of the wrapped operator.
/// The elapsed time is stored in `elapsed` field for later inspection
/// via EXPLAIN ANALYZE or profiling output.
pub struct Profile {
    pub inner: Box<dyn PhysicalOperatorExec + Send + Sync>,
    pub elapsed_nanos: std::sync::atomic::AtomicU64,
}

impl Profile {
    pub fn new(inner: Box<dyn PhysicalOperatorExec + Send + Sync>) -> Self {
        Self {
            inner,
            elapsed_nanos: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn elapsed_ns(&self) -> u64 {
        self.elapsed_nanos.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl PhysicalOperatorExec for Profile {
    fn operator_type(&self) -> &str {
        "profile"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let start = std::time::Instant::now();
        let result = self.inner.execute(input);
        let elapsed = start.elapsed();
        self.elapsed_nanos.fetch_add(elapsed.as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        result
    }
}

/// Partitioner — splits input DataChunks into fixed-size morsels for parallelism.
///
/// Each morsel (batch of `morsel_size` rows) can be processed independently
/// by downstream operators. The mapper uses this to distribute work across
/// threads via Rayon.
///
/// A morsel size of 0 disables splitting (pass-through).
pub struct Partitioner {
    pub morsel_size: usize,
}

impl Partitioner {
    pub fn new(morsel_size: usize) -> Self {
        Self { morsel_size }
    }

    /// Split a single DataChunk into morsels of `morsel_size` rows.
    fn split_chunk(&self, chunk: &DataChunk) -> Vec<DataChunk> {
        if self.morsel_size == 0 || chunk.size <= self.morsel_size {
            return vec![chunk.clone()];
        }
        let num_morsels = chunk.size.div_ceil(self.morsel_size);
        let mut morsels = Vec::with_capacity(num_morsels);
        for start in (0..chunk.size).step_by(self.morsel_size) {
            let end = (start + self.morsel_size).min(chunk.size);
            let morsel_fields: Vec<ValueVector> = chunk
                .fields
                .iter()
                .map(|fv| {
                    let mut morsel = ValueVector::new(fv.physical_type(), end - start);
                    for i in start..end {
                        let val = fv.get_value(i).unwrap_or(Value::Null);
                        let _ = morsel.set_value(i - start, &val);
                    }
                    morsel
                })
                .collect();
            morsels.push(DataChunk {
                fields: morsel_fields,
                size: end - start,
                field_names: chunk.field_names.clone(),
            });
        }
        morsels
    }
}

impl PhysicalOperatorExec for Partitioner {
    fn operator_type(&self) -> &str {
        "partitioner"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        if self.morsel_size == 0 {
            return Ok(input);
        }
        let morsels: Vec<DataChunk> = input.iter().flat_map(|chunk| self.split_chunk(chunk)).collect();
        Ok(morsels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partitioner_pass_through_zero_size() {
        let p = Partitioner::new(0);
        let chunk = DataChunk {
            fields: vec![ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, 10)],
            size: 10,
            field_names: vec!["val".into()],
        };
        let result = p.execute(vec![chunk.clone()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 10);
    }

    #[test]
    fn test_partitioner_no_split_small_chunk() {
        let p = Partitioner::new(100);
        let chunk = DataChunk {
            fields: vec![ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, 5)],
            size: 5,
            field_names: vec!["val".into()],
        };
        let result = p.execute(vec![chunk.clone()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 5);
    }

    #[test]
    fn test_partitioner_splits_large_chunk() {
        let p = Partitioner::new(10);
        let mut fv = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, 25);
        fv.resize(25);
        for i in 0..25 {
            fv.set_i64(i, i as i64);
        }
        let chunk = DataChunk {
            fields: vec![fv],
            size: 25,
            field_names: vec!["val".into()],
        };
        let result = p.execute(vec![chunk]).unwrap();
        // 25 rows with morsel_size 10 → 3 morsels (10 + 10 + 5)
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].size, 10);
        assert_eq!(result[1].size, 10);
        assert_eq!(result[2].size, 5);
        // Verify data integrity: first morsel
        assert_eq!(result[0].fields[0].get_i64(0), Some(0));
        assert_eq!(result[0].fields[0].get_i64(9), Some(9));
        // Second morsel
        assert_eq!(result[1].fields[0].get_i64(0), Some(10));
        // Third morsel
        assert_eq!(result[2].fields[0].get_i64(0), Some(20));
        assert_eq!(result[2].fields[0].get_i64(4), Some(24));
    }

    #[test]
    fn test_partitioner_5_rows_split_into_3_2() {
        let p = Partitioner::new(3);
        let mut fv = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, 5);
        fv.resize(5);
        for i in 0..5 { fv.set_i64(i, i as i64); }
        let chunk = DataChunk { fields: vec![fv], size: 5, field_names: vec!["val".into()] };
        let result = p.execute(vec![chunk]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].size, 3);
        assert_eq!(result[1].size, 2);
        assert_eq!(result[0].fields[0].get_i64(2), Some(2));
        assert_eq!(result[1].fields[0].get_i64(0), Some(3));
    }

    #[test]
    fn test_partitioner_multiple_chunks() {
        let p = Partitioner::new(3);
        let mut fv1 = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, 5);
        fv1.resize(5);
        for i in 0..5 { fv1.set_i64(i, i as i64); }
        let mut fv2 = ValueVector::new(kuzu_common::types::PhysicalTypeID::Int64, 5);
        fv2.resize(5);
        for i in 0..5 { fv2.set_i64(i, (i + 5) as i64); }

        let chunks = vec![
            DataChunk { fields: vec![fv1], size: 5, field_names: vec!["val".into()] },
            DataChunk { fields: vec![fv2], size: 5, field_names: vec!["val".into()] },
        ];
        let result = p.execute(chunks).unwrap();
        assert_eq!(result.len(), 4, "expected 4 morsels from 2 chunks of 5 rows each");
        assert_eq!(result[2].fields[0].get_i64(0), Some(5));
        assert_eq!(result[2].fields[0].get_i64(2), Some(7));
        assert_eq!(result[3].fields[0].get_i64(0), Some(8));
    }
}
