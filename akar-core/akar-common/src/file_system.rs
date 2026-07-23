//! File system abstraction layer.
//!
//! Provides a unified interface for reading/writing data across
//! local filesystem, HTTP, and other backends.

use std::io::{Read, Seek, Write};
use std::path::Path;

/// A generic file system interface.
pub trait FileSystem: Send + Sync {
    /// Check if this file system can handle the given path/URL.
    fn can_handle(&self, path: &str) -> bool;

    /// Open a file for reading.
    fn open_read(&self, path: &str) -> std::io::Result<Box<dyn FileRead>>;

    /// Open a file for writing (creates or truncates).
    fn open_write(&self, path: &str) -> std::io::Result<Box<dyn FileWrite>>;

    /// Check if a path exists.
    fn exists(&self, path: &str) -> bool;

    /// Remove a file.
    fn remove(&self, path: &str) -> std::io::Result<()>;

    /// Create a directory and all parents.
    fn create_dir_all(&self, path: &str) -> std::io::Result<()>;
}

/// A readable file handle.
pub trait FileRead: Read + Seek + Send {}

/// A writable file handle.
pub trait FileWrite: Write + Seek + Send {}

/// Local filesystem implementation.
#[derive(Default)]
pub struct LocalFileSystem;

impl FileSystem for LocalFileSystem {
    fn can_handle(&self, _path: &str) -> bool {
        // LocalFileSystem is the fallback/default for non-URL paths.
        true
    }

    fn open_read(&self, path: &str) -> std::io::Result<Box<dyn FileRead>> {
        Ok(Box::new(std::fs::File::open(Path::new(path))?))
    }

    fn open_write(&self, path: &str) -> std::io::Result<Box<dyn FileWrite>> {
        Ok(Box::new(std::fs::File::create(Path::new(path))?))
    }

    fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn remove(&self, path: &str) -> std::io::Result<()> {
        std::fs::remove_file(Path::new(path))
    }

    fn create_dir_all(&self, path: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(Path::new(path))
    }
}

impl FileRead for std::fs::File {}
impl FileWrite for std::fs::File {}

use std::sync::RwLock;

/// Registry for virtual file systems.
pub struct VirtualFileSystemRegistry {
    systems: RwLock<Vec<Box<dyn FileSystem>>>,
    default_fs: LocalFileSystem,
}

impl Default for VirtualFileSystemRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualFileSystemRegistry {
    pub fn new() -> Self {
        Self {
            systems: RwLock::new(Vec::new()),
            default_fs: LocalFileSystem,
        }
    }

    pub fn register_file_system(&self, fs: Box<dyn FileSystem>) {
        let mut systems = self.systems.write().unwrap();
        systems.push(fs);
    }

    pub fn open_read(&self, path: &str) -> std::io::Result<Box<dyn FileRead>> {
        let systems = self.systems.read().unwrap();
        for fs in systems.iter() {
            if fs.can_handle(path) {
                return fs.open_read(path);
            }
        }
        self.default_fs.open_read(path)
    }

    pub fn open_write(&self, path: &str) -> std::io::Result<Box<dyn FileWrite>> {
        let systems = self.systems.read().unwrap();
        for fs in systems.iter() {
            if fs.can_handle(path) {
                return fs.open_write(path);
            }
        }
        self.default_fs.open_write(path)
    }

    pub fn exists(&self, path: &str) -> bool {
        let systems = self.systems.read().unwrap();
        for fs in systems.iter() {
            if fs.can_handle(path) {
                return fs.exists(path);
            }
        }
        self.default_fs.exists(path)
    }

    pub fn remove(&self, path: &str) -> std::io::Result<()> {
        let systems = self.systems.read().unwrap();
        for fs in systems.iter() {
            if fs.can_handle(path) {
                return fs.remove(path);
            }
        }
        self.default_fs.remove(path)
    }

    pub fn create_dir_all(&self, path: &str) -> std::io::Result<()> {
        let systems = self.systems.read().unwrap();
        for fs in systems.iter() {
            if fs.can_handle(path) {
                return fs.create_dir_all(path);
            }
        }
        self.default_fs.create_dir_all(path)
    }
}
