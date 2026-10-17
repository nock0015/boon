//! Game-independent PBDEMS2 command framing.

use crate::error::{Error, Result};
use crate::io::ByteReader;
use crate::limits::DecodeLimits;

mod stream;

pub use stream::{
    COMPRESSED_FLAG, CommandFrame, CommandIter, CommandPosition, Demo, DemoIndex, HEADER_SIZE,
    command,
};

/// Magic bytes at the beginning of every Source 2 demo.
pub const MAGIC: [u8; 8] = *b"PBDEMS2\0";

/// Parsed fields from the fixed 16-byte PBDEMS2 file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DemoHeader {
    file_info_offset: Option<usize>,
    spawn_groups_offset: Option<usize>,
}

impl DemoHeader {
    /// Parse the magic prefix and validate both optional command offsets.
    pub fn parse(data: &[u8]) -> Result<Self> {
        verify_header(data, HEADER_SIZE)?;
        Ok(Self {
            file_info_offset: read_optional_header_offset(data, 8, "file-info")?,
            spawn_groups_offset: read_optional_header_offset(data, 12, "spawn-groups")?,
        })
    }

    /// Absolute offset of DEM_FileInfo, or None when the header stores zero.
    pub const fn file_info_offset(&self) -> Option<usize> {
        self.file_info_offset
    }

    /// Absolute offset of DEM_SpawnGroups, or None when the header stores zero.
    pub const fn spawn_groups_offset(&self) -> Option<usize> {
        self.spawn_groups_offset
    }
}

fn read_optional_header_offset(
    data: &[u8],
    start: usize,
    field: &'static str,
) -> Result<Option<usize>> {
    let raw = u32::from_le_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]);
    if raw == 0 {
        return Ok(None);
    }
    let offset = usize::try_from(raw).map_err(|_| Error::Parse {
        context: format!("{field} header offset {raw} does not fit this platform"),
    })?;
    if !(HEADER_SIZE..data.len()).contains(&offset) {
        return Err(Error::Parse {
            context: format!(
                "{field} header offset {offset} outside command stream {HEADER_SIZE}..{}",
                data.len()
            ),
        });
    }
    Ok(Some(offset))
}

/// Header shared by every command in a demo stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CmdHeader {
    /// Command identifier with compression flags removed.
    pub cmd: i32,
    /// Demo tick associated with the command.
    pub tick: i32,
    /// Whether the command body uses Snappy compression.
    pub compressed: bool,
    /// Command payload size before decompression.
    pub body_size: u32,
}

impl CmdHeader {
    /// Construct a command header, primarily for adapters and test fixtures.
    pub const fn new(cmd: i32, tick: i32, compressed: bool, body_size: u32) -> Self {
        Self {
            cmd,
            tick,
            compressed,
            body_size,
        }
    }
}

/// Verify the PBDEMS2 magic prefix and fixed-size file header.
pub fn verify_header(data: &[u8], header_size: usize) -> Result<()> {
    if data.len() < header_size {
        return Err(Error::Parse {
            context: format!(
                "file too small for demo header: expected at least {header_size} bytes, got {}",
                data.len()
            ),
        });
    }

    let mut got = [0_u8; 8];
    got.copy_from_slice(&data[..8]);
    if got != MAGIC {
        return Err(Error::InvalidMagic { got });
    }
    Ok(())
}

/// Read one command header from the current byte position.
pub fn read_cmd_header(reader: &mut ByteReader<'_>, compressed_flag: u32) -> Result<CmdHeader> {
    let raw_cmd = reader.read_uvarint32()?;
    Ok(CmdHeader {
        cmd: (raw_cmd & !compressed_flag) as i32,
        tick: reader.read_uvarint32()? as i32,
        compressed: raw_cmd & compressed_flag != 0,
        body_size: reader.read_uvarint32()?,
    })
}

/// Read and, when necessary, decompress a command body into the output buffer.
pub fn read_cmd_body(
    reader: &mut ByteReader<'_>,
    header: &CmdHeader,
    output: &mut Vec<u8>,
) -> Result<()> {
    read_cmd_body_with_limits(reader, header, output, &DecodeLimits::default())
}

/// Limit-aware variant of [`read_cmd_body`].
pub fn read_cmd_body_with_limits(
    reader: &mut ByteReader<'_>,
    header: &CmdHeader,
    output: &mut Vec<u8>,
    limits: &DecodeLimits,
) -> Result<()> {
    let body_size = header.body_size as usize;
    limits.ensure(
        "encoded command body",
        body_size,
        limits.max_command_body_bytes(),
    )?;
    let body = reader.read_bytes(body_size)?;
    output.clear();

    if header.compressed {
        let decompressed_len = snap::raw::decompress_len(body)
            .map_err(|error| Error::Decompress(error.to_string()))?;
        limits.ensure(
            "decompressed command body",
            decompressed_len,
            limits.max_decompressed_command_bytes(),
        )?;
        output
            .try_reserve(decompressed_len)
            .map_err(|_| Error::Allocation {
                resource: "decompressed command body",
                requested: decompressed_len,
            })?;
        output.resize(decompressed_len, 0);
        snap::raw::Decoder::new()
            .decompress(body, output)
            .map_err(|error| Error::Decompress(error.to_string()))?;
    } else {
        output
            .try_reserve(body.len())
            .map_err(|_| Error::Allocation {
                resource: "command body",
                requested: body.len(),
            })?;
        output.extend_from_slice(body);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_bounds_checks_fixed_header_offsets() {
        let mut data = Vec::from(MAGIC);
        data.resize(64, 0);
        data[8..12].copy_from_slice(&32_u32.to_le_bytes());
        data[12..16].copy_from_slice(&48_u32.to_le_bytes());

        let header = Demo::new(&data).expect("valid header").header();
        assert_eq!(header.file_info_offset(), Some(32));
        assert_eq!(header.spawn_groups_offset(), Some(48));

        data[8..12].copy_from_slice(&8_u32.to_le_bytes());
        assert!(matches!(
            DemoHeader::parse(&data),
            Err(Error::Parse { context }) if context.contains("file-info header offset 8")
        ));

        data[8..12].fill(0);
        let end_offset = data.len() as u32;
        data[12..16].copy_from_slice(&end_offset.to_le_bytes());
        assert!(matches!(
            DemoHeader::parse(&data),
            Err(Error::Parse { context }) if context.contains("spawn-groups header offset 64")
        ));
    }

    #[test]
    fn zero_header_offsets_are_absent() {
        let mut data = Vec::from(MAGIC);
        data.resize(HEADER_SIZE, 0);
        let header = DemoHeader::parse(&data).expect("valid header");
        assert_eq!(header.file_info_offset(), None);
        assert_eq!(header.spawn_groups_offset(), None);
    }

    #[test]
    fn validates_header() {
        let mut data = Vec::from(MAGIC);
        data.extend_from_slice(&[0; 8]);
        assert!(verify_header(&data, 16).is_ok());
        assert!(matches!(
            verify_header(b"NOTADEMO--------", 16),
            Err(Error::InvalidMagic { .. })
        ));
        assert!(verify_header(&MAGIC, 16).is_err());
    }

    #[test]
    fn reads_command_header() {
        let mut reader = ByteReader::new(&[7, 42, 3]);
        let header = read_cmd_header(&mut reader, 1 << 31).unwrap();
        assert_eq!(
            header,
            CmdHeader {
                cmd: 7,
                tick: 42,
                compressed: false,
                body_size: 3,
            }
        );
    }
}
