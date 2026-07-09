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

/// Partitioner — morsel-driven parallelism operator.
///
/// In a full implementation, this partitions input data into morsels
/// (batches of rows) for parallel execution by downstream operators.
/// Currently acts as a pass-through.
pub struct Partitioner;

impl PhysicalOperatorExec for Partitioner {
    fn operator_type(&self) -> &str {
        "partitioner"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}

/// PackedExtend — optimized multi-rel extend.
///
/// Extends from multiple relationships in a single pass, producing
/// packed columns. Currently acts as pass-through (caller must handle
/// the actual extend logic externally).
pub struct PackedExtend;

impl PhysicalOperatorExec for PackedExtend {
    fn operator_type(&self) -> &str {
        "packed_extend"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}

/// PathPropertyProbe — resolves properties on path-typed results.
///
/// Given a path (sequence of nodes and rels), probes each element's
/// properties from storage. Currently acts as pass-through.
pub struct PathPropertyProbe;

impl PhysicalOperatorExec for PathPropertyProbe {
    fn operator_type(&self) -> &str {
        "path_property_probe"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}


/// AggregateFinalize — finalizes a split aggregate computation.
///
/// In C++ Kuzu, aggregates are split into AGGREGATE_FINALIZE and AGGREGATE_SCAN
/// for better pipelining. Currently acts as pass-through since the Rust
/// PhysicalAggregate handles both phases in one operator.
pub struct AggregateFinalize;

impl PhysicalOperatorExec for AggregateFinalize {
    fn operator_type(&self) -> &str {
        "aggregate_finalize"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}
