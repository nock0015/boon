use std::collections::HashSet;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::io::BitReader;

use super::class_info::ClassInfo;
use super::field_decoder::FieldDecodeContext;
use super::field_path::{self, FieldPath};
use super::field_value::FieldValue;
use super::serializer_bindings::SerializerBindings;
use super::serializers::{Serializer, SerializerContainer, SerializerField};
use super::string_tables::StringTableContainer;

// Serial-number layout for the entity *create* delta (legacy Source CBaseHandle):
// a create carries a 15-bit entity entry (`MAX_EDICT_BITS + 1`) followed by a
// 17-bit serial number. This governs only the serial read in `handle_create`; it
// is NOT the layout of a networked CHandle *field value*, whose entity index is
// the low 14 bits — see [`ENTITY_HANDLE_INDEX_MASK`] / [`EntityContainer::get_by_handle`].
const MAX_EDICT_BITS: u32 = 14;
const NUM_ENT_ENTRY_BITS: u32 = MAX_EDICT_BITS + 1;
const NUM_SERIAL_NUM_BITS: u32 = 32 - NUM_ENT_ENTRY_BITS;
const MAX_ENTITY_INDEX: i64 = (1_i64 << MAX_EDICT_BITS) - 1;

/// Borrowed game-independent view of a packet-entities message.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct PacketEntities<'a> {
    /// Number of entity deltas encoded in `entity_data`.
    pub updated_entries: i32,
    /// Bit-packed entity deltas, decoded by
    /// [`EntityContainer::handle_packet_entities`].
    pub entity_data: &'a [u8],
    /// Count of leading PVS visibility bits preceding each delta, or `0` when
    /// the game does not send them.
    pub has_pvs_vis_bits: u32,
}

impl<'a> PacketEntities<'a> {
    /// Construct a neutral packet-entities view from a game protobuf adapter.
    pub const fn new(updated_entries: i32, entity_data: &'a [u8], has_pvs_vis_bits: u32) -> Self {
        Self {
            updated_entries,
            entity_data,
            has_pvs_vis_bits,
        }
    }
}

/// Mask that extracts the entity-array index from a networked `CHandle` value.
///
/// Source 2 packs a 14-bit edict index (entity indices run 0–16383) in the low
/// bits of a handle with a serial number above it, so the index is recovered
/// with this 14-bit mask. A wider mask (e.g. `0x7FFF`) leaks a serial bit into
/// the index and resolves the wrong entity for any handle with an odd serial.
pub const ENTITY_HANDLE_INDEX_MASK: u32 = 0x3FFF;

/// Sentinel value for a `CHandle` field that points at no entity.
///
/// Protobuf handle fields (e.g. `modifier.parent`) are optional; callers
/// substitute this when the field is absent and skip handles equal to it.
pub const INVALID_ENTITY_HANDLE: u32 = 0x00FF_FFFF;

/// Resolve a protobuf-style optional `CHandle` to an entity-array index.
///
/// Returns `None` when the handle is absent (the protobuf field was not set)
/// or holds the [`INVALID_ENTITY_HANDLE`] sentinel; otherwise applies
/// [`ENTITY_HANDLE_INDEX_MASK`] and returns the entity-array index. Use this
/// for `Option<u32>` `CHandle` fields on user messages (e.g. a death message's
/// `attacker` / `victim`); it pairs naturally with `let-else`:
///
/// ```ignore
/// let Some(attacker_idx) = awpy::protobuf_handle_index(msg.attacker) else { continue };
/// ```
pub fn protobuf_handle_index(handle: Option<u32>) -> Option<i32> {
    handle
        .filter(|&h| h != INVALID_ENTITY_HANDLE)
        .map(|h| (h & ENTITY_HANDLE_INDEX_MASK) as i32)
}

/// Walk a serializer hierarchy along a field path to the addressed field.
///
/// Returns a [`Error::Parse`] rather than panicking when any component indexes
/// past its serializer's fields — which is the tell-tale symptom of a bitstream
/// desync (a corrupted field path addresses a nonexistent field), so surfacing
/// it as an error keeps a bad decode from taking down the whole parse.
fn resolve_field_path<'a>(
    serializer: &'a Serializer,
    fp: &FieldPath,
) -> Result<&'a SerializerField> {
    let oob = || Error::Parse {
        context: format!(
            "field path out of range for serializer '{}' ({} fields)",
            serializer.name,
            serializer.fields.len()
        ),
    };

    let mut field = serializer.fields.get(fp.get(0)).ok_or_else(oob)?;
    for i in 1..=fp.last {
        let idx = fp.get(i);
        if field.is_dynamic_array() {
            match field.field_serializer.as_ref() {
                Some(fs) => field = fs.fields.first().ok_or_else(oob)?,
                None => break,
            }
        } else if let Some(fs) = field.field_serializer.as_ref() {
            field = fs.fields.get(idx).ok_or_else(oob)?;
        } else {
            break;
        }
    }
    Ok(field)
}

// Each entity in a packet carries a 2-bit delta command. Rather than four
// opaque codes, Source 2 encodes it as two independent flags:
//
//   bit 0 (`CMD_LEAVE`)          0 → the entity is created or updated
//                                1 → the entity leaves the client's PVS
//   bit 1 (`CMD_CREATE_DELETE`)  on the create/update side: create (vs update)
//                                on the leave side:          delete (vs leave)
//
// So `0b10` = create, `0b00` = update, `0b01` = leave (keep, mark inactive),
// `0b11` = leave + delete (remove). Crucially, a plain leave (`0b01`) does NOT
// remove the entity — it merely goes dormant and can be re-activated by a later
// update, which is why entities must not be dropped on leave.
const CMD_LEAVE: u8 = 0x01;
const CMD_CREATE_DELETE: u8 = 0x02;

/// Stable identity for one occupant of an entity slot.
///
/// An index can be reused, so consumers should use both fields when keying
/// long-lived state across ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct EntityId {
    /// The entity slot.
    pub index: i32,
    /// The serial number that distinguishes reuse of the slot.
    pub serial: u32,
}

impl EntityId {
    /// Construct an entity identity from its wire components.
    pub const fn new(index: i32, serial: u32) -> Self {
        Self { index, serial }
    }
}

/// The lifecycle transition produced by an entity command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EntityChangeKind {
    /// A new entity was created.
    Created,
    /// An active entity received a field update.
    Updated,
    /// An entity outside the PVS became active again through an update.
    Reactivated,
    /// An entity left the potentially visible set but still exists.
    LeftPvs,
    /// An entity was permanently removed or replaced.
    Deleted,
}

/// A compact record of one decoded entity lifecycle transition.
///
/// [EntityChange::class_name] shares the entity's allocation, so a change does not
/// allocate a new string or clone the full entity.
/// Deletions are compact tombstones rather than retained [Entity] values.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct EntityChange {
    /// The transition that occurred.
    pub kind: EntityChangeKind,
    /// The entity slot.
    pub index: i32,
    /// The serial number that distinguishes reuse of the same slot.
    pub serial: u32,
    /// The server class identifier.
    pub class_id: i32,
    /// The server class name.
    pub class_name: Arc<str>,
}

impl EntityChange {
    fn from_entity(kind: EntityChangeKind, entity: &Entity) -> Self {
        Self {
            kind,
            index: entity.index,
            serial: entity.serial,
            class_id: entity.class_id,
            class_name: Arc::clone(&entity.class_name),
        }
    }

    /// Composite identity for this lifecycle record.
    pub const fn id(&self) -> EntityId {
        EntityId::new(self.index, self.serial)
    }
}

/// A single entity with its class, fields, and current state.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Entity {
    /// Slot index in the entity array (0–16383).
    pub index: i32,
    /// Serial number for this slot (increments on reuse).
    pub serial: u32,
    /// Numeric class ID (indexes into [`ClassInfo`]).
    pub class_id: i32,
    /// Network class name (e.g. `"CBasePlayerPawn"`).
    ///
    /// Shared with [`ClassInfo`] rather than owned, so creating an entity does
    /// not allocate a fresh copy of a name the class table already holds.
    pub class_name: Arc<str>,
    /// Whether the entity is currently in the client's PVS.
    ///
    /// An entity that leaves the PVS (delta command `0b01`) is kept in the
    /// container but marked inactive rather than removed; a later update
    /// re-activates it. Consumers iterating [`EntityContainer`] that only want
    /// live entities should skip those with `active == false`.
    pub active: bool,
    /// Current field values, keyed by packed field path keys.
    pub fields: FxHashMap<u64, FieldValue>,
    /// Packed field keys decoded since the last successful tick callback.
    /// Values are read from `fields` at the completed tick, so repeated writes
    /// and baseline overrides preserve end-of-tick semantics without clones.
    changed_fields: Vec<u64>,
}

impl Entity {
    /// Composite identity for this slot occupant.
    pub const fn id(&self) -> EntityId {
        EntityId::new(self.index, self.serial)
    }

    /// Construct an entity with explicit decoded state.
    pub fn from_fields(
        index: i32,
        serial: u32,
        class_id: i32,
        class_name: impl Into<Arc<str>>,
        active: bool,
        fields: FxHashMap<u64, FieldValue>,
    ) -> Result<Self> {
        if !(0..=MAX_ENTITY_INDEX as i32).contains(&index) {
            return Err(Error::Parse {
                context: format!("entity index {index} out of range"),
            });
        }
        Ok(Self {
            index,
            serial,
            class_id,
            class_name: class_name.into(),
            active,
            changed_fields: fields.keys().copied().collect(),
            fields,
        })
    }

    fn new(index: i32, class_id: i32, class_name: Arc<str>) -> Self {
        Self {
            index,
            serial: 0,
            class_id,
            class_name,
            active: true,
            fields: FxHashMap::default(),
            changed_fields: Vec::new(),
        }
    }

    /// Apply field path deltas from a bit reader using the given serializer.
    #[allow(clippy::needless_range_loop)]
    fn apply_update(
        &mut self,
        br: &mut BitReader,
        serializer: &Serializer,
        ctx: &mut FieldDecodeContext,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<()> {
        let limits = ctx.limits();
        field_path::read_field_paths_with_limits(br, fp_buf, &limits)?;

        for fp_idx in 0..fp_buf.len() {
            let field = resolve_field_path(serializer, &fp_buf[fp_idx])?;

            let key = fp_buf[fp_idx].pack();
            let value = field
                .metadata
                .decoder
                .decode(ctx, br)
                .map_err(|e| Error::Parse {
                    context: format!(
                        "field #{} key={:#x} (type: {}, decoder: {:?}, pos: {}, remaining: {}): {}",
                        fp_idx,
                        key,
                        field.var_type,
                        field.metadata.decoder,
                        br.position(),
                        br.bits_remaining(),
                        e
                    ),
                })?;
            self.fields.insert(key, value);
            self.changed_fields.push(key);
        }

        Ok(())
    }

    /// Packed field paths written during this tick. Duplicates are permitted;
    /// consumers wanting one final value per key should deduplicate these keys.
    /// A create includes baseline keys, and values retain their original types.
    pub fn changed_field_keys(&self) -> &[u64] {
        &self.changed_fields
    }

    /// Skip field updates - reads the data to advance the bit reader but doesn't store anything.
    /// This avoids allocations and FxHashMap insertions for entities we don't care about.
    #[allow(clippy::needless_range_loop)]
    fn skip_update(
        br: &mut BitReader,
        serializer: &Serializer,
        ctx: &mut FieldDecodeContext,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<()> {
        let limits = ctx.limits();
        field_path::read_field_paths_with_limits(br, fp_buf, &limits)?;

        for fp_idx in 0..fp_buf.len() {
            let field = resolve_field_path(serializer, &fp_buf[fp_idx])?;
            // Skip the value - just advances the bit reader without decoding
            field.metadata.decoder.skip(ctx, br)?;
        }

        Ok(())
    }

    /// Read a field by its pre-resolved packed key.
    pub fn field_value(&self, key: Option<u64>) -> Option<&FieldValue> {
        key.and_then(|key| self.fields.get(&key))
    }

    /// Strictly convert a field without silently defaulting missing or invalid data.
    pub fn try_get<T>(
        &self,
        key: Option<u64>,
    ) -> std::result::Result<Option<T>, crate::error::FieldValueConversionError>
    where
        for<'value> T: TryFrom<&'value FieldValue, Error = crate::error::FieldValueConversionError>,
    {
        self.field_value(key).map(T::try_from).transpose()
    }

    /// Resolve a dotted field path and return its current value.
    ///
    /// For repeated reads, resolve the key once with
    /// [Serializer::resolve_field_key] and use [Self::field_value] or a typed
    /// accessor. Keys are specific to a serializer layout. Resolve them again
    /// when the schema changes.
    pub fn get_by_name(&self, path: &str, serializer: &Serializer) -> Option<&FieldValue> {
        let key = serializer.resolve_field_key(path)?;
        self.fields.get(&key)
    }

    // ── Typed field accessors ──
    //
    // Each takes a field key pre-resolved with [`Serializer::resolve_field_key`]
    // (so it can be resolved once and reused across ticks) and reads the field
    // leniently: a missing key, an absent field, or a value of a different type
    // all yield the type's default rather than an error. Integer variants accept
    // any of the four integer encodings, since the network type is not always
    // known ahead of time.

    /// Read a field as `i64`, returning `0` when absent or non-integer.
    pub fn get_i64(&self, key: Option<u64>) -> i64 {
        key.and_then(|k| self.fields.get(&k))
            .and_then(|v| match v {
                FieldValue::U32(n) => Some(*n as i64),
                FieldValue::U64(n) => Some(*n as i64),
                FieldValue::I32(n) => Some(*n as i64),
                FieldValue::I64(n) => Some(*n),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Read a field as `u32`, returning `0` when absent or non-integer.
    pub fn get_u32(&self, key: Option<u64>) -> u32 {
        key.and_then(|k| self.fields.get(&k))
            .and_then(|v| match v {
                FieldValue::U32(n) => Some(*n),
                FieldValue::U64(n) => Some(*n as u32),
                FieldValue::I32(n) => Some(*n as u32),
                FieldValue::I64(n) => Some(*n as u32),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Read a field as `f32`, returning `0.0` when absent or non-float.
    pub fn get_f32(&self, key: Option<u64>) -> f32 {
        key.and_then(|k| self.fields.get(&k))
            .and_then(|v| match v {
                FieldValue::F32(f) => Some(*f),
                _ => None,
            })
            .unwrap_or(0.0)
    }

    /// Read a field as `bool`, returning `false` when absent or non-bool.
    pub fn get_bool(&self, key: Option<u64>) -> bool {
        key.and_then(|k| self.fields.get(&k))
            .and_then(|v| match v {
                FieldValue::Bool(b) => Some(*b),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Read a field as a `QAngle`, returning `[0.0; 3]` when absent or non-angle.
    pub fn get_qangle(&self, key: Option<u64>) -> [f32; 3] {
        key.and_then(|k| self.fields.get(&k))
            .and_then(|v| match v {
                FieldValue::QAngle(a) => Some(*a),
                _ => None,
            })
            .unwrap_or([0.0; 3])
    }

    /// Read a field as a 3-component vector, returning `[0.0; 3]` when absent
    /// or not represented by a compatible vector variant.
    pub fn get_vector3(&self, key: Option<u64>) -> [f32; 3] {
        key.and_then(|k| self.fields.get(&k))
            .and_then(|v| match v {
                FieldValue::Vector3(v) => Some(*v),
                FieldValue::FloatVector(v) if v.len() >= 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            })
            .unwrap_or([0.0; 3])
    }

    /// Combine cell + in-cell offset fields into a world-coordinate `[x, y, z]`.
    ///
    /// Source 2 splits each axis of an entity's position across two networked
    /// fields — an integer cell index (e.g. `m_cellX`) and a quantized offset
    /// inside that cell (e.g. `m_vecOrigin.m_vecX`). Pass the resolved keys
    /// for both halves and this returns the full world position in Hammer
    /// units via [`cell_to_world`](crate::position::cell_to_world). Reading
    /// the offset alone gives a sawtooth that resets every cell boundary, not
    /// a usable coordinate.
    ///
    /// Cell keys with no resolved field decode as cell `0`, which means the
    /// result is shifted into a single cell-grid quadrant rather than
    /// returning a sentinel — verify the keys before relying on it.
    pub fn world_position(
        &self,
        cell_keys: [Option<u64>; 3],
        offset_keys: [Option<u64>; 3],
    ) -> [f32; 3] {
        let cell = [
            self.get_i64(cell_keys[0]) as i32,
            self.get_i64(cell_keys[1]) as i32,
            self.get_i64(cell_keys[2]) as i32,
        ];
        let offset = [
            self.get_f32(offset_keys[0]),
            self.get_f32(offset_keys[1]),
            self.get_f32(offset_keys[2]),
        ];
        [
            crate::position::cell_to_world(cell[0], offset[0]),
            crate::position::cell_to_world(cell[1], offset[1]),
            crate::position::cell_to_world(cell[2], offset[2]),
        ]
    }

    /// Read a raw `CHandle` field as `u32`, if present.
    ///
    /// Pass the result to [`EntityContainer::get_by_handle`] to follow the handle
    /// to its entity; that helper owns the index mask so callers never decode it
    /// by hand.
    pub fn get_handle(&self, key: Option<u64>) -> Option<u32> {
        key.and_then(|k| self.fields.get(&k)).and_then(|v| match v {
            FieldValue::U32(n) => Some(*n),
            FieldValue::U64(n) => Some(*n as u32),
            FieldValue::I32(n) => Some(*n as u32),
            FieldValue::I64(n) => Some(*n as u32),
            _ => None,
        })
    }

    /// Read a `u64`-ish field regardless of the concrete integer variant, or
    /// `None` when the field is absent or not an integer.
    pub fn get_u64(&self, key: Option<u64>) -> Option<u64> {
        key.and_then(|k| self.fields.get(&k)).and_then(|v| match v {
            FieldValue::U64(n) => Some(*n),
            FieldValue::U32(n) => Some(*n as u64),
            FieldValue::I64(n) => Some(*n as u64),
            FieldValue::I32(n) => Some(*n as u64),
            _ => None,
        })
    }

    /// Borrow a raw byte-string field without allocating.
    pub fn get_bytes(&self, key: Option<u64>) -> Option<&[u8]> {
        self.field_value(key).and_then(|value| match value {
            FieldValue::String(bytes) => Some(bytes.as_slice()),
            _ => None,
        })
    }

    /// Borrow a UTF-8 string field, preserving invalid-UTF-8 errors.
    pub fn get_str(
        &self,
        key: Option<u64>,
    ) -> std::result::Result<Option<&str>, std::str::Utf8Error> {
        self.get_bytes(key).map(std::str::from_utf8).transpose()
    }

    /// Read a string field, or `None` when the field is absent or not a string.
    pub fn get_string(&self, key: Option<u64>) -> Option<String> {
        self.get_bytes(key)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Container managing all active entities.
#[derive(Clone, Default)]
pub struct EntityContainer {
    serializer_bindings: SerializerBindings,
    /// Active (and dormant) entities, indexed **directly by entity index** — a
    /// dense slot array rather than a hash map, so every `get` / `get_mut` /
    /// `get_by_handle` (which fire on essentially every entity update) is a
    /// bounds-checked index instead of a hash lookup. Indices are `0..=16383`;
    /// empty slots hold `None`.
    entities: Vec<Option<Entity>>,
    /// Number of occupied slots, including dormant entities.
    occupied_count: usize,
    /// `class_id` per index for entities that were filtered out at create time,
    /// so a later filtered update can pick the right serializer to advance past
    /// them. `-1` means "no skipped entity here". Same dense-slot layout.
    skipped_entity_classes: Vec<i32>,
    /// Entity indices created or updated since the last tick callback.
    updated: Vec<i32>,
    /// Ordered lifecycle transitions decoded since the last tick callback.
    changes: Vec<EntityChange>,
}

impl EntityContainer {
    /// Create an empty container with no entity slots reserved.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resize the dense slot array to `slot_count` entries.
    ///
    /// Shrinking discards entities beyond the new end without lifecycle events.
    pub fn reserve_slots(&mut self, slot_count: usize) -> Result<()> {
        let max_slots = MAX_ENTITY_INDEX as usize + 1;
        if slot_count > max_slots {
            return Err(Error::LimitExceeded {
                resource: "entity slots",
                limit: max_slots,
                actual: slot_count,
            });
        }
        if slot_count < self.entities.len() {
            self.occupied_count -= self.entities[slot_count..]
                .iter()
                .filter(|slot| slot.is_some())
                .count();
        }
        self.entities.resize_with(slot_count, || None);
        Ok(())
    }

    /// Insert a validated entity into its dense slot, returning any previous value.
    pub fn insert(&mut self, entity: Entity) -> Result<Option<Entity>> {
        if !(0..=MAX_ENTITY_INDEX as i32).contains(&entity.index) {
            return Err(Error::Parse {
                context: format!("entity index {} out of range", entity.index),
            });
        }
        let index = entity.index as usize;
        if index >= self.entities.len() {
            self.entities.resize_with(index + 1, || None);
        }
        let created = EntityChange::from_entity(EntityChangeKind::Created, &entity);
        let previous = self.entities[index].replace(entity);
        self.occupied_count += usize::from(previous.is_none());
        if let Some(previous) = previous.as_ref() {
            self.changes.push(EntityChange::from_entity(
                EntityChangeKind::Deleted,
                previous,
            ));
        }
        self.changes.push(created);
        self.updated.push(index as i32);
        Ok(previous)
    }

    /// Immutable dense slots, including empty positions.
    pub fn slots(&self) -> &[Option<Entity>] {
        &self.entities
    }

    /// Number of allocated dense slots, including empty positions.
    pub fn slot_count(&self) -> usize {
        self.entities.len()
    }

    /// A slotted entity, mutably (`None` if the index is out of range or empty).
    #[inline]
    fn entity_mut(&mut self, index: i32) -> Option<&mut Entity> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.entities.get_mut(i))
            .and_then(Option::as_mut)
    }

    /// Store an entity at its index, growing the slot array as needed.
    #[inline]
    fn put_entity(&mut self, index: i32, entity: Entity) {
        let Ok(i) = usize::try_from(index) else {
            return;
        };
        if i >= self.entities.len() {
            self.entities.resize_with(i + 1, || None);
        }
        let created = EntityChange::from_entity(EntityChangeKind::Created, &entity);
        let previous = self.entities[i].replace(entity);
        self.occupied_count += usize::from(previous.is_none());
        if let Some(previous) = previous.as_ref() {
            self.changes.push(EntityChange::from_entity(
                EntityChangeKind::Deleted,
                previous,
            ));
        }
        self.changes.push(created);
        self.updated.push(index);
    }

    /// Empty an entity's slot without recording why it was removed.
    #[inline]
    fn take_entity(&mut self, index: i32) -> Option<Entity> {
        let entity = self
            .entities
            .get_mut(usize::try_from(index).ok()?)?
            .take()?;
        self.occupied_count -= 1;
        Some(entity)
    }

    /// Permanently remove an entity and retain its old identity in the log.
    #[inline]
    fn delete_entity(&mut self, index: i32) {
        if let Some(entity) = self.take_entity(index) {
            self.changes.push(EntityChange::from_entity(
                EntityChangeKind::Deleted,
                &entity,
            ));
        }
    }

    /// The class_id of a skipped entity at `index`, if one was recorded.
    #[inline]
    fn skipped_class(&self, index: i32) -> Option<i32> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.skipped_entity_classes.get(i))
            .copied()
            .filter(|&c| c >= 0)
    }

    /// Record (or clear, with `None`) the class_id of a skipped entity at `index`.
    #[inline]
    fn set_skipped(&mut self, index: i32, class_id: Option<i32>) {
        let Ok(i) = usize::try_from(index) else {
            return;
        };
        let value = class_id.unwrap_or(-1);
        if i >= self.skipped_entity_classes.len() {
            if value < 0 {
                return; // clearing an absent slot is a no-op
            }
            self.skipped_entity_classes.resize(i + 1, -1);
        }
        self.skipped_entity_classes[i] = value;
    }

    /// Handle a CSVCMsg_PacketEntities message.
    pub fn handle_packet_entities(
        &mut self,
        msg: PacketEntities<'_>,
        class_info: &ClassInfo,
        serializers: &SerializerContainer,
        string_tables: &StringTableContainer,
        field_decode_ctx: &mut FieldDecodeContext,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<()> {
        let update_count = usize::try_from(msg.updated_entries).map_err(|_| Error::Parse {
            context: format!(
                "negative packet-entity update count {}",
                msg.updated_entries
            ),
        })?;
        let limits = field_decode_ctx.limits();
        limits.ensure(
            "packet entity updates",
            update_count,
            limits.max_packet_entity_updates(),
        )?;
        let has_pvs_vis_bits = msg.has_pvs_vis_bits;
        self.serializer_bindings.refresh(class_info, serializers);
        let entity_data = msg.entity_data;
        let mut br = BitReader::new(entity_data);

        let mut entity_index: i32 = -1;

        for _ in 0..update_count {
            let next = i64::from(entity_index) + i64::from(br.read_ubitvar()?) + 1;
            if !(0..=MAX_ENTITY_INDEX).contains(&next) {
                return Err(Error::Parse {
                    context: format!("entity index {next} out of range"),
                });
            }
            entity_index = next as i32;

            // Read the 2-bit delta command (see CMD_LEAVE / CMD_CREATE_DELETE).
            let cmd = br.read_bits(2)? as u8;

            if cmd & CMD_LEAVE == 0 {
                if cmd & CMD_CREATE_DELETE != 0 {
                    self.handle_create(
                        entity_index,
                        &mut br,
                        class_info,
                        serializers,
                        string_tables,
                        field_decode_ctx,
                        fp_buf,
                    )
                    .map_err(|e| Error::Parse {
                        context: format!("entity create #{entity_index}: {e}"),
                    })?;
                } else {
                    // Legacy PVS-visibility bits (absent in modern demos):
                    // when present, a set low bit means "not actually updated".
                    if has_pvs_vis_bits > 0 && (br.read_bits(2)? & 0x01) != 0 {
                        continue;
                    }
                    self.handle_update(
                        entity_index,
                        &mut br,
                        serializers,
                        field_decode_ctx,
                        fp_buf,
                    )
                    .map_err(|e| Error::Parse {
                        context: format!(
                            "entity update #{} (class: {:?}): {}",
                            entity_index,
                            self.get(entity_index).map(|e| &e.class_name),
                            e
                        ),
                    })?;
                }
            } else {
                self.handle_leave(entity_index, cmd & CMD_CREATE_DELETE != 0);
            }
        }

        Ok(())
    }

    /// Handle an entity leaving the client's PVS.
    ///
    /// A plain leave (`delete == false`) marks the entity inactive but keeps it,
    /// so a subsequent update can re-activate it. A leave-and-delete removes it.
    /// Repeated plain leaves are ignored. Deletion also removes an entity that
    /// became dormant through an earlier leave.
    fn handle_leave(&mut self, index: i32, delete: bool) {
        if delete {
            self.delete_entity(index);
            return;
        }

        let change = self.entity_mut(index).and_then(|e| {
            if !e.active {
                return None;
            }
            e.active = false;
            Some(EntityChange::from_entity(EntityChangeKind::LeftPvs, e))
        });
        if let Some(change) = change {
            self.changes.push(change);
        }
    }

    /// Handle a CSVCMsg_PacketEntities message, only tracking specified entity classes.
    /// Entities not in the filter are parsed (to advance the bit reader) but not stored.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_packet_entities_filtered(
        &mut self,
        msg: PacketEntities<'_>,
        class_info: &ClassInfo,
        serializers: &SerializerContainer,
        string_tables: &StringTableContainer,
        field_decode_ctx: &mut FieldDecodeContext,
        class_filter: &HashSet<&str>,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<()> {
        let update_count = usize::try_from(msg.updated_entries).map_err(|_| Error::Parse {
            context: format!(
                "negative packet-entity update count {}",
                msg.updated_entries
            ),
        })?;
        let limits = field_decode_ctx.limits();
        limits.ensure(
            "packet entity updates",
            update_count,
            limits.max_packet_entity_updates(),
        )?;
        let has_pvs_vis_bits = msg.has_pvs_vis_bits;
        self.serializer_bindings.refresh(class_info, serializers);
        let entity_data = msg.entity_data;
        let mut br = BitReader::new(entity_data);

        let mut entity_index: i32 = -1;

        for _ in 0..update_count {
            let next = i64::from(entity_index) + i64::from(br.read_ubitvar()?) + 1;
            if !(0..=MAX_ENTITY_INDEX).contains(&next) {
                return Err(Error::Parse {
                    context: format!("entity index {next} out of range"),
                });
            }
            entity_index = next as i32;

            // Read the 2-bit delta command (see CMD_LEAVE / CMD_CREATE_DELETE).
            let cmd = br.read_bits(2)? as u8;

            if cmd & CMD_LEAVE == 0 {
                if cmd & CMD_CREATE_DELETE != 0 {
                    self.handle_create_filtered(
                        entity_index,
                        &mut br,
                        class_info,
                        serializers,
                        string_tables,
                        field_decode_ctx,
                        class_filter,
                        fp_buf,
                    )?;
                } else {
                    if has_pvs_vis_bits > 0 && (br.read_bits(2)? & 0x01) != 0 {
                        continue;
                    }
                    self.handle_update_filtered(
                        entity_index,
                        &mut br,
                        class_info,
                        serializers,
                        field_decode_ctx,
                        fp_buf,
                    )?;
                }
            } else {
                self.handle_leave_filtered(entity_index, cmd & CMD_CREATE_DELETE != 0);
            }
        }

        Ok(())
    }

    /// Leave/delete handling for the class-filtered path.
    ///
    /// Tracked entities follow the same active/remove logic as [`handle_leave`].
    /// Untracked (skipped) entities carry no active flag; they are only dropped
    /// from the skip set on delete so their class is still known for updates
    /// while they merely go dormant.
    fn handle_leave_filtered(&mut self, index: i32, delete: bool) {
        if self.get(index).is_some() {
            self.handle_leave(index, delete);
        } else if delete {
            self.set_skipped(index, None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_create(
        &mut self,
        index: i32,
        br: &mut BitReader,
        class_info: &ClassInfo,
        serializers: &SerializerContainer,
        string_tables: &StringTableContainer,
        field_decode_ctx: &mut FieldDecodeContext,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<()> {
        let class_id = br.read_bits(class_info.bits())? as i32;
        let serial = br.read_bits(NUM_SERIAL_NUM_BITS as usize)? as u32;
        let _unknown = br.read_uvarint32()?;

        let class_entry = class_info.by_id(class_id).ok_or_else(|| Error::Parse {
            context: format!("unknown class_id {class_id}"),
        })?;

        let serializer = self
            .serializer_bindings
            .get(class_id, &class_entry.network_name, serializers)
            .ok_or_else(|| Error::Parse {
                context: format!("no serializer for {}", class_entry.network_name),
            })?;

        let mut entity = Entity::new(index, class_id, Arc::clone(&class_entry.network_name));
        entity.serial = serial;
        // Pre-size the field map to the class's field count: a create applies the
        // baseline plus the create delta, setting many fields at once, so starting
        // from an empty map otherwise rehashes repeatedly as it grows.
        entity.fields.reserve(serializer.fields.len());

        // Apply baseline from instancebaseline string table
        if let Some(baseline_data) = string_tables.instance_baseline(class_id) {
            let mut baseline_br = BitReader::new(baseline_data);
            entity
                .apply_update(&mut baseline_br, serializer, field_decode_ctx, fp_buf)
                .map_err(|err| Error::Parse {
                    context: format!(
                        "baseline for {} (class_id {}): {}",
                        class_entry.network_name, class_id, err
                    ),
                })?;
        }

        // Apply create delta
        entity
            .apply_update(br, serializer, field_decode_ctx, fp_buf)
            .map_err(|err| Error::Parse {
                context: format!(
                    "create delta for {} (class_id {}): {}",
                    class_entry.network_name, class_id, err
                ),
            })?;
        self.put_entity(index, entity);

        Ok(())
    }

    fn handle_update(
        &mut self,
        index: i32,
        br: &mut BitReader,
        serializers: &SerializerContainer,
        field_decode_ctx: &mut FieldDecodeContext,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<()> {
        let change = {
            let entity = usize::try_from(index)
                .ok()
                .and_then(|i| self.entities.get_mut(i))
                .and_then(Option::as_mut)
                .ok_or_else(|| Error::Parse {
                    context: format!("tried to update non-existent entity #{index}"),
                })?;
            let kind = if entity.active {
                EntityChangeKind::Updated
            } else {
                EntityChangeKind::Reactivated
            };
            entity.active = true;

            let serializer = self
                .serializer_bindings
                .get(entity.class_id, &entity.class_name, serializers)
                .ok_or_else(|| Error::Parse {
                    context: format!("no serializer for {}", entity.class_name),
                })?;

            entity.apply_update(br, serializer, field_decode_ctx, fp_buf)?;
            EntityChange::from_entity(kind, entity)
        };
        self.updated.push(index);
        self.changes.push(change);
        Ok(())
    }

    /// Returns `true` if the entity's class passed the filter and was stored.
    #[allow(clippy::too_many_arguments)]
    fn handle_create_filtered(
        &mut self,
        index: i32,
        br: &mut BitReader,
        class_info: &ClassInfo,
        serializers: &SerializerContainer,
        string_tables: &StringTableContainer,
        field_decode_ctx: &mut FieldDecodeContext,
        class_filter: &HashSet<&str>,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<bool> {
        let class_id = br.read_bits(class_info.bits())? as i32;
        let serial = br.read_bits(NUM_SERIAL_NUM_BITS as usize)? as u32;
        let _unknown = br.read_uvarint32()?;

        let class_entry = class_info.by_id(class_id).ok_or_else(|| Error::Parse {
            context: format!("unknown class_id {class_id}"),
        })?;

        let serializer = self
            .serializer_bindings
            .get(class_id, &class_entry.network_name, serializers)
            .ok_or_else(|| Error::Parse {
                context: format!("no serializer for {}", class_entry.network_name),
            })?;

        // A create is a fresh entity at this slot. Clear any stale entry for the
        // index from the *other* map so a later update can't misroute to a
        // previous occupant of the slot (e.g. a tracked weapon that left the PVS
        // and stayed cached while the slot was reused by a skipped entity). This
        // matters because leaves keep entities around, so slots get reused.
        if !class_filter.contains(class_entry.network_name.as_ref()) {
            // Skip this entity - just advance the bit reader, but track its
            // class_id so later updates to it can be skipped correctly.
            Entity::skip_update(br, serializer, field_decode_ctx, fp_buf)?;
            self.delete_entity(index);
            self.set_skipped(index, Some(class_id));
            return Ok(false);
        }
        // Full processing for filtered entities
        let mut entity = Entity::new(index, class_id, Arc::clone(&class_entry.network_name));
        entity.serial = serial;
        // Pre-size the field map to the class's field count: a create applies the
        // baseline plus the create delta, setting many fields at once, so starting
        // from an empty map otherwise rehashes repeatedly as it grows.
        entity.fields.reserve(serializer.fields.len());

        if let Some(baseline_data) = string_tables.instance_baseline(class_id) {
            let mut baseline_br = BitReader::new(baseline_data);
            entity.apply_update(&mut baseline_br, serializer, field_decode_ctx, fp_buf)?;
        }

        entity.apply_update(br, serializer, field_decode_ctx, fp_buf)?;
        self.set_skipped(index, None);
        self.put_entity(index, entity);

        Ok(true)
    }

    /// Returns `true` if this entity is tracked and was updated (vs. skipped).
    fn handle_update_filtered(
        &mut self,
        index: i32,
        br: &mut BitReader,
        class_info: &ClassInfo,
        serializers: &SerializerContainer,
        field_decode_ctx: &mut FieldDecodeContext,
        fp_buf: &mut Vec<FieldPath>,
    ) -> Result<bool> {
        // Check if we're tracking this entity
        if self.get(index).is_some() {
            self.handle_update(index, br, serializers, field_decode_ctx, fp_buf)?;
            return Ok(true);
        }

        // Entity is not tracked - check if we know its class from skipped creates
        if let Some(class_id) = self.skipped_class(index) {
            let class_entry = class_info.by_id(class_id).ok_or_else(|| Error::Parse {
                context: format!("unknown class_id {class_id}"),
            })?;

            let serializer = self
                .serializer_bindings
                .get(class_id, &class_entry.network_name, serializers)
                .ok_or_else(|| Error::Parse {
                    context: format!("no serializer for {}", class_entry.network_name),
                })?;

            // Skip this update
            Entity::skip_update(br, serializer, field_decode_ctx, fp_buf)?;
            return Ok(false);
        }

        // Index is in neither map. This can only happen if the entity was
        // created before filtering began; a well-formed stream from the start
        // never reaches here.
        Ok(false)
    }

    /// Look up an entity by its slot index.
    pub fn get(&self, index: i32) -> Option<&Entity> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.entities.get(i))
            .and_then(Option::as_ref)
    }

    /// Resolve a networked `CHandle`, checking both index and serial number.
    ///
    /// Returns `None` for invalid handles, empty slots, and reused slots. Dormant
    /// entities remain accessible; check [`Entity::active`] for PVS visibility.
    /// Game-event handles can use a different layout and must be converted first.
    pub fn get_by_handle(&self, handle: u32) -> Option<&Entity> {
        if handle == INVALID_ENTITY_HANDLE || handle == u32::MAX {
            return None;
        }
        self.get((handle & ENTITY_HANDLE_INDEX_MASK) as i32)
            .filter(|entity| entity.serial == handle >> MAX_EDICT_BITS)
    }

    /// Iterate over all slotted entities as `(index, entity)` pairs. Includes
    /// dormant (inactive) entities, so consumers that only want live ones must
    /// check [`Entity::active`]. Emitted in ascending index order.
    pub fn iter(&self) -> impl Iterator<Item = (i32, &Entity)> {
        self.entities
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|e| (i as i32, e)))
    }

    /// Indices of tracked entities created or updated during this tick.
    pub fn updated_indices(&self) -> &[i32] {
        &self.updated
    }

    /// Ordered lifecycle transitions decoded for the current tick.
    ///
    /// During playback these are visible inside the tick callback. They are
    /// cleared after the callback succeeds and before commands for the next
    /// tick are decoded. Initial sign-on changes are not exposed as playback
    /// changes, and a full-packet keyframe reports only the commands it actually
    /// decodes rather than synthesizing a snapshot of unchanged entities.
    pub fn entity_changes(&self) -> &[EntityChange] {
        &self.changes
    }

    /// Clear only the legacy created-or-updated index list.
    ///
    /// This keeps the existing [Self::updated_indices] behavior. Use
    /// [Self::clear_tick_changes] when consuming both tracking APIs.
    pub fn clear_updated(&mut self) {
        self.updated.clear();
    }

    /// Clear only the entity lifecycle log.
    pub fn clear_entity_changes(&mut self) {
        self.changes.clear();
    }

    /// Clear all per-tick entity tracking after a consumer callback.
    pub fn clear_tick_changes(&mut self) {
        // Only touched slots, never every live entity. Repeated updates are
        // harmless, and a reused slot owns its new generation's journal.
        for &index in &self.updated {
            if let Some(Some(entity)) = usize::try_from(index)
                .ok()
                .and_then(|i| self.entities.get_mut(i))
            {
                entity.changed_fields.clear();
            }
        }
        self.updated.clear();
        self.changes.clear();
    }

    /// Number of slotted entities (active or dormant), in constant time.
    pub fn len(&self) -> usize {
        self.occupied_count
    }

    /// Returns `true` when no slot holds an entity, in constant time.
    pub fn is_empty(&self) -> bool {
        self.occupied_count == 0
    }
}

#[cfg(test)]
mod coverage_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_new_is_empty() {
        let c = EntityContainer::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert!(c.get(0).is_none());
    }

    #[test]
    fn entity_fields_insert_and_get() {
        let mut e = Entity::new(1, 10, "TestClass".into());
        e.fields.insert(42, FieldValue::I32(100));
        assert!(matches!(e.fields.get(&42), Some(FieldValue::I32(100))));
    }

    #[test]
    fn container_insert_and_iter() {
        let mut c = EntityContainer::new();
        let e = Entity::new(5, 10, "Hero".into());
        c.put_entity(5, e);
        assert_eq!(c.len(), 1);
        assert!(!c.is_empty());
        assert!(c.get(5).is_some());
        assert_eq!(&*c.get(5).unwrap().class_name, "Hero");
    }

    #[test]
    fn container_iter_yields_entries() {
        let mut c = EntityContainer::new();
        c.put_entity(1, Entity::new(1, 1, "A".into()));
        c.put_entity(2, Entity::new(2, 2, "B".into()));
        let keys: Vec<i32> = c.iter().map(|(k, _)| k).collect();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn entity_basic_fields() {
        let e = Entity::new(7, 42, "NPC".into());
        assert_eq!(e.index, 7);
        assert_eq!(e.class_id, 42);
        assert_eq!(&*e.class_name, "NPC");
        assert_eq!(e.serial, 0);
        assert!(e.fields.is_empty());
    }
}
