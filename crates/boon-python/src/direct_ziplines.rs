//! Experimental, source-shaped zipline columns. No JSON entity reconstruction,
//! ownership/frontier inference, or interpretation of disappearance as destruction.
use crate::*;
use boon_parser::{Entity, FieldValue};

const CLASS: &str = "CCitadelZipLineNode";

fn integer(value: Option<&FieldValue>) -> Option<i64> {
    match value? {
        FieldValue::I32(v) => Some(i64::from(*v)),
        FieldValue::U32(v) => Some(i64::from(*v)),
        FieldValue::I64(v) => Some(*v),
        FieldValue::U64(v) => i64::try_from(*v).ok(),
        _ => None,
    }
}
fn number(value: Option<&FieldValue>) -> Option<f64> {
    match value? {
        FieldValue::F32(v) if v.is_finite() => Some(f64::from(*v)),
        value => integer(Some(value)).map(|v| v as f64),
    }
}
fn boolean(value: Option<&FieldValue>) -> Option<bool> {
    match value? { FieldValue::Bool(v) => Some(*v), _ => None }
}
#[derive(Default)]
struct Keys {
    team: Option<u64>, lane: Option<u64>, node: Option<u64>, distance: Option<u64>,
    capturable: Option<u64>, disabled: Option<u64>, capture: Option<u64>,
    positions: [[Option<u64>;6];2],
}
impl Keys {
    fn resolve(ctx: &boon_parser::Context) -> Self {
        let key = |name: &str| ctx.serializers().get(CLASS).and_then(|s| s.resolve_field_key(name));
        Self {
            team: key("m_iTeamNum"), lane: key("m_iPrimaryLane"), node: key("m_iNodeIndex"),
            distance: key("m_flCumulativeDistance"), capturable: key("m_bCapturable"),
            disabled: key("m_bDisableZippingToByPlayers"), capture: key("m_eCaptureState"),
            positions: ["CBodyComponent.m_skeletonInstance.m_vecOrigin.", "CBodyComponent."]
                .map(|base| ["m_cellX","m_cellY","m_cellZ","m_vecX","m_vecY","m_vecZ"]
                    .map(|name| key(&format!("{base}{name}")))),
        }
    }
    fn position(&self, entity: &Entity) -> Option<[f64;3]> {
        for keys in self.positions {
            let raw = keys.map(|key| number(entity.field_value(key)));
            if raw.iter().all(Option::is_some) {
                let raw = raw.map(Option::unwrap);
                return Some([raw[0]*512.-16384.+raw[3], raw[1]*512.-16384.+raw[4], raw[2]*512.-16384.+raw[5]]);
            }
        }
        None
    }
}
#[derive(Default)]
struct Columns {
    tick: Vec<i32>, index: Vec<i32>, serial: Vec<u32>, active: Vec<bool>,
    team: Vec<Option<i64>>, lane: Vec<Option<i64>>, node: Vec<Option<i64>>,
    distance: Vec<Option<f64>>, capturable: Vec<Option<bool>>, disabled: Vec<Option<bool>>,
    capture: Vec<Option<i64>>, x: Vec<Option<f64>>, y: Vec<Option<f64>>, z: Vec<Option<f64>>,
}
impl Columns {
    fn push(&mut self, tick:i32, entity:&Entity, keys:&Keys) {
        self.tick.push(tick); self.index.push(entity.index); self.serial.push(entity.serial);
        self.active.push(entity.active);
        // Inactive entries retain only identity and the explicit source activity
        // flag. Their cached fields are not an observation at this sample tick.
        let value = |key| if entity.active {entity.field_value(key)} else {None};
        self.team.push(integer(value(keys.team))); self.lane.push(integer(value(keys.lane)));
        self.node.push(integer(value(keys.node))); self.distance.push(number(value(keys.distance)));
        self.capturable.push(boolean(value(keys.capturable))); self.disabled.push(boolean(value(keys.disabled)));
        self.capture.push(integer(value(keys.capture)));
        let position = entity.active.then(|| keys.position(entity)).flatten();
        self.x.push(position.map(|v|v[0])); self.y.push(position.map(|v|v[1])); self.z.push(position.map(|v|v[2]));
    }
    fn frame(self) -> PolarsResult<DataFrame> {
        df_from_columns(vec![
            Column::new("tick".into(),self.tick), Column::new("entity_index".into(),self.index),
            Column::new("entity_serial".into(),self.serial), Column::new("active".into(),self.active),
            Column::new("team".into(),self.team), Column::new("lane".into(),self.lane),
            Column::new("world_x".into(),self.x), Column::new("world_y".into(),self.y), Column::new("world_z".into(),self.z),
            Column::new("node_index".into(),self.node), Column::new("cumulative_distance".into(),self.distance),
            Column::new("capturable".into(),self.capturable), Column::new("disabled".into(),self.disabled),
            Column::new("capture_state".into(),self.capture),
        ])
    }
}
#[pymethods]
impl Demo {
    /// Sample only CCitadelZipLineNode at exact completed source ticks.
    /// Null means absent, wrong type, nonfinite, or inactive; identity/activity
    /// are always explicit. A missing row is not a destruction/death claim.
    /// Coordinates use only the two documented cell/offset field layouts.
    /// No inferred frontier or connections are emitted by this minimal API.
    #[pyo3(signature = (*, ticks))]
    pub(crate) fn direct_zipline_ticks(&self, py:Python<'_>, ticks:Vec<i32>) -> PyResult<PyDataFrame> {
        py.detach(|| {
            let selected:HashSet<i32> = ticks.into_iter().collect();
            let mut columns = Columns::default();
            if selected.is_empty() {return columns.frame().map(PyDataFrame).map_err(|e|InvalidDemoError::new_err(e.to_string()));}
            let init = self.parser.parse_init().map_err(to_py_err)?;
            let keys = Keys::resolve(&init);
            drop(init);
            let filter = HashSet::from([CLASS]);
            let mut error = None;
            let mut last_check = std::time::Instant::now();
            self.parser.try_run_to_end_filtered(&filter, |ctx| {
                if last_check.elapsed().as_millis() >= 100 {
                    if let Err(e) = Python::attach(|py|py.check_signals()) {error=Some(e); return Err(crate::bounded_spool::aborted());}
                    last_check=std::time::Instant::now();
                }
                if selected.contains(&ctx.tick()) {
                    for (_,entity) in ctx.entities().iter() {
                        if entity.class_name.as_ref()==CLASS {columns.push(ctx.tick(),entity,&keys);}
                    }
                }
                Ok(())
            }).map_err(|e|error.take().unwrap_or_else(||to_py_err(e)))?;
            columns.frame().map(PyDataFrame).map_err(|e|InvalidDemoError::new_err(e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nulls_types_and_activity_are_not_defaulted() {
        assert_eq!(integer(None),None);
        assert_eq!(integer(Some(&FieldValue::Bool(false))),None);
        assert_eq!(integer(Some(&FieldValue::F32(2.))),None);
        assert_eq!(integer(Some(&FieldValue::U64(u64::MAX))),None);
        assert_eq!(number(Some(&FieldValue::F32(f32::NAN))),None);
        assert_eq!(number(Some(&FieldValue::U32(0))),Some(0.));
        assert_eq!(boolean(None),None);
        assert_eq!(boolean(Some(&FieldValue::U32(0))),None);
        assert_eq!(boolean(Some(&FieldValue::Bool(false))),Some(false));
        let fields=[(1,FieldValue::U32(2))].into_iter().collect();
        let keys=Keys{team:Some(1),..Default::default()};
        let entity=Entity::from_fields(9,7,0,CLASS,false,fields).unwrap();
        let mut cols=Columns::default(); cols.push(100,&entity,&keys);
        assert_eq!(cols.team,vec![None]); assert_eq!(cols.active,vec![false]); assert_eq!(cols.x,vec![None]);
        let frame=cols.frame().unwrap(); assert_eq!(frame.height(),1); assert_eq!(frame.width(),14);
    }
}
