//! External (disk-spilling) hash join — grace hash join with radix partitioning
//! (P111).
//!
//! [`PhysicalHashJoin::execute_binary`] builds the full build-side hash table in
//! memory, so a join whose build side exceeds the instance budget can OOM the
//! process. This module adds the external counterpart: when the build side does
//! not fit in the query's memory grant, both sides are radix-partitioned by the
//! join key onto disk, and each partition pair is joined independently with the
//! *existing in-memory join*.
//!
//! Reusing the in-memory join for the per-partition work is deliberate: the
//! output rows, column order, and key-equality rules are then identical by
//! construction rather than by a parallel reimplementation, which is exactly the
//! property that a second join algorithm would otherwise break silently.
//!
//! Layout of a spill run:
//!
//! ```text
//! <spill_dir>/join_<qid>_<side>_<pass>_<part>.bin   // Arrow IPC file
//! ```
//!
//! `pass` increments each time a partition is repartitioned because it still did
//! not fit ([`JoinSpillConfig::max_passes`]); `part` is the radix index within
//! that pass. The files are deleted when the join finishes.

use crate::physical::join_ops::{PhysicalHashJoin, hash_chunk_cell};
use crate::physical::types::OperatorResult;
use akar_common::arrow_vector::physical_type_from_arrow;
use akar_common::error::ProcessorError;
use akar_common::query_pool::{Grant, QueryMemoryPool};
use akar_common::vector::DataChunk;
use arrow::array::{Array, ArrayRef, RecordBatch, UInt32Array};
use arrow::datatypes::{Field, Schema};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// How many radix partitions the first pass creates, as a power of two.
///
/// 8 bits → 256 partitions. Enough that a build side several hundred times
/// larger than the grant still lands in fitting partitions, without creating so
/// many files that tiny inputs pay for a filesystem round trip per partition.
pub const DEFAULT_RADIX_BITS: u32 = 8;

/// Everything the external join needs beyond the two input sides.
#[derive(Debug, Clone)]
pub struct JoinSpillConfig {
    /// Directory partition files are written to. Created if missing.
    pub spill_dir: PathBuf,
    /// The query's memory pool: the build side of a partition must fit in
    /// [`QueryMemoryPool::remaining`] for the partition to be joined in memory.
    pub pool: Arc<QueryMemoryPool>,
    /// Radix bits per partitioning pass.
    pub radix_bits: u32,
    /// Maximum number of repartitioning passes before a partition is joined in
    /// memory regardless of its size, so a pathologically skewed key space
    /// degrades in memory instead of failing the query (P111.2).
    pub max_passes: u32,
    /// Hard ceiling on the number of partition files one join may write.
    ///
    /// Repartitioning stops once it is reached. This is what keeps the recursion
    /// total when the grant is simply too small to ever hold a partition: rather
    /// than covering the disk with files that can never fit, the remaining
    /// partitions are joined in memory.
    pub max_spill_files: usize,
}

impl JoinSpillConfig {
    /// A config with a 256-way first pass, 3 repartitioning passes and a 4096
    /// file ceiling.
    pub fn new(spill_dir: impl Into<PathBuf>, pool: Arc<QueryMemoryPool>) -> Self {
        Self {
            spill_dir: spill_dir.into(),
            pool,
            radix_bits: DEFAULT_RADIX_BITS,
            max_passes: 3,
            max_spill_files: 4096,
        }
    }

    /// Number of partitions the first pass creates.
    fn partitions(&self) -> usize {
        1usize << self.radix_bits.clamp(1, 16)
    }

    /// Radix partition count to advertise in EXPLAIN (`Spill=N`).
    pub fn explain_partitions(&self) -> usize {
        self.partitions()
    }

    /// Sub-partition count for a partition holding `bytes` of build side.
    ///
    /// Sized to the shortfall rather than fixed: a partition only slightly over
    /// the grant is split in two, not into 256 files. Capped at the first-pass
    /// count so deeper levels never write more files than the first one.
    fn sub_partitions(&self, bytes: u64) -> usize {
        let remaining = self.pool.remaining();
        if remaining == 0 {
            return self.partitions();
        }
        let needed = bytes.div_ceil(remaining).max(2);
        (needed as usize).next_power_of_two().clamp(2, self.partitions())
    }
}

/// Bytes of Arrow memory a set of chunks currently holds.
fn estimate_bytes(chunks: &[DataChunk]) -> u64 {
    chunks
        .iter()
        .flat_map(|c| c.fields.iter())
        .map(|f| f.get_array_memory_size() as u64)
        .sum()
}

impl PhysicalHashJoin {
    /// Join, spilling to disk when the build side does not fit the memory grant.
    ///
    /// Falls back to [`PhysicalHashJoin::execute_binary`] — byte for byte the
    /// same result, and no filesystem access at all — whenever the build side
    /// fits the grant, so joins that were never at risk of OOM keep their
    /// existing execution path and cost.
    pub fn execute_with_spill(
        &self,
        build_chunks: &[DataChunk],
        probe_chunks: &[DataChunk],
        cfg: &JoinSpillConfig,
    ) -> OperatorResult {
        if build_chunks.is_empty() || probe_chunks.is_empty() {
            return Ok(vec![]);
        }

        let build_bytes = estimate_bytes(build_chunks);
        if build_bytes <= cfg.pool.remaining() {
            // Fits: account for it briefly so the reservation is visible to
            // concurrent queries, then hand back to the in-memory path.
            match cfg.pool.try_reserve(build_bytes) {
                Grant::Granted => {
                    let result = self.execute_binary(build_chunks, probe_chunks);
                    cfg.pool.release(build_bytes);
                    return result;
                }
                Grant::Exhausted => { /* fall through to the external path */ }
            }
        }

        self.execute_external(build_chunks, probe_chunks, cfg)
    }

    /// Unconditionally use the partitioned external path.
    fn execute_external(
        &self,
        build_chunks: &[DataChunk],
        probe_chunks: &[DataChunk],
        cfg: &JoinSpillConfig,
    ) -> OperatorResult {
        std::fs::create_dir_all(&cfg.spill_dir)
            .map_err(|e| ProcessorError::Io(format!("Cannot create spill directory: {e}")))?;

        let build_key = self.build_columns.first().copied().unwrap_or(0) as usize;
        let probe_key = self.probe_columns.first().copied().unwrap_or(0) as usize;

        let partitions = cfg.partitions();
        let mut files = PartitionFiles::default();
        let mut out: Vec<DataChunk> = Vec::new();

        // Pass 0: partition the in-memory inputs straight to disk.
        let (build_parts, probe_parts) = partition_to_files(
            cfg,
            0,
            partitions,
            build_chunks,
            build_key,
            probe_chunks,
            probe_key,
            &mut files,
        )?;

        for part in 0..partitions {
            join_partition(
                self,
                cfg,
                &[build_parts[part].clone()],
                &[probe_parts[part].clone()],
                1,
                &mut files,
                &mut out,
            )?;
        }

        // Every partition file is consumed; remove the run.
        files.cleanup();

        // Name every output chunk from the original inputs (mirroring what
        // `execute_binary` does for its single result chunk), so operators above
        // the join resolve `b.id` / `p.id` regardless of which partition a row
        // came out of.
        if !out.is_empty() {
            let mut names: Vec<String> = build_chunks.first().map(|c| c.field_names.clone()).unwrap_or_default();
            names.extend(probe_chunks.first().map(|c| c.field_names.clone()).unwrap_or_default());
            if !names.is_empty() {
                for chunk in &mut out {
                    chunk.field_names = names.clone();
                }
            }
        }
        Ok(out)
    }
}

/// Bookkeeping for the files a spill run created, so they are removed even when
/// a later pass returns an error.
#[derive(Default)]
struct PartitionFiles {
    written: Vec<PathBuf>,
}

impl PartitionFiles {
    fn register(&mut self, path: PathBuf) {
        self.written.push(path);
    }

    /// Number of partition files written so far in this run.
    fn len(&self) -> usize {
        self.written.len()
    }

    /// Remove every file written by this run.
    fn cleanup(&mut self) {
        for path in self.written.drain(..) {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for PartitionFiles {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Join one partition pair — in memory when it fits, otherwise repartition it
/// one level deeper and recurse (P111.2).
#[allow(clippy::too_many_arguments)]
fn join_partition(
    join: &PhysicalHashJoin,
    cfg: &JoinSpillConfig,
    build_files: &[PathBuf],
    probe_files: &[PathBuf],
    pass: u32,
    files: &mut PartitionFiles,
    out: &mut Vec<DataChunk>,
) -> Result<(), ProcessorError> {
    let build_chunks = read_files(build_files)?;
    let probe_chunks = read_files(probe_files)?;

    let build_bytes = estimate_bytes(&build_chunks);
    let remaining = cfg.pool.remaining();

    // Terminate whenever the partition can plausibly be joined in memory:
    // - it fits the grant;
    // - there is no grant at all, so no amount of partitioning can converge;
    // - the file ceiling or the pass budget is reached;
    // - the probe side is empty, so there is nothing to look up.
    if build_bytes <= remaining
        || remaining == 0
        || pass >= cfg.max_passes
        || files.len() >= cfg.max_spill_files
        || probe_chunks.is_empty()
    {
        out.extend(join.execute_binary(&build_chunks, &probe_chunks)?);
        return Ok(());
    }

    // Still too large: split this partition, keyed by the same join key so
    // matching rows stay together. The sub-partition count is sized to the
    // shortfall, so one extra level is normally enough.
    let build_key = join.build_columns.first().copied().unwrap_or(0) as usize;
    let probe_key = join.probe_columns.first().copied().unwrap_or(0) as usize;
    let partitions = cfg.sub_partitions(build_bytes);

    let (sub_build, sub_probe) = partition_to_files(
        cfg,
        pass,
        partitions,
        &build_chunks,
        build_key,
        &probe_chunks,
        probe_key,
        files,
    )?;

    for part in 0..partitions {
        join_partition(
            join,
            cfg,
            &[sub_build[part].clone()],
            &[sub_probe[part].clone()],
            pass + 1,
            files,
            out,
        )?;
    }
    Ok(())
}

/// Radix-partition both sides onto disk by join-key hash.
///
/// Returns, per radix index, the single file holding that partition's rows for
/// the build and probe side. Rows whose key is NULL are dropped, matching the
/// in-memory hash table, which never indexes a null key.
#[allow(clippy::too_many_arguments)]
fn partition_to_files(
    cfg: &JoinSpillConfig,
    pass: u32,
    partitions: usize,
    build_chunks: &[DataChunk],
    build_key: usize,
    probe_chunks: &[DataChunk],
    probe_key: usize,
    files: &mut PartitionFiles,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), ProcessorError> {
    let build = split_by_radix(build_chunks, build_key, partitions);
    let probe = split_by_radix(probe_chunks, probe_key, partitions);

    let mut build_paths = Vec::with_capacity(partitions);
    let mut probe_paths = Vec::with_capacity(partitions);

    for part in 0..partitions {
        let bpath = partition_path(cfg, "build", pass, part);
        write_partition(&bpath, std::slice::from_ref(&build[part]))?;
        files.register(bpath.clone());
        build_paths.push(bpath);

        let ppath = partition_path(cfg, "probe", pass, part);
        write_partition(&ppath, std::slice::from_ref(&probe[part]))?;
        files.register(ppath.clone());
        probe_paths.push(ppath);
    }

    // One spill event per partitioning pass that reached disk. This is the
    // count EXPLAIN's `Spill=N` describes and what
    // `QueryMemoryPool::spill_events` reports.
    cfg.pool.note_spill();
    Ok((build_paths, probe_paths))
}

/// Name of one partition file: `join_<qid>_<side>_<pass>_<part>.bin`.
fn partition_path(cfg: &JoinSpillConfig, side: &str, pass: u32, part: usize) -> PathBuf {
    cfg.spill_dir
        .join(format!("join_{}_{side}_{pass}_{part}.bin", cfg.pool.query_id()))
}

/// Group rows of `chunks` into `partitions` buckets by the hash of `key_col`.
///
/// Returns one `DataChunk` per bucket, holding only the columns given by
/// `take`. Null-key rows are excluded, and so are rows in buckets with no rows.
fn split_by_radix(chunks: &[DataChunk], key_col: usize, partitions: usize) -> Vec<DataChunk> {
    let mask = partitions - 1;
    // Row indices per bucket, global across all input chunks.
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); partitions];
    let mut offsets: Vec<usize> = Vec::with_capacity(chunks.len());
    let mut total = 0usize;
    for chunk in chunks {
        offsets.push(total);
        total += chunk.size;
    }
    for (ci, chunk) in chunks.iter().enumerate() {
        for row in 0..chunk.size {
            if let Some(hash) = hash_chunk_cell(chunk, key_col, row) {
                buckets[(hash as usize) & mask].push((offsets[ci] + row) as u32);
            }
        }
    }

    let num_cols = chunks.first().map(|c| c.num_fields()).unwrap_or(0);
    let field_types = chunks.first().map(|c| c.field_types.clone()).unwrap_or_default();
    // Column names must survive the disk trip: operators above the join resolve
    // `b.id` / `p.id` against `field_names`, so a partition that lost them would
    // break every query whose projection references the joined variables.
    let field_names = chunks.first().map(|c| c.field_names.clone()).unwrap_or_default();

    // Concatenate each column once, then `take` every bucket out of it.
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(num_cols);
    for col in 0..num_cols {
        let parts: Vec<ArrayRef> = chunks.iter().map(|c| c.fields[col].clone()).collect();
        columns.push(concat_or_first(parts));
    }

    buckets
        .into_iter()
        .map(|indices| {
            let take = UInt32Array::from(indices);
            let fields: Vec<ArrayRef> = columns
                .iter()
                .map(|col| arrow::compute::take(col.as_ref(), &take, None).unwrap_or_else(|_| col.clone()))
                .collect();
            let size = fields.first().map(|f| f.len()).unwrap_or(0);
            DataChunk {
                fields,
                field_types: field_types.clone(),
                size,
                field_names: field_names.clone(),
                sel_vector: None,
            }
        })
        .collect()
}

fn concat_or_first(parts: Vec<ArrayRef>) -> ArrayRef {
    if parts.len() == 1 {
        return parts.into_iter().next().unwrap();
    }
    let refs: Vec<&dyn Array> = parts.iter().map(|a| a.as_ref()).collect();
    arrow::compute::concat(&refs).unwrap_or_else(|_| parts[0].clone())
}

/// Write a partition to an Arrow IPC file.
///
/// An empty partition still gets a file: the reader then yields no batches,
/// which is exactly the "this side contributed nothing" case.
fn write_partition(path: &Path, chunks: &[DataChunk]) -> Result<(), ProcessorError> {
    let schema = Arc::new(schema_for(chunks));
    let file = std::fs::File::create(path)
        .map_err(|e| ProcessorError::Io(format!("Cannot create spill file '{}': {e}", path.display())))?;
    let mut writer = arrow::ipc::writer::FileWriter::try_new(file, &schema)
        .map_err(|e| ProcessorError::Io(format!("Cannot start spill writer: {e}")))?;

    for chunk in chunks {
        if chunk.size == 0 {
            continue;
        }
        let batch = RecordBatch::try_new(schema.clone(), chunk.fields.clone())
            .map_err(|e| ProcessorError::Io(format!("Cannot build spill batch: {e}")))?;
        writer
            .write(&batch)
            .map_err(|e| ProcessorError::Io(format!("Cannot write spill batch: {e}")))?;
    }
    writer
        .finish()
        .map_err(|e| ProcessorError::Io(format!("Cannot finish spill file: {e}")))?;
    Ok(())
}

/// Read a partition back into chunks, preserving physical types.
fn read_files(paths: &[PathBuf]) -> Result<Vec<DataChunk>, ProcessorError> {
    let mut chunks = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let file = std::fs::File::open(path)
            .map_err(|e| ProcessorError::Io(format!("Cannot open spill file '{}': {e}", path.display())))?;
        let reader = arrow::ipc::reader::FileReader::try_new(file, None)
            .map_err(|e| ProcessorError::Io(format!("Cannot read spill file '{}': {e}", path.display())))?;

        let names: Vec<String> = reader.schema().fields().iter().map(|f| f.name().clone()).collect();

        for batch in reader {
            let batch =
                batch.map_err(|e| ProcessorError::Io(format!("Corrupt spill file '{}': {e}", path.display())))?;
            let field_types = batch
                .schema()
                .fields()
                .iter()
                .map(|f| physical_type_from_arrow(f.data_type()))
                .collect();
            let size = batch.num_rows();
            chunks.push(DataChunk {
                fields: batch.columns().to_vec(),
                field_types,
                size,
                field_names: names.clone(),
                sel_vector: None,
            });
        }
    }
    Ok(chunks)
}

/// Arrow schema matching the columns of `chunks`.
///
/// Field names carry the chunk's `field_names` so they survive the disk trip;
/// positional fallbacks keep the schema construction total for inputs that carry
/// no names.
fn schema_for(chunks: &[DataChunk]) -> Schema {
    let Some(first) = chunks.first() else {
        return Schema::empty();
    };
    let fields: Vec<Field> = first
        .fields
        .iter()
        .enumerate()
        .map(|(idx, f)| {
            let name = first.field_names.get(idx).cloned().unwrap_or_else(|| format!("c{idx}"));
            Field::new(name, f.data_type().clone(), true)
        })
        .collect();
    Schema::new(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::types::PhysicalTypeID;
    use akar_common::vector::ValueVector;
    use std::collections::BTreeMap;

    /// Two Int64 columns: column 0 is a payload, column 1 is the join key.
    fn two_col_chunk(rows: &[(i64, i64)]) -> DataChunk {
        let mut payload = ValueVector::new(PhysicalTypeID::Int64, rows.len().max(1));
        let mut key = ValueVector::new(PhysicalTypeID::Int64, rows.len().max(1));
        for (i, (p, k)) in rows.iter().enumerate() {
            payload.set_i64(i, *p);
            key.set_i64(i, *k);
        }
        payload.resize(rows.len());
        key.resize(rows.len());
        let ptype = payload.physical_type();
        let fields = vec![
            akar_common::arrow_vector::ArrowVector::from_legacy(&payload).array,
            akar_common::arrow_vector::ArrowVector::from_legacy(&key).array,
        ];
        let mut chunk = DataChunk::new(fields, vec![ptype, ptype]);
        chunk.field_names = vec!["payload".into(), "key".into()];
        chunk
    }

    /// Result rows as a sorted multiset, so a comparison does not depend on the
    /// order partitions happen to be processed in.
    fn sorted_rows(chunks: &[DataChunk]) -> Vec<Vec<String>> {
        let mut rows: Vec<Vec<String>> = Vec::new();
        for chunk in chunks {
            for row in chunk.iter_rows() {
                let mut vals = Vec::new();
                for col in 0..chunk.fields.len() {
                    vals.push(match chunk.get_value(col, row) {
                        Some(v) => format!("{v:?}"),
                        None => "null".to_string(),
                    });
                }
                rows.push(vals);
            }
        }
        rows.sort();
        rows
    }

    /// 300 build rows keyed 0..300 and 400 probe rows drawing from the same key
    /// space, so a correct partitioner must produce matches in every partition.
    fn fixture() -> (Vec<DataChunk>, Vec<DataChunk>) {
        let build_rows: Vec<(i64, i64)> = (0..300).map(|i| (i * 10, i)).collect();
        let probe_rows: Vec<(i64, i64)> = (0..400).map(|i| (i, (i * 7) % 300)).collect();
        let build = vec![two_col_chunk(&build_rows[..150]), two_col_chunk(&build_rows[150..])];
        let probe = vec![
            two_col_chunk(&probe_rows[..90]),
            two_col_chunk(&probe_rows[90..310]),
            two_col_chunk(&probe_rows[310..]),
        ];
        (build, probe)
    }

    fn spill_files(dir: &Path) -> Vec<String> {
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                names
            }
            Err(_) => Vec::new(),
        }
    }

    #[test]
    fn join_within_the_grant_does_not_touch_disk() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(QueryMemoryPool::new(1, u64::MAX));
        let cfg = JoinSpillConfig::new(dir.path(), pool.clone());
        let (build, probe) = fixture();
        let join = PhysicalHashJoin::new(vec![1], vec![1]);

        let spilled = join.execute_with_spill(&build, &probe, &cfg).unwrap();
        let in_memory = join.execute_binary(&build, &probe).unwrap();

        assert_eq!(sorted_rows(&spilled), sorted_rows(&in_memory));
        assert_eq!(pool.spill_events(), 0, "a join that fits must not spill");
        assert!(spill_files(dir.path()).is_empty(), "no spill files expected");
    }

    #[test]
    fn spilled_join_matches_the_in_memory_join() {
        let dir = tempfile::tempdir().unwrap();
        // A grant far below the build side forces the external path.
        let pool = Arc::new(QueryMemoryPool::new(2, 4096));
        let cfg = JoinSpillConfig::new(dir.path(), pool.clone());
        let (build, probe) = fixture();
        let join = PhysicalHashJoin::new(vec![1], vec![1]);

        let spilled = join.execute_with_spill(&build, &probe, &cfg).unwrap();
        let in_memory = join.execute_binary(&build, &probe).unwrap();

        let expected = sorted_rows(&in_memory);
        assert!(!expected.is_empty(), "fixture must produce matches");
        assert_eq!(
            sorted_rows(&spilled),
            expected,
            "the external join must return exactly the in-memory join's rows"
        );
        assert!(pool.spill_events() > 0, "the external path must report a spill");
        assert!(
            spill_files(dir.path()).is_empty(),
            "spill files must be removed when the join finishes: {:?}",
            spill_files(dir.path())
        );
    }

    #[test]
    fn spill_files_are_named_for_the_query_and_side() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(QueryMemoryPool::new(4242, 0));
        let cfg = JoinSpillConfig::new(dir.path(), pool);

        // Spill files are removed as soon as the join finishes, so the naming
        // scheme is pinned on the constructor rather than on a directory listing.
        let build0 = partition_path(&cfg, "build", 0, 7);
        assert_eq!(build0.file_name().unwrap(), "join_4242_build_0_7.bin");
        assert_eq!(build0.parent(), Some(dir.path()));

        let probe2 = partition_path(&cfg, "probe", 2, 255);
        assert_eq!(probe2.file_name().unwrap(), "join_4242_probe_2_255.bin");
    }

    #[test]
    fn joins_a_partitioned_side_split_across_chunks() {
        // Rows for one key are split across several input chunks; the radix
        // partitioner must still bring them together in one partition file.
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(QueryMemoryPool::new(3, 1024));
        let cfg = JoinSpillConfig::new(dir.path(), pool.clone());
        let build = vec![
            two_col_chunk(&[(1, 5)]),
            two_col_chunk(&[(2, 5)]),
            two_col_chunk(&[(3, 5)]),
        ];
        let probe = vec![two_col_chunk(&[(9, 5)])];
        let join = PhysicalHashJoin::new(vec![1], vec![1]);

        let spilled = join.execute_with_spill(&build, &probe, &cfg).unwrap();
        let in_memory = join.execute_binary(&build, &probe).unwrap();
        assert_eq!(sorted_rows(&spilled), sorted_rows(&in_memory));
        assert_eq!(sorted_rows(&spilled).len(), 3, "one output row per build match");
    }

    #[test]
    fn multi_pass_repartitioning_still_returns_the_same_rows() {
        let dir = tempfile::tempdir().unwrap();
        // A one-byte grant can never hold a partition, so every level keeps
        // repartitioning until the file ceiling stops it. The result must still
        // be the join's true answer.
        let pool = Arc::new(QueryMemoryPool::new(4, 1));
        let mut cfg = JoinSpillConfig::new(dir.path(), pool.clone());
        // Small fan-out and a low ceiling keep the test quick while still going
        // through the first pass, at least one deeper pass, and the ceiling.
        cfg.radix_bits = 2;
        cfg.max_spill_files = 24;
        let (build, probe) = fixture();
        let join = PhysicalHashJoin::new(vec![1], vec![1]);

        let spilled = join.execute_with_spill(&build, &probe, &cfg).unwrap();
        let in_memory = join.execute_binary(&build, &probe).unwrap();
        assert_eq!(sorted_rows(&spilled), sorted_rows(&in_memory));
        assert!(
            pool.spill_events() >= 2,
            "the first pass plus at least one deeper pass must both spill, saw {}",
            pool.spill_events()
        );
        assert!(spill_files(dir.path()).is_empty());
    }

    #[test]
    fn zero_grant_falls_back_to_an_in_memory_join() {
        let dir = tempfile::tempdir().unwrap();
        // With no grant at all there is nothing to partition toward: the join
        // runs in memory rather than spinning through repartitioning passes.
        let pool = Arc::new(QueryMemoryPool::new(5, 0));
        let cfg = JoinSpillConfig::new(dir.path(), pool);
        let (build, probe) = fixture();
        let join = PhysicalHashJoin::new(vec![1], vec![1]);

        let spilled = join.execute_with_spill(&build, &probe, &cfg).unwrap();
        let in_memory = join.execute_binary(&build, &probe).unwrap();
        assert_eq!(sorted_rows(&spilled), sorted_rows(&in_memory));
    }

    #[test]
    fn null_keys_never_match_on_either_path() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(QueryMemoryPool::new(6, 256));
        let cfg = JoinSpillConfig::new(dir.path(), pool);
        let join = PhysicalHashJoin::new(vec![1], vec![1]);

        // Key column all-null on both sides.
        let mut build = two_col_chunk(&[(1, 0), (2, 0)]);
        let mut probe = two_col_chunk(&[(3, 0), (4, 0)]);
        for chunk in [&mut build, &mut probe] {
            let nulls = arrow::array::new_null_array(&arrow::datatypes::DataType::Int64, chunk.size);
            chunk.fields[1] = nulls;
        }

        let spilled = join
            .execute_with_spill(&[build.clone()], &[probe.clone()], &cfg)
            .unwrap();
        let in_memory = join.execute_binary(&[build], &[probe]).unwrap();
        assert!(sorted_rows(&in_memory).is_empty());
        assert_eq!(sorted_rows(&spilled), sorted_rows(&in_memory));
    }

    #[test]
    fn empty_inputs_produce_no_output_and_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(QueryMemoryPool::new(7, 0));
        let cfg = JoinSpillConfig::new(dir.path(), pool.clone());
        let join = PhysicalHashJoin::new(vec![1], vec![1]);

        assert!(join.execute_with_spill(&[], &[], &cfg).unwrap().is_empty());
        let probe = vec![two_col_chunk(&[(1, 1)])];
        assert!(join.execute_with_spill(&[], &probe, &cfg).unwrap().is_empty());
        let build = vec![two_col_chunk(&[(1, 1)])];
        assert!(join.execute_with_spill(&build, &[], &cfg).unwrap().is_empty());
        assert_eq!(pool.spill_events(), 0);
        assert!(spill_files(dir.path()).is_empty());
    }

    /// The radix splitter's contract: every non-null key lands in exactly one
    /// partition, and equal keys land in the same one.
    #[test]
    fn radix_split_is_a_partition_of_the_rows() {
        let (build, _) = fixture();
        let parts = split_by_radix(&build, 1, 8);
        assert_eq!(parts.len(), 8);

        let mut seen: BTreeMap<i64, usize> = BTreeMap::new();
        let mut total = 0usize;
        for (part, chunk) in parts.iter().enumerate() {
            for row in 0..chunk.size {
                let key = chunk.get_i64(1, row).unwrap();
                seen.insert(key, part);
                total += 1;
            }
        }
        assert_eq!(total, 300, "every non-null row must appear exactly once");
        // Re-splitting must be deterministic: the same key, the same partition.
        let again = split_by_radix(&build, 1, 8);
        for (part, chunk) in again.iter().enumerate() {
            for row in 0..chunk.size {
                let key = chunk.get_i64(1, row).unwrap();
                assert_eq!(seen[&key], part, "key {key} moved between splits");
            }
        }
    }
}
