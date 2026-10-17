//! Bounded evidence validation and archive indexing outside Python's GIL.
//! No downstream labels, sampling, visibility decisions, or record aggregation.
use crate::entity_evidence::{movement_policy,evidence_clock};
use flate2::{Compression, write::ZlibEncoder};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::io::Write;

const PREFIX: &str = "CBodyComponent.m_animationController.";
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

#[derive(Default)]
struct Index {
    pending: Vec<u8>,
    header: bool,
    entity_class_exclusions: bool,
    movement_policy: bool,
    ability_usage_clock: bool,
    complete: bool,
    finished: bool,
    count: u64,
    previous: i64,
    chunk: u64,
    first: Option<i64>,
    buffer: Vec<u8>,
    buffer_count: usize,
    members: BTreeSet<(i64, i64)>,
    classes: BTreeSet<String>,
    unsupported: u64,
    animation: u64,
    identities: HashMap<(i64, i64), String>,
}
struct Block {
    number: u64,
    first: i64,
    last: i64,
    records: u64,
    payload: Vec<u8>,
    members: Vec<(i64, i64)>,
}
type Result<T> = std::result::Result<T, String>;
fn integer(row: &Value, key: &str) -> Result<i64> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| "Malformed/out-of-order entity-evidence record".into())
}
impl Index {
    fn new() -> Self {
        Self {
            previous: -1,
            ..Self::default()
        }
    }
    fn flush(&mut self, out: &mut Vec<Block>) -> Result<()> {
        if self.buffer_count == 0 {
            return Ok(());
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(2));
        encoder.write_all(&self.buffer).map_err(|e| e.to_string())?;
        let payload = encoder.finish().map_err(|e| e.to_string())?;
        out.push(Block {
            number: self.chunk,
            first: self.first.unwrap(),
            last: self.previous,
            records: self.count,
            payload,
            members: self.members.iter().copied().collect(),
        });
        self.chunk += 1;
        self.buffer.clear();
        self.buffer_count = 0;
        self.members.clear();
        self.first = None;
        Ok(())
    }
    fn line(&mut self, line: &[u8], out: &mut Vec<Block>) -> Result<()> {
        let mut row: Value = serde_json::from_slice(line)
            .map_err(|e| format!("Malformed entity-evidence JSON: {e}"))?;
        if !self.header {
            for (key, expected) in [
                ("kind", json!("header")),
                ("schema", json!("boon-entity-evidence-v1")),
                ("scope", json!("recorded-server-state")),
                ("clock", json!("source-demo-tick")),
                ("granularity", json!("end-of-source-tick")),
                ("encoding", json!("entity-field-deltas")),
                ("averaging", json!(false)),
            ] {
                if row.get(key) != Some(&expected) {
                    return Err("Unsupported or missing entity-evidence header".into());
                }
            }
            let exclusions = row.get("excludedFieldPrefixes");
            let legacy = row.get("allDecodedFields") == Some(&json!(true))
                && exclusions.is_none_or(|v| v.is_null() || v == &json!([]));
            let current = row.get("allDecodedFields") == Some(&json!(false))
                && exclusions == Some(&json!([PREFIX]));
            if legacy {
                return Err("Legacy evidence requires the reference importer".into());
            }
            if !current {
                return Err("Unknown entity-evidence field exclusions".into());
            }
            let entity_policy=row.get("excludedEntityClassPolicy");
            let entity_exclusions=row.get("excludedEntityClasses");
            self.entity_class_exclusions=if entity_policy.is_none() && entity_exclusions.is_none() {
                false // Prior valid archives retain their original class coverage.
            } else if entity_policy==Some(&json!(EXCLUDED_ENTITY_CLASS_POLICY))
                && entity_exclusions==Some(&json!(EXCLUDED_ENTITY_CLASSES)) {
                true
            } else {return Err("Unknown entity-evidence class exclusions".into());};
            self.movement_policy=movement_policy::validate_header(&row)?;
            self.ability_usage_clock=evidence_clock::validate_header(&row)?;
            self.header = true;
            return Ok(());
        }
        if self.complete {
            return Err("Data follows entity-evidence completion footer".into());
        }
        if row.get("kind").and_then(Value::as_str) == Some("complete") {
            if row.get("records").and_then(Value::as_u64) != Some(self.count) {
                return Err("Entity-evidence completion count differs".into());
            }
            self.complete = true;
            return Ok(());
        }
        let kind = row
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let tick = integer(&row, "tick")?;
        let index = integer(&row, "entity_index")?;
        let serial = integer(&row, "entity_serial")?;
        let class = row
            .get("entity_class")
            .and_then(Value::as_str)
            .ok_or("Malformed/out-of-order entity-evidence record")?
            .to_owned();
        if !matches!(kind.as_str(), "state" | "delta" | "absent")
            || row.get("sequence").and_then(Value::as_u64) != Some(self.count)
            || tick < self.previous
            || index < 0
            || serial < 0
        {
            return Err("Malformed/out-of-order entity-evidence record".into());
        }
        if self.entity_class_exclusions && excluded_entity_class(&class) {
            return Err("Declared excluded entity class is present in current evidence".into());
        }
        movement_policy::validate_samples(&row,self.movement_policy)?;
        evidence_clock::validate_record(&row,self.ability_usage_clock)?;
        let identity = (index, serial);
        let mut excluded = Vec::new();
        if kind == "absent" {
            self.identities.remove(&identity);
        } else {
            if kind == "delta" && self.identities.get(&identity) != Some(&class) {
                return Err("Entity delta lacks a matching initial identity/class".into());
            }
            if kind == "state" && self.identities.contains_key(&identity) {
                return Err("Duplicate initial entity state".into());
            }
            if !row.get("active").is_some_and(Value::is_boolean) {
                return Err("Malformed entity fields/activity".into());
            }
            let fields = row
                .get("fields")
                .and_then(Value::as_object)
                .ok_or("Malformed entity fields/activity")?;
            if let Some(removed) = row.get("removed") {
                let removed = removed
                    .as_array()
                    .ok_or("Malformed removed entity fields")?;
                if removed.iter().any(|v| !v.is_string()) {
                    return Err("Malformed removed entity fields".into());
                }
                if kind == "state" && !removed.is_empty() {
                    return Err("Initial state cannot remove fields".into());
                }
            }
            for (key, field) in fields {
                let data = field
                    .get("data")
                    .and_then(Value::as_object)
                    .ok_or("Malformed typed entity field")?;
                let data_type = data
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or("Malformed typed entity field")?;
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_digit()) || !field.is_object()
                {
                    return Err("Malformed typed entity field".into());
                }
                self.unsupported += u64::from(data_type == "unsupported");
                if field
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.starts_with(PREFIX))
                {
                    excluded.push(key.clone())
                }
            }
            self.identities.insert(identity, class.clone());
        }
        if !excluded.is_empty() {
            return Err(
                "Declared excluded animation fields are present in current evidence".into(),
            );
        }
        self.animation += excluded.len() as u64;
        let encoded = if excluded.is_empty() {
            line.to_vec()
        } else {
            let fields = row.get_mut("fields").unwrap().as_object_mut().unwrap();
            for key in excluded {
                fields.remove(&key);
            }
            let mut bytes = serde_json::to_vec(&row).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
            bytes
        };
        self.first.get_or_insert(tick);
        self.buffer.extend_from_slice(&encoded);
        self.buffer_count += 1;
        self.members.insert(identity);
        self.count += 1;
        self.previous = tick;
        self.classes.insert(class);
        if self.buffer.len() >= 1024 * 1024 || self.buffer_count >= 1024 {
            self.flush(out)?
        }
        Ok(())
    }
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Block>> {
        if self.finished {
            return Err("Evidence indexer already finished".into());
        }
        let mut out = Vec::new();
        self.pending.extend_from_slice(bytes);
        let input = std::mem::take(&mut self.pending);
        let mut start = 0;
        for (position, byte) in input.iter().enumerate() {
            if *byte == b'\n' {
                self.line(&input[start..=position], &mut out)?;
                start = position + 1;
            }
        }
        self.pending.extend_from_slice(&input[start..]);
        Ok(out)
    }
    fn finish(&mut self) -> Result<Vec<Block>> {
        if self.finished {
            return Err("Evidence indexer already finished".into());
        }
        let mut out = Vec::new();
        let remaining = std::mem::take(&mut self.pending);
        if !remaining.is_empty() {
            self.line(&remaining, &mut out)?
        }
        if !self.header || !self.complete {
            return Err("Entity-evidence stream is incomplete; restore this replay again".into());
        }
        self.flush(&mut out)?;
        self.finished = true;
        Ok(out)
    }
    fn metadata(&self) -> Value {
        let mut result=json!({"schema":"boon-entity-evidence-v1","storage":"lossless-bounded-record-blocks-v1","chunks":self.chunk,"records":self.count,"entityClasses":self.classes,"excludedFieldPrefixes":[PREFIX],"legacyAnimationFieldsRemoved":self.animation,"unsupportedFieldOccurrences":self.unsupported,"clock":"source-demo-tick","granularity":"end-of-source-tick","scope":"recorded-server-state"});
        if self.entity_class_exclusions {
            result["excludedEntityClassPolicy"]=json!(EXCLUDED_ENTITY_CLASS_POLICY);
            result["excludedEntityClasses"]=json!(EXCLUDED_ENTITY_CLASSES);
        }
        if self.movement_policy {result["movementPolicy"]=movement_policy::header();}
        if self.ability_usage_clock {result["abilityUsageClock"]=evidence_clock::header();}
        result
    }
}

#[pyclass(module = "boon._boon")]
pub(crate) struct EvidenceIndex {
    inner: Index,
}
type PythonBlock = (u64, i64, i64, u64, Py<PyBytes>, Vec<(i64, i64)>);
fn blocks(py: Python<'_>, values: Vec<Block>) -> Vec<PythonBlock> {
    values
        .into_iter()
        .map(|b| {
            (
                b.number,
                b.first,
                b.last,
                b.records,
                PyBytes::new(py, &b.payload).unbind(),
                b.members,
            )
        })
        .collect()
}
fn error(e: String) -> PyErr {
    pyo3::exceptions::PyValueError::new_err(e)
}
#[pymethods]
impl EvidenceIndex {
    #[new]
    fn new() -> Self {
        Self {
            inner: Index::new(),
        }
    }
    fn push(&mut self, py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<Vec<PythonBlock>> {
        let owned = data.as_bytes().to_vec();
        let result = py.detach(|| self.inner.push(&owned)).map_err(error)?;
        Ok(blocks(py, result))
    }
    fn finish(&mut self, py: Python<'_>) -> PyResult<Vec<PythonBlock>> {
        let result = py.detach(|| self.inner.finish()).map_err(error)?;
        Ok(blocks(py, result))
    }
    fn metadata(&self) -> PyResult<String> {
        if !self.inner.finished {
            return Err(error("Entity-evidence stream is incomplete".into()));
        }
        Ok(self.inner.metadata().to_string())
    }
    fn progress(&self) -> (i64, u64) {
        (self.inner.previous, self.inner.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header() -> Value {
        json!({"kind":"header","schema":"boon-entity-evidence-v1","scope":"recorded-server-state","clock":"source-demo-tick","granularity":"end-of-source-tick","encoding":"entity-field-deltas","averaging":false,"allDecodedFields":false,"excludedFieldPrefixes":[PREFIX]})
    }
    #[test]
    fn versioned_render_exclusions_are_exact_and_keep_old_archives() {
        let mut old=Index::new();feed(&mut old,header()).unwrap();
        let mut row=record();row["entity_class"]=json!("CParticleSystem");
        feed(&mut old,row.clone()).unwrap();
        assert!(!old.metadata().as_object().unwrap().contains_key("excludedEntityClasses"));
        let mut h=header();h["excludedEntityClassPolicy"]=json!(EXCLUDED_ENTITY_CLASS_POLICY);
        h["excludedEntityClasses"]=json!(EXCLUDED_ENTITY_CLASSES);
        for class in EXCLUDED_ENTITY_CLASSES {
            let mut current=Index::new();feed(&mut current,h.clone()).unwrap();
            row["entity_class"]=json!(class);
            assert!(feed(&mut current,row.clone()).err().unwrap().contains("excluded entity class"));
        }
        let mut current=Index::new();feed(&mut current,h.clone()).unwrap();
        feed(&mut current,record()).unwrap();
        assert_eq!(current.metadata()["excludedEntityClasses"],json!(EXCLUDED_ENTITY_CLASSES));
        for key in ["excludedEntityClassPolicy","excludedEntityClasses"] {
            let mut bad=h.clone();bad.as_object_mut().unwrap().remove(key);
            assert!(feed(&mut Index::new(),bad).err().unwrap().contains("class exclusions"));
        }
        for bad in [json!([]),json!(["CParticleSystem"]),json!(["CCitadelPlayerPawn"])] {
            let mut bad_header=h.clone();bad_header["excludedEntityClasses"]=bad;
            assert!(feed(&mut Index::new(),bad_header).err().unwrap().contains("class exclusions"));
        }
        h["excludedEntityClassPolicy"]=json!("unknown");
        assert!(feed(&mut Index::new(),h).err().unwrap().contains("class exclusions"));
    }
    fn record() -> Value {
        json!({"kind":"state","sequence":0,"tick":4,"entity_index":1,"entity_serial":3,"entity_class":"NPC","active":true,"fields":{}})
    }
    fn feed(index: &mut Index, value: Value) -> Result<Vec<Block>> {
        index.push(format!("{value}\n").as_bytes())
    }
    #[test]
    fn partial_batches_keep_records_and_require_footer() {
        let mut index = Index::new();
        let input = format!(
            "{}\n{}\n{}",
            header(),
            record(),
            json!({"kind":"complete","records":1})
        );
        for part in input.as_bytes().chunks(7) {
            assert!(index.push(part).unwrap().is_empty())
        }
        let blocks = index.finish().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].members, vec![(1, 3)]);
        assert_eq!(index.metadata()["records"], 1);
        assert!(index.finish().is_err());
    }
    #[test]
    fn malformed_sequence_delta_and_completion_are_rejected() {
        for row in [
            json!({"kind":"complete","records":2}),
            {
                let mut r = record();
                r["sequence"] = json!(true);
                r
            },
            {
                let mut r = record();
                r["kind"] = json!("delta");
                r
            },
        ] {
            let mut i = Index::new();
            feed(&mut i, header()).unwrap();
            assert!(feed(&mut i, row).is_err())
        }
    }
    #[test]
    fn missing_footer_and_post_footer_data_are_rejected() {
        let mut i = Index::new();
        feed(&mut i, header()).unwrap();
        feed(&mut i, record()).unwrap();
        assert!(i.finish().is_err());
        let mut i = Index::new();
        feed(&mut i, header()).unwrap();
        feed(&mut i, json!({"kind":"complete","records":0})).unwrap();
        assert!(feed(&mut i, record()).is_err())
    }
}
