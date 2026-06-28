//! File system abstraction layer.
//!
//! Provides a unified interface for reading/writing data across
//! local filesystem, HTTP, and other backends.

use std::io::{Read, Seek, Write};
use std::path::Path;

/// A generic file system interface.
pub trait FileSystem: Send + Sync {
    /// Open a file for reading.
    fn open_read(&self, path: &Path) -> std::io::Result<Box<dyn FileRead>>;

    /// Open a file for writing (creates or truncates).
    fn open_write(&self, path: &Path) -> std::io::Result<Box<dyn FileWrite>>;

    /// Check if a path exists.
    fn exists(&self, path: &Path) -> bool;

    /// Remove a file.
    fn remove(&self, path: &Path) -> std::io::Result<()>;

    /// Create a directory and all parents.
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()>;
}

/// A readable file handle.
pub trait FileRead: Read + Seek + Send {}

/// A writable file handle.
pub trait FileWrite: Write + Seek + Send {}

/// Local filesystem implementation.
#[derive(Default)]
pub struct LocalFileSystem;

impl FileSystem for LocalFileSystem {
    fn open_read(&self, path: &Path) -> std::io::Result<Box<dyn FileRead>> {
        Ok(Box::new(std::fs::File::open(path)?))
    }

    fn open_write(&self, path: &Path) -> std::io::Result<Box<dyn FileWrite>> {
        Ok(Box::new(std::fs::File::create(path)?))
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }
}

impl FileRead for std::fs::File {}
impl FileWrite for std::fs::File {}
