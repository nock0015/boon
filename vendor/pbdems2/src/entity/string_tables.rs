use std::collections::{HashMap, hash_map::Entry};

use crate::error::{Error, Result};
use crate::io::BitReader;
use crate::limits::DecodeLimits;

// String table delta encoding uses a circular buffer of recently-seen
// strings. New strings can reference a history entry by index and copy a
// prefix, then append the remainder. These constants control the buffer.
const HISTORY_SIZE: usize = 32;
const HISTORY_BITMASK: usize = HISTORY_SIZE - 1;

/// Maximum length (in characters) of a string table key.
const MAX_STRING_BITS: usize = 5;
const MAX_STRING_SIZE: usize = 1 << MAX_STRING_BITS;

/// Maximum size (in bytes) of per-entry user data.
const MAX_USERDATA_BITS: usize = 17;
const MAX_USERDATA_SIZE: usize = 1 << MAX_USERDATA_BITS;

/// The `instancebaseline` table stores default field values for each entity class.
pub const INSTANCE_BASELINE_TABLE_NAME: &str = "instancebaseline";

/// Game-agnostic input for a create-string-table message.
#[derive(Debug)]
#[non_exhaustive]
pub struct CreateStringTable {
    /// Table name (e.g. [`INSTANCE_BASELINE_TABLE_NAME`]).
    pub name: String,
    /// Number of entries encoded in `string_data`.
    pub num_entries: i32,
    /// Whether every entry carries the same user-data width.
    pub user_data_fixed_size: bool,
    /// User-data width in bytes when `user_data_fixed_size` is set.
    pub user_data_size: i32,
    /// User-data width in bits when `user_data_fixed_size` is set.
    pub user_data_size_bits: i32,
    /// Protocol flags, passed through from the game message.
    pub flags: i32,
    /// Bit-packed entry payload.
    pub string_data: Vec<u8>,
    /// Whether `string_data` is Snappy-compressed.
    pub data_compressed: bool,
    /// Whether user-data sizes use varint bit counts rather than a fixed width.
    pub using_varint_bitcounts: bool,
    /// Retain ordered delta payloads until the next tick callback completes.
    pub track_changes: bool,
}

impl CreateStringTable {
    /// Construct a string-table create message with variable-size user data.
    pub fn new(name: impl Into<String>, num_entries: i32, string_data: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            num_entries,
            user_data_fixed_size: false,
            user_data_size: 0,
            user_data_size_bits: 0,
            flags: 0,
            string_data,
            data_compressed: false,
            using_varint_bitcounts: false,
            track_changes: false,
        }
    }

    /// Retain ordered changes for tables used as event streams.
    /// Disabled by default to avoid copying payloads for ordinary lookup tables.
    #[must_use]
    pub const fn with_change_tracking(mut self) -> Self {
        self.track_changes = true;
        self
    }

    /// Configure fixed-width per-entry user data.
    #[must_use]
    pub const fn with_fixed_user_data(mut self, size: i32, size_bits: i32) -> Self {
        self.user_data_fixed_size = true;
        self.user_data_size = size;
        self.user_data_size_bits = size_bits;
        self
    }

    /// Configure protocol flags.
    #[must_use]
    pub const fn with_flags(mut self, flags: i32) -> Self {
        self.flags = flags;
        self
    }

    /// Mark the outer string-data payload as Snappy-compressed.
    #[must_use]
    pub const fn with_compressed_data(mut self) -> Self {
        self.data_compressed = true;
        self
    }

    /// Use variable-length bit counts for user-data sizes.
    #[must_use]
    pub const fn with_varint_bitcounts(mut self) -> Self {
        self.using_varint_bitcounts = true;
        self
    }
}

/// Game-agnostic input for an update-string-table message.
#[derive(Debug)]
#[non_exhaustive]
pub struct UpdateStringTable {
    /// Index of the table to update, in table creation order.
    pub table_id: i32,
    /// Number of entries encoded in `string_data`.
    pub num_changed_entries: i32,
    /// Bit-packed delta payload.
    pub string_data: Vec<u8>,
}

impl UpdateStringTable {
    /// Construct an incremental string-table update.
    pub const fn new(table_id: i32, num_changed_entries: i32, string_data: Vec<u8>) -> Self {
        Self {
            table_id,
            num_changed_entries,
            string_data,
        }
    }
}

/// A single entry in a string table.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StringTableEntry {
    /// Entry key, absent when the update changed only the user data.
    pub string: Option<String>,
    /// Entry payload, absent when the table carries keys alone.
    pub user_data: Option<Vec<u8>>,
}

impl StringTableEntry {
    /// Construct one full-packet string-table entry.
    pub const fn new(string: Option<String>, user_data: Option<Vec<u8>>) -> Self {
        Self { string, user_data }
    }
}

/// Origin of a string-table write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringTableChangeKind {
    /// A create or update message; fields omitted by the wire stay absent.
    Delta,
    /// A full-packet snapshot, not a new event.
    Snapshot,
}

/// One ordered write, retained independently of later writes to the same slot.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StringTableChange {
    /// Slot written by this change.
    pub index: usize,
    /// Incremental delta or full-packet snapshot.
    pub kind: StringTableChangeKind,
    /// Fields supplied by this write, before later writes or field merging.
    pub entry: StringTableEntry,
}

/// A string table.
#[derive(Debug, Clone)]
pub struct StringTable {
    name: String,
    user_data_fixed_size: bool,
    user_data_size: i32,
    user_data_size_bits: i32,
    flags: i32,
    using_varint_bitcounts: bool,
    entries: Vec<StringTableEntry>,
    /// Entry indices written since the last [`StringTableContainer::clear_dirty`]
    /// (i.e. since the previous per-tick callback). Lets consumers decode only
    /// what changed this tick instead of rescanning the whole table — see
    /// [`StringTable::dirty_indices`]. May contain duplicates if an index is
    /// touched more than once between callbacks.
    dirty: Vec<usize>,
    track_changes: bool,
    changes: Vec<StringTableChange>,
}

impl StringTable {
    fn new(
        name: &str,
        user_data_fixed_size: bool,
        user_data_size: i32,
        user_data_size_bits: i32,
        flags: i32,
        using_varint_bitcounts: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            user_data_fixed_size,
            user_data_size,
            user_data_size_bits,
            flags,
            using_varint_bitcounts,
            entries: Vec::new(),
            dirty: Vec::new(),
            track_changes: false,
            changes: Vec::new(),
        }
    }

    /// Protocol table name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// All current table entries.
    pub fn entries(&self) -> &[StringTableEntry] {
        &self.entries
    }

    /// Look up an entry by numeric index.
    pub fn get(&self, index: usize) -> Option<&StringTableEntry> {
        self.entries.get(index)
    }

    /// Entry indices written since the last per-tick callback.
    ///
    /// A consumer that maintains its own per-index state can react to only
    /// these entries rather than iterating (and decoding) every entry every
    /// tick. The list is cleared by the parser after each callback via
    /// [`StringTableContainer::clear_dirty`]. Indices may repeat.
    pub fn dirty_indices(&self) -> &[usize] {
        &self.dirty
    }

    /// Ordered writes since the previous callback, including repeated slots.
    /// Enable with [`CreateStringTable::with_change_tracking`]. Snapshot writes
    /// are marked separately. [`StringTableContainer::clear_dirty`] clears these.
    pub fn changes(&self) -> &[StringTableChange] {
        &self.changes
    }

    fn record_change(
        &mut self,
        index: usize,
        kind: StringTableChangeKind,
        entry: &StringTableEntry,
    ) {
        if self.track_changes {
            self.changes.push(StringTableChange {
                index,
                kind,
                entry: entry.clone(),
            });
        }
    }

    /// Parse a string table update from a bit reader.
    pub fn parse_update(&mut self, br: &mut BitReader, num_entries: i32) -> Result<()> {
        self.parse_update_with_limits(br, num_entries, &DecodeLimits::default())
    }

    /// Limit-aware variant of [`StringTable::parse_update`].
    pub fn parse_update_with_limits(
        &mut self,
        br: &mut BitReader,
        num_entries: i32,
        limits: &DecodeLimits,
    ) -> Result<()> {
        let entry_count = usize::try_from(num_entries).map_err(|_| Error::Parse {
            context: format!("negative string-table entry count {num_entries}"),
        })?;
        limits.ensure(
            "string table entries",
            entry_count,
            limits.max_string_table_entries(),
        )?;

        let mut entry_index: i64 = -1;
        let mut history = [[0u8; MAX_STRING_SIZE]; HISTORY_SIZE];
        let mut history_delta_index: usize = 0;
        let mut string_buf = [0u8; 1024];
        let mut user_data_buf = Vec::new();
        let mut user_data_uncompressed_buf = Vec::new();

        for _ in 0..entry_count {
            // Index jumps are relative to the previous entry, starting at -1.
            entry_index += if br.read_bool()? {
                1
            } else {
                i64::from(br.read_uvarint32()?) + 2
            };
            let idx = usize::try_from(entry_index).map_err(|_| Error::Parse {
                context: format!("negative string-table entry index {entry_index}"),
            })?;
            limits.ensure(
                "string table entry index",
                idx.saturating_add(1),
                limits.max_string_table_entries(),
            )?;

            // Read string
            let has_string = br.read_bool()?;
            let string = if has_string {
                let mut size: usize = 0;

                if br.read_bool()? {
                    // Uses history reference
                    let mut history_delta_zero = 0;
                    if history_delta_index > HISTORY_SIZE {
                        history_delta_zero = history_delta_index & HISTORY_BITMASK;
                    }

                    let index = (history_delta_zero + br.read_bits(5)? as usize) & HISTORY_BITMASK;
                    let bytes_to_copy = br.read_bits(MAX_STRING_BITS)? as usize;
                    size += bytes_to_copy;

                    string_buf[..bytes_to_copy].copy_from_slice(&history[index][..bytes_to_copy]);
                    size += br.read_string_into(&mut string_buf[bytes_to_copy..])?;
                } else {
                    size += br.read_string_into(&mut string_buf)?;
                }

                // Update history
                let mut she = [0u8; MAX_STRING_SIZE];
                let copy_len = size.min(MAX_STRING_SIZE);
                she[..copy_len].copy_from_slice(&string_buf[..copy_len]);
                history[history_delta_index & HISTORY_BITMASK] = she;
                history_delta_index += 1;

                Some(String::from_utf8_lossy(&string_buf[..size]).into_owned())
            } else {
                None
            };

            // Read user data
            let has_user_data = br.read_bool()?;
            let user_data = if has_user_data {
                if self.user_data_fixed_size {
                    let size = usize::try_from(self.user_data_size).unwrap_or(usize::MAX);
                    let size_bits = usize::try_from(self.user_data_size_bits).unwrap_or(usize::MAX);
                    limits.ensure(
                        "string-table fixed user data",
                        size,
                        limits.max_string_table_user_data_bytes(),
                    )?;
                    limits.ensure(
                        "string-table fixed user data bits",
                        size_bits.div_ceil(8),
                        limits.max_string_table_user_data_bytes(),
                    )?;
                    let required = size.max(size_bits.div_ceil(8));
                    if required > MAX_USERDATA_SIZE {
                        return Err(Error::Parse {
                            context: format!(
                                "string-table fixed user data ({size} bytes / {size_bits} bits) exceeds {} bytes",
                                MAX_USERDATA_SIZE
                            ),
                        });
                    }
                    user_data_buf.resize(required, 0);
                    br.read_bits_to_bytes(&mut user_data_buf, size_bits)?;
                    Some(user_data_buf[..size].to_vec())
                } else {
                    let mut is_compressed = false;
                    if (self.flags & 0x1) != 0 {
                        is_compressed = br.read_bool()?;
                    }

                    let size = if self.using_varint_bitcounts {
                        br.read_ubitvar()? as usize
                    } else {
                        br.read_bits(MAX_USERDATA_BITS)? as usize
                    };

                    limits.ensure(
                        "string-table user data",
                        size,
                        limits.max_string_table_user_data_bytes(),
                    )?;
                    if size > MAX_USERDATA_SIZE {
                        return Err(Error::Parse {
                            context: format!(
                                "string-table user data size {size} exceeds {} bytes",
                                MAX_USERDATA_SIZE
                            ),
                        });
                    }
                    user_data_buf.resize(size, 0);
                    br.read_bytes(&mut user_data_buf[..size])?;

                    if is_compressed {
                        let decomp_len = snap::raw::decompress_len(&user_data_buf[..size])
                            .map_err(|e| Error::Decompress(e.to_string()))?;
                        limits.ensure(
                            "decompressed string-table user data",
                            decomp_len,
                            limits.max_string_table_user_data_bytes(),
                        )?;
                        user_data_uncompressed_buf.clear();
                        user_data_uncompressed_buf
                            .try_reserve(decomp_len)
                            .map_err(|_| Error::Allocation {
                                resource: "decompressed string-table user data",
                                requested: decomp_len,
                            })?;
                        user_data_uncompressed_buf.resize(decomp_len, 0);
                        snap::raw::Decoder::new()
                            .decompress(&user_data_buf[..size], &mut user_data_uncompressed_buf)
                            .map_err(|e| Error::Decompress(e.to_string()))?;
                        Some(user_data_uncompressed_buf[..decomp_len].to_vec())
                    } else {
                        Some(user_data_buf[..size].to_vec())
                    }
                }
            } else {
                None
            };

            let entry = StringTableEntry::new(string, user_data);
            self.record_change(idx, StringTableChangeKind::Delta, &entry);
            let StringTableEntry { string, user_data } = entry;
            if idx < self.entries.len() {
                if let Some(ud) = user_data {
                    self.entries[idx].user_data = Some(ud);
                }
                if let Some(s) = string {
                    self.entries[idx].string = Some(s);
                }
            } else {
                // Extend entries to reach idx
                while self.entries.len() < idx {
                    self.entries.push(StringTableEntry {
                        string: None,
                        user_data: None,
                    });
                }
                self.entries.push(StringTableEntry { string, user_data });
            }
            self.dirty.push(idx);
        }

        Ok(())
    }
}

/// Container for all string tables.
#[derive(Clone, Default)]
pub struct StringTableContainer {
    tables: Vec<StringTable>,
    /// Cached instance baselines: class_id -> baseline data.
    instance_baselines: HashMap<i32, Vec<u8>>,
}

impl StringTableContainer {
    /// Create a container holding no tables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a table from a game protobuf adapter.
    pub fn handle_create(&mut self, msg: CreateStringTable) -> Result<bool> {
        self.handle_create_with_limits(msg, &DecodeLimits::default())
    }

    /// Limit-aware variant of [`StringTableContainer::handle_create`].
    pub fn handle_create_with_limits(
        &mut self,
        msg: CreateStringTable,
        limits: &DecodeLimits,
    ) -> Result<bool> {
        limits.ensure(
            "string-table data",
            msg.string_data.len(),
            limits.max_string_table_data_bytes(),
        )?;
        let is_baseline = msg.name == INSTANCE_BASELINE_TABLE_NAME;
        let mut table = StringTable::new(
            &msg.name,
            msg.user_data_fixed_size,
            msg.user_data_size,
            msg.user_data_size_bits,
            msg.flags,
            msg.using_varint_bitcounts,
        );

        table.track_changes = msg.track_changes;

        let string_data = if msg.data_compressed {
            let decompressed_len = snap::raw::decompress_len(&msg.string_data)
                .map_err(|error| Error::Decompress(error.to_string()))?;
            limits.ensure(
                "decompressed string-table data",
                decompressed_len,
                limits.max_decompressed_string_table_bytes(),
            )?;
            let mut output = Vec::new();
            output
                .try_reserve(decompressed_len)
                .map_err(|_| Error::Allocation {
                    resource: "decompressed string-table data",
                    requested: decompressed_len,
                })?;
            output.resize(decompressed_len, 0);
            snap::raw::Decoder::new()
                .decompress(&msg.string_data, &mut output)
                .map_err(|error| Error::Decompress(error.to_string()))?;
            output
        } else {
            msg.string_data
        };

        table.parse_update_with_limits(
            &mut BitReader::new(&string_data),
            msg.num_entries,
            limits,
        )?;
        self.tables.push(table);
        Ok(is_baseline)
    }

    /// Update a table from a game protobuf adapter.
    pub fn handle_update(&mut self, msg: UpdateStringTable) -> Result<bool> {
        self.handle_update_with_limits(msg, &DecodeLimits::default())
    }

    /// Limit-aware variant of [`StringTableContainer::handle_update`].
    pub fn handle_update_with_limits(
        &mut self,
        msg: UpdateStringTable,
        limits: &DecodeLimits,
    ) -> Result<bool> {
        limits.ensure(
            "string-table data",
            msg.string_data.len(),
            limits.max_string_table_data_bytes(),
        )?;
        let table_id = usize::try_from(msg.table_id).map_err(|_| Error::Parse {
            context: format!("invalid negative string table ID {}", msg.table_id),
        })?;
        let table = self.tables.get_mut(table_id).ok_or_else(|| Error::Parse {
            context: format!("string table update for non-existent table {table_id}"),
        })?;
        let is_baseline = table.name == INSTANCE_BASELINE_TABLE_NAME;
        table.parse_update_with_limits(
            &mut BitReader::new(&msg.string_data),
            msg.num_changed_entries,
            limits,
        )?;
        Ok(is_baseline)
    }

    /// Apply a full-packet string-table snapshot.
    pub fn do_full_update(
        &mut self,
        incoming_tables: impl IntoIterator<Item = (String, Vec<StringTableEntry>)>,
    ) -> Result<()> {
        self.do_full_update_with_limits(incoming_tables, &DecodeLimits::default())
    }

    /// Limit-aware variant of [`StringTableContainer::do_full_update`].
    pub fn do_full_update_with_limits(
        &mut self,
        incoming_tables: impl IntoIterator<Item = (String, Vec<StringTableEntry>)>,
        limits: &DecodeLimits,
    ) -> Result<()> {
        for (table_name, items) in incoming_tables {
            limits.ensure(
                "full string-table entries",
                items.len(),
                limits.max_string_table_entries(),
            )?;
            let Some(table) = self
                .tables
                .iter_mut()
                .find(|table| table.name == table_name)
            else {
                continue;
            };
            for (index, entry) in items.into_iter().enumerate() {
                table.record_change(index, StringTableChangeKind::Snapshot, &entry);
                if index < table.entries.len() {
                    if entry.user_data.is_some() {
                        table.entries[index].user_data = entry.user_data;
                        table.dirty.push(index);
                    }
                } else {
                    table.entries.resize_with(index, || StringTableEntry {
                        string: None,
                        user_data: None,
                    });
                    table.entries.push(entry);
                    table.dirty.push(index);
                }
            }
        }
        Ok(())
    }

    /// Clear every table's dirty-index list.
    ///
    /// The parser calls this after each per-tick callback so that
    /// [`StringTable::dirty_indices`] reflects only the entries written since
    /// the previous callback.
    pub fn clear_dirty(&mut self) {
        for table in &mut self.tables {
            table.dirty.clear();
            table.changes.clear();
        }
    }

    /// Update instance baselines from the instancebaseline string table.
    pub fn update_instance_baselines(&mut self) {
        if let Some(table) = self
            .tables
            .iter()
            .find(|t| t.name == INSTANCE_BASELINE_TABLE_NAME)
        {
            for entry in &table.entries {
                if let (Some(s), Some(data)) = (&entry.string, &entry.user_data)
                    && let Ok(class_id) = s.parse::<i32>()
                {
                    match self.instance_baselines.entry(class_id) {
                        Entry::Occupied(mut entry) => {
                            if entry.get() != data {
                                entry.get_mut().clone_from(data);
                            }
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(data.clone());
                        }
                    }
                }
            }
        }
    }

    /// Cached instance baseline bytes for a numeric class ID.
    pub fn instance_baseline(&self, class_id: i32) -> Option<&[u8]> {
        self.instance_baselines.get(&class_id).map(Vec::as_slice)
    }

    /// Look up a string table by name.
    pub fn find_table(&self, name: &str) -> Option<&StringTable> {
        self.tables.iter().find(|t| t.name == name)
    }

    /// Returns a slice of all string tables.
    pub fn tables(&self) -> &[StringTable] {
        &self.tables
    }
}

#[cfg(test)]
mod coverage_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_starts_empty() {
        let container = StringTableContainer::new();
        assert!(container.tables().is_empty());
        assert!(container.instance_baseline(0).is_none());
    }

    #[test]
    fn rejects_unknown_table_update() {
        let mut container = StringTableContainer::new();
        let result = container.handle_update(UpdateStringTable {
            table_id: 99,
            num_changed_entries: 0,
            string_data: Vec::new(),
        });
        assert!(result.is_err());
    }

    #[test]
    fn creates_an_empty_table() {
        let mut container = StringTableContainer::new();
        container
            .handle_create(CreateStringTable {
                name: "test".to_owned(),
                num_entries: 0,
                user_data_fixed_size: false,
                user_data_size: 0,
                user_data_size_bits: 0,
                flags: 0,
                string_data: Vec::new(),
                data_compressed: false,
                using_varint_bitcounts: false,
                track_changes: false,
            })
            .unwrap();
        assert!(container.find_table("test").is_some());
    }
}
