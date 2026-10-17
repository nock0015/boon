# Serializers and field paths

Source 2 serializers define the schema and wire encoding of networked entity
fields. They are the Source 2 successor to Source 1 send tables. Without them,
an entity delta is only a sequence of field-path operations and undecorated
bits.

## Flattened serializer input

Serializer definitions normally arrive in `DEM_SendTables`. The generated
protobuf type is game-owned, so an adapter converts it into
[`FlattenedSerializer`](crate::entity::FlattenedSerializer), which contains:

1. **symbols**: a shared string pool containing serializer names, field names,
   type strings, send nodes, and encoder names;
2. **fields**: a shared pool of [`FlattenedField`](crate::entity::FlattenedField)
   records whose string properties are symbol indices; and
3. **serializer definitions**: a serializer-name symbol plus an ordered list of
   indices into the field pool.

[`SerializerContainer::parse`](crate::entity::SerializerContainer::parse)
resolves those indices and builds an immutable graph of
[`Serializer`](crate::entity::Serializer) and
[`SerializerField`](crate::entity::SerializerField) values. The container is
safe to share across threads after construction.

A serializer is an ordered collection of fields:

```text
Serializer "CExamplePlayerPawn"
  [0] m_iHealth: int32
  [1] m_vecVelocity: Vector
  [2] m_pMovementServices: CExampleMovementServices*
      [0] m_nButtons: uint64
      [1] m_flSpeed: float32
```

Nested serializers form the hierarchy addressed by dotted names and field
paths.

## Field metadata

Each flattened field can provide:

| Property | Purpose |
|---|---|
| `var_type` | Source 2 type string |
| `var_name` | Networked field name |
| `bit_count` | Quantized width or angle precision |
| `low_value`, `high_value` | Quantized numeric range |
| `encode_flags` | Quantized-float behavior |
| `var_encoder` | Hint such as `fixed8`, `coord`, `normal`, or `qangle_precise` |
| `field_serializer_name` | Nested serializer for a composite value |
| `send_node` | Dotted name prefix |
| `polymorphic` | Pointer whose concrete serializer is selected on the wire |

pbdems2 parses type expressions such as fixed arrays, pointers, and
`CNetworkUtlVectorBase<T>` into [`FieldType`](crate::entity::FieldType). It then
selects a primitive decoder and any structural behavior in
[`FieldMetadata`](crate::entity::FieldMetadata).

Decoded primitives are stored as [`FieldValue`](crate::entity::FieldValue):
booleans, signed and unsigned integers, floats, raw byte strings, fixed vectors,
arbitrary float vectors, and Euler angles. Strings stay as bytes because the
wire format does not guarantee UTF-8.

The `fixed8` encoder reads one byte per scalar or array element. Signed integer
types are sign-extended. Fields without this encoder keep their type-based
encoding, including variable-length integers.

## Game decode profiles

The schema is shared, but a few type names have game-dependent wire behavior.
Each adapter supplies a [`DecodeProfile`](crate::entity::DecodeProfile), which
selects details such as:

- whether a bare `char` is an integer or a null-terminated string;
- the centering mode for precise angles;
- symbolic fixed-array lengths;
- game-defined pointer and dynamic-serializer types; and
- the field used for ammo-specific integer decoding.

Generated protobuf types and these explicit dialect choices remain outside
pbdems2. This keeps the serializer engine neutral without guessing from a game
name.

## Field paths

An entity update first encodes the paths of every changed field. A path is a
hierarchical sequence of at most seven indices:

```text
[3]       fourth top-level field
[3, 2]    third field in the fourth field's nested serializer
[3, 2, 0] first field one level deeper
```

The paths are not sent literally. Source 2 uses a fixed weighted Huffman tree
of **40 field-path operations**. Common operations increment the current index;
others push or pop nesting levels, jump non-topographically, or terminate the
list. [`read_field_paths`](crate::entity::field_path::read_field_paths) applies
those operations to a running [`FieldPath`](crate::entity::field_path::FieldPath).

Each completed path is packed into a `u64` key. Entity field maps use that key
for compact lookup. Consumers generally resolve names once through
[`Serializer::resolve_field_key`](crate::entity::Serializer::resolve_field_key)
and reuse the resulting key for every entity and tick, rather than repeatedly
walking strings on the hot path.

## Decode sequence

For one entity create or update, pbdems2:

1. decodes the Huffman-coded list of changed field paths;
2. walks the serializer graph for each path;
3. invokes the decoder chosen from that field's metadata; and
4. inserts the resulting [`FieldValue`](crate::entity::FieldValue) under the
   packed path key.

Malformed paths, symbol indices, serializer references, and array lengths
return contextual errors rather than indexing unchecked memory. Their counts
and allocations are also bounded by [`DecodeLimits`](crate::DecodeLimits).
