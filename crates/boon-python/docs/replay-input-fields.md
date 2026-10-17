# Replay input fields

These fields expose facts already recorded by the Deadlock demo. They do not
infer an enemy's position from the server's world state while the enemy is
hidden.

```python
from boon import Demo

demo = Demo("match.dem", preload=False)
players = demo.snapshots("player_ticks", seconds=1.0)
enemy_icons = demo.enemy_minimap_ticks(every=64)
targets = demo.target_observations(ticks=[187554, 188194])
mid_boss_hp = demo.objectives.filter(
    demo.objectives["objective_type"] == "mid_boss"
)
```

## Current upgrades and boon count

`player_ticks.upgrade_ids` is the complete current
`CCitadelPlayerController.m_PlayerDataGlobal.m_vecUpgrades` vector at that
tick. An empty list is a recorded empty vector; null means the vector or an
entry was unavailable or could not be decoded. The vector contains raw upgrade
tokens, which may include hero abilities. Use the matching game-data catalog
to identify purchased items. Purchase events are not needed to reconstruct the
current token vector.

`player_ticks.boon_count` is read only from a directly recorded controller
field (`m_iBoonCount` or `m_nBoonCount`, with or without the `m_PlayerDataGlobal`
prefix). It is null when no such field exists in the replay schema or its value
is not an integer. This column never substitutes level or a calculated boon
schedule for a direct observation.

## Enemy minimap observations

`enemy_minimap_ticks()` reads `CCitadelTeam.m_vecFOWEntities`, classifying
recorded class-code 1 rows for enemy heroes. `observer_team` identifies the
team's shared minimap channel; this channel applies to each player on that
team. `visible_on_map` is the recorded flag. `map_x` and `map_y` are the native
0–255 minimap coordinates and are null while the icon is hidden or the
coordinates are invalid. These values are not world coordinates.

`target_entity_index` and `target_entity_serial` are filled only when the
recorded FOW index resolves uniquely to a current player pawn. `target_hero_id`
then comes from that pawn. `target_steam_id` comes from the one active player
controller whose current serial-bearing pawn handle points to the same pawn.
It is null if the controller or Steam ID is unavailable, the forward handles
disagree, or multiple controllers claim that pawn. The raw FOW index is always
retained so unresolved rows remain inspectable. `last_seen_tick`, `last_seen_x`, and `last_seen_y`
carry the last *shown* coordinate for that same resolved pawn identity on that
team's channel. These columns stay null when the pawn identity cannot be
resolved or has not yet been shown. The raw `tick_hidden_raw` is also exposed;
its units and display meaning are not interpreted here. Last-seen memory is
updated while scanning every tick, even when the output is sampled.

The interpretation of FOW class code and index encoding should be checked
against each demo build. `enemy_minimap_ticks()` does not claim direct
line-of-sight visibility or use a player pawn's hidden world position.

## Exact target observations

`target_observations(ticks=...)` emits rows for the currently active Guardian,
Walker, and Rift cash-in entities, paired with each recorded team minimap
channel. `target_entity_index` and `target_entity_serial` identify the exact
current entity lifetime and are metadata-only join keys, not gameplay
measurements.

Guardian and Walker minimap facts come only from a unique FOW row whose raw
index resolves to one current active exact-class entity and whose recorded
class code and target team agree. The decoder tests both possible index
encodings; if they resolve to different entities, or the row/channel is
ambiguous or incomplete, the fact is unknown. `shown` is nullable. `map_x`,
`map_y`, and `raw_health_byte` are read only when the exact row is recorded as
shown. The raw health byte has no verified fraction mapping. Server health and
world coordinates for Guardians and Walkers are not exported.

`target_class` contains the exact Boon entity class name. `fow_class_raw` is
audit metadata retained with a uniquely joined FOW row; its interpretation must
be qualified against each replay's source-patch provenance. The decoder does
not infer that an engine build number is an archive patch. `fow_entry_present`,
`identity_known`, `position_available`, and `health_byte_available` are audit/availability fields, not choice labels.

Rift state is a documented public/global game rule. The method reads only the
current cash-in entity's typed `m_bDisabled` and `m_nEnableState`, a boolean
capture-active observation from the current positive-or-zero GameRules start
field, and the current GameRules location when exactly one cash-in is active
and the current capture clock is positive. It does not export raw timer values,
scoring team, participant count, capture result, or future lifecycle rows.
Missing values remain null. The current location and active flag describe the
recorded global Rift state; they do not imply player attention, safety, or
actionability.

## Mid-boss health

The existing `objectives` dataset includes rows with
`objective_type == "mid_boss"`, `health`, `max_health`, and `tick`. It emits the
first observed nonzero-max-health state of each entity lifetime and subsequent
health/max-health changes. `entity_id` plus `entity_serial` distinguish a new
mid-boss from a later boss that reuses the same entity slot. Consumers can
detect an observed 50% crossing from those rows.
The separate `mid_boss` dataset records lifecycle events. Neither dataset
needs an invented roar event for a model to learn its response timing.


## Bounded replay ingress

`Demo(path, preload=False, bounded=True)` uses independent cold sequential
sessions over the read-only source mapping. It does not retain the prepared
full-packet/distinct-tick index. Exact seeks scan command headers with constant
memory and retain the same nearest-keyframe semantics.

`bounded_replay_spool(path, every=...)` and
`bounded_target_spool(path, ticks=...)` write source-ordered typed IPC batches
while collecting native rows. The destination must not exist. No complete
snapshot/event/target DataFrame is built or installed in `cached_datasets`.
Legacy public DataFrame APIs remain available with their existing behavior.
The completion receipt is written last and records actual cache/index owners,
peak table collector/batch rows and represented bytes. Batches contain at most
1,024 rows and 4 MiB of represented columns, with an explicit allowance for a
single oversized valid row. Collectors can contain the threshold plus one
completed source tick. Header-selected sample ticks are bounded at 60,000.

Barrier, remembered minimap and objective-generation history live on disk;
source clock pause operands live in a temporary tick file. Current decoder
entities/schema and the largest valid decoded command or PostMatchDetails
record remain live owners. `release_import_outputs()` releases redundant
legacy audit output caches after callers extract their auxiliary sidecars.
The optional progress callback receives cumulative/future spool bytes and batch
count; an exception aborts decode, allowing orchestration to enforce shared
per-device reservations and cancellation.

`native_spool_reader(path).frame(demo, tick, selections)` accepts only source
occurrence locations. It reads bounded IPC batches and emits versioned BTS66
tables and independent event-clock lists without Python row dictionaries. f32
values widen exactly to f64; maps/row order/nulls/integer ranges and duplicate
occurrences survive. Nonfinite typed cells become null as in the existing
persistence clean policy. The spool flags nonfinite player lifecycle marker
fields so callers can use the bounded reference row path for the exceptional
raw-NaN segmentation case. Kill endpoints use the legacy final hero map;
damage endpoints retain their source-time identities. Unknown-clock/postgame
raw occurrences remain present.

## Detailed rendering-entity exclusions

New detailed entity evidence declares `boon-render-entity-exclusions-v1` in
`excludedEntityClassPolicy`. Its `excludedEntityClasses` list is exact:

- `CAssignedLaneParticle`
- `CEnvCombinedLightProbeVolume`
- `CEnvCubemapFog`
- `CEnvVolumetricFogController`
- `CEnvVolumetricFogVolume`
- `CFogController`
- `CFuncBrush`
- `CParticleSystem`
- `CPathParticleRope`

The detailed producer skips those classes before field-value materialization.
The decoder retains skipped-slot class bookkeeping. Replacing an included
entity still records its disappearance with its original index and serial.
No rendering payload or identity-only archive placeholder is retained.

Ability, modifier, team/FOW, player, objective and other gameplay classes remain
included. The class policy does not change snapshot, event or target collectors.
Consumers reject an unknown policy, a different exclusion list, or a record
whose class is declared excluded. Previous valid archives retain their original
coverage. Importing an old archive does not remove its rendering-class rows.
The import metadata and replay receipt disclose the declared class policy.
Existing databases are not rewritten. New archive row counts and hashes can
differ from previous producers. Completion counts and integrity checks remain
mandatory.
