//! Protobuf-independent demo initialization, seeking, and tick playback.

use std::collections::HashSet;

mod prepared;

pub use prepared::{
    CheckpointAdapter, PlaybackCheckpoint, PlaybackSegment, PlaybackSession, PreparedPlayback,
};

use crate::demo::{CommandFrame, Demo, DemoIndex, command};
use crate::entity::field_path::FieldPath;
use crate::entity::{
    ClassEntry, ClassInfo, CreateStringTable, DecodeProfile, EntityContainer, FieldDecodeContext,
    FlattenedSerializer, PacketEntities, SerializerContainer, StringTableContainer,
    StringTableEntry, UpdateStringTable,
};
use crate::error::{Error, Result};
use crate::limits::DecodeLimits;
use crate::packet::PacketMessageIter;

/// Complete game-neutral parser state at a point in demo playback.
#[derive(Clone)]
pub struct ParserState {
    serializers: SerializerContainer,
    class_info: ClassInfo,
    string_tables: StringTableContainer,
    entities: EntityContainer,
    tick_interval: f32,
    tick: i32,
    has_serializers: bool,
    has_class_info: bool,
}

impl ParserState {
    /// Create empty state using a game's default tick interval.
    pub fn new(default_tick_interval: f32) -> Result<Self> {
        validate_tick_interval(default_tick_interval)?;
        Ok(Self {
            serializers: SerializerContainer::default(),
            class_info: ClassInfo::empty(),
            string_tables: StringTableContainer::new(),
            entities: EntityContainer::new(),
            tick_interval: default_tick_interval,
            tick: -1,
            has_serializers: false,
            has_class_info: false,
        })
    }

    /// Parsed flattened serializer graph.
    pub const fn serializers(&self) -> &SerializerContainer {
        &self.serializers
    }

    /// Numeric network class definitions.
    pub const fn class_info(&self) -> &ClassInfo {
        &self.class_info
    }

    /// Current string-table state.
    pub const fn string_tables(&self) -> &StringTableContainer {
        &self.string_tables
    }

    /// Current entity state.
    pub const fn entities(&self) -> &EntityContainer {
        &self.entities
    }

    /// Seconds represented by one game tick.
    pub const fn tick_interval(&self) -> f32 {
        self.tick_interval
    }

    /// Most recently processed tick, or `-1` before playback starts.
    pub const fn tick(&self) -> i32 {
        self.tick
    }

    /// Whether both serializers and class information have been installed.
    pub const fn is_initialized(&self) -> bool {
        self.has_serializers && self.has_class_info
    }

    fn require_initialized(&self) -> Result<()> {
        if !self.has_serializers {
            return Err(Error::Parse {
                context: "flattened serializers were not installed before DEM_SyncTick".into(),
            });
        }
        if !self.has_class_info {
            return Err(Error::Parse {
                context: "class information was not installed before DEM_SyncTick".into(),
            });
        }
        Ok(())
    }

    fn clear_tick_changes(&mut self) {
        self.string_tables.clear_dirty();
        self.entities.clear_tick_changes();
    }
}

const fn segment_replay_end_tick(exclusive_end_tick: i32) -> Option<i32> {
    if exclusive_end_tick == i32::MAX {
        None
    } else {
        Some(exclusive_end_tick.saturating_sub(1))
    }
}

fn validate_tick_interval(tick_interval: f32) -> Result<()> {
    if !tick_interval.is_finite() || tick_interval <= 0.0 {
        return Err(Error::Parse {
            context: format!("invalid tick interval {tick_interval}"),
        });
    }
    Ok(())
}

/// Mutable game-neutral operations available to a protobuf adapter.
///
/// The adapter decodes its generated messages and immediately converts them
/// into these neutral inputs. It never has to manipulate container internals.
pub struct CommandContext<'state, 'filter> {
    state: &'state mut ParserState,
    field_decode_context: &'state mut FieldDecodeContext,
    field_paths: &'state mut Vec<FieldPath>,
    limits: DecodeLimits,
    class_filter: Option<&'filter HashSet<&'filter str>>,
}

impl<'state, 'filter> CommandContext<'state, 'filter> {
    /// Read current parser state while handling a command.
    pub const fn state(&self) -> &ParserState {
        self.state
    }

    /// Limits applied to adapter-decoded message sizes and neutral operations.
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    /// Validate an inner packet-message payload before an adapter allocates it.
    pub fn check_packet_message_size(&self, size: usize) -> Result<()> {
        self.limits.ensure(
            "inner packet message",
            size,
            self.limits.max_packet_message_bytes(),
        )
    }

    /// Iterate the game-neutral message framing inside a packet payload.
    ///
    /// The adapter still decides which generated protobuf type corresponds to
    /// each message identifier. Unaligned payloads can be copied into one
    /// reusable buffer with
    /// [PacketMessageFrame::copy_payload](crate::packet::PacketMessageFrame::copy_payload).
    pub fn packet_messages<'packet>(&self, data: &'packet [u8]) -> PacketMessageIter<'packet> {
        PacketMessageIter::with_limits(data, self.limits)
    }

    /// Parse and install flattened serializers.
    pub fn install_serializers(
        &mut self,
        serializers: FlattenedSerializer,
        profile: DecodeProfile,
    ) -> Result<()> {
        self.state.serializers =
            SerializerContainer::parse_with_limits(serializers, profile, &self.limits)?;
        self.state.has_serializers = true;
        Ok(())
    }

    /// Validate and install network classes.
    pub fn install_class_info(
        &mut self,
        classes: impl IntoIterator<Item = ClassEntry>,
    ) -> Result<()> {
        self.state.class_info = ClassInfo::try_from_entries_with_limits(classes, &self.limits)?;
        self.state.has_class_info = true;
        self.state.string_tables.update_instance_baselines();
        Ok(())
    }

    /// Update the tick interval reported by server-info.
    pub fn set_tick_interval(&mut self, tick_interval: f32) -> Result<()> {
        validate_tick_interval(tick_interval)?;
        self.state.tick_interval = tick_interval;
        self.field_decode_context.set_tick_interval(tick_interval);
        Ok(())
    }

    /// Create a string table from a neutral adapter message.
    pub fn create_string_table(&mut self, table: CreateStringTable) -> Result<()> {
        let baseline = self
            .state
            .string_tables
            .handle_create_with_limits(table, &self.limits)?;
        if baseline {
            self.state.string_tables.update_instance_baselines();
        }
        Ok(())
    }

    /// Apply an incremental string-table update.
    pub fn update_string_table(&mut self, update: UpdateStringTable) -> Result<()> {
        let baseline = self
            .state
            .string_tables
            .handle_update_with_limits(update, &self.limits)?;
        if baseline {
            self.state.string_tables.update_instance_baselines();
        }
        Ok(())
    }

    /// Apply the string-table snapshot carried by a full packet.
    pub fn apply_full_string_tables(
        &mut self,
        tables: impl IntoIterator<Item = (String, Vec<StringTableEntry>)>,
    ) -> Result<()> {
        self.state
            .string_tables
            .do_full_update_with_limits(tables, &self.limits)?;
        self.state.string_tables.update_instance_baselines();
        Ok(())
    }

    /// Apply a packet-entities message, honoring the playback class filter.
    pub fn apply_packet_entities(&mut self, packet: PacketEntities<'_>) -> Result<()> {
        let state = &mut self.state;
        if let Some(filter) = self.class_filter {
            state.entities.handle_packet_entities_filtered(
                packet,
                &state.class_info,
                &state.serializers,
                &state.string_tables,
                self.field_decode_context,
                filter,
                self.field_paths,
            )
        } else {
            state.entities.handle_packet_entities(
                packet,
                &state.class_info,
                &state.serializers,
                &state.string_tables,
                self.field_decode_context,
                self.field_paths,
            )
        }
    }
}

/// Game-specific protobuf bridge used by [`DemoParser`].
///
/// Implementations normally match on `frame.header().cmd`, decode a generated
/// protobuf from `body`, convert it to neutral pbdems2 values, and call methods
/// on `context`. The associated error can wrap both protobuf errors and
/// [`crate::Error`]; the `From<Error>` bound lets the driver propagate framing
/// and limit failures without stringifying them.
pub trait DemoAdapter {
    /// Failure type produced by the adapter, able to carry a [`crate::Error`].
    type Error: From<Error>;

    /// Handle one decompressed command from the demo stream.
    ///
    /// `body` is the decoded payload for `frame`; commands the adapter does
    /// not care about should return `Ok(())` without touching `context`.
    fn handle_command(
        &mut self,
        frame: &CommandFrame<'_>,
        body: &[u8],
        context: &mut CommandContext<'_, '_>,
    ) -> std::result::Result<(), Self::Error>;
}

/// Borrowed neutral parser driver for one validated PBDEMS2 file.
#[derive(Debug, Clone, Copy)]
pub struct DemoParser<'a> {
    demo: Demo<'a>,
}

impl<'a> DemoParser<'a> {
    /// Validate a complete demo using default limits.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        Ok(Self {
            demo: Demo::new(data)?,
        })
    }

    /// Validate a complete demo using explicit limits.
    pub fn with_limits(data: &'a [u8], limits: DecodeLimits) -> Result<Self> {
        Ok(Self {
            demo: Demo::with_limits(data, limits)?,
        })
    }

    /// The underlying command-stream view.
    pub const fn demo(&self) -> &Demo<'a> {
        &self.demo
    }

    /// Build the header-only seek index.
    pub fn index(&self) -> Result<DemoIndex> {
        self.demo.index()
    }

    /// Decode signon state through `DEM_SyncTick`.
    pub fn initial_state<A: DemoAdapter>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
    ) -> std::result::Result<ParserState, A::Error> {
        self.initialize(adapter, default_tick_interval)
            .map(|(state, _)| state)
    }

    /// Decode all post-signon commands and invoke `on_tick` once per completed tick.
    pub fn run_to_end<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState),
    {
        let mut on_tick = on_tick;
        self.try_run_to_end(adapter, default_tick_interval, |state| {
            on_tick(state);
            Ok(())
        })
    }

    /// Decode all commands with a callback that can abort playback with an error.
    ///
    /// The callback uses the adapter's application error type so a consumer can
    /// propagate output, database, cancellation, or other processing failures
    /// without erasing either those errors or parser errors.
    pub fn try_run_to_end<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState) -> std::result::Result<(), A::Error>,
    {
        let mut on_tick = on_tick;
        self.run_to_end_impl(adapter, default_tick_interval, None, |state, _| {
            on_tick(state)
        })
    }

    /// Decode all commands and expose game-specific adapter state at each tick.
    pub fn run_to_end_with_adapter<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A),
    {
        let mut on_tick = on_tick;
        self.try_run_to_end_with_adapter(adapter, default_tick_interval, |state, adapter| {
            on_tick(state, adapter);
            Ok(())
        })
    }

    /// Fallible playback with mutable adapter access at each completed tick.
    ///
    /// This supports consumers that collect game-specific state, such as
    /// per-tick events, inside their protobuf adapter.
    pub fn try_run_to_end_with_adapter<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A) -> std::result::Result<(), A::Error>,
    {
        self.run_to_end_impl(adapter, default_tick_interval, None, on_tick)
    }

    /// Decode all commands while only materializing entities in `class_filter`.
    pub fn run_to_end_filtered<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState),
    {
        let mut on_tick = on_tick;
        self.try_run_to_end_filtered(adapter, default_tick_interval, class_filter, |state| {
            on_tick(state);
            Ok(())
        })
    }

    /// Filter entities and allow the per-tick callback to abort with an error.
    pub fn try_run_to_end_filtered<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState) -> std::result::Result<(), A::Error>,
    {
        let mut on_tick = on_tick;
        self.run_to_end_impl(
            adapter,
            default_tick_interval,
            Some(class_filter),
            |state, _| on_tick(state),
        )
    }

    /// Filter entities and expose mutable adapter state at each completed tick.
    pub fn run_to_end_filtered_with_adapter<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A),
    {
        let mut on_tick = on_tick;
        self.try_run_to_end_filtered_with_adapter(
            adapter,
            default_tick_interval,
            class_filter,
            |state, adapter| {
                on_tick(state, adapter);
                Ok(())
            },
        )
    }

    /// Fallible filtered playback with mutable adapter access at each tick.
    pub fn try_run_to_end_filtered_with_adapter<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A) -> std::result::Result<(), A::Error>,
    {
        self.run_to_end_impl(adapter, default_tick_interval, Some(class_filter), on_tick)
    }

    /// Restore the latest full packet at or before `target_tick`, then replay deltas.
    pub fn parse_to_tick<A: DemoAdapter>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        target_tick: i32,
    ) -> std::result::Result<ParserState, A::Error> {
        let (mut state, stream_start) = self.initialize(adapter, default_tick_interval)?;
        let index = self.demo.index().map_err(A::Error::from)?;
        let start = index
            .full_packet_at_or_before(target_tick)
            .map(|position| position.offset())
            .or(stream_start);
        if let Some(start) = start {
            self.replay(
                adapter,
                &mut state,
                start,
                Some(target_tick),
                None,
                |_, _| Ok(()),
            )?;
        }
        Ok(state)
    }

    /// Cold-start one segment from signon or a full-packet keyframe.
    ///
    /// A full-packet restart is exact only for entity classes that the game
    /// fully re-keyframes in that snapshot. Consumers are responsible for
    /// choosing a compatible `class_filter`; pbdems2 cannot infer which
    /// classes carry unsnapshotted state between keyframes.
    pub fn decode_segment<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        start: Option<usize>,
        end_tick: i32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState),
    {
        let mut on_tick = on_tick;
        self.try_decode_segment(
            adapter,
            default_tick_interval,
            start,
            end_tick,
            class_filter,
            |state| {
                on_tick(state);
                Ok(())
            },
        )
    }

    /// Decode a segment with a callback that can abort playback with an error.
    pub fn try_decode_segment<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        start: Option<usize>,
        end_tick: i32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState) -> std::result::Result<(), A::Error>,
    {
        let mut on_tick = on_tick;
        self.try_decode_segment_with_adapter(
            adapter,
            default_tick_interval,
            start,
            end_tick,
            class_filter,
            |state, _| on_tick(state),
        )
    }

    /// Decode one segment and expose mutable adapter state at each tick.
    pub fn decode_segment_with_adapter<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        start: Option<usize>,
        end_tick: i32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A),
    {
        let mut on_tick = on_tick;
        self.try_decode_segment_with_adapter(
            adapter,
            default_tick_interval,
            start,
            end_tick,
            class_filter,
            |state, adapter| {
                on_tick(state, adapter);
                Ok(())
            },
        )
    }

    /// Fallible segment playback with mutable adapter access at each tick.
    pub fn try_decode_segment_with_adapter<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        start: Option<usize>,
        end_tick: i32,
        class_filter: &HashSet<&str>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A) -> std::result::Result<(), A::Error>,
    {
        let (mut state, stream_start) = self.initialize(adapter, default_tick_interval)?;
        if let Some(start) = start.or(stream_start) {
            self.replay(
                adapter,
                &mut state,
                start,
                segment_replay_end_tick(end_tick),
                Some(class_filter),
                on_tick,
            )?;
        }
        Ok(state)
    }

    fn run_to_end_impl<A, F>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
        class_filter: Option<&HashSet<&str>>,
        on_tick: F,
    ) -> std::result::Result<ParserState, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A) -> std::result::Result<(), A::Error>,
    {
        let (mut state, stream_start) = self.initialize(adapter, default_tick_interval)?;
        if let Some(start) = stream_start {
            self.replay(adapter, &mut state, start, None, class_filter, on_tick)?;
        }
        Ok(state)
    }

    fn initialize<A: DemoAdapter>(
        &self,
        adapter: &mut A,
        default_tick_interval: f32,
    ) -> std::result::Result<(ParserState, Option<usize>), A::Error> {
        let limits = self.demo.limits();
        let mut state = ParserState::new(default_tick_interval).map_err(A::Error::from)?;
        let mut body = Vec::new();
        let mut field_decode_context =
            FieldDecodeContext::with_limits(default_tick_interval, limits);
        let mut field_paths = Vec::new();
        let mut stream_start = None;

        for frame in self.demo.commands() {
            let frame = frame.map_err(A::Error::from)?;
            match frame.header().cmd {
                command::STOP => break,
                command::SYNC_TICK => {
                    stream_start = Some(frame.end_offset());
                    break;
                }
                _ => {}
            }
            frame.decode_body(&mut body).map_err(A::Error::from)?;
            let mut context = CommandContext {
                state: &mut state,
                field_decode_context: &mut field_decode_context,
                field_paths: &mut field_paths,
                limits,
                class_filter: None,
            };
            adapter.handle_command(&frame, &body, &mut context)?;
        }

        state.require_initialized().map_err(A::Error::from)?;
        state.string_tables.update_instance_baselines();
        // Sign-on establishes the baseline state rather than an emitted gameplay
        // tick. Do not leak setup lifecycle records into the first callback.
        // The legacy updated-index list is intentionally left untouched.
        state.entities.clear_entity_changes();
        Ok((state, stream_start))
    }

    #[allow(clippy::too_many_arguments)]
    fn replay<A, F>(
        &self,
        adapter: &mut A,
        state: &mut ParserState,
        start: usize,
        end_tick: Option<i32>,
        class_filter: Option<&HashSet<&str>>,
        on_tick: F,
    ) -> std::result::Result<(), A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A) -> std::result::Result<(), A::Error>,
    {
        self.replay_cursor(adapter, state, start, end_tick, class_filter, on_tick)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_cursor<A, F>(
        &self,
        adapter: &mut A,
        state: &mut ParserState,
        start: usize,
        end_tick: Option<i32>,
        class_filter: Option<&HashSet<&str>>,
        mut on_tick: F,
    ) -> std::result::Result<usize, A::Error>
    where
        A: DemoAdapter,
        F: FnMut(&ParserState, &mut A) -> std::result::Result<(), A::Error>,
    {
        let limits = self.demo.limits();
        let commands = self.demo.commands_from(start).map_err(A::Error::from)?;
        let mut body = Vec::new();
        let mut field_decode_context = FieldDecodeContext::with_limits(state.tick_interval, limits);
        let mut field_paths = Vec::new();
        let mut last_tick = None;
        let mut emitted_final_tick = false;
        let mut next_offset = self.demo.data().len();

        for frame in commands {
            let frame = frame.map_err(A::Error::from)?;
            let header = frame.header();
            if end_tick.is_some_and(|end| header.tick > end) {
                next_offset = frame.offset();
                break;
            }

            if last_tick.is_some_and(|last| last != header.tick) {
                on_tick(state, adapter)?;
                state.clear_tick_changes();
            }
            last_tick = Some(header.tick);
            state.tick = header.tick;

            if header.cmd == command::STOP {
                if header.tick >= 0 {
                    on_tick(state, adapter)?;
                    emitted_final_tick = true;
                }
                break;
            }

            frame.decode_body(&mut body).map_err(A::Error::from)?;
            let mut context = CommandContext {
                state,
                field_decode_context: &mut field_decode_context,
                field_paths: &mut field_paths,
                limits,
                class_filter,
            };
            adapter.handle_command(&frame, &body, &mut context)?;
        }

        if !emitted_final_tick && last_tick.is_some_and(|tick| tick >= 0) {
            on_tick(state, adapter)?;
        }
        Ok(next_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{HEADER_SIZE, MAGIC};
    use crate::entity::{BareCharEncoding, FlattenedSerializerDefinition, PreciseQAngleMode};
    use crate::entity::{Entity, EntityChangeKind};

    #[derive(Default)]
    struct Adapter;

    impl DemoAdapter for Adapter {
        type Error = Error;

        fn handle_command(
            &mut self,
            frame: &CommandFrame<'_>,
            _body: &[u8],
            context: &mut CommandContext<'_, '_>,
        ) -> Result<()> {
            match frame.header().cmd {
                command::SEND_TABLES => context.install_serializers(
                    FlattenedSerializer {
                        serializers: vec![FlattenedSerializerDefinition {
                            serializer_name_sym: Some(0),
                            fields_index: vec![],
                        }],
                        symbols: vec!["CTest".into()],
                        fields: vec![],
                    },
                    DecodeProfile::new(
                        BareCharEncoding::NullTerminatedString,
                        PreciseQAngleMode::Centered,
                    ),
                ),
                command::CLASS_INFO => {
                    context.install_class_info([ClassEntry::new(0, "CTest", "CTest")])
                }
                _ => Ok(()),
            }
        }
    }

    #[derive(Default)]
    struct LifecycleAdapter;

    impl DemoAdapter for LifecycleAdapter {
        type Error = Error;

        fn handle_command(
            &mut self,
            frame: &CommandFrame<'_>,
            body: &[u8],
            context: &mut CommandContext<'_, '_>,
        ) -> Result<()> {
            Adapter.handle_command(frame, body, context)?;
            let index = match frame.header().cmd {
                command::CLASS_INFO => 99,
                command::PACKET => frame.header().tick,
                _ => return Ok(()),
            };
            let class_name = context
                .state
                .class_info
                .by_id(0)
                .map_or_else(Default::default, |entry| entry.network_name.clone());
            context.state.entities.insert(Entity::from_fields(
                index,
                index as u32,
                0,
                class_name,
                true,
                Default::default(),
            )?)?;
            Ok(())
        }
    }

    fn push_command(bytes: &mut Vec<u8>, command: u8, tick: u8) {
        bytes.extend_from_slice(&[command, tick, 0]);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = Vec::from(MAGIC);
        bytes.resize(HEADER_SIZE, 0);
        push_command(&mut bytes, command::SEND_TABLES as u8, 0);
        push_command(&mut bytes, command::CLASS_INFO as u8, 0);
        push_command(&mut bytes, command::SYNC_TICK as u8, 0);
        push_command(&mut bytes, command::PACKET as u8, 1);
        push_command(&mut bytes, command::PACKET as u8, 2);
        push_command(&mut bytes, command::STOP as u8, 3);
        bytes
    }

    #[test]
    fn initializes_and_emits_completed_ticks() {
        let bytes = fixture();
        let parser = DemoParser::new(&bytes).expect("valid demo");
        let mut ticks = Vec::new();
        let state = parser
            .run_to_end(&mut Adapter, 1.0 / 64.0, |state| ticks.push(state.tick()))
            .expect("valid playback");
        assert!(state.is_initialized());
        assert_eq!(ticks, [1, 2, 3]);
    }

    #[test]
    fn entity_changes_are_scoped_to_successive_tick_callbacks() {
        let bytes = fixture();
        let parser = DemoParser::new(&bytes).unwrap();
        let mut observed = Vec::new();
        parser
            .run_to_end(&mut LifecycleAdapter, 1.0 / 64.0, |state| {
                observed.push((
                    state.tick(),
                    state
                        .entities
                        .entity_changes()
                        .iter()
                        .map(|change| change.kind)
                        .collect::<Vec<_>>(),
                ));
            })
            .unwrap();

        assert_eq!(
            observed,
            [
                (1, vec![EntityChangeKind::Created]),
                (2, vec![EntityChangeKind::Created]),
                (3, vec![]),
            ]
        );
    }

    #[test]
    fn parse_to_tick_stops_after_target() {
        let bytes = fixture();
        let state = DemoParser::new(&bytes)
            .expect("valid demo")
            .parse_to_tick(&mut Adapter, 1.0 / 64.0, 1)
            .expect("valid playback");
        assert_eq!(state.tick(), 1);
    }

    #[test]
    fn fallible_tick_callback_stops_and_preserves_the_error() {
        let bytes = fixture();
        let parser = DemoParser::new(&bytes).expect("valid demo");
        let mut ticks = Vec::new();
        let error = parser
            .try_run_to_end(&mut Adapter, 1.0 / 64.0, |state| {
                ticks.push(state.tick());
                if state.tick() == 2 {
                    return Err(Error::Parse {
                        context: "tick consumer failed".into(),
                    });
                }
                Ok(())
            })
            .err()
            .expect("callback failure must abort playback");

        assert_eq!(ticks, [1, 2]);
        assert!(matches!(
            error,
            Error::Parse { context } if context == "tick consumer failed"
        ));
    }

    #[test]
    fn fallible_filtered_callback_preserves_the_error() {
        let bytes = fixture();
        let parser = DemoParser::new(&bytes).expect("valid demo");
        let filter = HashSet::from(["CTest"]);
        let error = parser
            .try_run_to_end_filtered(&mut Adapter, 1.0 / 64.0, &filter, |_| -> Result<()> {
                Err(Error::Parse {
                    context: "filtered consumer failed".into(),
                })
            })
            .err()
            .expect("callback failure must abort playback");

        assert!(matches!(
            error,
            Error::Parse { context } if context == "filtered consumer failed"
        ));
    }

    #[test]
    fn fallible_segment_callback_preserves_the_error() {
        let bytes = fixture();
        let parser = DemoParser::new(&bytes).expect("valid demo");
        let filter = HashSet::new();
        let error = parser
            .try_decode_segment(&mut Adapter, 1.0 / 64.0, None, 4, &filter, |_| {
                Err(Error::Parse {
                    context: "segment consumer failed".into(),
                })
            })
            .err()
            .expect("callback failure must abort playback");

        assert!(matches!(
            error,
            Error::Parse { context } if context == "segment consumer failed"
        ));
    }

    #[test]
    fn rejects_missing_initialization_messages() {
        let mut bytes = Vec::from(MAGIC);
        bytes.resize(HEADER_SIZE, 0);
        push_command(&mut bytes, command::SYNC_TICK as u8, 0);
        assert!(
            DemoParser::new(&bytes)
                .expect("valid framing")
                .initial_state(&mut Adapter, 1.0 / 64.0)
                .is_err()
        );
    }
}
