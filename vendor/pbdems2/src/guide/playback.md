# Playback and adapters

[`DemoParser`](crate::DemoParser) combines the neutral command, serializer,
string-table, and entity systems. It deliberately delegates protobuf decoding
and game semantics to a [`DemoAdapter`](crate::DemoAdapter).

## Adapter boundary

For every relevant command, an adapter normally:

1. matches the protocol command ID;
2. decodes the command body with the game's generated protobuf type;
3. converts the shared fields into pbdems2 neutral structures; and
4. calls an operation on [`CommandContext`](crate::playback::CommandContext).

The context can:

- iterate inner packet framing;
- install flattened serializers and class information;
- update the tick interval;
- create, update, or fully refresh string tables; and
- apply packet-entity deltas, with or without a class filter.

Message IDs, generated protobuf types, game events, and domain models never
enter pbdems2. An adapter may collect those game-specific values in its own
state and expose that state to playback callbacks.

## Signon and playback phases

The first `DEM_SyncTick` divides processing into two phases:

```text
PBDEMS2 header
  -> signon commands
       serializers
       class information
       server/tick information
       initial string tables and entities
  -> DEM_SyncTick
  -> playback commands grouped by tick
  -> DEM_Stop
```

During signon, commands are processed sequentially with no entity class filter.
Before continuing, pbdems2 requires both serializers and class information and
refreshes the instance-baseline cache.

[`ParserState`](crate::ParserState) then owns the current neutral state:
serializers, class information, string tables, entities, tick interval, and
current tick.

There is no universal Source 2 tick interval. The caller supplies a positive
default appropriate for the game or recording source, and a server-information
message can replace it through
[`CommandContext::set_tick_interval`](crate::playback::CommandContext::set_tick_interval).

During playback, all commands with the same tick are applied before the tick
callback runs. On a tick change, pbdems2 emits the completed state, then clears
the per-tick entity and string-table change lists. `DEM_Stop` emits its
nonnegative final tick before ending playback.

## Playback modes

The main driver supports several access patterns:

| API | Behavior |
|---|---|
| [`initial_state`](crate::DemoParser::initial_state) | Decode through the first sync boundary only |
| [`run_to_end`](crate::DemoParser::run_to_end) | Stream every completed playback tick |
| [`try_run_to_end`](crate::DemoParser::try_run_to_end) | Let the callback stop with the adapter's error type |
| [`run_to_end_filtered`](crate::DemoParser::run_to_end_filtered) | Materialize only selected entity classes |
| [`parse_to_tick`](crate::DemoParser::parse_to_tick) | Restore the nearest keyframe and replay through a target tick |
| [`decode_segment`](crate::DemoParser::decode_segment) | Cold-start one bounded, filtered range |

The corresponding `*_with_adapter` methods expose mutable game-adapter state at
each tick. This is useful when one pass collects neutral entity state and
game-specific events together.

Filtered playback still consumes the complete entity bit stream. It skips
storage and value materialization for unselected classes while retaining enough
class state to decode their later deltas correctly.

## Full-packet keyframes and seeking

`DEM_FullPacket` contains a string-table snapshot plus packet data. The
header-only [`DemoIndex`](crate::demo::DemoIndex) records every full-packet
offset and tick.

To parse a target tick, pbdems2 always rebuilds signon state, then starts replay
at the last full packet at or before the target (or immediately after signon if
there is none). The adapter applies the full string-table snapshot and packet
before subsequent deltas.

A full packet is a protocol seek keyframe, but it is not a guarantee that every
game re-emits every class's complete semantic history. Full-packet restarts and
independent segments are exact only for entity classes the game fully
re-keyframes there. The consumer must choose compatible filters; pbdems2 cannot
infer this from neutral framing.

## Prepared and parallel playback

Adapters implementing [`CheckpointAdapter`](crate::CheckpointAdapter) can
capture their semantic signon state. Calling
[`DemoParser::prepare`](crate::DemoParser::prepare) produces a
[`PreparedPlayback`](crate::PreparedPlayback) containing:

- cloned neutral state immediately after signon;
- the adapter's signon checkpoint;
- the header-only seek index; and
- an identity check for the original demo allocation and decode limits.

The prepared value does not borrow the demo bytes. Each
[`PlaybackSession`](crate::PlaybackSession) restores independent neutral and
adapter state over a borrowed parser, so repeated runs cannot leak mutations
into one another.

[`PreparedPlayback::segment_plan`](crate::PreparedPlayback::segment_plan)
partitions the post-signon baseline, available full-packet intervals, and demo
tail into a bounded, nonempty list of
[`PlaybackSegment`](crate::PlaybackSegment) values. Sessions can decode those
segments on separate threads when the checkpoint and consumer state are
`Send + Sync`, subject to the full-packet completeness caveat above.

## Failures and limits

The adapter's associated error type must implement `From<pbdems2::Error>`. This
preserves command offsets, packet bit offsets, ticks, message IDs, decode-limit
violations, and protobuf/application errors without flattening them to strings.

Use the `try_*` methods when callbacks can fail because of output, cancellation,
database, or downstream validation. Parser and callback failures then travel
through the same typed error channel.
