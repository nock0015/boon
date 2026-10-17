<div align="center">

# pbdems2

<p>
  <a href="https://crates.io/crates/pbdems2"><img src="https://img.shields.io/crates/v/pbdems2.svg?style=for-the-badge" alt="crates.io"></a>
  <a href="https://crates.io/crates/pbdems2"><img src="https://img.shields.io/crates/d/pbdems2.svg?style=for-the-badge" alt="crates.io Downloads"></a>
  <a href="https://docs.rs/pbdems2"><img src="https://img.shields.io/docsrs/pbdems2?style=for-the-badge" alt="docs.rs"></a>
  <a href="https://github.com/pnxenopoulos/pbdems2/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/pnxenopoulos/pbdems2/ci.yml?branch=main&style=for-the-badge" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg?style=for-the-badge" alt="License: MIT"></a>
</p>

</div>

pbdems2 is a game-neutral Rust library for Valve Source 2 demos. It handles the
shared PBDEMS2 container and entity wire format. Games keep their own protobufs
and domain logic.

It is built for projects like [Boon](https://github.com/pnxenopoulos/boon) for
Deadlock and [Awpy](https://github.com/pnxenopoulos/awpy) for CS2.

## What it handles

- PBDEMS2 headers, commands, packet messages, and Snappy bodies
- byte and bit readers
- serializers, field paths, and configurable field decoding
- entities, string tables, baselines, and coordinates
- playback, tick callbacks, seeking, filtering, and decode limits
- borrowed input and optional memory-mapped files

Game crates handle generated protobufs, events, domain models, constants, and
language bindings. Basically, if it's specific to a game, you won't find it in pbdems2!

## Format guide

The [PBDEMS2 format guide](https://docs.rs/pbdems2/latest/pbdems2/guide/index.html)
documents the shared file header and command stream, packet-message framing,
serializers and field paths, string tables and baselines, entities and handles,
and the adapter/playback lifecycle. Game-specific parsers can link to these
pages and keep only their protobuf and domain documentation locally.

## Game adapters

Each game supplies a neutral decode profile for its wire quirks:

```rust
use pbdems2::entity::{
    BareCharEncoding, DecodeProfile, PreciseQAngleMode,
};

const PROFILE: DecodeProfile = DecodeProfile::new(
    BareCharEncoding::NullTerminatedString,
    PreciseQAngleMode::Centered,
)
.with_ammo_field("m_iClip1");
```

Adapters decode their protobuf messages, convert them to pbdems2 input types,
and pass them to `CommandContext`. Generated messages never enter pbdems2.

## Parsing

`Demo` gives you allocation-free command iteration and a header-only seek
index. `DemoParser` adds signon setup, playback, tick callbacks, filtering, and
full-packet seeks.

```rust
let limits = DecodeLimits::default()
    .with_max_command_body_bytes(32 * 1024 * 1024)
    .with_max_packet_message_bytes(16 * 1024 * 1024);
let parser = DemoParser::with_limits(&demo_bytes, limits)?;
let state = parser.try_run_to_end(&mut adapter, 1.0 / 64.0, |tick| {
    consume_tick(tick.tick(), tick.entities())?;
    Ok(())
})?;
```

Use the `try_*` methods when a tick callback can fail. The regular methods take
infallible callbacks.

Inside a tick callback, <code>EntityContainer::entity_changes</code> exposes ordered
create, update, reactivation, leave-PVS, and delete records. Each record keeps
the entity index, serial, class ID, and shared class name. This makes slot reuse
unambiguous without cloning full entities or allocating a class name per
change. The log clears after a successful callback. Sign-on establishes the
initial state without emitting changes, and full-packet keyframes report the
commands they decode rather than a synthetic snapshot. The older
<code>updated_indices</code> API keeps its existing behavior.

`CommandContext::packet_messages` reads the common packet framing while the
adapter decides what each message ID means. Aligned payloads are borrowed.
`PacketMessageFrame::payload_or_copy` returns that borrowed slice when possible
and otherwise reconstructs the payload in one caller-owned reusable buffer.

`Demo::header` exposes validated file-info and spawn-groups offsets.
`parse_to_tick` starts at the nearest full packet and replays the needed deltas.

## Prepared playback

Adapters that implement `CheckpointAdapter` can decode signon and build the
seek index once, then create independent sessions for repeated or parallel
work:

```rust
let prepared = parser.prepare(adapter, 1.0 / 64.0)?;

let state = prepared
    .session(parser)?
    .try_run_to_end(|tick| {
        consume_tick(tick.tick(), tick.entities())?;
        Ok(())
    })?;
```

`PreparedPlayback` does not borrow the encoded bytes, so an owning game parser
can cache it without becoming self-referential. Each session verifies that its
`DemoParser` uses the same allocation and decode limits, then clones neutral
state and restores a fresh adapter from the signon checkpoint.

Use `try_run_to_end_with_adapter` and the corresponding filtered or segment
methods when a callback also needs game-specific adapter state. This supports a
single entity-and-event pass without putting generated messages or event types
in pbdems2.

Prepared values can be shared across threads when their checkpoint state is
`Send + Sync`. `PreparedPlayback::segment_plan` produces a bounded,
never-empty set of ranges across the post-signon baseline, full-packet
keyframes, and the demo tail; each range can be decoded in an independent
session. Full-packet segment restarts are exact only for classes that the game
fully re-keyframes in those snapshots.

## Limits and large files

`DecodeLimits` bounds untrusted lengths and counts before allocation. Limit,
allocation, command, and packet errors keep useful context.

The optional `mmap` feature provides an owning read-only map:

```toml
pbdems2 = { version = "0.3", features = ["mmap"] }
```

```rust
// SAFETY: Keep the file unchanged and untruncated while `mapped` exists.
let mapped = unsafe { pbdems2::MappedDemo::open("match.dem")? };
let parser = mapped.parser()?;
```

The constructor is unsafe because the caller must keep the mapped file stable.

## Install

```toml
[dependencies]
pbdems2 = "0.3"
```

Serde support is on by default and includes <code>EntityId</code> and lifecycle
records. Turn it off if you do not serialize decoded values:

```toml
pbdems2 = { version = "0.3", default-features = false }
```

Consumer crates can keep old module paths with re-exports:

```rust
pub mod entity {
    pub use pbdems2::entity::*;
}
```

## CLI

The private `pbdems2-cli` crate builds a `pbdems2` inspector. It maps the file,
validates every command, and reports offsets, sizes, ticks, seek points, and
compression stats.

```bash
cargo run -p pbdems2-cli -- summary match.dem
cargo run -p pbdems2-cli -- commands match.dem
cargo run -p pbdems2-cli -- index match.dem
cargo run -p pbdems2-cli -- validate match.dem
```

Add `--json` for machine-readable output. Release archives include Linux,
macOS, and Windows binaries. The CLI is not published to crates.io, but is available on the [Releases](https://github.com/pnxenopoulos/pbdems2/releases).
