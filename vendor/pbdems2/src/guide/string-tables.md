# String tables and instance baselines

String tables synchronize indexed collections of optional string keys and
optional binary values. Games use them for many purposes, but pbdems2 treats
their storage and delta encoding neutrally.

## Structure

A [`StringTable`](crate::entity::StringTable) has a protocol name, table-level
encoding flags, and an ordered list of
[`StringTableEntry`](crate::entity::StringTableEntry) values:

```text
table "instancebaseline"
  [0] key = "5",  user data = <encoded entity fields>
  [1] key = "12", user data = <encoded entity fields>
```

Either half of an entry may be absent. An update can, for example, replace user
data while retaining the previous key.

## Lifecycle

String-table state changes through three paths:

1. **Create**: a network message defines table metadata and its initial encoded
   entries. The adapter converts it to
   [`CreateStringTable`](crate::entity::CreateStringTable).
2. **Incremental update**: a message identifies a table by creation-order ID
   and carries changed entries, converted to
   [`UpdateStringTable`](crate::entity::UpdateStringTable).
3. **Full-packet snapshot**: `DEM_FullPacket` supplies current entries for
   existing tables, applied through
   [`CommandContext::apply_full_string_tables`](crate::playback::CommandContext::apply_full_string_tables).

Create data may be Snappy-compressed independently of outer-command
compression. Table metadata also determines whether user data has a fixed bit
width or carries a variable-length bit count.

[`StringTableContainer`](crate::entity::StringTableContainer) retains tables in
creation order because incremental updates address them numerically.

## Entry delta encoding

Within one encoded create/update payload, entry indices and strings are
delta-compressed:

- sequential entries can encode an implicit next index;
- other entries encode a jump from the previous index, plus two;
- a 32-slot circular history stores recent keys;
- a new key can name a history slot, copy a prefix from it, and append a
  null-terminated suffix; and
- history indices and prefix lengths use five bits.

The protocol key/history width is 32 bytes. User data is read either at the
table's fixed width or at the length encoded for that entry. pbdems2 bounds the
entry count, indices, encoded data, decompressed data, and user-data lengths
before allocating.

History is local to the encoded update being read; the current table entries
remain the durable state across network messages.

## Instance baselines

The table named `instancebaseline` is part of the entity protocol. Its entries
map a numeric class ID, represented as a decimal string key, to an encoded set
of default entity fields.

When a baseline table is created, updated, or replaced from a full packet,
pbdems2 refreshes a class-ID-to-bytes cache. Creating an entity then follows
this order:

1. construct an empty entity of the selected class;
2. decode and apply that class's cached baseline, if present; and
3. decode and apply the entity-specific creation delta.

This lets the wire stream omit values shared by every instance of a class.
Baseline bytes are interpreted with the same serializer and decode profile as
ordinary entity updates.

## Change-only consumers

[`StringTable::dirty_indices`](crate::entity::StringTable::dirty_indices)
contains the indices written since the previous playback callback. Consumers
that track a game-defined table can process only those entries rather than
rescanning the full table on every tick. Indices may repeat if an entry changes
more than once during the same callback interval.

The playback driver clears dirty lists after each callback. The current entries
remain available through [`StringTable::entries`](crate::entity::StringTable::entries)
and table lookup through
[`StringTableContainer::find_table`](crate::entity::StringTableContainer::find_table).

See [Entities and class information](crate::guide::entities) for how class IDs,
serializers, baselines, and entity deltas meet during creation.

For event tables, enable `CreateStringTable::with_change_tracking()` and read
`StringTable::changes()`. Each write retains its payload and order, even when
several writes use one slot in one tick. Omitted fields remain absent in the
change; `entries()` keeps the merged current state.

`StringTableChangeKind::Delta` identifies create/update writes.
`StringTableChangeKind::Snapshot` identifies full-packet state. A snapshot is
not a new event. `clear_dirty()` clears both indices and retained changes.
Tracking is off by default to avoid payload copies for lookup-only consumers.
