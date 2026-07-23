use crate::file_system::{FileRead, FileSystem, FileWrite};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::io::{Read, Seek, Write};

/// A virtual file system that transparently compresses/decompresses
/// files whose path ends in `.gz`.
///
/// Delegates actual I/O to an inner `FileSystem` (typically
/// [`LocalFileSystem`](crate::file_system::LocalFileSystem)) and wraps
/// the resulting reader/writer with gzip streaming.
///
/// # Example
///
/// ```no_run
/// use akar_common::file_system::{LocalFileSystem, VirtualFileSystemRegistry};
/// use akar_common::gzip_file_system::GzipFileSystem;
///
/// let vfs = VirtualFileSystemRegistry::new();
/// vfs.register_file_system(Box::new(GzipFileSystem::new(Box::new(LocalFileSystem))));
/// ```
pub struct GzipFileSystem {
    inner: Box<dyn FileSystem>,
}

impl GzipFileSystem {
    /// Create a gzip-wrapping file system over the given inner FS.
    pub fn new(inner: Box<dyn FileSystem>) -> Self {
        Self { inner }
    }
}

impl FileSystem for GzipFileSystem {
    fn can_handle(&self, path: &str) -> bool {
        path.ends_with(".gz")
    }

    fn open_read(&self, path: &str) -> std::io::Result<Box<dyn FileRead>> {
        let inner_reader = self.inner.open_read(path)?;
        let decoder = GzDecoder::new(inner_reader);
        Ok(Box::new(GzipFileRead(decoder)))
    }

    fn open_write(&self, path: &str) -> std::io::Result<Box<dyn FileWrite>> {
        let inner_writer = self.inner.open_write(path)?;
        let encoder = GzEncoder::new(inner_writer, Compression::default());
        Ok(Box::new(GzipFileWrite(encoder)))
    }

    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }

    fn remove(&self, path: &str) -> std::io::Result<()> {
        self.inner.remove(path)
    }

    fn create_dir_all(&self, path: &str) -> std::io::Result<()> {
        self.inner.create_dir_all(path)
    }
}

/// Wraps a `GzDecoder` so it implements `FileRead`.
struct GzipFileRead(GzDecoder<Box<dyn FileRead>>);

impl Read for GzipFileRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for GzipFileRead {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "gzip does not support seeking",
        ))
    }
}

impl FileRead for GzipFileRead {}

/// Wraps a `GzEncoder` so it implements `FileWrite`.
struct GzipFileWrite(GzEncoder<Box<dyn FileWrite>>);

impl Write for GzipFileWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Seek for GzipFileWrite {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "gzip does not support seeking",
        ))
    }
}

impl FileWrite for GzipFileWrite {}
