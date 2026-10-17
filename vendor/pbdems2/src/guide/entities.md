# Entities and class information

Entities are the long-lived network objects reconstructed during playback:
players, projectiles, objectives, weapons, world state, and game rules all use
the same underlying mechanism. pbdems2 owns their shared wire lifecycle while
game crates decide which classes and fields have domain meaning.

## Class information

`DEM_ClassInfo` maps compact numeric IDs to network class and serializer names.
An adapter converts its generated message into
[`ClassEntry`](crate::entity::ClassEntry) values and installs them through
[`CommandContext::install_class_info`](crate::playback::CommandContext::install_class_info).

[`ClassInfo`](crate::entity::ClassInfo) provides constant-time lookup in both
directions. It also computes the number of bits used for a class ID in an
entity-create delta:

```text
class_id_bits = max(1, ceil(log2(max_class_id + 1)))
```

Equivalently, this is the bit length of the largest class ID, with a minimum of
one bit. Using `ceil(log2(max_class_id))` is incorrect at exact powers of two.

During creation, the class ID selects a class entry; its network name selects
the parsed [serializer](crate::guide::serializers) and becomes the entity's
`class_name`. The same name is used for class-filtered playback.

## Entity state

An [`Entity`](crate::Entity) contains:

- an array index in `0..=16383`;
- a numeric class ID and shared class name;
- an `active` flag recording whether it is currently in the client's
  potentially visible set (PVS); and
- decoded [`FieldValue`](crate::entity::FieldValue) values keyed by packed
  field paths.

[`EntityContainer`](crate::entity::EntityContainer) stores entities in a dense
slot array indexed directly by entity index. Empty slots contain no entity;
entities that merely leave the PVS remain in their slot with `active == false`.

## Packet-entity deltas

A game adapter converts its packet-entities protobuf to
[`PacketEntities`](crate::entity::PacketEntities). Its `updated_entries` field
says how many deltas follow in the bit-packed `entity_data`.

Entity indices are ascending and delta-encoded. Starting at `-1`, each record
computes:

```text
entity_index += read_ubitvar() + 1
```

The next two bits select the lifecycle operation:

| Bits | Operation | Result |
|---|---|---|
| `0b00` | Update | Apply field deltas; reactivate a dormant entity |
| `0b10` | Create | Select class, apply baseline, then apply create delta |
| `0b01` | Leave PVS | Keep the entity but mark it inactive |
| `0b11` | Leave and delete | Remove the entity from its slot |

A plain leave is not deletion. Dropping the entity on `0b01` loses the state
needed when a later update brings it back into the PVS.

Older packet-entities messages can place legacy PVS visibility bits before an
update. [`PacketEntities::has_pvs_vis_bits`](crate::entity::PacketEntities::has_pvs_vis_bits)
controls whether pbdems2 consumes them; modern demos normally set it to zero.

## Creation and update

The create-specific payload contains the class ID, a 17-bit create serial, and
an additional protocol varint before the field delta. pbdems2 then:

1. validates the class ID and looks up its serializer;
2. allocates an entity and pre-sizes its field map;
3. decodes the class's `instancebaseline`, if one exists; and
4. overlays the creation field delta.

An update looks up the existing entity and its serializer, marks the entity
active, and overlays only the changed fields. Field decoding follows the
Huffman-coded process described in
[Serializers and field paths](crate::guide::serializers).

Malformed indices, missing classes, missing serializers, and invalid field
paths return contextual errors. The update count and maximum entity index are
validated before any slot-array growth.

## Filtered playback

Filtered playback materializes only classes in a caller-provided set. A skipped
entity still has its class ID recorded, and its field paths and encoded values
are decoded or skipped according to the serializer so the bit reader stays
aligned. Later updates can therefore be skipped with the correct schema.

Tracked entities expose indices created or updated since the previous tick
through [`EntityContainer::updated_indices`](crate::entity::EntityContainer::updated_indices).
The playback driver clears this list after each callback, enabling change-only
datasets without rescanning every active entity.

## Entity handles

A networked `CHandle` field and an entity-create serial are related concepts
but do **not** use the same index width. A networked handle stores the entity
array index in its low **14 bits**:

```text
entity_index = handle & 0x3FFF
serial = handle >> 14
```

[`ENTITY_HANDLE_INDEX_MASK`](crate::entity::ENTITY_HANDLE_INDEX_MASK) owns this
mask. Using `0x7FFF` leaks one serial bit into the index and resolves every
handle with an odd serial to the wrong slot.

For a handle stored in an entity field, use
[`Entity::get_handle`](crate::Entity::get_handle) followed by
[`EntityContainer::get_by_handle`](crate::entity::EntityContainer::get_by_handle).
The lookup checks the serial and rejects invalid handles and reused slots.
Dormant entities remain available. Check `Entity::active` for visibility.

For optional protobuf handles, call `get_by_handle` after checking field presence.
[`protobuf_handle_index`](crate::entity::protobuf_handle_index) extracts only the
slot; it does not validate the occupant. Game-event handles can use a different
layout. Convert them before calling `get_by_handle`.

## Reading fields

Resolve dotted property names once with
[`Serializer::resolve_field_key`](crate::entity::Serializer::resolve_field_key),
then reuse the packed key with the typed `Entity::get_*` helpers. These helpers
avoid string traversal on each tick and define predictable defaults or
`Option` results for absent and differently typed values.

Source 2 positions are split between integer cell coordinates and quantized
in-cell offsets. [`Entity::world_position`](crate::Entity::world_position)
combines both halves into Hammer-unit coordinates; reading only the offset
produces values that reset at every cell boundary.
