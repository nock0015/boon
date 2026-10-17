use crate::error::{Error, Result};
use crate::io::ByteReader;
use crate::limits::DecodeLimits;

#[cfg(test)]
use super::MAGIC;
use super::{CmdHeader, DemoHeader, read_cmd_body_with_limits, read_cmd_header};

/// Fixed number of bytes before the first PBDEMS2 command.
pub const HEADER_SIZE: usize = 16;

/// Bit set on a raw demo command identifier when its body is Snappy-compressed.
pub const COMPRESSED_FLAG: u32 = 64;

/// Protocol-level outer demo command identifiers shared by Source 2 games.
pub mod command {
    /// `DEM_Stop`: end of the command stream.
    pub const STOP: i32 = 0;
    /// `DEM_FileHeader`: demo-level metadata written when recording started.
    pub const FILE_HEADER: i32 = 1;
    /// `DEM_FileInfo`: playback summary appended when recording finished.
    pub const FILE_INFO: i32 = 2;
    /// `DEM_SyncTick`: marks the first tick of real-time playback.
    pub const SYNC_TICK: i32 = 3;
    /// `DEM_SendTables`: flattened serializer definitions for the entity system.
    pub const SEND_TABLES: i32 = 4;
    /// `DEM_ClassInfo`: entity class IDs paired with their network names.
    pub const CLASS_INFO: i32 = 5;
    /// `DEM_StringTables`: a complete string-table snapshot.
    pub const STRING_TABLES: i32 = 6;
    /// `DEM_Packet`: the network messages recorded for one tick.
    pub const PACKET: i32 = 7;
    /// `DEM_SignonPacket`: network messages from the pre-game handshake.
    pub const SIGNON_PACKET: i32 = 8;
    /// `DEM_ConsoleCmd`: a console command issued during recording.
    pub const CONSOLE_CMD: i32 = 9;
    /// `DEM_CustomData`: game-defined payload with no neutral interpretation.
    pub const CUSTOM_DATA: i32 = 10;
    /// `DEM_CustomDataCallbacks`: registry naming the custom-data payloads.
    pub const CUSTOM_DATA_CALLBACKS: i32 = 11;
    /// `DEM_UserCmd`: recorded client input for one command number.
    pub const USER_CMD: i32 = 12;
    /// `DEM_FullPacket`: string tables plus a packet, usable as a seek keyframe.
    ///
    /// [`DemoIndex::full_packets`](super::DemoIndex::full_packets) collects
    /// every occurrence so playback can restart from any of them.
    pub const FULL_PACKET: i32 = 13;
    /// `DEM_SaveGame`: embedded save-game blob.
    pub const SAVE_GAME: i32 = 14;
    /// `DEM_SpawnGroups`: spawn-group load and unload events.
    pub const SPAWN_GROUPS: i32 = 15;
    /// `DEM_AnimationData`: recorded animation samples.
    pub const ANIMATION_DATA: i32 = 16;
    /// `DEM_AnimationHeader`: descriptor for a run of animation data.
    pub const ANIMATION_HEADER: i32 = 17;
    /// `DEM_Recovery`: resynchronization data emitted after a recording glitch.
    pub const RECOVERY: i32 = 18;
}

/// A command's absolute byte offset and tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandPosition {
    offset: usize,
    tick: i32,
}

impl CommandPosition {
    pub(crate) const fn new(offset: usize, tick: i32) -> Self {
        Self { offset, tick }
    }

    /// Absolute byte offset from the beginning of the demo file.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Tick recorded in the command header.
    pub const fn tick(&self) -> i32 {
        self.tick
    }
}

/// Header-only index used for seeking and segmented playback.
#[derive(Debug, Clone, Default)]
pub struct DemoIndex {
    full_packets: Vec<CommandPosition>,
    distinct_ticks: Vec<i32>,
    stream_start: Option<usize>,
}

impl DemoIndex {
    /// Full-packet keyframes in ascending file order.
    pub fn full_packets(&self) -> &[CommandPosition] {
        &self.full_packets
    }

    /// Distinct non-negative ticks after the synchronization command.
    pub fn distinct_ticks(&self) -> &[i32] {
        &self.distinct_ticks
    }

    /// Absolute offset immediately after `DEM_SyncTick`, when present.
    pub const fn stream_start(&self) -> Option<usize> {
        self.stream_start
    }

    /// Last full-packet keyframe at or before `target_tick`.
    pub fn full_packet_at_or_before(&self, target_tick: i32) -> Option<CommandPosition> {
        self.full_packets
            .iter()
            .rev()
            .find(|position| position.tick <= target_tick)
            .copied()
    }
}

/// Borrowed view of one framed command and its encoded body.
#[derive(Debug, Clone, Copy)]
pub struct CommandFrame<'a> {
    index: usize,
    offset: usize,
    end_offset: usize,
    header: CmdHeader,
    encoded_body: &'a [u8],
    limits: DecodeLimits,
}

impl<'a> CommandFrame<'a> {
    /// Zero-based command index relative to the iterator's starting offset.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Absolute byte offset at which this command header begins.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Absolute byte offset immediately after this command body.
    pub const fn end_offset(&self) -> usize {
        self.end_offset
    }

    /// Parsed command header.
    pub const fn header(&self) -> &CmdHeader {
        &self.header
    }

    /// Encoded body bytes as stored in the demo.
    pub const fn encoded_body(&self) -> &'a [u8] {
        self.encoded_body
    }

    /// Copy or decompress the body into a reusable output buffer.
    pub fn decode_body(&self, output: &mut Vec<u8>) -> Result<()> {
        let mut reader = ByteReader::new(self.encoded_body);
        read_cmd_body_with_limits(&mut reader, &self.header, output, &self.limits).map_err(
            |source| Error::Command {
                offset: self.offset,
                command: Some(self.header.cmd),
                tick: Some(self.header.tick),
                source: Box::new(source),
            },
        )
    }
}

/// Strict, allocation-free iterator over framed outer demo commands.
pub struct CommandIter<'a> {
    reader: ByteReader<'a>,
    base_offset: usize,
    index: usize,
    limits: DecodeLimits,
    failed: bool,
}

impl<'a> CommandIter<'a> {
    fn new(data: &'a [u8], offset: usize, limits: DecodeLimits) -> Self {
        Self {
            reader: ByteReader::new(data),
            base_offset: offset,
            index: 0,
            limits,
            failed: false,
        }
    }

    fn fail(&mut self, offset: usize, header: Option<&CmdHeader>, source: Error) -> Error {
        self.failed = true;
        Error::Command {
            offset,
            command: header.map(|value| value.cmd),
            tick: header.map(|value| value.tick),
            source: Box::new(source),
        }
    }
}

impl<'a> Iterator for CommandIter<'a> {
    type Item = Result<CommandFrame<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.is_empty() {
            return None;
        }

        let offset = self.base_offset + self.reader.position();
        let header = match read_cmd_header(&mut self.reader, COMPRESSED_FLAG) {
            Ok(header) => header,
            Err(error) => return Some(Err(self.fail(offset, None, error))),
        };
        let body_size = header.body_size as usize;
        if let Err(error) = self.limits.ensure(
            "encoded command body",
            body_size,
            self.limits.max_command_body_bytes(),
        ) {
            return Some(Err(self.fail(offset, Some(&header), error)));
        }
        let encoded_body = match self.reader.read_bytes(body_size) {
            Ok(body) => body,
            Err(error) => return Some(Err(self.fail(offset, Some(&header), error))),
        };
        let end_offset = self.base_offset + self.reader.position();
        let frame = CommandFrame {
            index: self.index,
            offset,
            end_offset,
            header,
            encoded_body,
            limits: self.limits,
        };
        self.index += 1;
        Some(Ok(frame))
    }
}

/// Validated borrowed PBDEMS2 file with command iteration and indexing.
///
/// # Example
///
/// ```no_run
/// use pbdems2::demo::{Demo, command};
///
/// let bytes = std::fs::read("match.dem")?;
/// let demo = Demo::new(&bytes)?;
///
/// let packets = demo
///     .commands()
///     .filter_map(Result::ok)
///     .filter(|frame| frame.header().cmd == command::PACKET)
///     .count();
/// # Ok::<(), pbdems2::Error>(())
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Demo<'a> {
    data: &'a [u8],
    limits: DecodeLimits,
    header: DemoHeader,
}

impl<'a> Demo<'a> {
    /// Validate and borrow a complete PBDEMS2 file using default limits.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        Self::with_limits(data, DecodeLimits::default())
    }

    /// Validate and borrow a complete PBDEMS2 file with custom limits.
    pub fn with_limits(data: &'a [u8], limits: DecodeLimits) -> Result<Self> {
        let header = DemoHeader::parse(data)?;
        Ok(Self {
            data,
            limits,
            header,
        })
    }

    /// Parsed fixed PBDEMS2 file header.
    pub const fn header(&self) -> DemoHeader {
        self.header
    }

    /// Complete encoded file bytes.
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Limits applied by iterators and body decompression.
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    /// Iterate commands beginning immediately after the fixed file header.
    pub fn commands(&self) -> CommandIter<'a> {
        CommandIter::new(&self.data[HEADER_SIZE..], HEADER_SIZE, self.limits)
    }

    /// Iterate commands from an absolute byte offset.
    ///
    /// Offsets returned by [`CommandFrame::offset`], [`DemoIndex::stream_start`],
    /// and [`CommandPosition::offset`] are valid inputs.
    pub fn commands_from(&self, offset: usize) -> Result<CommandIter<'a>> {
        if !(HEADER_SIZE..=self.data.len()).contains(&offset) {
            return Err(Error::Parse {
                context: format!(
                    "command offset {offset} outside valid range {HEADER_SIZE}..={}",
                    self.data.len()
                ),
            });
        }
        Ok(CommandIter::new(&self.data[offset..], offset, self.limits))
    }

    /// Build the header-only seek index.
    pub fn index(&self) -> Result<DemoIndex> {
        let mut index = DemoIndex::default();
        let mut past_sync = false;
        let mut last_tick = None;

        for frame in self.commands() {
            let frame = frame?;
            let header = frame.header();
            if header.cmd == command::SYNC_TICK {
                if !past_sync {
                    index.stream_start = Some(frame.end_offset());
                }
                past_sync = true;
                continue;
            }
            if header.cmd == command::STOP {
                break;
            }
            if !past_sync {
                continue;
            }
            if header.cmd == command::FULL_PACKET {
                index
                    .full_packets
                    .push(CommandPosition::new(frame.offset(), header.tick));
            }
            if header.tick >= 0 && last_tick != Some(header.tick) {
                index.distinct_ticks.push(header.tick);
                last_tick = Some(header.tick);
            }
        }

        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(raw_command: u8, tick: u8, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![raw_command, tick, body.len() as u8];
        bytes.extend_from_slice(body);
        bytes
    }

    fn demo(commands: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::from(MAGIC);
        bytes.extend_from_slice(&[0; HEADER_SIZE - MAGIC.len()]);
        bytes.extend_from_slice(commands);
        bytes
    }

    #[test]
    fn iterates_borrowed_command_frames() {
        let data = demo(&command(command::PACKET as u8, 42, b"abc"));
        let demo = Demo::new(&data).expect("valid fixture");
        let frame = demo
            .commands()
            .next()
            .expect("one frame")
            .expect("valid frame");
        assert_eq!(frame.offset(), HEADER_SIZE);
        assert_eq!(frame.header().cmd, command::PACKET);
        assert_eq!(frame.header().tick, 42);
        assert_eq!(frame.encoded_body(), b"abc");
        assert_eq!(frame.end_offset(), data.len());
    }

    #[test]
    fn indexes_first_sync_ticks_and_full_packets() {
        let mut commands = command(command::SYNC_TICK as u8, 0, &[]);
        let expected_start = HEADER_SIZE + commands.len();
        commands.extend(command(command::PACKET as u8, 1, &[]));
        commands.extend(command(command::SYNC_TICK as u8, 1, &[]));
        let full_offset = HEADER_SIZE + commands.len();
        commands.extend(command(command::FULL_PACKET as u8, 2, &[]));
        commands.extend(command(command::PACKET as u8, 2, &[]));
        commands.extend(command(command::STOP as u8, 3, &[]));

        let data = demo(&commands);
        let index = Demo::new(&data)
            .expect("valid fixture")
            .index()
            .expect("valid index");
        assert_eq!(index.stream_start(), Some(expected_start));
        assert_eq!(index.distinct_ticks(), [1, 2]);
        assert_eq!(index.full_packets().len(), 1);
        assert_eq!(index.full_packets()[0].offset(), full_offset);
        assert_eq!(index.full_packet_at_or_before(1), None);
        assert_eq!(
            index.full_packet_at_or_before(2).map(|value| value.tick()),
            Some(2)
        );
    }

    #[test]
    fn rejects_truncated_and_oversized_bodies() {
        let truncated = demo(&[command::PACKET as u8, 1, 4, 1]);
        assert!(
            Demo::new(&truncated)
                .expect("header is valid")
                .commands()
                .next()
                .expect("an error item")
                .is_err()
        );

        let limits = DecodeLimits::default().with_max_command_body_bytes(2);
        let oversized = demo(&command(command::PACKET as u8, 1, b"abc"));
        assert!(
            Demo::with_limits(&oversized, limits)
                .expect("header is valid")
                .commands()
                .next()
                .expect("an error item")
                .is_err()
        );
    }
}
