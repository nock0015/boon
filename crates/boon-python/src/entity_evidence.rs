//! Typed gameplay entity evidence, streamed instead of materializing a replay.
//! This is recorded server state, not a player-knowledge projection.
use crate::*;
use boon_parser::{EntityChangeKind, FieldValue};
use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufWriter, Write};

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
fn excluded_field(name: Option<&str>) -> bool {
    name.is_some_and(|name|name.starts_with(EXCLUDED_FIELD_PREFIX))
}

fn line(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")
}

#[pymethods]
impl Demo {
    /// Stream exact end-of-tick entity changes to a new gzip JSONL file.
    ///
    /// Includes all network classes, including teams/FOW, player/controller,
    /// inventory/ability entities, objectives, NPCs, spawners and ziplines when
    /// recorded. Each state carries index+serial, class, source tick, activity
    /// and decoded gameplay fields with original type and float bits.
    /// Serialized animation-controller fields are excluded and declared in the header. First records
    /// are complete states; later records contain changed/removed fields only.
    /// Reconstruct in sequence order, keyed by index AND serial. No averaging,
    /// sampling, semantic source guessing or NPC-damage aggregation is applied.
    /// Inactive/deleted is a lifecycle fact, not an inferred death.
    /// A completion footer is mandatory; incomplete files must not be admitted.
    #[pyo3(signature = (*, output_path))]
    pub(crate) fn write_entity_evidence(
        &self,
        py: Python<'_>,
        output_path: PathBuf,
    ) -> PyResult<u64> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        let result=py.detach(|| {
            let mut writer=GzEncoder::new(BufWriter::new(file),Compression::fast());
            line(&mut writer,&json!({"schema":"boon-entity-evidence-v1","kind":"header",
                "scope":"recorded-server-state","clock":"source-demo-tick","granularity":"end-of-source-tick",
                "allDecodedFields":false,"excludedFieldPrefixes":[EXCLUDED_FIELD_PREFIX],"averaging":false,"encoding":"entity-field-deltas",
                "sourcePath":self.path.to_string_lossy()}))
                .map_err(|e|pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
            let mut failure=None;let mut count=0u64;let mut first=true;
            // Only current entity states are retained, never the replay timeline.
            let mut previous=HashMap::<(i32,u32),HashMap<u64,FieldValue>>::new();
            let mut names=HashMap::<(i32,u64),Option<String>>::new();
            let started=std::time::Instant::now();
            let mut last_progress=started;
            self.parser.run_to_end(|ctx| {
                if failure.is_some(){return;}
                let mut identities=BTreeMap::new();
                if first {
                    for (_,entity) in ctx.entities().iter(){identities.insert((entity.index,entity.serial),entity.class_name.to_string());}
                    first=false;
                }
                for change in ctx.entities().entity_changes() {
                    identities.insert((change.index,change.serial),change.class_name.to_string());
                }
                for ((index,serial),class_name) in identities {
                    let current=ctx.entities().get(index).filter(|e|e.serial==serial);
                    let record=if let Some(entity)=current {
                        let serializer=ctx.serializers().get(&entity.class_name);
                        let existed=previous.contains_key(&(index,serial));
                        let prior=previous.entry((index,serial)).or_default();
                        let removed_keys=prior.keys().filter(|key|!entity.fields.contains_key(key)).copied().collect::<Vec<_>>();
                        let mut removed=removed_keys.iter().map(ToString::to_string).collect::<Vec<_>>();
                        removed.sort();
                        for key in removed_keys {prior.remove(&key);}
                        let mut changed=BTreeMap::new();
                        for (key,value) in &entity.fields {
                            let name=names.entry((entity.class_id,*key)).or_insert_with(||
                                serializer.and_then(|s|s.field_name_for_key(*key)));
                            if excluded_field(name.as_deref()){continue;}
                            if prior.get(key).is_some_and(|old|same_recorded_value(old,value)){continue;}
                            changed.insert(key.to_string(),json!({"name":name,"data":typed(value)}));
                            prior.insert(*key,value.clone());
                        }
                        json!({"kind":if existed{"delta"}else{"state"},"sequence":count,"tick":ctx.tick(),"entity_index":index,
                            "entity_serial":serial,"entity_class":class_name,"active":entity.active,"fields":changed,"removed":removed})
                    } else {
                        let deleted=ctx.entities().entity_changes().iter().any(|c|c.index==index && c.serial==serial && c.kind==EntityChangeKind::Deleted);
                        previous.remove(&(index,serial));
                        json!({"kind":"absent","sequence":count,"tick":ctx.tick(),"entity_index":index,
                            "entity_serial":serial,"entity_class":class_name,"deleted":deleted})
                    };
                    if let Err(error)=line(&mut writer,&record){failure=Some(error.to_string());return;}
                    count+=1;
                }
                if last_progress.elapsed().as_secs()>=5 {
                    eprintln!("{}",json!({"event":"replay-import","phase":"archiving-individual-entity-evidence",
                        "sourceTick":ctx.tick(),"records":count,"elapsedSeconds":started.elapsed().as_secs()}));
                    last_progress=std::time::Instant::now();
                }
            }).map_err(to_py_err)?;
            if let Some(error)=failure{return Err(pyo3::exceptions::PyIOError::new_err(error));}
            line(&mut writer,&json!({"kind":"complete","records":count}))
                .and_then(|_|writer.try_finish()).map_err(|e|pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
            writer.finish().and_then(|mut output|output.flush())
                .map_err(|e|pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
            Ok(count)
        });
        if result.is_err() {
            let _ = std::fs::remove_file(output_path);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_animation_controller_fields_are_excluded() {
        assert!(excluded_field(Some("CBodyComponent.m_animationController.m_SerializePoseRecipeAG2Dynamic.0")));
        for name in [None,Some("CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecX"),
            Some("m_iHealth"),Some("m_vecStatViewerModifierValues.0.m_flValue"),
            Some("CBodyComponent.m_animationControllerExtra")] {
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
