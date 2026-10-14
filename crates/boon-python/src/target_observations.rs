//! Exact-identity objective observations from recorded team FOW and current Rift rules.
//!
//! Entity index/serial columns are join metadata. They are not model features.
//! Guardian and Walker server health/position are intentionally not exported.

use crate::*;
use boon_parser::{Context, Entity, FieldValue, Serializer};

fn numeric_column<T>(name: &str, values: Vec<T>) -> Column
where
    Series: NamedFromOwned<Vec<T>>,
{
    Series::from_vec(name.into(), values).into_column()
}

const TARGETS: [TargetSpec; 3] = [
    TargetSpec {
        class_name: "CNPC_TrooperBoss",
        target_class: "CNPC_TrooperBoss",
        objective_kind: "guardian",
        fow_class: 5,
    },
    TargetSpec {
        class_name: "CNPC_Boss_Tier2",
        target_class: "CNPC_Boss_Tier2",
        objective_kind: "walker",
        fow_class: 29,
    },
    TargetSpec {
        class_name: "CCitadel_KothCashIn",
        target_class: "CCitadel_KothCashIn",
        objective_kind: "rift",
        fow_class: 92,
    },
];

// FOW codes are emitted as audit metadata and associated only with the exact
// current entity class/team/index. Callers qualify their meanings by source.
const RIFT_COORD_SANITY: f32 = 1.0e6;

#[derive(Clone, Copy)]
struct TargetSpec {
    class_name: &'static str,
    target_class: &'static str,
    objective_kind: &'static str,
    fow_class: i64,
}

#[derive(Clone, Copy, Default)]
struct TargetKeys {
    team: Option<u64>,
    rift_disabled: Option<u64>,
    rift_enable_state: Option<u64>,
}

#[derive(Clone, Copy, Default)]
struct RulesKeys {
    capture_started: Option<u64>,
    location: Option<u64>,
}

#[derive(Clone, Copy)]
struct Target<'a> {
    entity: &'a Entity,
    spec: TargetSpec,
    team: Option<i64>,
    keys: TargetKeys,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FowFact {
    entry_present: Option<bool>,
    fow_class_raw: Option<i64>,
    shown: Option<bool>,
    map_x: Option<i64>,
    map_y: Option<i64>,
    raw_health_byte: Option<i64>,
    position_available: bool,
    health_byte_available: bool,
    reason: &'static str,
}

impl FowFact {
    fn unknown(reason: &'static str) -> Self {
        Self {
            entry_present: None,
            fow_class_raw: None,
            shown: None,
            map_x: None,
            map_y: None,
            raw_health_byte: None,
            position_available: false,
            health_byte_available: false,
            reason,
        }
    }
}

fn integer(value: &FieldValue) -> Option<i64> {
    match value {
        FieldValue::I32(n) => Some(i64::from(*n)),
        FieldValue::U32(n) => Some(i64::from(*n)),
        FieldValue::I64(n) => Some(*n),
        FieldValue::U64(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn field_i64(entity: &Entity, serializer: &Serializer, path: &str) -> Option<i64> {
    integer(entity.get_by_name(path, serializer)?)
}

fn entry_value<'a>(
    entity: &'a Entity,
    serializer: &Serializer,
    index: usize,
    name: &str,
) -> Option<&'a FieldValue> {
    entity
        .get_by_name(&format!("m_vecFOWEntities.{index}.{name}"), serializer)
        .or_else(|| entity.get_by_name(&format!("m_vecFOWEntities.{index:04}.{name}"), serializer))
}

fn entry_i64(entity: &Entity, serializer: &Serializer, index: usize, name: &str) -> Option<i64> {
    integer(entry_value(entity, serializer, index, name)?)
}

fn entry_bool(entity: &Entity, serializer: &Serializer, index: usize, name: &str) -> Option<bool> {
    match entry_value(entity, serializer, index, name)? {
        FieldValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn field_bool(entity: &Entity, key: Option<u64>) -> Option<bool> {
    match entity.field_value(key)? {
        FieldValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn field_integer(entity: &Entity, key: Option<u64>) -> Option<i64> {
    integer(entity.field_value(key)?)
}

fn field_f32(entity: &Entity, key: Option<u64>) -> Option<f32> {
    match entity.field_value(key)? {
        FieldValue::F32(value) => Some(*value),
        _ => None,
    }
}

fn field_vector3(entity: &Entity, key: Option<u64>) -> Option<[f32; 3]> {
    match entity.field_value(key)? {
        FieldValue::Vector3(value) => Some(*value),
        FieldValue::FloatVector(value) if value.len() >= 3 => Some([value[0], value[1], value[2]]),
        _ => None,
    }
}

fn byte(value: Option<i64>) -> Option<i64> {
    value.filter(|value| (0..=255).contains(value))
}

/// Return the exact target entity identities that one raw team-FOW row could
/// describe. Both plausible index encodings are tested; callers must reject
/// zero or multiple exact identities rather than prefer one encoding.
fn candidate_target_positions(
    raw_index: i64,
    target_class: &'static str,
    fow_class: i64,
    target_team: i64,
    row_class: Option<i64>,
    row_team: Option<i64>,
    target_ids: &[(i32, &'static str, Option<i64>)],
) -> Vec<usize> {
    if row_class != Some(fow_class) || row_team != Some(target_team) {
        return Vec::new();
    }
    let direct = i32::try_from(raw_index).ok();
    let alias = (raw_index >= 0 && raw_index % 2 == 0)
        .then(|| i32::try_from(raw_index / 2).ok())
        .flatten();
    let mut candidates = Vec::new();
    for (position, (target_index, candidate_class, candidate_team)) in target_ids.iter().enumerate()
    {
        if *candidate_class == target_class
            && *candidate_team == Some(target_team)
            && [direct, alias]
                .into_iter()
                .flatten()
                .any(|index| index == *target_index)
            && !candidates.contains(&position)
        {
            candidates.push(position);
        }
    }
    candidates
}

/// Read FOW payload only after the caller has uniquely matched this shown row
/// to the exact current serial-bearing target identity.
fn shown_fact(serializer: &Serializer, team: &Entity, index: usize) -> FowFact {
    let shown = entry_bool(team, serializer, index, "m_bVisibleOnMap");
    match shown {
        Some(false) => FowFact {
            entry_present: Some(true),
            fow_class_raw: entry_i64(team, serializer, index, "m_eClass"),
            shown,
            map_x: None,
            map_y: None,
            raw_health_byte: None,
            position_available: false,
            health_byte_available: false,
            reason: "icon_hidden",
        },
        Some(true) => {
            // Hidden FOW coordinates and health are never read or exported.
            let map_x = byte(entry_i64(team, serializer, index, "m_nPositionX"));
            let map_y = byte(entry_i64(team, serializer, index, "m_nPositionY"));
            let raw_health_byte = byte(entry_i64(team, serializer, index, "m_nHealthPercent"));
            let position_available = map_x.is_some() && map_y.is_some();
            let health_byte_available = raw_health_byte.is_some();
            FowFact {
                entry_present: Some(true),
                fow_class_raw: entry_i64(team, serializer, index, "m_eClass"),
                shown,
                map_x,
                map_y,
                raw_health_byte,
                position_available,
                health_byte_available,
                reason: if position_available || health_byte_available {
                    "shown_exact_target_row"
                } else {
                    "shown_payload_unavailable"
                },
            }
        }
        None => FowFact {
            entry_present: Some(true),
            fow_class_raw: entry_i64(team, serializer, index, "m_eClass"),
            shown: None,
            map_x: None,
            map_y: None,
            raw_health_byte: None,
            position_available: false,
            health_byte_available: false,
            reason: "shown_state_unavailable",
        },
    }
}

#[derive(Default)]
struct TargetObservationCols {
    tick: Vec<i32>,
    observer_team: Vec<i64>,
    target_class: Vec<&'static str>,
    fow_class_raw: Vec<Option<i64>>,
    target_team: Vec<Option<i64>>,
    target_entity_index: Vec<i32>,
    target_entity_serial: Vec<u32>,
    identity_known: Vec<bool>,
    fow_entry_present: Vec<Option<bool>>,
    shown: Vec<Option<bool>>,
    map_x: Vec<Option<i64>>,
    map_y: Vec<Option<i64>>,
    raw_health_byte: Vec<Option<i64>>,
    position_available: Vec<bool>,
    health_byte_available: Vec<bool>,
    observation_reason: Vec<&'static str>,
    rift_disabled: Vec<Option<bool>>,
    rift_enable_state: Vec<Option<i64>>,
    capture_active_known: Vec<bool>,
    capture_active: Vec<Option<bool>>,
    rift_position_available: Vec<bool>,
    rift_x: Vec<Option<f32>>,
    rift_y: Vec<Option<f32>>,
    rift_z: Vec<Option<f32>>,
}

impl TargetObservationCols {
    fn push(
        &mut self,
        tick: i32,
        observer_team: i64,
        target: Target<'_>,
        fact: FowFact,
        rules: Option<&Entity>,
        rules_keys: RulesKeys,
        unique_live_rift: bool,
    ) {
        let id = target.entity.id();
        self.tick.push(tick);
        self.observer_team.push(observer_team);
        self.target_class.push(target.spec.class_name);
        self.fow_class_raw.push(fact.fow_class_raw);
        self.target_team.push(target.team);
        self.target_entity_index.push(id.index);
        self.target_entity_serial.push(id.serial);
        self.identity_known.push(true);
        self.fow_entry_present.push(fact.entry_present);
        self.shown.push(fact.shown);
        self.map_x.push(fact.map_x);
        self.map_y.push(fact.map_y);
        self.raw_health_byte.push(fact.raw_health_byte);
        self.position_available.push(fact.position_available);
        self.health_byte_available.push(fact.health_byte_available);
        self.observation_reason.push(fact.reason);

        let is_rift = target.spec.objective_kind == "rift";
        self.rift_disabled.push(if is_rift {
            field_bool(target.entity, target.keys.rift_disabled)
        } else {
            None
        });
        self.rift_enable_state.push(if is_rift {
            field_integer(target.entity, target.keys.rift_enable_state)
        } else {
            None
        });

        let capture_started = if is_rift {
            rules.and_then(|entity| field_f32(entity, rules_keys.capture_started))
        } else {
            None
        };
        let capture_known = capture_started.is_some_and(|value| value.is_finite() && value >= 0.0);
        let capture_active = capture_started
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value > 0.0);
        let location = if is_rift && unique_live_rift && capture_active == Some(true) {
            rules.and_then(|entity| field_vector3(entity, rules_keys.location))
        } else {
            None
        };
        let location = location.filter(valid_rift_location);
        self.capture_active_known.push(capture_known);
        self.capture_active.push(capture_active);
        self.rift_position_available.push(location.is_some());
        self.rift_x.push(location.map(|value| value[0]));
        self.rift_y.push(location.map(|value| value[1]));
        self.rift_z.push(location.map(|value| value[2]));
    }

    fn into_dataframe(self) -> PyResult<DataFrame> {
        df_from_columns(vec![
            numeric_column("tick", self.tick),
            numeric_column("observer_team", self.observer_team),
            Column::new("target_class".into(), self.target_class),
            Column::new("fow_class_raw".into(), self.fow_class_raw),
            Column::new("target_team".into(), self.target_team),
            numeric_column("target_entity_index", self.target_entity_index),
            numeric_column("target_entity_serial", self.target_entity_serial),
            Column::new("identity_known".into(), self.identity_known),
            Column::new("fow_entry_present".into(), self.fow_entry_present),
            Column::new("shown".into(), self.shown),
            Column::new("map_x".into(), self.map_x),
            Column::new("map_y".into(), self.map_y),
            Column::new("raw_health_byte".into(), self.raw_health_byte),
            Column::new("position_available".into(), self.position_available),
            Column::new("health_byte_available".into(), self.health_byte_available),
            Column::new("observation_reason".into(), self.observation_reason),
            Column::new("rift_disabled".into(), self.rift_disabled),
            Column::new("rift_enable_state".into(), self.rift_enable_state),
            Column::new("capture_active_known".into(), self.capture_active_known),
            Column::new("capture_active".into(), self.capture_active),
            Column::new(
                "rift_position_available".into(),
                self.rift_position_available,
            ),
            Column::new("rift_x".into(), self.rift_x),
            Column::new("rift_y".into(), self.rift_y),
            Column::new("rift_z".into(), self.rift_z),
        ])
        .map_err(|error| {
            InvalidDemoError::new_err(format!(
                "Failed to create target-observation DataFrame: {error}"
            ))
        })
    }
}

fn valid_rift_location(location: &[f32; 3]) -> bool {
    location
        .iter()
        .all(|value| value.is_finite() && value.abs() < RIFT_COORD_SANITY)
        && *location != [0.0; 3]
}

fn target_keys(ctx: &Context) -> HashMap<&'static str, TargetKeys> {
    TARGETS
        .iter()
        .filter_map(|spec| {
            let serializer = ctx.serializers().get(spec.class_name)?;
            Some((
                spec.class_name,
                TargetKeys {
                    team: serializer.resolve_field_key("m_iTeamNum"),
                    rift_disabled: (spec.objective_kind == "rift")
                        .then(|| serializer.resolve_field_key("m_bDisabled"))
                        .flatten(),
                    rift_enable_state: (spec.objective_kind == "rift")
                        .then(|| serializer.resolve_field_key("m_nEnableState"))
                        .flatten(),
                },
            ))
        })
        .collect()
}

fn rules_keys(ctx: &Context) -> RulesKeys {
    let serializer = ctx.serializers().get("CCitadelGameRulesProxy");
    RulesKeys {
        capture_started: serializer
            .and_then(|s| s.resolve_field_key("m_pGameRules.m_timeKothCashInStarted")),
        location: serializer
            .and_then(|s| s.resolve_field_key("m_pGameRules.m_vKothCashInCurrentLocation")),
    }
}

fn targets_at<'a>(ctx: &'a Context, keys: &HashMap<&'static str, TargetKeys>) -> Vec<Target<'a>> {
    let mut targets = Vec::new();
    for (_, entity) in ctx.entities().iter().filter(|(_, entity)| entity.active) {
        let Some(spec) = TARGETS
            .iter()
            .find(|spec| entity.class_name.as_ref() == spec.class_name)
            .copied()
        else {
            continue;
        };
        let key = keys.get(spec.class_name).copied().unwrap_or_default();
        let team = entity.field_value(key.team).and_then(integer);
        targets.push(Target {
            entity,
            spec,
            team,
            keys: key,
        });
    }
    targets
}

fn collect_team(
    ctx: &Context,
    observer_team: i64,
    targets: &[Target<'_>],
    rules: Option<&Entity>,
    rules_keys: RulesKeys,
    columns: &mut TargetObservationCols,
) {
    let serializer = ctx.serializers().get("CCitadelTeam");
    let channels: Vec<_> = ctx
        .entities()
        .iter()
        .filter(|(_, entity)| {
            entity.active
                && entity.class_name.as_ref() == "CCitadelTeam"
                && serializer.and_then(|serializer| field_i64(entity, serializer, "m_iTeamNum"))
                    == Some(observer_team)
        })
        .map(|(_, entity)| entity)
        .collect();
    let unique_live_rift = targets
        .iter()
        .filter(|target| target.spec.objective_kind == "rift")
        .count()
        == 1
        && rules
            .and_then(|entity| field_f32(entity, rules_keys.capture_started))
            .is_some_and(|value| value.is_finite() && value > 0.0);

    for target in targets {
        let fact = if channels.is_empty() {
            FowFact::unknown("team_channel_missing")
        } else if channels.len() != 1 {
            FowFact::unknown("team_channel_ambiguous")
        } else if target.team.is_none() {
            FowFact::unknown("target_team_unavailable")
        } else {
            let team = channels[0];
            let Some(serializer) = serializer else {
                columns.push(
                    ctx.tick(),
                    observer_team,
                    *target,
                    FowFact::unknown("team_serializer_unavailable"),
                    rules,
                    rules_keys,
                    unique_live_rift,
                );
                continue;
            };
            let Some(count) = field_i64(team, serializer, "m_vecFOWEntities")
                .filter(|count| (0..=255).contains(count))
            else {
                columns.push(
                    ctx.tick(),
                    observer_team,
                    *target,
                    FowFact::unknown("fow_count_unavailable"),
                    rules,
                    rules_keys,
                    unique_live_rift,
                );
                continue;
            };
            let mut matches = Vec::new();
            let mut ambiguous = false;
            let mut incomplete_rows = false;
            let target_ids: Vec<_> = targets
                .iter()
                .map(|candidate| {
                    (
                        candidate.entity.id().index,
                        candidate.spec.target_class,
                        candidate.team,
                    )
                })
                .collect();
            for i in 0..count as usize {
                let raw_index = entry_i64(team, serializer, i, "m_nEntIndex");
                let row_class = entry_i64(team, serializer, i, "m_eClass");
                let row_team = entry_i64(team, serializer, i, "m_nTeam");
                let (Some(raw_index), Some(row_class), Some(row_team)) =
                    (raw_index, row_class, row_team)
                else {
                    incomplete_rows = true;
                    continue;
                };
                let candidates = candidate_target_positions(
                    raw_index,
                    target.spec.target_class,
                    target.spec.fow_class,
                    target.team.unwrap_or(i64::MIN),
                    Some(row_class),
                    Some(row_team),
                    &target_ids,
                );
                if candidates.len() > 1
                    && candidates.contains(
                        &targets
                            .iter()
                            .position(|candidate| candidate.entity.id() == target.entity.id())
                            .unwrap_or(usize::MAX),
                    )
                {
                    ambiguous = true;
                } else if candidates.len() == 1
                    && targets[candidates[0]].entity.id() == target.entity.id()
                {
                    matches.push(i);
                }
            }
            if ambiguous {
                FowFact::unknown("fow_identity_ambiguous")
            } else if matches.len() > 1 {
                FowFact::unknown("duplicate_exact_fow_rows")
            } else if incomplete_rows {
                FowFact::unknown("fow_rows_incomplete")
            } else if let Some(index) = matches.first().copied() {
                shown_fact(serializer, team, index)
            } else {
                FowFact {
                    entry_present: Some(false),
                    fow_class_raw: None,
                    shown: None,
                    map_x: None,
                    map_y: None,
                    raw_health_byte: None,
                    position_available: false,
                    health_byte_available: false,
                    reason: "no_exact_fow_row",
                }
            }
        };
        columns.push(
            ctx.tick(),
            observer_team,
            *target,
            fact,
            rules,
            rules_keys,
            unique_live_rift,
        );
    }
}

#[pymethods]
impl Demo {
    /// Exact-identity observations for selected target-query ticks.
    ///
    /// Emits one row per active Guardian, Walker, or Rift cash-in and observer
    /// team (Amber/Sapphire). Identity fields are metadata-only join keys, not
    /// features. FOW coordinates and raw health bytes are read only for a
    /// uniquely class/team/index/serial-qualified icon whose recorded
    /// `m_bVisibleOnMap` value is true. Hidden icon rows retain no coordinates
    /// or health byte. Guardian/Walker server health and world position are
    /// never exported. Missing and ambiguous data remain null with masks.
    ///
    /// Rift fields expose only the current public GameRules capture-active flag
    /// and location at this exact tick, plus current typed cash-in fields.
    /// No future lifecycle, scoring team, capture outcome, participant count,
    /// or timer value is included. The rules location is available only for a
    /// unique current cash-in and a finite positive current capture clock.
    ///
    /// `fow_class_raw` is audit metadata; callers qualify its meaning against
    /// source-patch provenance. Exact current target class/team/index matching
    /// is required before a FOW row or its visible payload is associated.
    #[pyo3(signature = (*, ticks))]
    pub(crate) fn target_observations(
        &self,
        py: Python<'_>,
        ticks: Vec<i32>,
    ) -> PyResult<PyDataFrame> {
        let selected: HashSet<i32> = ticks.into_iter().collect();
        if selected
            .iter()
            .any(|tick| *tick < 0 || *tick > self.total_ticks)
        {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "ticks must be within the demo tick range",
            ));
        }
        if selected.is_empty() {
            return TargetObservationCols::default()
                .into_dataframe()
                .map(PyDataFrame);
        }
        py.detach(|| {
            let init = self.parser.parse_init().map_err(to_py_err)?;
            let keys = target_keys(&init);
            let rules_keys = rules_keys(&init);
            let filter = HashSet::from([
                "CCitadelTeam",
                "CCitadelGameRulesProxy",
                "CNPC_TrooperBoss",
                "CNPC_Boss_Tier2",
                "CCitadel_KothCashIn",
            ]);
            let mut columns = TargetObservationCols::default();
            let mut emitted = HashSet::new();
            self.parser
                .run_to_end_filtered(&filter, |ctx| {
                    let tick = ctx.tick();
                    if !selected.contains(&tick) || !emitted.insert(tick) {
                        return;
                    }
                    let targets = targets_at(ctx, &keys);
                    let rules = ctx
                        .entities()
                        .iter()
                        .find(|(_, entity)| {
                            entity.active && entity.class_name.as_ref() == "CCitadelGameRulesProxy"
                        })
                        .map(|(_, entity)| entity);
                    for observer_team in [2, 3] {
                        collect_team(
                            ctx,
                            observer_team,
                            &targets,
                            rules,
                            rules_keys,
                            &mut columns,
                        );
                    }
                })
                .map_err(to_py_err)?;
            columns.into_dataframe().map(PyDataFrame)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{FowFact, candidate_target_positions, valid_rift_location};

    #[test]
    fn candidate_join_accepts_direct_or_unsigned_alias_only_when_unique() {
        let targets = [(12, "guardian", Some(3)), (24, "guardian", Some(3))];
        assert_eq!(
            candidate_target_positions(12, "guardian", 5, 3, Some(5), Some(3), &targets),
            vec![0]
        );
        // The raw value can be an index or an unsigned alias; when those
        // interpretations name different current identities, the join is ambiguous.
        assert_eq!(
            candidate_target_positions(24, "guardian", 5, 3, Some(5), Some(3), &targets),
            vec![0, 1]
        );
        assert!(
            candidate_target_positions(12, "guardian", 5, 2, Some(5), Some(3), &targets).is_empty()
        );
        assert!(
            candidate_target_positions(12, "guardian", 5, 3, Some(29), Some(3), &targets)
                .is_empty()
        );
    }

    #[test]
    fn unknown_fow_fact_keeps_all_payload_null() {
        let fact = FowFact::unknown("fow_identity_ambiguous");
        assert_eq!(fact.entry_present, None);
        assert_eq!(fact.shown, None);
        assert_eq!(fact.map_x, None);
        assert_eq!(fact.map_y, None);
        assert_eq!(fact.raw_health_byte, None);
        assert!(!fact.position_available);
        assert!(!fact.health_byte_available);
    }

    #[test]
    fn rift_location_rejects_missing_sentinel_and_zero_vector() {
        assert!(valid_rift_location(&[-7560.0, 0.0, 10.0]));
        assert!(!valid_rift_location(&[0.0, 0.0, 0.0]));
        assert!(!valid_rift_location(&[f32::MAX, 0.0, 0.0]));
        assert!(!valid_rift_location(&[f32::NAN, 1.0, 2.0]));
    }
}
