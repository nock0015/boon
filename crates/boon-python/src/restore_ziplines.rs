//! Individual public zipline views. This contains no minion-rider inference.
//! Cached normalized fields are updated only for changed zipline entities;
//! projection and serialization run without holding Python's interpreter lock.
use pyo3::prelude::*;
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

type Identity = (i64, i64);
struct Node {
    active: bool,
    fields: Map<String, Value>,
    signature: String,
}
#[derive(Default)]
struct State {
    nodes: HashMap<Identity, Node>,
    order: Vec<Identity>,
    cached: Vec<View>,
    dirty: bool,
    builds: u64,
}
fn integer(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_i64)
}
fn numeric(v: &Value) -> bool {
    v.as_f64().is_some_and(f64::is_finite)
}
fn key((index, serial): Identity) -> String {
    format!("entity:{}", i128::from(index) + (i128::from(serial) << 14))
}
fn display(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".into(),
        Some(Value::Bool(true)) => "True".into(),
        Some(Value::Bool(false)) => "False".into(),
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
    }
}
// Preserve Python's field/fact insertion order: masked-target sampling consumes
// extra facts in this order. JSON object equality alone is not enough.
struct Ordered(Vec<(String, Value)>);
impl Serialize for Ordered {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    key: String,
    kind: &'static str,
    team: Value,
    entity_class: &'static str,
    public: Ordered,
    private: Value,
    public_facts: Ordered,
    source_tick: i64,
    lane: Value,
    connections: Vec<String>,
    connections_complete: bool,
    evidence: &'static str,
    display_name: String,
}
impl State {
    fn project(&mut self, tick: i64) -> &[View] {
        if self.dirty {
            self.cached = self.compute(tick);
            self.dirty = false;
            self.builds += 1;
        }
        for view in &mut self.cached {
            view.source_tick = tick;
        }
        &self.cached
    }
    fn compute(&self, tick: i64) -> Vec<View> {
        let mut result = Vec::new();
        for identity in &self.order {
            let node = &self.nodes[identity];
            if !node.active {
                continue;
            }
            let f = &node.fields;
            let mut position = Vec::new();
            for base in [
                "CBodyComponent.m_skeletonInstance.m_vecOrigin.",
                "CBodyComponent.",
            ] {
                let cells = ["X", "Y", "Z"].map(|axis| f.get(&format!("{base}m_cell{axis}")));
                let offsets = ["X", "Y", "Z"].map(|axis| f.get(&format!("{base}m_vec{axis}")));
                if cells
                    .iter()
                    .chain(offsets.iter())
                    .all(|v| v.is_some_and(numeric))
                {
                    for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                        // Preserve integer-only arithmetic as well as normal floating offsets.
                        let value = if let (Some(c), Some(o)) = (
                            cells[i].and_then(Value::as_i64),
                            offsets[i].and_then(Value::as_i64),
                        ) {
                            c.checked_mul(512)
                                .and_then(|v| v.checked_sub(16384))
                                .and_then(|v| v.checked_add(o))
                                .map(Value::from)
                        } else {
                            None
                        }
                        .unwrap_or_else(|| {
                            json!(
                                cells[i].unwrap().as_f64().unwrap() * 512. - 16384.
                                    + offsets[i].unwrap().as_f64().unwrap()
                            )
                        });
                        position.push((format!("world_{axis}"), value));
                    }
                    break;
                }
            }
            if position.is_empty() {
                continue;
            }
            let owner = f.get("m_iTeamNum");
            let lane = f.get("m_iPrimaryLane");
            let mut facts = Vec::new();
            for (source, dest) in [
                ("m_flCumulativeDistance", "zipline_route_distance"),
                ("m_iNodeIndex", "zipline_node_index"),
                ("m_bCapturable", "zipline_capturable"),
                ("m_bDisableZippingToByPlayers", "zipline_disabled"),
                ("m_eCaptureState", "zipline_capture_state"),
            ] {
                if let Some(value) = f.get(source).filter(|v| numeric(v) || v.is_boolean()) {
                    facts.push((dest.into(), value.clone()));
                }
            }
            if let Some(lane) = integer(lane) {
                facts.push((format!("lane-code:{lane}"), json!(1.)));
            }
            let count = integer(f.get("m_vecConnections"));
            let mut complete = count.is_some_and(|n| (0..=32).contains(&n));
            let mut connections = Vec::new();
            if complete {
                for slot in 0..count.unwrap() {
                    let target = integer(f.get(&format!("m_vecConnections.{slot}")))
                        .filter(|h| *h >= 0)
                        .map(|h| (h & 16383, h >> 14));
                    if let Some(target) =
                        target.filter(|id| self.nodes.get(id).is_some_and(|n| n.active))
                    {
                        connections.push(target)
                    } else {
                        complete = false
                    }
                }
            }
            // Python's equality treats numeric 2.0/3.0 like 2/3. Boon's source is integer,
            // but retain that behavior for saved evidence with wider numeric encoding.
            let owner_number = owner.and_then(Value::as_f64);
            if complete && matches!(owner_number, Some(2. | 3.)) {
                let eligible = connections
                    .iter()
                    .map(|id| &self.nodes[id].fields)
                    .filter(|other| {
                        other.get("m_iPrimaryLane").unwrap_or(&Value::Null)
                            == lane.unwrap_or(&Value::Null)
                    })
                    .collect::<Vec<_>>();
                if !eligible.is_empty()
                    && eligible.iter().all(|other| {
                        matches!(
                            other.get("m_iTeamNum").and_then(Value::as_f64),
                            Some(0. | 2. | 3.)
                        )
                    })
                {
                    facts.push((
                        "zipline_frontier".into(),
                        json!(
                            eligible
                                .iter()
                                .any(|other| other.get("m_iTeamNum").and_then(Value::as_f64)
                                    != owner_number)
                        ),
                    ));
                }
            }
            result.push(View {
                key: key(*identity),
                kind: "zipline",
                team: owner.cloned().unwrap_or(Value::Null),
                entity_class: "CCitadelZipLineNode",
                public: Ordered(position),
                private: json!({}),
                public_facts: Ordered(facts),
                source_tick: tick,
                lane: lane.cloned().unwrap_or(Value::Null),
                connections: connections.into_iter().map(key).collect(),
                connections_complete: complete,
                evidence: "public-zipline-ownership",
                display_name: format!("Zipline · team {} · lane {}", display(owner), display(lane)),
            });
        }
        result
    }
}
#[pyclass(module = "boon._boon")]
pub(crate) struct ZiplineViews {
    inner: State,
}
#[pymethods]
impl ZiplineViews {
    #[new]
    fn new() -> Self {
        Self {
            inner: State::default(),
        }
    }
    fn update(
        &mut self,
        py: Python<'_>,
        index: i64,
        serial: i64,
        active: bool,
        fields: String,
    ) -> PyResult<()> {
        py.detach(|| {
            let identity = (index, serial);
            if self
                .inner
                .nodes
                .get(&identity)
                .is_some_and(|old| old.active == active && old.signature == fields)
            {
                return Ok(());
            }
            let decoded: Map<String, Value> = serde_json::from_str(&fields)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
            if !self.inner.nodes.contains_key(&identity) {
                self.inner.order.push(identity);
            }
            self.inner.nodes.insert(
                identity,
                Node {
                    active,
                    fields: decoded,
                    signature: fields,
                },
            );
            self.inner.dirty = true;
            Ok(())
        })
    }

    fn statistics(&self) -> (usize, u64) {
        (self.inner.nodes.len(), self.inner.builds)
    }
    fn remove(&mut self, index: i64, serial: i64) {
        let id = (index, serial);
        if self.inner.nodes.remove(&id).is_some() {
            self.inner.dirty = true;
        }
        self.inner.order.retain(|v| *v != id);
    }
    fn project(&mut self, py: Python<'_>, tick: i64) -> PyResult<String> {
        py.detach(|| {
            serde_json::to_string(self.inner.project(tick))
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn individual_nodes_connections_ownership_and_absence() {
        let mut state = State::default();
        for (index, team, next) in [(60, 2, 61), (61, 3, 60)] {
            let fields=json!({"m_iTeamNum":team,"m_iPrimaryLane":4,"m_vecConnections":1,"m_vecConnections.0":next+(1<<14),"CBodyComponent.m_cellX":32,"CBodyComponent.m_cellY":32,"CBodyComponent.m_cellZ":32,"CBodyComponent.m_vecX":10.,"CBodyComponent.m_vecY":20.,"CBodyComponent.m_vecZ":30.}).as_object().unwrap().clone();
            state.order.push((index, 1));
            state.nodes.insert(
                (index, 1),
                Node {
                    active: true,
                    signature: String::new(),
                    fields,
                },
            );
        }
        state.dirty = true;
        let views = serde_json::to_value(state.project(7)).unwrap();
        assert_eq!(views.as_array().unwrap().len(), 2);
        assert_eq!(views[0]["publicFacts"]["zipline_frontier"], true);
        assert_eq!(views[0]["public"]["world_x"], 10.);
        assert_eq!(views[0]["connections"], json!(["entity:16445"]));
        state.nodes.get_mut(&(61, 1)).unwrap().active = false;
        state.dirty = true;
        assert_eq!(
            serde_json::to_value(state.project(8)).unwrap()[0]["connectionsComplete"],
            false
        );
    }
}
