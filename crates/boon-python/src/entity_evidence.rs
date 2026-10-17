//! Typed gameplay entity evidence, streamed instead of materializing a replay.
//! This is recorded server state, not a player-knowledge projection.
#[path = "evidence_clock.rs"]
pub(crate) mod evidence_clock;
#[path = "movement_policy.rs"]
pub(crate) mod movement_policy;
use crate::*;
use boon_parser::{EntityChangeKind, FieldValue};
use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufWriter, Write};
use std::sync::Arc;
#[path = "evidence_wire.rs"]
mod evidence_wire;

fn typed(value: &FieldValue) -> Value {
    match value {
        FieldValue::Bool(v) => json!({"type":"bool","value":v}),
        FieldValue::I32(v) => json!({"type":"i32","value":v}),
        FieldValue::I64(v) => json!({"type":"i64","value":v.to_string()}),
        FieldValue::U32(v) => json!({"type":"u32","value":v}),
        FieldValue::U64(v) => json!({"type":"u64","value":v.to_string()}),
        FieldValue::F32(v) => {
            json!({"type":"f32","bits":v.to_bits(),"value":if v.is_finite(){Some(v)}else{None}})
        }
        FieldValue::String(v) => json!({"type":"bytes","value":v}),
        FieldValue::Vector2(v) => vector("vector2", v),
        FieldValue::Vector3(v) => vector("vector3", v),
        FieldValue::Vector4(v) => vector("vector4", v),
        FieldValue::FloatVector(v) => vector("float_vector", v),
        FieldValue::QAngle(v) => vector("qangle", v),
        _ => json!({"type":"unsupported","value":null}),
    }
}
fn vector(kind: &str, v: &[f32]) -> Value {
    json!({"type":kind,"bits":v.iter().map(|v|v.to_bits()).collect::<Vec<_>>(),
        "value":v.iter().map(|v|if v.is_finite(){Some(*v)}else{None}).collect::<Vec<_>>()})
}
// Compare original values before allocating JSON or resolving field names.
// Floating-point bits matter: -0 and +0 differ, and equal NaN payloads do not
// need to be written on every tick. This matches the typed archive encoding.
fn same_recorded_value(a: &FieldValue, b: &FieldValue) -> bool {
    use FieldValue::*;
    fn floats(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
    }
    match (a, b) {
        (Bool(a), Bool(b)) => a == b,
        (I32(a), I32(b)) => a == b,
        (I64(a), I64(b)) => a == b,
        (U32(a), U32(b)) => a == b,
        (U64(a), U64(b)) => a == b,
        (F32(a), F32(b)) => a.to_bits() == b.to_bits(),
        (String(a), String(b)) => a == b,
        (Vector2(a), Vector2(b)) => floats(a, b),
        (Vector3(a), Vector3(b)) | (QAngle(a), QAngle(b)) => floats(a, b),
        (Vector4(a), Vector4(b)) => floats(a, b),
        (FloatVector(a), FloatVector(b)) => floats(a, b),
        _ => typed(a) == typed(b),
    }
}

// Serialized pose recipes are rendering/animation state, not individual
// positions, stats, inventory, objective state or timestamped game events.
const EXCLUDED_FIELD_PREFIX: &str = "CBodyComponent.m_animationController.";
// Exact network-class exclusions authorized for detailed rendering evidence only.
// Ability, modifier, team/FOW, position and objective entities remain included.
const EXCLUDED_ENTITY_CLASS_POLICY: &str = "boon-render-entity-exclusions-v1";
const EXCLUDED_ENTITY_CLASSES: [&str; 9] = [
    "CAssignedLaneParticle",
    "CEnvCombinedLightProbeVolume",
    "CEnvCubemapFog",
    "CEnvVolumetricFogController",
    "CEnvVolumetricFogVolume",
    "CFogController",
    "CFuncBrush",
    "CParticleSystem",
    "CPathParticleRope",
];
fn excluded_entity_class(name: &str) -> bool {
    EXCLUDED_ENTITY_CLASSES.contains(&name)
}

fn excluded_field(name: Option<&str>) -> bool {
    name.is_some_and(|name| name.starts_with(EXCLUDED_FIELD_PREFIX))
}

fn line(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")
}

#[pymethods]
impl Demo {
    /// Stream exact end-of-tick entity changes to a new gzip JSONL file.
    ///
    /// Includes gameplay network classes, including teams/FOW, player/controller,
    /// inventory/ability entities, objectives, NPCs, spawners and ziplines when
    /// recorded. Each state carries index+serial, class, source tick, activity
    /// and decoded gameplay fields with original type and float bits.
    /// Explicit rendering classes and animation-controller fields are excluded in the header. First records
    /// are complete states; later records contain changed/removed fields only.
    /// Reconstruct in sequence order, keyed by index AND serial. No averaging,
    /// semantic source guessing or NPC-damage aggregation is applied. Only the
    /// explicitly versioned movement policy samples map-player/trooper positions.
    /// Inactive/deleted is a lifecycle fact, not an inferred death.
    /// A completion footer is mandatory; incomplete files must not be admitted.
    #[pyo3(signature = (*, output_path))]
    pub(crate) fn write_entity_evidence(
        slf: Py<Self>,
        py: Python<'_>,
        output_path: PathBuf,
    ) -> PyResult<u64> {
        let pregame = {
            let mut demo = slf.try_borrow_mut(py)?;
            demo.pregame_seconds()
                .ok()
                .flatten()
                .filter(|value| value.is_finite())
        };
        let owner = slf.try_borrow(py)?;
        let demo: &Self = &owner;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        let result = py.detach(|| {
            let mut writer = GzEncoder::new(BufWriter::new(file), Compression::fast());
            let count = demo.write_evidence_to(JsonWriter(&mut writer), pregame)?;
            writer
                .finish()
                .and_then(|mut output| output.flush())
                .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
            Ok(count)
        });
        if result.is_err() {
            let _ = std::fs::remove_file(output_path);
        }
        result
    }
    /// Deliver complete newline-delimited records in bounded byte batches.
    /// The callback provides backpressure; no intermediate compressed file.
    #[pyo3(signature = (*, sink))]
    pub(crate) fn stream_entity_evidence(
        slf: Py<Self>,
        py: Python<'_>,
        sink: Py<PyAny>,
    ) -> PyResult<u64> {
        // Resolve the canonical clock before retaining the original shared-read
        // streaming borrow. Never hold a new exclusive borrow for the stream.
        let pregame = {
            let mut demo = slf.try_borrow_mut(py)?;
            demo.pregame_seconds()
                .ok()
                .flatten()
                .filter(|value| value.is_finite())
        };
        let owner = slf.try_borrow(py)?;
        let demo: &Self = &owner;
        py.detach(|| {
            let mut writer = BufWriter::with_capacity(256 * 1024, EvidenceSink(sink));
            let result = demo.write_evidence_to(JsonWriter(&mut writer), pregame);
            if result.is_err() {
                // A failed callback must not be retried by BufWriter::drop.
                let (_sink, _pending) = writer.into_parts();
            }
            result
        })
    }

    /// Version 1 ownership-safe typed transport; chunks are bounded to 256 KiB.
    #[pyo3(signature = (*, sink))]
    pub(crate) fn stream_entity_evidence_typed(
        slf: Py<Self>,
        py: Python<'_>,
        sink: Py<PyAny>,
    ) -> PyResult<u64> {
        let pregame = {
            let mut demo = slf.try_borrow_mut(py)?;
            demo.pregame_seconds()
                .ok()
                .flatten()
                .filter(|v| v.is_finite())
        };
        let owner = slf.try_borrow(py)?;
        let demo: &Self = &owner;
        py.detach(|| {
            let mut writer = BufWriter::with_capacity(256 * 1024, EvidenceSink(sink));
            let result =
                demo.write_evidence_to(evidence_wire::WireWriter::new(&mut writer), pregame);
            if result.is_err() {
                let (_sink, _pending) = writer.into_parts();
            }
            result
        })
    }
    /// The same immutable semantic header carried inside the integrity-bound stream.
    pub(crate) fn entity_evidence_wire_header(&self) -> PyResult<String> {
        Ok(evidence_header(self).to_string())
    }
}

struct EvidenceSink(Py<PyAny>);
impl Write for EvidenceSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for chunk in bytes.chunks(256 * 1024) {
            Python::attach(|py| self.0.call1(py, (pyo3::types::PyBytes::new(py, chunk),)))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn evidence_header(demo: &Demo) -> Value {
    json!({"schema":"boon-entity-evidence-v1","kind":"header",
        "scope":"recorded-server-state","clock":"source-demo-tick","granularity":"end-of-source-tick",
        "allDecodedFields":false,"excludedFieldPrefixes":[EXCLUDED_FIELD_PREFIX],"averaging":false,"encoding":"entity-field-deltas",
        "excludedEntityClassPolicy":EXCLUDED_ENTITY_CLASS_POLICY,"excludedEntityClasses":EXCLUDED_ENTITY_CLASSES,
        "movementPolicy":movement_policy::header(),"abilityUsageClock":evidence_clock::header(),
        "sourcePath":demo.path.to_string_lossy()})
}
struct EvidenceField<'a> {
    key: u64,
    name: Option<Arc<str>>,
    value: &'a FieldValue,
}
struct EvidenceRecord<'a> {
    kind: &'static str,
    sequence: u64,
    tick: i32,
    index: i32,
    serial: u32,
    class_name: &'a str,
    class_id: i32,
    active: bool,
    deleted: bool,
    fields: BTreeMap<String, EvidenceField<'a>>,
    removed: Vec<u64>,
    samples: Option<Value>,
    source_game_time: Option<Option<f64>>,
}
trait EvidenceOutput {
    fn header(&mut self, header: &Value) -> std::io::Result<()>;
    fn entity(&mut self, row: EvidenceRecord<'_>) -> std::io::Result<()>;
    fn complete(&mut self, count: u64) -> std::io::Result<()>;
}
struct JsonWriter<W>(W);
impl<W: Write> EvidenceOutput for JsonWriter<W> {
    fn header(&mut self, header: &Value) -> std::io::Result<()> {
        line(&mut self.0, header)
    }
    fn entity(&mut self, row: EvidenceRecord<'_>) -> std::io::Result<()> {
        let mut record = if row.kind == "absent" {
            json!({"kind":"absent","sequence":row.sequence,"tick":row.tick,
            "entity_index":row.index,"entity_serial":row.serial,"entity_class":row.class_name,"deleted":row.deleted})
        } else {
            let fields = row
                .fields
                .into_iter()
                .map(|(key, field)| {
                    (
                        key,
                        json!({"name":field.name.as_deref(),"data":typed(field.value)}),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            json!({"kind":row.kind,"sequence":row.sequence,"tick":row.tick,"entity_index":row.index,
                "entity_serial":row.serial,"entity_class":row.class_name,"active":row.active,"fields":fields,
                "removed":row.removed.iter().map(ToString::to_string).collect::<Vec<_>>()})
        };
        if let Some(samples) = row.samples {
            record["movement_samples"] = samples;
        }
        if let Some(clock) = row.source_game_time {
            record["source_game_time"] = json!(clock);
        }
        line(&mut self.0, &record)
    }
    fn complete(&mut self, count: u64) -> std::io::Result<()> {
        line(&mut self.0, &json!({"kind":"complete","records":count}))?;
        self.0.flush()
    }
}
impl Demo {
    fn write_evidence_to(
        &self,
        mut writer: impl EvidenceOutput,
        pregame: Option<f64>,
    ) -> PyResult<u64> {
        writer.header(&evidence_header(self)).map_err(io_error)?;
        let mut count = 0u64;
        let mut first = true;
        let mut previous = HashMap::<(i32, u32), HashMap<u64, FieldValue>>::new();
        let mut previous_active = HashMap::<(i32, u32), bool>::new();
        let mut names = HashMap::<(i32, u64), Option<Arc<str>>>::new();
        let mut movement = movement_policy::MovementPolicy::default();
        let mut policy_keys = HashMap::<i32, HashSet<u64>>::new();
        // Only the three policy families are revisited at a cadence boundary.
        // Generation changes are derived from lifecycle events, not a full pawn scan.
        let mut policy_entities = BTreeMap::<(i32, u32), String>::new();
        let mut generations = HashSet::new();
        let started = std::time::Instant::now();
        let mut last_progress = started;
        let mut callback_seconds = 0.;
        let mut journal_keys = 0u64;
        let mut full_state_keys = 0u64;
        let mut emitted_fields = 0u64;
        let class_info = self.parser.parse_class_info().map_err(to_py_err)?;
        let included = class_info
            .classes()
            .iter()
            .map(|entry| entry.network_name.as_ref())
            .filter(|name| !excluded_entity_class(name))
            .collect::<HashSet<_>>();
        self.parser.try_run_to_end_filtered(&included,|ctx|{
            let callback_started=std::time::Instant::now();
            let mut identities=BTreeMap::new();
            let changes=ctx.entities().entity_changes();
            let source_changed=changes.iter().map(|c|(c.index,c.serial)).collect::<HashSet<_>>();
            let source_classes=changes.iter().map(|c|((c.index,c.serial),c.class_id)).collect::<HashMap<_,_>>();
            let created=changes.iter().filter(|c|c.kind==EntityChangeKind::Created).map(|c|(c.index,c.serial)).collect::<HashSet<_>>();
            let deleted=changes.iter().filter(|c|c.kind==EntityChangeKind::Deleted).map(|c|(c.index,c.serial)).collect::<HashSet<_>>();
            if first{
                for(_,entity)in ctx.entities().iter(){
                    let id=(entity.index,entity.serial);identities.insert(id,entity.class_name.to_string());
                    if movement_policy::applies(&entity.class_name){policy_entities.insert(id,entity.class_name.to_string());}
                    if entity.class_name.as_ref()=="CCitadelPlayerPawn"{generations.insert(id);}
                }
                first=false;
            }
            for change in changes{
                let id=(change.index,change.serial);
                identities.insert(id,change.class_name.to_string());
                if change.kind==EntityChangeKind::Deleted{
                    policy_entities.remove(&id);generations.remove(&id);
                }else{
                    if movement_policy::applies(&change.class_name){policy_entities.insert(id,change.class_name.to_string());}
                    if change.class_name.as_ref()=="CCitadelPlayerPawn"{generations.insert(id);}
                }
            }
            // Slot-array shrink/reset may discard entities without a lifecycle
            // event. Recheck only the tiny known pawn-generation set.
            generations.retain(|(index,serial)|ctx.entities().get(*index).is_some_and(|entity|entity.serial==*serial&&entity.class_name.as_ref()=="CCitadelPlayerPawn"));
            let generation_changed=movement.observe_player_generations(generations.clone());
            if generation_changed{
                policy_entities.retain(|(index,serial),_|ctx.entities().get(*index).is_some_and(|entity|entity.serial==*serial));
                for(id,class)in &policy_entities{if class=="CCitadelTeam"{movement.force_next_fow(*id);identities.insert(*id,class.clone());}}
            }
            if ctx.tick()>=0 && ctx.tick()%5==0{
                policy_entities.retain(|(index,serial),_|ctx.entities().get(*index).is_some_and(|entity|entity.serial==*serial));
                for(id,class)in &policy_entities{if movement_policy::due(class,ctx.tick()){identities.insert(*id,class.clone());}}
            }
            let source_game_time=pregame.and_then(|offset|self.resolved_tick_to_match_seconds(ctx.tick(),Some(offset)));
            for((index,serial),class_name)in identities{
                if excluded_entity_class(&class_name){continue;}
                let current=ctx.entities().get(index).filter(|entity|entity.serial==serial);
                let clock=evidence_clock::eligible(&class_name).then_some(source_game_time);
                let class_id=current.map(|entity|entity.class_id).or_else(||source_classes.get(&(index,serial)).copied()).unwrap_or(-1);
                let mut row=EvidenceRecord{kind:"absent",sequence:count,tick:ctx.tick(),index,serial,class_name:&class_name,class_id,
                    active:false,deleted:false,fields:BTreeMap::new(),removed:Vec::new(),samples:None,source_game_time:clock};
                if let Some(entity)=current{
                    let serializer=ctx.serializers().get(&entity.class_name);
                    let decision=if movement_policy::applies(&class_name){
                        let keys=policy_keys.entry(entity.class_id).or_default();
                        // Dependency indexing does not prune exported fields.
                        let discover=if previous.contains_key(&(index,serial)) {
                            entity.changed_field_keys().to_vec()
                        }else{entity.fields.keys().copied().collect()};
                        for key in discover {
                            let name=names.entry((entity.class_id,key)).or_insert_with(||
                                serializer.and_then(|s|s.field_name_for_key(key)).map(Arc::from));
                            if name.as_deref().is_some_and(|name|movement_policy::relevant(&class_name,name)){keys.insert(key);}
                        }
                        let fields=keys.iter().filter_map(|key| {
                            let value=entity.fields.get(key)?;
                            names.get(&(entity.class_id,*key)).and_then(Option::as_deref)
                                .map(|name|movement_policy::NamedField{key:*key,name,value})
                        }).collect::<Vec<_>>();
                        movement.plan((index,serial),&class_name,ctx.tick(),entity.active,&fields)
                    }else{movement_policy::Decision::default()};
                    let existed=previous.contains_key(&(index,serial));
                    let same_activity=previous_active.get(&(index,serial))==Some(&entity.active);
                    let prior=previous.entry((index,serial)).or_default();
                    // A create/keyframe replacement can remove old keys. Ordinary
                    // decoded updates only insert fields; use the true journal.
                    let replaced=created.contains(&(index,serial));
                    let mut removed=decision.clear.iter().filter(|key|prior.contains_key(key)).copied().collect::<HashSet<_>>();
                    if replaced{removed.extend(prior.keys().filter(|key|!entity.fields.contains_key(key)).copied());}
                    for key in &removed{prior.remove(key);}
                    let candidates=if !existed||replaced{full_state_keys+=entity.fields.len()as u64;entity.fields.keys().copied().collect::<HashSet<_>>()}
                        else{journal_keys+=entity.changed_field_keys().len()as u64;entity.changed_field_keys().iter().chain(&decision.force).copied().collect::<HashSet<_>>()};
                    for key in candidates{
                        let Some(value)=entity.fields.get(&key)else{continue};
                        let name=names.entry((entity.class_id,key)).or_insert_with(||serializer.and_then(|s|s.field_name_for_key(key)).map(Arc::from));
                        if excluded_field(name.as_deref())||decision.suppress.contains(&key){continue;}
                        if !decision.force.contains(&key)&&prior.get(&key).is_some_and(|old|same_recorded_value(old,value)){continue;}
                        row.fields.insert(key.to_string(),EvidenceField{key,name:name.clone(),value});
                        prior.insert(key,value.clone());removed.remove(&key);
                    }
                    row.removed=removed.into_iter().collect();row.removed.sort_by_cached_key(ToString::to_string);
                    let only_new_boundary=movement_policy::due(&class_name,ctx.tick())&&!source_changed.contains(&(index,serial))&&!generation_changed;
                    if movement_policy::omit_new_boundary_noop(only_new_boundary,existed,row.fields.is_empty(),row.removed.is_empty(),same_activity,decision.samples.is_some()){continue;}
                    previous_active.insert((index,serial),entity.active);row.active=entity.active;row.kind=if existed{"delta"}else{"state"};row.samples=decision.samples;
                }else{
                    row.deleted=deleted.contains(&(index,serial));
                    previous.remove(&(index,serial));previous_active.remove(&(index,serial));movement.remove((index,serial));
                }
                emitted_fields+=row.fields.len()as u64;writer.entity(row)?;count+=1;
            }
            if last_progress.elapsed().as_secs()>=5{
                eprintln!("{}",json!({"event":"replay-import","phase":"archiving-individual-entity-evidence","sourceTick":ctx.tick(),"records":count,"elapsedSeconds":started.elapsed().as_secs()}));last_progress=std::time::Instant::now();
            }
            callback_seconds+=callback_started.elapsed().as_secs_f64();
            Ok(())
        }).map_err(to_py_err)?;
        writer.complete(count).map_err(io_error)?;
        eprintln!(
            "{}",
            json!({"event":"restore-source-timing","schema":"boon-journal-source-timing-v1",
            "wallSeconds":started.elapsed().as_secs_f64(),"exportCallbackWallSeconds":callback_seconds,
            "records":count,"exportedFields":emitted_fields,"journalKeysVisited":journal_keys,"initialOrReplacementFieldsVisited":full_state_keys,
            "timingPolicy":"Callback wall includes policy, exact comparison, transport encoding and bounded sink backpressure; parser traversal and callback spans are not additive CPU demands."})
        );
        Ok(count)
    }
}
fn io_error(error: std::io::Error) -> PyErr {
    pyo3::exceptions::PyIOError::new_err(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_rendering_classes_only_are_excluded() {
        assert_eq!(
            EXCLUDED_ENTITY_CLASS_POLICY,
            "boon-render-entity-exclusions-v1"
        );
        for name in EXCLUDED_ENTITY_CLASSES {
            assert!(excluded_entity_class(name));
        }
        for name in [
            "CCitadelPlayerPawn",
            "CCitadelTeam",
            "CCitadel_Ability_ZipLine",
            "CCitadelModifierAura",
            "CParticleSystemGameplay",
            "CNPC_TrooperBoss",
            "",
        ] {
            assert!(!excluded_entity_class(name));
        }
    }
    #[test]
    fn only_explicit_animation_controller_fields_are_excluded() {
        assert!(excluded_field(Some(
            "CBodyComponent.m_animationController.m_SerializePoseRecipeAG2Dynamic.0"
        )));
        for name in [
            None,
            Some("CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecX"),
            Some("m_iHealth"),
            Some("m_vecStatViewerModifierValues.0.m_flValue"),
            Some("CBodyComponent.m_animationControllerExtra"),
        ] {
            assert!(!excluded_field(name));
        }
    }
    #[test]
    fn raw_comparison_matches_archived_types_and_float_bits() {
        let values = vec![
            FieldValue::Bool(false),
            FieldValue::I32(0),
            FieldValue::I64(0),
            FieldValue::U32(0),
            FieldValue::U64(u64::MAX),
            FieldValue::F32(0.),
            FieldValue::F32(-0.),
            FieldValue::F32(f32::from_bits(0x7fc00041)),
            FieldValue::F32(f32::from_bits(0x7fc00042)),
            FieldValue::String(vec![255, 0]),
            FieldValue::Vector2([0., -0.]),
            FieldValue::Vector3([1., 2., 3.]),
            FieldValue::QAngle([1., 2., 3.]),
            FieldValue::Vector4([1., 2., 3., 4.]),
            FieldValue::FloatVector(vec![1., 2., 3., 4.]),
        ];
        for a in &values {
            for b in &values {
                assert_eq!(same_recorded_value(a, b), typed(a) == typed(b));
            }
        }
    }

    #[test]
    fn exact_types_keep_zero_large_id_invalid_utf8_and_float_bits() {
        assert_eq!(
            typed(&FieldValue::U64(u64::MAX))["value"],
            u64::MAX.to_string()
        );
        assert_eq!(typed(&FieldValue::I32(0))["value"], 0);
        assert_eq!(
            typed(&FieldValue::String(vec![255, 0]))["value"],
            json!([255, 0])
        );
        let value = f32::from_bits(0x7fc00041);
        assert_eq!(typed(&FieldValue::F32(value))["bits"], value.to_bits());
        assert!(typed(&FieldValue::F32(value))["value"].is_null());
        assert_eq!(typed(&FieldValue::F32(-0.0))["bits"], (-0.0f32).to_bits());
    }
}
