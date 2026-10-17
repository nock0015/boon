# Packet messages

`DEM_Packet`, `DEM_SignonPacket`, and the packet portion of
`DEM_FullPacket` carry an inner bit stream of network messages. A game adapter
first decodes the outer command protobuf to obtain its packet-data bytes, then
hands those bytes to [`PacketMessageIter`](crate::PacketMessageIter).

## Wire framing

Messages are concatenated at bit precision:

```text
[message type]  ubitvar    game network-message ID
[body size]     uvarint32  protobuf body length in bytes
[body]          N * 8 bits encoded protobuf bytes
```

`ubitvar` begins with a six-bit value and conditionally consumes more bits.
Consequently, the following size and payload need not start on a byte boundary.
The iterator stops when at most one byte remains, matching Source 2's packet
padding convention.

Each [`PacketMessageFrame`](crate::PacketMessageFrame) reports both bit offsets
and the encoded byte size. It does not interpret the message ID: identifiers
and generated protobuf types belong to the game adapter.

## Borrowing aligned payloads

When a body happens to be byte-aligned,
[`PacketMessageFrame::payload`](crate::PacketMessageFrame::payload) returns a
slice borrowed directly from the packet. Unaligned bodies must be reconstructed
because their bytes straddle the surrounding packet bytes.

Use [`PacketMessageFrame::payload_or_copy`](crate::PacketMessageFrame::payload_or_copy)
to take the zero-copy path when possible and reuse one scratch buffer otherwise:

```rust
use pbdems2::{PacketMessageIter, Result};

fn inspect_packet(packet_data: &[u8]) -> Result<()> {
    let mut scratch = Vec::new();
    for frame in PacketMessageIter::new(packet_data) {
        let frame = frame?;
        let message_type = frame.message_type();
        let payload = frame.payload_or_copy(&mut scratch)?;
        println!("message {message_type}: {} encoded bytes", payload.len());
    }
    Ok(())
}
```

The scratch buffer is untouched on the borrowed path. On the unaligned path it
is cleared and reused without an intermediate allocation.

## Message semantics

Games assign concrete IDs and protobuf schemas, but most adapters handle a
common set of Source 2 concepts:

| Concept | Adapter action |
|---|---|
| Server information | Update the tick interval |
| Flattened serializers | Install the entity field schemas |
| Create string table | Create and decode a neutral string table |
| Update string table | Apply entry deltas |
| Packet entities | Apply entity create/update/leave/delete deltas |
| User/game messages | Collect game-specific events or ignore them |

[`CommandContext::packet_messages`](crate::playback::CommandContext::packet_messages)
constructs the iterator with the parser's current limits. The adapter should
decode only message IDs it consumes; pbdems2 can skip the framed payloads of all
others without copying them.

## Compression boundaries

There are two independent compression layers:

- the complete outer command body may be Snappy-compressed, as described in
  [File structure](crate::guide::file_structure); and
- the entry-data blob inside a create-string-table protobuf may have its own
  Snappy flag.

Inner packet-message protobuf bodies do not carry a pbdems2 compression flag.
The adapter decodes the message body after packet framing; pbdems2 handles
string-table entry-data compression when the adapter submits a neutral
[`CreateStringTable`](crate::entity::CreateStringTable).

Message sizes are checked against
[`DecodeLimits::max_packet_message_bytes`](crate::DecodeLimits::max_packet_message_bytes)
before the adapter allocates or decodes a generated protobuf.
