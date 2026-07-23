use crate::physical::types::{OperatorResult, PhysicalOperatorExec};
use crate::processor::chunk_helpers::{extract_all_rows_from_chunks, rows_to_columns};
use akar_common::types::Value;
use akar_common::vector::DataChunk;

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

        let all_rows = extract_all_rows_from_chunks(&input);
        if all_rows.is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        let (fields, field_types) = rows_to_columns(&all_rows);
        let size = all_rows.len();
        let field_names = input[0].field_names.clone();

        Ok(vec![DataChunk {
            fields,
            field_types,
            size,
            field_names,
            sel_vector: None,
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

        let all_rows = extract_all_rows_from_chunks(&input);
        if all_rows.is_empty() {
            return Ok(vec![DataChunk::new(vec![], vec![])]);
        }

        let (fields, field_types) = rows_to_columns(&all_rows);
        let size = all_rows.len();
        let field_names = input[0].field_names.clone();

        Ok(vec![DataChunk {
            fields,
            field_types,
            size,
            field_names,
            sel_vector: None,
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
        Ok(vec![DataChunk::new(Vec::new(), Vec::new())])
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
        self.elapsed_nanos
            .fetch_add(elapsed.as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
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
            let morsel_fields: Vec<arrow::array::ArrayRef> = chunk
                .fields
                .iter()
                .enumerate()
                .map(|(col_idx, _fv)| {
                    let phys_type = chunk.field_types[col_idx];
                    let mut morsel = akar_common::vector::ValueVector::new(phys_type, end - start);
                    for i in start..end {
                        let val = chunk.get_value(col_idx, i).unwrap_or(Value::Null);
                        let _ = morsel.set_value(i - start, &val);
                    }
                    akar_common::arrow_vector::ArrowVector::from_legacy(&morsel).array
                })
                .collect();
            morsels.push(DataChunk {
                fields: morsel_fields,
                field_types: chunk.field_types.clone(),
                size: end - start,
                field_names: chunk.field_names.clone(),
                sel_vector: None,
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
        let mut fv = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, 10);
        fv.resize(10);
        let chunk = {
            let arrow_fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&fv).array];
            let arrow_field_types = vec![fv.physical_type()];
            DataChunk::new(arrow_fields, arrow_field_types).with_names(vec!["val".into()])
        };
        let result = p.execute(vec![chunk.clone()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 10);
    }

    #[test]
    fn test_partitioner_no_split_small_chunk() {
        let p = Partitioner::new(100);
        let mut fv = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, 5);
        fv.resize(5);
        let chunk = {
            let arrow_fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&fv).array];
            let arrow_field_types = vec![fv.physical_type()];
            DataChunk::new(arrow_fields, arrow_field_types).with_names(vec!["val".into()])
        };
        let result = p.execute(vec![chunk.clone()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].size, 5);
    }

    #[test]
    fn test_partitioner_splits_large_chunk() {
        let p = Partitioner::new(10);
        let mut fv = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, 25);
        fv.resize(25);
        for i in 0..25 {
            fv.set_i64(i, i as i64);
        }
        let chunk = {
            let arrow_fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&fv).array];
            let arrow_field_types = vec![fv.physical_type()];
            DataChunk::new(arrow_fields, arrow_field_types).with_names(vec!["val".into()])
        };
        let result = p.execute(vec![chunk]).unwrap();
        // 25 rows with morsel_size 10 → 3 morsels (10 + 10 + 5)
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].size, 10);
        assert_eq!(result[1].size, 10);
        assert_eq!(result[2].size, 5);
        // Verify data integrity: first morsel
        assert_eq!(result[0].get_value(0, 0), Some(akar_common::types::Value::Int64(0)));
        assert_eq!(result[0].get_value(0, 9), Some(akar_common::types::Value::Int64(9)));
        // Second morsel
        assert_eq!(result[1].get_value(0, 0), Some(akar_common::types::Value::Int64(10)));
        // Third morsel
        assert_eq!(result[2].get_value(0, 0), Some(akar_common::types::Value::Int64(20)));
        assert_eq!(result[2].get_value(0, 4), Some(akar_common::types::Value::Int64(24)));
    }

    #[test]
    fn test_partitioner_5_rows_split_into_3_2() {
        let p = Partitioner::new(3);
        let mut fv = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, 5);
        fv.resize(5);
        for i in 0..5 {
            fv.set_i64(i, i as i64);
        }
        let chunk = {
            let arrow_fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&fv).array];
            let arrow_field_types = vec![fv.physical_type()];
            DataChunk::new(arrow_fields, arrow_field_types).with_names(vec!["val".into()])
        };
        let result = p.execute(vec![chunk]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].size, 3);
        assert_eq!(result[1].size, 2);
        assert_eq!(result[0].get_value(0, 2), Some(akar_common::types::Value::Int64(2)));
        assert_eq!(result[1].get_value(0, 0), Some(akar_common::types::Value::Int64(3)));
    }

    #[test]
    fn test_partitioner_multiple_chunks() {
        let p = Partitioner::new(3);
        let mut fv1 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, 5);
        fv1.resize(5);
        for i in 0..5 {
            fv1.set_i64(i, i as i64);
        }
        let mut fv2 = akar_common::vector::ValueVector::new(akar_common::types::PhysicalTypeID::Int64, 5);
        fv2.resize(5);
        for i in 0..5 {
            fv2.set_i64(i, (i + 5) as i64);
        }

        let chunks = vec![
            {
                let arrow_fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&fv1).array];
                let arrow_field_types = vec![fv1.physical_type()];
                DataChunk::new(arrow_fields, arrow_field_types).with_names(vec!["val".into()])
            },
            {
                let arrow_fields = vec![akar_common::arrow_vector::ArrowVector::from_legacy(&fv2).array];
                let arrow_field_types = vec![fv2.physical_type()];
                DataChunk::new(arrow_fields, arrow_field_types).with_names(vec!["val".into()])
            },
        ];
        let result = p.execute(chunks).unwrap();
        assert_eq!(result.len(), 4, "expected 4 morsels from 2 chunks of 5 rows each");
        assert_eq!(result[2].get_value(0, 0), Some(akar_common::types::Value::Int64(5)));
        assert_eq!(result[2].get_value(0, 2), Some(akar_common::types::Value::Int64(7)));
        assert_eq!(result[3].get_value(0, 0), Some(akar_common::types::Value::Int64(8)));
    }
}
