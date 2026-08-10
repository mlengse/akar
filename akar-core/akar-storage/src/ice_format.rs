use crate::parquet_reader::{ParquetReaderError, ParquetStreamReader, read_parquet, stream_parquet};
use akar_catalog::CatalogColumn;
use akar_common::file_system::VirtualFileSystemRegistry;
use akar_common::types::Value;
use std::path::{Path, PathBuf};

/// Layout options for IceDiskRelTable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceDiskRelTableLayout {
    Flat,
    Csr,
}

/// The ICE (IceDisk) native disk format for relationship tables.
/// Based on Ladybug's IceDiskRelTable implementation, which stores
/// relational data directly in Parquet format files (`indices.parquet` and optionally `indptr.parquet`).
pub struct IceDiskRelTable {
    pub name: String,
    pub layout: IceDiskRelTableLayout,
    pub indices_file_path: PathBuf,
    pub indptr_file_path: Option<PathBuf>,
}

/// Scan state for IceDiskRelTable that streams rows on demand.
///
/// Instead of loading the entire Parquet file into a `Vec<Vec<Value>>`,
/// this state holds a streaming reader and buffers one batch at a time.
pub struct IceDiskRelTableScanState {
    pub stream: ParquetStreamReader,
    pub current_batch: Vec<Vec<Value>>,
    pub current_row: usize,
}

impl IceDiskRelTable {
    /// Initialize a new IceDiskRelTable pointing to its Parquet files.
    pub fn new(name: String, base_path: &Path, layout: IceDiskRelTableLayout) -> Self {
        let indices_path = match layout {
            IceDiskRelTableLayout::Flat => base_path.join(format!("{}.flat.parquet", name)),
            IceDiskRelTableLayout::Csr => base_path.join(format!("{}.indices.parquet", name)),
        };

        let indptr_path = match layout {
            IceDiskRelTableLayout::Flat => None,
            IceDiskRelTableLayout::Csr => Some(base_path.join(format!("{}.indptr.parquet", name))),
        };

        Self {
            name,
            layout,
            indices_file_path: indices_path,
            indptr_file_path: indptr_path,
        }
    }

    /// Scan the indices parquet file and return a streaming scan state.
    pub fn scan_indices(
        &self,
        vfs: &VirtualFileSystemRegistry,
        columns: &[CatalogColumn],
    ) -> Result<IceDiskRelTableScanState, ParquetReaderError> {
        let stream = stream_parquet(self.indices_file_path.to_str().unwrap(), vfs, columns)?;
        Ok(IceDiskRelTableScanState {
            stream,
            current_batch: Vec::new(),
            current_row: 0,
        })
    }

    /// Scan the indptr parquet file (if using CSR layout).
    pub fn scan_indptr(
        &self,
        vfs: &VirtualFileSystemRegistry,
        columns: &[CatalogColumn],
    ) -> Result<Vec<Vec<Value>>, ParquetReaderError> {
        if let Some(path) = &self.indptr_file_path {
            read_parquet(path.to_str().unwrap(), vfs, columns)
        } else {
            Ok(vec![])
        }
    }
}

impl IceDiskRelTableScanState {
    /// Read the next row from the streaming parquet reader.
    ///
    /// Rows are pulled from the underlying `ParquetStreamReader` one batch
    /// at a time, avoiding materialization of the entire dataset in memory.
    pub fn next_row(&mut self) -> Option<&Vec<Value>> {
        // Advance to next batch if current one is exhausted
        if self.current_row >= self.current_batch.len() {
            match self.stream.next() {
                Some(Ok(batch)) => {
                    self.current_batch = batch;
                    self.current_row = 0;
                }
                Some(Err(_)) | None => {
                    self.current_batch = Vec::new();
                    self.current_row = 0;
                    return None;
                }
            }
        }

        if self.current_row < self.current_batch.len() {
            let row = &self.current_batch[self.current_row];
            self.current_row += 1;
            Some(row)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ice_disk_paths() {
        let base = Path::new("/tmp/akar");

        let flat_table = IceDiskRelTable::new("knows".into(), base, IceDiskRelTableLayout::Flat);
        assert_eq!(flat_table.indices_file_path, base.join("knows.flat.parquet"));
        assert!(flat_table.indptr_file_path.is_none());

        let csr_table = IceDiskRelTable::new("study_at".into(), base, IceDiskRelTableLayout::Csr);
        assert_eq!(
            csr_table.indices_file_path,
            base.join("study_at.indices.parquet")
        );
        assert_eq!(
            csr_table.indptr_file_path,
            Some(base.join("study_at.indptr.parquet"))
        );
    }
}
