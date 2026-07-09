use crate::parquet_reader::{read_parquet, ParquetReaderError};
use kuzu_catalog::CatalogColumn;
use kuzu_common::types::Value;
use kuzu_common::file_system::VirtualFileSystemRegistry;
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

/// Scan state for IceDiskRelTable to cache and track reading progress.
pub struct IceDiskRelTableScanState {
    pub cached_batch_data: Vec<Vec<Value>>,
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

    /// Scan the indices parquet file into memory matching the requested schema.
    pub fn scan_indices(&self, vfs: &VirtualFileSystemRegistry, columns: &[CatalogColumn]) -> Result<IceDiskRelTableScanState, ParquetReaderError> {
        let data = read_parquet(self.indices_file_path.to_str().unwrap(), vfs, columns)?;
        Ok(IceDiskRelTableScanState {
            cached_batch_data: data,
            current_row: 0,
        })
    }
    
    /// Scan the indptr parquet file (if using CSR layout).
    pub fn scan_indptr(&self, vfs: &VirtualFileSystemRegistry, columns: &[CatalogColumn]) -> Result<Vec<Vec<Value>>, ParquetReaderError> {
        if let Some(path) = &self.indptr_file_path {
            read_parquet(path.to_str().unwrap(), vfs, columns)
        } else {
            Ok(vec![])
        }
    }
}

impl IceDiskRelTableScanState {
    /// Read the next row from the cached indices batch.
    pub fn next_row(&mut self) -> Option<&Vec<Value>> {
        if self.current_row < self.cached_batch_data.len() {
            let row = &self.cached_batch_data[self.current_row];
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
        let base = Path::new("/tmp/kuzu");
        
        let flat_table = IceDiskRelTable::new("knows".into(), base, IceDiskRelTableLayout::Flat);
        assert_eq!(flat_table.indices_file_path.to_str().unwrap(), "/tmp/kuzu\\knows.flat.parquet");
        assert!(flat_table.indptr_file_path.is_none());
        
        let csr_table = IceDiskRelTable::new("study_at".into(), base, IceDiskRelTableLayout::Csr);
        assert_eq!(csr_table.indices_file_path.to_str().unwrap(), "/tmp/kuzu\\study_at.indices.parquet");
        assert_eq!(csr_table.indptr_file_path.unwrap().to_str().unwrap(), "/tmp/kuzu\\study_at.indptr.parquet");
    }
}
