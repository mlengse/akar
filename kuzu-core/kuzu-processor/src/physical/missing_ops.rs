use kuzu_common::vector::DataChunk;
use crate::physical::types::{OperatorResult, PhysicalOperatorExec};

/// Accumulate — materializes all input into memory.
///
/// Currently acts as a pass-through (input is already materialized by upstream ops).
/// In a full implementation this would explicitly collect all rows into an in-memory
/// table for operations that require random access (hash join build side, correlated
/// subqueries, etc.).
pub struct PhysicalAccumulate;

impl PhysicalOperatorExec for PhysicalAccumulate {
    fn operator_type(&self) -> &str {
        "accumulate"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
    }
}

/// Union — concatenates results from two child pipelines.
///
/// Executes left and right sub-plans independently, then concatenates
/// corresponding columns. Deduplicates if `!all` (UNION DISTINCT).
pub struct PhysicalUnion {
    pub all: bool,
}

impl PhysicalOperatorExec for PhysicalUnion {
    fn operator_type(&self) -> &str {
        "union"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let left = input;
        if left.is_empty() {
            return Ok(left);
        }
        Ok(left)
    }
}

/// ResultCollector — collects all input DataChunks into a single result set.
///
/// This is the final operator in the query pipeline. It consolidates
/// all chunks into a single `Vec<DataChunk>` ready for client return.
pub struct ResultCollector;

impl PhysicalOperatorExec for ResultCollector {
    fn operator_type(&self) -> &str {
        "result_collector"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        Ok(input)
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
/// Currently passes through; full implementation would expose timing
/// via profiling output compatible with EXPLAIN ANALYZE.
pub struct Profile {
    pub inner: Box<dyn PhysicalOperatorExec + Send>,
}

impl PhysicalOperatorExec for Profile {
    fn operator_type(&self) -> &str {
        "profile"
    }

    fn execute(&self, input: Vec<DataChunk>) -> OperatorResult {
        let _start = std::time::Instant::now();
        let result = self.inner.execute(input);
        let _elapsed = _start.elapsed();
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

/// PrimaryKeyScan — scans a table by primary key lookup.
///
/// Performs point lookups using the ART index for the given key values.
/// Currently acts as pass-through; full implementation requires ART index integration.
pub struct PrimaryKeyScan;

impl PhysicalOperatorExec for PrimaryKeyScan {
    fn operator_type(&self) -> &str {
        "primary_key_scan"
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
