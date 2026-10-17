//! Optional owning memory-mapped demo input.

use std::fs::File;
use std::path::Path;

use memmap2::{Mmap, MmapOptions};

use crate::demo::{Demo, DemoIndex};
use crate::error::Result;
use crate::limits::DecodeLimits;
use crate::playback::DemoParser;

/// An owning, read-only memory map of one validated PBDEMS2 file.
///
/// This keeps large demos out of heap-backed `Vec<u8>` allocations while the
/// existing [`Demo`] and [`DemoParser`] APIs continue to operate on borrowed
/// slices. Views returned by [`MappedDemo::demo`] and [`MappedDemo::parser`]
/// cannot outlive the mapping.
///
/// # Example
///
/// ```no_run
/// # use pbdems2::MappedDemo;
/// // SAFETY: nothing else writes to or truncates this file while it is mapped.
/// let mapped = unsafe { MappedDemo::open("match.dem")? };
/// let ticks = mapped.demo()?.index()?.distinct_ticks().len();
/// # Ok::<(), pbdems2::Error>(())
/// ```
#[derive(Debug)]
pub struct MappedDemo {
    mapping: Mmap,
    limits: DecodeLimits,
}

impl MappedDemo {
    /// Open, map, and validate a demo file with default decode limits.
    ///
    /// # Safety
    ///
    /// The caller must ensure the mapped file is not modified or truncated by
    /// any process for the lifetime of the returned value. See
    /// [`MmapOptions::map`] for the platform-specific file-backed mapping
    /// requirements.
    pub unsafe fn open(path: impl AsRef<Path>) -> Result<Self> {
        // SAFETY: The caller accepts the same file-stability requirements as
        // `open_with_limits`.
        unsafe { Self::open_with_limits(path, DecodeLimits::default()) }
    }

    /// Open, map, and validate a demo file with explicit decode limits.
    ///
    /// # Safety
    ///
    /// The caller must ensure the mapped file is not modified or truncated by
    /// any process for the lifetime of the returned value. See
    /// [`MmapOptions::map`] for the complete safety contract.
    pub unsafe fn open_with_limits(path: impl AsRef<Path>, limits: DecodeLimits) -> Result<Self> {
        let file = File::open(path)?;
        // SAFETY: This method forwards `MmapOptions::map`'s file-stability
        // requirements to its caller.
        let mapping = unsafe { MmapOptions::new().map(&file)? };
        Self::from_mmap_with_limits(mapping, limits)
    }

    /// Validate and own an existing read-only mapping using default limits.
    pub fn from_mmap(mapping: Mmap) -> Result<Self> {
        Self::from_mmap_with_limits(mapping, DecodeLimits::default())
    }

    /// Validate and own an existing read-only mapping using explicit limits.
    pub fn from_mmap_with_limits(mapping: Mmap, limits: DecodeLimits) -> Result<Self> {
        Demo::with_limits(&mapping, limits)?;
        Ok(Self { mapping, limits })
    }

    /// Complete encoded demo bytes backed directly by the file mapping.
    pub fn as_bytes(&self) -> &[u8] {
        &self.mapping
    }

    /// Encoded file size in bytes.
    pub fn len(&self) -> usize {
        self.mapping.len()
    }

    /// Whether the mapped file is empty.
    ///
    /// A successfully constructed `MappedDemo` is never empty because PBDEMS2
    /// header validation requires at least 16 bytes.
    pub fn is_empty(&self) -> bool {
        self.mapping.is_empty()
    }

    /// Decode limits inherited by borrowed demo and parser views.
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    /// Create a validated borrowed command-stream view of this mapping.
    pub fn demo(&self) -> Result<Demo<'_>> {
        Demo::with_limits(&self.mapping, self.limits)
    }

    /// Create a validated borrowed parser driver over this mapping.
    pub fn parser(&self) -> Result<DemoParser<'_>> {
        DemoParser::with_limits(&self.mapping, self.limits)
    }

    /// Build the header-only seek index without copying the mapped bytes.
    pub fn index(&self) -> Result<DemoIndex> {
        self.demo()?.index()
    }
}

impl AsRef<[u8]> for MappedDemo {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::demo::{HEADER_SIZE, MAGIC, command};

    fn fixture() -> Vec<u8> {
        let mut bytes = Vec::from(MAGIC);
        bytes.resize(HEADER_SIZE, 0);
        bytes.extend_from_slice(&[command::SYNC_TICK as u8, 0, 0]);
        bytes.extend_from_slice(&[command::PACKET as u8, 1, 0]);
        bytes
    }

    #[test]
    fn opens_and_indexes_a_file_without_copying_it() {
        let bytes = fixture();
        let mut file = NamedTempFile::new().expect("temporary file");
        file.write_all(&bytes).expect("write fixture");
        file.flush().expect("flush fixture");

        // SAFETY: The test does not modify or truncate the temporary file while
        // the mapping is alive.
        let mapped = unsafe { MappedDemo::open(file.path()) }.expect("valid mapping");
        assert_eq!(mapped.as_bytes(), bytes);
        assert_eq!(mapped.len(), bytes.len());
        assert_eq!(mapped.demo().expect("borrowed demo").data(), bytes);
        assert_eq!(
            mapped.parser().expect("borrowed parser").demo().data(),
            bytes
        );
        assert_eq!(mapped.index().expect("valid index").distinct_ticks(), [1]);
    }

    #[test]
    fn rejects_a_mapped_non_demo_file() {
        let mut file = NamedTempFile::new().expect("temporary file");
        file.write_all(b"not a source 2 demo")
            .expect("write fixture");
        file.flush().expect("flush fixture");

        // SAFETY: The test keeps the temporary file unchanged while mapping it.
        let result = unsafe { MappedDemo::open(file.path()) };
        assert!(result.is_err());
    }
}
