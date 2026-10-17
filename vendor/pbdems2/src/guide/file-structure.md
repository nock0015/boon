# File structure

A Source 2 demo is an outer **PBDEMS2 container** followed by a stream of
framed commands. pbdems2 owns this framing. The protobuf schema carried by a
command is selected and decoded by the game adapter.

## Fixed header

Every file starts with 16 bytes:

| Offset | Size | Encoding | Meaning |
|---:|---:|---|---|
| 0 | 8 | bytes | Magic `PBDEMS2\0` |
| 8 | 4 | little-endian `u32` | Absolute offset of `DEM_FileInfo` |
| 12 | 4 | little-endian `u32` | Absolute offset of `DEM_SpawnGroups` |

An offset of zero means that the corresponding command is absent. A nonzero
offset must point inside the command stream, after the fixed header and before
the end of the file. [`Demo::new`](crate::demo::Demo::new) validates the magic
and both optional offsets before exposing the file.

The file-info offset makes footer metadata directly addressable without a
linear scan. The spawn-groups offset serves the same role for games that record
spawn-group state.

## Command frames

Starting at byte 16, commands are concatenated without padding:

```text
[raw command]  uvarint32  command ID plus compression flag
[tick]         uvarint32  demo tick, exposed as i32
[body size]    uvarint32  encoded body length in bytes
[body]         bytes      protobuf payload or Snappy-compressed payload
```

The compression flag is bit `0x40` (`64`). pbdems2 exposes the command ID with
that flag removed and reports compression separately through
[`CmdHeader`](crate::demo::CmdHeader). Compressed bodies use raw Snappy framing;
the decoded size is checked before allocation.

[`CommandFrame`](crate::demo::CommandFrame) borrows the encoded body from the
demo. Calling [`CommandFrame::decode_body`](crate::demo::CommandFrame::decode_body)
copies an uncompressed body or decompresses a compressed body into a reusable
caller-owned buffer.

## Protocol command IDs

The outer IDs are shared across Source 2 games and are available as constants
in [`demo::command`](crate::demo::command):

| ID | Command | Role |
|---:|---|---|
| 0 | `DEM_Stop` | End of the command stream |
| 1 | `DEM_FileHeader` | Recording metadata |
| 2 | `DEM_FileInfo` | Playback summary/footer |
| 3 | `DEM_SyncTick` | Boundary between signon and playback |
| 4 | `DEM_SendTables` | Flattened serializer definitions |
| 5 | `DEM_ClassInfo` | Numeric entity class mapping |
| 6 | `DEM_StringTables` | Complete string-table snapshot |
| 7 | `DEM_Packet` | Network messages for a playback tick |
| 8 | `DEM_SignonPacket` | Network messages from signon |
| 9 | `DEM_ConsoleCmd` | Recorded console command |
| 10 | `DEM_CustomData` | Game-defined data |
| 11 | `DEM_CustomDataCallbacks` | Custom-data registry |
| 12 | `DEM_UserCmd` | Recorded client input |
| 13 | `DEM_FullPacket` | String-table snapshot plus packet keyframe |
| 14 | `DEM_SaveGame` | Embedded save-game data |
| 15 | `DEM_SpawnGroups` | Spawn-group state |
| 16 | `DEM_AnimationData` | Animation samples |
| 17 | `DEM_AnimationHeader` | Animation-run metadata |
| 18 | `DEM_Recovery` | Recording recovery data |

The IDs are neutral, but their bodies are protobuf messages. pbdems2 deliberately
does not contain those generated types: a [`DemoAdapter`](crate::DemoAdapter)
decodes them and converts the relevant values into neutral pbdems2 inputs.

## Iteration and indexing

[`Demo::commands`](crate::demo::Demo::commands) is a strict, allocation-free
iterator over command frames. Framing, sizes, and truncation errors retain the
absolute command offset, command ID, and tick when available.

[`Demo::index`](crate::demo::Demo::index) performs a header-only pass and
records:

- the offset immediately after the first `DEM_SyncTick`;
- every `DEM_FullPacket` offset and tick; and
- distinct nonnegative playback ticks.

The resulting [`DemoIndex`](crate::demo::DemoIndex) supports seeking without
decoding command bodies. See [Playback and adapters](crate::guide::playback) for how
those positions are used.

All encoded and decoded sizes are subject to [`DecodeLimits`](crate::DecodeLimits).
Applications parsing untrusted demos should select limits before constructing
the [`Demo`](crate::demo::Demo) or [`DemoParser`](crate::DemoParser).
