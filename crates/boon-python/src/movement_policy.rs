//! Explicitly authorized archive projection. Metadata is archive provenance,
//! never a decoded game field. All state is causal and bounded to live entities.
use boon_parser::FieldValue;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) const POLICY_ID: &str = "boon-movement-evidence-v1";
pub(super) const INTERVAL: i32 = 10;
const PLAYER_EXCLUDED: [&str; 5] = [
    "m_angEyeAngles",
    "m_angLockedEyeAngles",
    "m_pCameraServices.m_vecPunchAngle",
    "m_pCameraServices.m_vecPunchAngleVel",
    "m_pCameraServices.m_nPunchAngleJoltTick",
];
const PLAYER_BODY_ROTATION: &str = "CBodyComponent.m_skeletonInstance.m_angRotation";
const ORIGIN: &str = "CBodyComponent.m_skeletonInstance.m_vecOrigin.";
const AXES: [&str; 7] = [
    "m_vecX",
    "m_vecY",
    "m_vecZ",
    "m_cellX",
    "m_cellY",
    "m_cellZ",
    "m_nOutsideWorld",
];
const ROTATION: [&str; 8] = [
    "m_angRotation",
    "m_angAbsRotation",
    "m_angNetworkAngles",
    "m_angEyeAngles",
    "m_flYaw",
    "m_nRotation",
    "m_flRotation",
    "m_angAngles",
];
const VELOCITY: [&str; 7] = [
    "m_vecVelocity",
    "m_vecAbsVelocity",
    "m_vecBaseVelocity",
    "m_vecAngVelocity",
    "m_angVelocity",
    "m_vecLocalVelocity",
    "m_vInitialVelocity",
];

pub(crate) fn header() -> Value {
    json!({"version":POLICY_ID,"intervalSourceTicks":INTERVAL,"cadence":"source-tick-modulo-10",
        "playerMap":{"entityClass":"CCitadelTeam","array":"m_vecFOWEntities","classValue":1,
            "positionFields":["m_nPositionX","m_nPositionY"],"visibleOnly":true,"rotation":false},
        "playerPawn":{"entityClass":"CCitadelPlayerPawn","excludedFields":PLAYER_EXCLUDED,
            "bodyRotationField":PLAYER_BODY_ROTATION,"bodyRotationIntervalSourceTicks":5,
            "bodyRotationCadence":"source-tick-modulo-5"},
        "trooper":{"entityClass":"CNPC_Trooper","positionPrefix":ORIGIN,"positionComponents":AXES,
            "velocity":false,"rotation":false},
        "rotationFieldComponents":ROTATION,"velocityFieldComponents":VELOCITY,
        "immediateTransitions":["spawn","activity","visibility","slot-identity","serial"],
        "provenance":"movement_samples","averaging":false})
}
/// Policy dependency index only; this never removes exported gameplay fields.
pub(super) fn relevant(class: &str, name: &str) -> bool {
    match class {
        "CCitadelPlayerPawn" => PLAYER_EXCLUDED.contains(&name) || name == PLAYER_BODY_ROTATION,
        "CNPC_Trooper" => {
            component(name, &ROTATION)
                || component(name, &VELOCITY)
                || name
                    .strip_prefix(ORIGIN)
                    .is_some_and(|axis| AXES.contains(&axis))
        }
        "CCitadelTeam" => name == "m_vecFOWEntities" || name.starts_with("m_vecFOWEntities."),
        _ => false,
    }
}
pub(super) fn applies(class: &str) -> bool {
    matches!(
        class,
        "CCitadelTeam" | "CNPC_Trooper" | "CCitadelPlayerPawn"
    )
}
pub(crate) fn boundary(tick: i32) -> bool {
    tick >= 0 && tick % INTERVAL == 0
}
pub(super) fn due(class: &str, tick: i32) -> bool {
    if class == "CCitadelPlayerPawn" {
        tick >= 0 && tick % 5 == 0
    } else {
        applies(class) && boundary(tick)
    }
}

/// Omit only a newly requested cadence envelope that carries no observation.
/// Source changes, lifecycle/activity transitions and sample provenance are kept.
pub(super) fn omit_new_boundary_noop(
    only_new_boundary: bool,
    existed: bool,
    changed_empty: bool,
    removed_empty: bool,
    same_activity: bool,
    has_provenance: bool,
) -> bool {
    only_new_boundary
        && existed
        && changed_empty
        && removed_empty
        && same_activity
        && !has_provenance
}

pub(super) struct NamedField<'a> {
    pub key: u64,
    pub name: &'a str,
    pub value: &'a FieldValue,
}
#[derive(Default)]
pub(super) struct Decision {
    /// Do not refresh the archived value, preserving a prior visible sample.
    pub suppress: HashSet<u64>,
    /// Remove a stale generation or a forbidden rotation/velocity value.
    pub clear: HashSet<u64>,
    /// Emit the whole position atomically, including unchanged components.
    pub force: HashSet<u64>,
    pub samples: Option<Value>,
}
#[derive(Default)]
struct SlotState {
    identity: Option<(i128, i128, i128)>,
    visible: bool,
    sampled: bool,
}
#[derive(Default)]
struct EntityState {
    active: bool,
    initialized: bool,
    position_valid: bool,
    slots: BTreeMap<u32, SlotState>,
}
#[derive(Default)]
pub(super) struct MovementPolicy {
    entities: HashMap<(i32, u32), EntityState>,
    player_generations: HashSet<(i32, u32)>,
}
fn integer(value: &FieldValue) -> Option<i128> {
    match value {
        FieldValue::U64(v) => Some((*v).into()),
        FieldValue::I64(v) => Some((*v).into()),
        FieldValue::U32(v) => Some((*v).into()),
        FieldValue::I32(v) => Some((*v).into()),
        _ => None,
    }
}
fn component(name: &str, candidates: &[&str]) -> bool {
    name.split('.').any(|part| candidates.contains(&part))
}
fn fow(name: &str) -> Option<(u32, &str)> {
    let tail = name.strip_prefix("m_vecFOWEntities.")?;
    let (slot, field) = tail.split_once('.')?;
    Some((slot.parse().ok()?, field))
}
impl MovementPolicy {
    pub(super) fn observe_player_generations(&mut self, generations: HashSet<(i32, u32)>) -> bool {
        if self.player_generations == generations {
            return false;
        }
        self.player_generations = generations;
        true
    }
    pub(super) fn force_next_fow(&mut self, identity: (i32, u32)) {
        if let Some(state) = self.entities.get_mut(&identity) {
            state.initialized = false;
        }
    }
    pub(super) fn remove(&mut self, identity: (i32, u32)) {
        self.entities.remove(&identity);
    }
    pub(super) fn plan(
        &mut self,
        identity: (i32, u32),
        class: &str,
        tick: i32,
        active: bool,
        fields: &[NamedField<'_>],
    ) -> Decision {
        let mut out = Decision::default();
        if !applies(class) {
            return out;
        }
        let state = self.entities.entry(identity).or_default();
        let lifecycle = !state.initialized || state.active != active;
        state.initialized = true;
        state.active = active;
        if class == "CCitadelPlayerPawn" {
            let mut has_body = false;
            for f in fields {
                if PLAYER_EXCLUDED.contains(&f.name) {
                    out.suppress.insert(f.key);
                    out.clear.insert(f.key);
                } else if f.name == PLAYER_BODY_ROTATION {
                    has_body = true;
                    if lifecycle || due(class, tick) || !state.position_valid {
                        out.force.insert(f.key);
                    } else {
                        out.suppress.insert(f.key);
                    }
                }
            }
            state.position_valid = has_body;
            return out;
        }
        if class == "CNPC_Trooper" {
            let mut position = Vec::new();
            let mut components = HashSet::new();
            for f in fields {
                if component(f.name, &ROTATION) || component(f.name, &VELOCITY) {
                    out.suppress.insert(f.key);
                    out.clear.insert(f.key);
                } else if let Some(axis) = f
                    .name
                    .strip_prefix(ORIGIN)
                    .filter(|axis| AXES.contains(axis))
                {
                    position.push(f.key);
                    components.insert(axis);
                }
            }
            // Every cell and local component is one coordinate, not independent
            // deltas. Refuse an incomplete group rather than mix old/new axes.
            let complete = AXES.iter().all(|axis| components.contains(axis));
            if complete && (lifecycle || boundary(tick) || !state.position_valid) {
                out.force.extend(position);
                out.samples = Some(json!({"trooper_position_source_tick":tick}));
            } else {
                out.suppress.extend(position.iter().copied());
                if !complete {
                    out.clear.extend(position);
                }
            }
            state.position_valid = complete;
            return out;
        }
        let count = fields
            .iter()
            .find(|f| f.name == "m_vecFOWEntities")
            .and_then(|f| integer(f.value))
            .filter(|n| (0..=255).contains(n));
        let mut slots = BTreeMap::<u32, BTreeMap<&str, &NamedField<'_>>>::new();
        for f in fields {
            if let Some((slot, name)) = fow(f.name) {
                slots.entry(slot).or_default().insert(name, f);
            }
        }
        let mut samples = Vec::new();
        // Array count is authoritative. Drop remembered generations when its
        // shrink removes a slot, even if stale decoded fields remain present.
        state.slots.retain(|slot, _| {
            count.is_some_and(|n| i128::from(*slot) < n) && slots.contains_key(slot)
        });
        for (slot, entry) in slots {
            let get_int = |name| entry.get(name).and_then(|f| integer(f.value));
            let recorded_class = get_int("m_eClass");
            if recorded_class != Some(1) {
                // A missing class must not briefly release a known player's
                // stale slot coordinates. An explicit nonplayer class ends
                // this policy's scope and retains its original raw behavior.
                if recorded_class.is_some() || !state.slots.contains_key(&slot) {
                    state.slots.remove(&slot);
                    continue;
                }
            }
            let position = [entry.get("m_nPositionX"), entry.get("m_nPositionY")];
            let keys = position
                .iter()
                .filter_map(|f| f.map(|f| f.key))
                .collect::<Vec<_>>();
            for f in entry.values() {
                if component(f.name, &ROTATION) {
                    out.suppress.insert(f.key);
                    out.clear.insert(f.key);
                }
            }
            let current_identity = get_int("m_nEntIndex")
                .zip(get_int("m_nTeam"))
                .zip(get_int("m_eClass"))
                .map(|((index, team), class)| (index, team, class))
                .filter(|(index, team, _)| *index >= 0 && matches!(team, 2 | 3));
            let visible = active
                && entry
                    .get("m_bVisibleOnMap")
                    .is_some_and(|f| matches!(f.value, FieldValue::Bool(true)))
                && count.is_some_and(|n| i128::from(slot) < n);
            let prior = state.slots.entry(slot).or_default();
            let identity_changed = prior.identity != current_identity || current_identity.is_none();
            let show = visible && !prior.visible;
            if identity_changed {
                prior.sampled = false;
                out.clear.extend(keys.iter().copied());
            }
            let complete = position.iter().all(Option::is_some);
            if visible
                && current_identity.is_some()
                && complete
                && (lifecycle || identity_changed || show || boundary(tick) || !prior.sampled)
            {
                out.force.extend(keys);
                prior.sampled = true;
                samples.push(json!({"slot":slot,"raw_entity_index":current_identity.unwrap().0,"observed_team":current_identity.unwrap().1,"sample_source_tick":tick}));
            } else {
                out.suppress.extend(keys.iter().copied());
                if !complete {
                    out.clear.extend(keys);
                    prior.sampled = false;
                }
            }
            prior.identity = current_identity;
            prior.visible = visible;
        }
        if !samples.is_empty() {
            out.samples = Some(json!({"fow":samples}));
        }
        out
    }
}

pub(crate) fn validate_policy(value: &Value) -> Result<(), String> {
    if value == &header() {
        Ok(())
    } else {
        Err("Unknown entity-evidence movement policy".into())
    }
}
pub(crate) fn validate_header(row: &Value) -> Result<bool, String> {
    match row.get("movementPolicy") {
        None => Ok(false),
        Some(value) if value == &header() => Ok(true),
        _ => Err("Unknown entity-evidence movement policy".into()),
    }
}
pub(crate) fn validate_samples(row: &Value, declared: bool) -> Result<(), String> {
    let Some(value) = row.get("movement_samples") else {
        return Ok(());
    };
    let invalid = || "Malformed or undeclared entity-evidence movement provenance".to_owned();
    if !declared
        || !matches!(
            row.get("kind").and_then(Value::as_str),
            Some("state" | "delta")
        )
    {
        return Err(invalid());
    }
    let metadata = value
        .as_object()
        .filter(|m| !m.is_empty())
        .ok_or_else(invalid)?;
    let tick = row
        .get("tick")
        .and_then(Value::as_i64)
        .ok_or_else(invalid)?;
    match row.get("entity_class").and_then(Value::as_str) {
        Some("CNPC_Trooper") => {
            if metadata.len() != 1
                || metadata
                    .get("trooper_position_source_tick")
                    .and_then(Value::as_i64)
                    != Some(tick)
            {
                return Err(invalid());
            }
        }
        Some("CCitadelTeam") => {
            if metadata.len() != 1 {
                return Err(invalid());
            }
            let samples = metadata
                .get("fow")
                .and_then(Value::as_array)
                .filter(|s| !s.is_empty())
                .ok_or_else(invalid)?;
            let mut seen = std::collections::HashSet::new();
            for sample in samples {
                let sample = sample
                    .as_object()
                    .filter(|s| s.len() == 4)
                    .ok_or_else(invalid)?;
                let slot = sample
                    .get("slot")
                    .and_then(Value::as_u64)
                    .filter(|s| *s < 255)
                    .ok_or_else(invalid)?;
                if !seen.insert(slot)
                    || sample
                        .get("raw_entity_index")
                        .and_then(Value::as_u64)
                        .is_none()
                    || !matches!(
                        sample.get("observed_team").and_then(Value::as_i64),
                        Some(2 | 3)
                    )
                    || sample.get("sample_source_tick").and_then(Value::as_i64) != Some(tick)
                {
                    return Err(invalid());
                }
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fow_fields(
        slot: u32,
        raw: u64,
        visible: bool,
        x: u64,
        y: u64,
        class: u64,
    ) -> Vec<(u64, String, FieldValue)> {
        let values = [
            ("m_nEntIndex", FieldValue::U64(raw)),
            ("m_nTeam", FieldValue::I64(3)),
            ("m_eClass", FieldValue::U64(class)),
            ("m_bVisibleOnMap", FieldValue::Bool(visible)),
            ("m_nPositionX", FieldValue::U64(x)),
            ("m_nPositionY", FieldValue::U64(y)),
            ("m_nRotation", FieldValue::U64(90)),
        ];
        let mut fields = vec![(
            0,
            "m_vecFOWEntities".into(),
            FieldValue::U64((slot + 1).into()),
        )];
        fields.extend(values.into_iter().enumerate().map(|(i, (name, value))| {
            (
                (i + 1) as u64,
                format!("m_vecFOWEntities.{slot}.{name}"),
                value,
            )
        }));
        fields
    }
    fn plan(
        p: &mut MovementPolicy,
        id: (i32, u32),
        class: &str,
        tick: i32,
        active: bool,
        fields: &[(u64, String, FieldValue)],
    ) -> Decision {
        p.plan(
            id,
            class,
            tick,
            active,
            &fields
                .iter()
                .map(|(key, name, value)| NamedField {
                    key: *key,
                    name,
                    value,
                })
                .collect::<Vec<_>>(),
        )
    }
    fn trooper() -> Vec<(u64, String, FieldValue)> {
        let mut fields = AXES
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    i as u64,
                    format!("{ORIGIN}{name}"),
                    FieldValue::U64(i as u64),
                )
            })
            .collect::<Vec<_>>();
        fields.push((
            7,
            "CBodyComponent.m_skeletonInstance.m_angRotation".into(),
            FieldValue::QAngle([0., 1., 0.]),
        ));
        fields.push((8, "m_vecVelocity.m_vecX".into(), FieldValue::F32(1.)));
        fields.push((9, "m_iHealth".into(), FieldValue::I32(42)));
        fields
    }
    #[test]
    fn only_strict_new_boundary_empty_noops_are_omitted() {
        assert!(omit_new_boundary_noop(true, true, true, true, true, false));
        // Existing source records, initial states, nonempty fields/removals,
        // activity transitions and provenance must retain their envelopes.
        for args in [
            (false, true, true, true, true, false),
            (true, false, true, true, true, false),
            (true, true, false, true, true, false),
            (true, true, true, false, true, false),
            (true, true, true, true, false, false),
            (true, true, true, true, true, true),
        ] {
            assert!(!omit_new_boundary_noop(
                args.0, args.1, args.2, args.3, args.4, args.5
            ));
        }
    }
    #[test]
    fn exact_classes_leave_actual_players_and_bosses_unchanged() {
        let fields = trooper();
        let mut p = MovementPolicy::default();
        for class in [
            "CCitadelPlayerPawnExtra",
            "CCitadelPlayerController",
            "CNPC_TrooperBoss",
            "CNPC_TrooperExtra",
            "CCitadelTeamExtra",
        ] {
            let d = plan(&mut p, (1, 1), class, 10, true, &fields);
            assert!(
                d.suppress.is_empty()
                    && d.force.is_empty()
                    && d.clear.is_empty()
                    && d.samples.is_none()
            );
        }
    }
    #[test]
    fn only_exact_player_camera_eye_fields_drop_and_body_uses_five_ticks() {
        let mut p = MovementPolicy::default();
        let mut fields = PLAYER_EXCLUDED
            .iter()
            .enumerate()
            .map(|(i, n)| (i as u64, (*n).to_owned(), FieldValue::QAngle([1., 2., 3.])))
            .collect::<Vec<_>>();
        fields.extend([
            (
                5,
                PLAYER_BODY_ROTATION.into(),
                FieldValue::QAngle([0., 90., 0.]),
            ),
            (
                6,
                "m_angClientCamera".into(),
                FieldValue::QAngle([1., 2., 3.]),
            ),
            (7, "m_vecVelocity.m_vecX".into(), FieldValue::F32(7.)),
            (8, format!("{ORIGIN}m_vecX"), FieldValue::F32(8.)),
            (
                9,
                "m_angEyeAnglesExtra".into(),
                FieldValue::QAngle([1., 2., 3.]),
            ),
        ]);
        for tick in 3..=11 {
            let d = plan(&mut p, (1, 1), "CCitadelPlayerPawn", tick, true, &fields);
            for key in 0..5 {
                assert!(d.clear.contains(&key) && d.suppress.contains(&key));
            }
            assert_eq!(d.force.contains(&5), tick == 3 || tick % 5 == 0);
            for key in 6..=9 {
                assert!(
                    !d.clear.contains(&key)
                        && !d.suppress.contains(&key)
                        && !d.force.contains(&key)
                );
            }
            assert!(d.samples.is_none());
        }
        assert!(
            plan(&mut p, (1, 2), "CCitadelPlayerPawn", 12, true, &fields)
                .force
                .contains(&5)
        );
        assert!(
            plan(&mut p, (1, 2), "CCitadelPlayerPawn", 13, false, &fields)
                .force
                .contains(&5)
        );
        for class in [
            "CCitadelPlayerController",
            "CCitadelPlayerPawnExtra",
            "CNPC_TrooperBoss",
        ] {
            let d = plan(&mut p, (1, 3), class, 10, true, &fields);
            assert!(d.suppress.is_empty() && d.force.is_empty() && d.clear.is_empty());
        }
    }
    #[test]
    fn exact_source_boundaries_and_atomic_trooper_axes() {
        let mut p = MovementPolicy::default();
        let fields = trooper();
        let initial = plan(&mut p, (1, 1), "CNPC_Trooper", 3, true, &fields);
        assert_eq!(initial.force.len(), 7);
        assert_eq!(initial.samples.unwrap()["trooper_position_source_tick"], 3);
        for tick in 4..=21 {
            let d = plan(&mut p, (1, 1), "CNPC_Trooper", tick, true, &fields);
            assert_eq!(d.force.len(), if tick % 10 == 0 { 7 } else { 0 });
            assert!(d.suppress.contains(&7) && d.suppress.contains(&8));
            assert!(!d.suppress.contains(&9));
        }
        assert!(!boundary(-10));
        assert!(boundary(0));
        assert!(!boundary(9));
        assert!(boundary(10));
    }
    #[test]
    fn repeated_source_tick_events_are_not_deduplicated_or_averaged() {
        let mut p = MovementPolicy::default();
        let mut fields = trooper();
        for value in [1., 2., 3.] {
            fields[0].2 = FieldValue::F32(value);
            let d = plan(&mut p, (1, 1), "CNPC_Trooper", 10, true, &fields);
            assert_eq!(d.force.len(), 7);
        }
    }
    #[test]
    fn trooper_lifecycle_and_serial_changes_sample_immediately() {
        let mut p = MovementPolicy::default();
        let fields = trooper();
        for (id, tick, active) in [
            ((1, 1), 3, true),
            ((1, 1), 4, false),
            ((1, 1), 5, true),
            ((1, 2), 6, true),
        ] {
            assert_eq!(
                plan(&mut p, id, "CNPC_Trooper", tick, active, &fields)
                    .force
                    .len(),
                7
            );
        }
        p.remove((1, 2));
        assert_eq!(
            plan(&mut p, (1, 2), "CNPC_Trooper", 7, true, &fields)
                .force
                .len(),
            7
        );
    }
    #[test]
    fn missing_trooper_component_never_mixes_coordinate_epochs() {
        let mut p = MovementPolicy::default();
        let mut fields = trooper();
        fields.remove(1);
        let d = plan(&mut p, (1, 1), "CNPC_Trooper", 10, true, &fields);
        assert!(d.force.is_empty() && d.samples.is_none());
        assert_eq!(d.clear.len(), 8);
    }
    #[test]
    fn fow_visible_hide_show_reuse_and_metadata_are_causal() {
        let mut p = MovementPolicy::default();
        let visible = fow_fields(0, 148, true, 10, 20, 1);
        let d = plan(&mut p, (5, 1), "CCitadelTeam", 3, true, &visible);
        assert_eq!(d.force, HashSet::from([5, 6]));
        assert!(d.suppress.contains(&7));
        assert_eq!(
            d.samples.unwrap(),
            json!({"fow":[{"slot":0,"raw_entity_index":148,"observed_team":3,"sample_source_tick":3}]})
        );
        let d = plan(&mut p, (5, 1), "CCitadelTeam", 9, true, &visible);
        assert!(d.force.is_empty() && d.samples.is_none());
        let d = plan(&mut p, (5, 1), "CCitadelTeam", 10, true, &visible);
        assert_eq!(d.force.len(), 2);
        let hidden = fow_fields(0, 148, false, 99, 99, 1);
        for tick in [11, 20] {
            let d = plan(&mut p, (5, 1), "CCitadelTeam", tick, true, &hidden);
            assert!(d.force.is_empty() && d.samples.is_none());
            assert!(d.suppress.contains(&5) && d.suppress.contains(&6));
        }
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 21, true, &visible)
                .force
                .len(),
            2
        );
        let reused_hidden = fow_fields(0, 150, false, 88, 88, 1);
        let d = plan(&mut p, (5, 1), "CCitadelTeam", 22, true, &reused_hidden);
        assert!(d.force.is_empty());
        assert!(d.clear.contains(&5) && d.clear.contains(&6));
        let reused = fow_fields(0, 150, true, 88, 88, 1);
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 23, true, &reused)
                .force
                .len(),
            2
        );
    }
    #[test]
    fn same_index_new_pawn_serial_forces_visible_sample_off_cadence() {
        let mut p = MovementPolicy::default();
        let visible = fow_fields(0, 148, true, 10, 20, 1);
        assert!(p.observe_player_generations(HashSet::from([(74, 2)])));
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 3, true, &visible)
                .force
                .len(),
            2
        );
        assert!(!p.observe_player_generations(HashSet::from([(74, 2)])));
        assert!(
            plan(&mut p, (5, 1), "CCitadelTeam", 4, true, &visible)
                .force
                .is_empty()
        );
        assert!(p.observe_player_generations(HashSet::from([(74, 3)])));
        p.force_next_fow((5, 1));
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 5, true, &visible)
                .force
                .len(),
            2
        );
        let hidden = fow_fields(0, 148, false, 99, 99, 1);
        p.force_next_fow((5, 1));
        assert!(
            plan(&mut p, (5, 1), "CCitadelTeam", 6, true, &hidden)
                .force
                .is_empty()
        );
    }
    #[test]
    fn initial_hidden_and_nonplayer_fow_never_gain_player_projection() {
        let mut p = MovementPolicy::default();
        let d = plan(
            &mut p,
            (5, 1),
            "CCitadelTeam",
            10,
            true,
            &fow_fields(0, 148, false, 90, 91, 1),
        );
        assert!(d.force.is_empty() && d.samples.is_none());
        assert!(d.suppress.contains(&5) && d.suppress.contains(&6));
        let d = plan(
            &mut p,
            (5, 1),
            "CCitadelTeam",
            20,
            true,
            &fow_fields(0, 148, true, 90, 91, 200),
        );
        assert!(
            d.suppress.is_empty()
                && d.force.is_empty()
                && d.clear.is_empty()
                && d.samples.is_none()
        );
    }
    #[test]
    fn missing_class_in_a_known_player_slot_never_releases_hidden_coordinates() {
        let mut p = MovementPolicy::default();
        let mut fields = fow_fields(0, 148, true, 10, 20, 1);
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 3, true, &fields)
                .force
                .len(),
            2
        );
        fields.retain(|(_, name, _)| !name.ends_with(".m_eClass"));
        for tick in [4, 5, 10] {
            let d = plan(&mut p, (5, 1), "CCitadelTeam", tick, true, &fields);
            assert!(d.force.is_empty() && d.samples.is_none());
            assert!(d.suppress.contains(&5) && d.suppress.contains(&6));
            assert!(d.clear.contains(&5) && d.clear.contains(&6));
        }
        let nonplayer = fow_fields(0, 148, true, 99, 99, 200);
        let d = plan(&mut p, (5, 1), "CCitadelTeam", 11, true, &nonplayer);
        assert!(d.suppress.is_empty() && d.force.is_empty());
    }
    #[test]
    fn padded_array_slots_are_the_same_recorded_slot() {
        let mut p = MovementPolicy::default();
        let mut fields = fow_fields(0, 148, true, 10, 20, 1);
        for (_, name, _) in &mut fields {
            *name = name.replace(".0.", ".0000.");
        }
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 3, true, &fields)
                .force
                .len(),
            2
        );
    }
    #[test]
    fn absent_slot_then_identical_reappearance_starts_a_new_sample() {
        let mut p = MovementPolicy::default();
        let fields = fow_fields(0, 148, true, 10, 20, 1);
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 3, true, &fields)
                .force
                .len(),
            2
        );
        let empty = vec![(0, "m_vecFOWEntities".into(), FieldValue::U64(1))];
        assert!(
            plan(&mut p, (5, 1), "CCitadelTeam", 4, true, &empty)
                .force
                .is_empty()
        );
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 5, true, &fields)
                .force
                .len(),
            2
        );
    }
    #[test]
    fn all_fow_axes_required_activity_and_slot_count_preserved() {
        let mut p = MovementPolicy::default();
        let mut fields = fow_fields(0, 148, true, 10, 20, 1);
        fields.remove(6);
        let d = plan(&mut p, (5, 1), "CCitadelTeam", 10, true, &fields);
        assert!(d.force.is_empty());
        assert!(d.clear.contains(&5));
        let fields = fow_fields(0, 148, true, 10, 20, 1);
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 11, true, &fields)
                .force
                .len(),
            2
        );
        assert!(
            plan(&mut p, (5, 1), "CCitadelTeam", 12, false, &fields)
                .force
                .is_empty()
        );
        assert_eq!(
            plan(&mut p, (5, 1), "CCitadelTeam", 13, true, &fields)
                .force
                .len(),
            2
        );
        let mut shrunk = fields.clone();
        shrunk[0].2 = FieldValue::U64(0);
        let d = plan(&mut p, (5, 1), "CCitadelTeam", 20, true, &shrunk);
        assert!(d.force.is_empty() && d.samples.is_none());
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn header_requires_complete_known_typed_policy() {
        assert!(!validate_header(&json!({})).unwrap());
        let mut h = json!({"movementPolicy":header()});
        assert!(validate_header(&h).unwrap());
        h["movementPolicy"]["version"] = json!("boon-movement-evidence-v2");
        assert!(validate_header(&h).is_err());
        h["movementPolicy"] = header();
        h["movementPolicy"]["intervalSourceTicks"] = json!(true);
        assert!(validate_header(&h).is_err());
        h["movementPolicy"] = header();
        h["movementPolicy"]
            .as_object_mut()
            .unwrap()
            .remove("provenance");
        assert!(validate_header(&h).is_err());
    }
    fn fow() -> Value {
        json!({"kind":"state","tick":10,"entity_class":"CCitadelTeam","movement_samples":{"fow":[{"slot":0,"raw_entity_index":148,"observed_team":3,"sample_source_tick":10}]}})
    }
    #[test]
    fn provenance_is_not_a_game_field_or_resolved_identity() {
        let good = fow();
        assert!(validate_samples(&good, true).is_ok());
        assert!(validate_samples(&good, false).is_err());
        for tick in [9, 11] {
            let mut row = good.clone();
            row["movement_samples"]["fow"][0]["sample_source_tick"] = json!(tick);
            assert!(validate_samples(&row, true).is_err());
        }
        for (field, value) in [
            ("slot", json!(255)),
            ("slot", json!(true)),
            ("raw_entity_index", json!(-1)),
            ("observed_team", json!(4)),
        ] {
            let mut row = good.clone();
            row["movement_samples"]["fow"][0][field] = value;
            assert!(validate_samples(&row, true).is_err());
        }
        let mut row = good.clone();
        row["movement_samples"]["fow"]
            .as_array_mut()
            .unwrap()
            .push(good["movement_samples"]["fow"][0].clone());
        assert!(validate_samples(&row, true).is_err());
        let mut row = good.clone();
        row["entity_class"] = json!("CCitadelPlayerPawn");
        assert!(validate_samples(&row, true).is_err());
        row["entity_class"] = json!("CNPC_Trooper");
        row["movement_samples"] = json!({"trooper_position_source_tick":10});
        assert!(validate_samples(&row, true).is_ok());
    }
}
