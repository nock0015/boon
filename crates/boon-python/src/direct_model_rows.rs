//! Append-only columns for emitted changes and lifecycle tombstones.
//! Runtime storage never retains a complete output Node. Strings are copied into
//! Arrow string-view builders at emission; owned extras/relations are consumed.
use super::*;

pub(super) struct EmittedRows {
    capture_stamps:[Vec<CaptureStamp>;6],
    emission_clock:Vec<CaptureStamp>,
    e1_time_available:Vec<bool>,
    tick: Vec<i32>,
    match_seconds: Vec<Option<f64>>,
    entity_index: Vec<Option<i32>>,
    entity_serial: Vec<Option<u32>>,
    owner_steam_id: Vec<Option<u64>>,
    team: Vec<Option<i64>>,
    hero_id: Vec<Option<i64>>,
    ability_id: Vec<Option<i64>>,
    audience: Vec<i32>,
    observer_team: Vec<Option<i64>>,
    active: Vec<bool>,
    source_time: Vec<Option<f64>>,
    map_source_time: Vec<Option<f64>>,
    health_source_time: Vec<Option<f64>>,
    world_source_time: Vec<Option<f64>>,
    position_observation_time: Vec<Option<f64>>,
    position_observation_live: Vec<bool>,
    position_emitted_time: Vec<Option<f64>>,
    node_key: StringChunkedBuilder,
    kind: StringChunkedBuilder,
    entity_class: StringChunkedBuilder,
    values: [Vec<Option<f64>>; 66],
    extra_rows: Vec<u64>,
    extra_names: StringChunkedBuilder,
    extra_values: Vec<f64>,
    relation_rows: Vec<u64>,
    relation_targets: StringChunkedBuilder,
    relation_kinds: Vec<i32>,
    // Existing projection tests inspect emitted snapshots. Production builds
    // have no snapshot vector and incur no additional Node clone here.
    #[cfg(test)]
    records: Vec<(i32, Option<f64>, Node)>,
}

impl Default for EmittedRows {
    fn default() -> Self { Self::with_capacity(1024) }
}

impl EmittedRows {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            capture_stamps:std::array::from_fn(|_|Vec::with_capacity(capacity)),
            emission_clock:Vec::with_capacity(capacity),
            e1_time_available:Vec::with_capacity(capacity),
            tick: Vec::with_capacity(capacity),
            match_seconds: Vec::with_capacity(capacity),
            entity_index: Vec::with_capacity(capacity),
            entity_serial: Vec::with_capacity(capacity),
            owner_steam_id: Vec::with_capacity(capacity),
            team: Vec::with_capacity(capacity),
            hero_id: Vec::with_capacity(capacity),
            ability_id: Vec::with_capacity(capacity),
            audience: Vec::with_capacity(capacity),
            observer_team: Vec::with_capacity(capacity),
            active: Vec::with_capacity(capacity),
            source_time: Vec::with_capacity(capacity),
            map_source_time: Vec::with_capacity(capacity),
            health_source_time: Vec::with_capacity(capacity),
            world_source_time: Vec::with_capacity(capacity),
            position_observation_time: Vec::with_capacity(capacity),
            position_observation_live: Vec::with_capacity(capacity),
            position_emitted_time: Vec::with_capacity(capacity),
            node_key: StringChunkedBuilder::new("node_key".into(), capacity),
            kind: StringChunkedBuilder::new("kind".into(), capacity),
            entity_class: StringChunkedBuilder::new("entity_class".into(), capacity),
            values: std::array::from_fn(|_| Vec::with_capacity(capacity)),
            extra_rows: Vec::with_capacity(capacity),
            extra_names: StringChunkedBuilder::new("name".into(), capacity),
            extra_values: Vec::with_capacity(capacity),
            relation_rows: Vec::with_capacity(capacity),
            relation_targets: StringChunkedBuilder::new("target_key".into(), capacity),
            relation_kinds: Vec::with_capacity(capacity),
            #[cfg(test)]
            records: Vec::with_capacity(capacity),
        }
    }

    pub(super) fn len(&self) -> usize { self.tick.len() }

    pub(super) fn push(&mut self, (tick, time, node): (i32, Option<f64>, Node)) {
        let update_row = self.len() as u64;
        #[cfg(test)]
        self.records.push((tick, time, node.clone()));
        for (values,capture) in self.capture_stamps.iter_mut().zip(node.capture_stamps){values.push(capture);}
        self.emission_clock.push(node.emission_clock);self.e1_time_available.push(node.e1_time_available);
        self.tick.push(tick);
        self.match_seconds.push(time);
        self.entity_index.push(node.id.map(|id| id.0));
        self.entity_serial.push(node.id.map(|id| id.1));
        self.owner_steam_id.push(node.owner);
        self.team.push(node.team);
        self.hero_id.push(node.hero);
        self.ability_id.push(node.ability);
        self.audience.push(node.audience);
        self.observer_team.push(node.observer);
        self.active.push(node.active);
        self.source_time.push(node.source_time);
        self.map_source_time.push(node.map_time);
        self.health_source_time.push(node.health_time);
        self.world_source_time.push(node.world_time);
        self.position_observation_time.push(node.position_observation_time);
        self.position_observation_live.push(node.position_observation_live);
        self.position_emitted_time.push(node.position_emitted_time);
        self.node_key.append_value(&node.key);
        self.kind.append_value(&node.kind);
        self.entity_class.append_option(node.class.as_deref());
        for (column, value) in self.values.iter_mut().zip(node.values) {
            column.push(value);
        }
        // BTreeMap iteration retains the original per-row sorted extras order;
        // relation Vec iteration retains insertion order and duplicates.
        for (name, value) in node.extras {
            self.extra_rows.push(update_row);
            self.extra_names.append_value(name);
            self.extra_values.push(value);
        }
        for (target, relation) in node.relations {
            self.relation_rows.push(update_row);
            self.relation_targets.append_value(target);
            self.relation_kinds.push(relation);
        }
    }

    pub(super) fn frames(self) -> PolarsResult<(DataFrame, DataFrame, DataFrame)> {
        let len = self.len();
        macro_rules! column {
            ($field:ident) => { Column::new(stringify!($field).into(), self.$field) };
        }
        let mut columns = Vec::with_capacity(22 + FIELDS.len());
        columns.push(Column::new("update_row".into(), (0..len as u64).collect::<Vec<_>>()));
        columns.push(column!(tick));
        columns.push(column!(match_seconds));
        columns.push(self.node_key.finish().into_column());
        columns.push(self.kind.finish().into_column());
        columns.push(self.entity_class.finish().into_column());
        columns.push(column!(entity_index));
        columns.push(column!(entity_serial));
        columns.push(column!(owner_steam_id));
        columns.push(column!(team));
        columns.push(column!(hero_id));
        columns.push(column!(ability_id));
        columns.push(column!(audience));
        columns.push(column!(observer_team));
        columns.push(column!(active));
        columns.push(column!(source_time));
        columns.push(column!(map_source_time));
        columns.push(column!(health_source_time));
        columns.push(column!(world_source_time));
        columns.push(column!(position_observation_time));
        columns.push(column!(position_observation_live));
        columns.push(column!(position_emitted_time));
        for (name, values) in FIELDS.into_iter().zip(self.values) {
            columns.push(Column::new(name.into(), values));
        }
        columns.push(Column::new("elapsed_play_seconds".into(),self.emission_clock.iter().map(|s|s.elapsed_play_seconds).collect::<Vec<_>>()));
        columns.push(Column::new("elapsed_play_known".into(),self.emission_clock.iter().map(|s|s.elapsed_play_seconds.is_some()).collect::<Vec<_>>()));
        columns.push(Column::new("timing_segment_id".into(),self.emission_clock.iter().map(|s|s.timing_segment_id).collect::<Vec<_>>()));
        columns.push(Column::new("e1_time_available".into(),self.e1_time_available));
        for(prefix,captures)in ["source","map","health","world","position_observation","position_emitted"].into_iter().zip(self.capture_stamps){
            columns.push(Column::new(format!("{prefix}_capture_tick").into(),captures.iter().map(|s|s.capture_tick).collect::<Vec<_>>()));
            columns.push(Column::new(format!("{prefix}_timing_segment_id").into(),captures.iter().map(|s|s.timing_segment_id).collect::<Vec<_>>()));
            columns.push(Column::new(format!("{prefix}_elapsed_play_seconds").into(),captures.iter().map(|s|s.elapsed_play_seconds).collect::<Vec<_>>()));
            columns.push(Column::new(format!("{prefix}_elapsed_play_known").into(),captures.iter().map(|s|s.elapsed_play_seconds.is_some()).collect::<Vec<_>>()));
        }
        let updates = df_from_columns(columns)?;
        let extras = df_from_columns(vec![
            Column::new("update_row".into(), self.extra_rows),
            self.extra_names.finish().into_column(),
            Column::new("value".into(), self.extra_values),
        ])?;
        let relations = df_from_columns(vec![
            Column::new("update_row".into(), self.relation_rows),
            self.relation_targets.finish().into_column(),
            Column::new("relation".into(), self.relation_kinds),
        ])?;
        Ok((updates, extras, relations))
    }
}

#[cfg(test)]
impl std::ops::Deref for EmittedRows {
    type Target = Vec<(i32, Option<f64>, Node)>;
    fn deref(&self) -> &Self::Target { &self.records }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent legacy row scan, kept solely as a materialization oracle.
    fn legacy_frames(rows: &[(i32, Option<f64>, Node)]) -> (DataFrame, DataFrame, DataFrame) {
        macro_rules! c {
            ($name:expr, $get:expr) => {
                Column::new($name.into(), rows.iter().map($get).collect::<Vec<_>>())
            };
        }
        let mut columns = vec![
            Column::new("update_row".into(), (0..rows.len() as u64).collect::<Vec<_>>()),
            c!("tick", |r| r.0),
            c!("match_seconds", |r| r.1),
            c!("node_key", |r| r.2.key.clone()),
            c!("kind", |r| r.2.kind.clone()),
            c!("entity_class", |r| r.2.class.clone()),
            c!("entity_index", |r| r.2.id.map(|id| id.0)),
            c!("entity_serial", |r| r.2.id.map(|id| id.1)),
            c!("owner_steam_id", |r| r.2.owner),
            c!("team", |r| r.2.team),
            c!("hero_id", |r| r.2.hero),
            c!("ability_id", |r| r.2.ability),
            c!("audience", |r| r.2.audience),
            c!("observer_team", |r| r.2.observer),
            c!("active", |r| r.2.active),
            c!("source_time", |r| r.2.source_time),
            c!("map_source_time", |r| r.2.map_time),
            c!("health_source_time", |r| r.2.health_time),
            c!("world_source_time", |r| r.2.world_time),
            c!("position_observation_time", |r| r.2.position_observation_time),
            c!("position_observation_live", |r| r.2.position_observation_live),
            c!("position_emitted_time", |r| r.2.position_emitted_time),
        ];
        for (j, name) in FIELDS.iter().enumerate() {
            columns.push(c!(*name, |r| r.2.values[j]));
        }
        let (mut er, mut en, mut ev) = (Vec::<u64>::new(), Vec::<String>::new(), Vec::<f64>::new());
        let (mut rr, mut rt, mut rk) = (Vec::<u64>::new(), Vec::<String>::new(), Vec::<i32>::new());
        for (row, (_, _, node)) in rows.iter().enumerate() {
            for (name, value) in &node.extras {
                er.push(row as u64); en.push(name.clone()); ev.push(*value);
            }
            for (target, kind) in &node.relations {
                rr.push(row as u64); rt.push(target.clone()); rk.push(*kind);
            }
        }
        (
            df_from_columns(columns).unwrap(),
            df_from_columns(vec![Column::new("update_row".into(), er), Column::new("name".into(), en), Column::new("value".into(), ev)]).unwrap(),
            df_from_columns(vec![Column::new("update_row".into(), rr), Column::new("target_key".into(), rt), Column::new("relation".into(), rk)]).unwrap(),
        )
    }

    fn assert_frames_equal(actual: (DataFrame, DataFrame, DataFrame), expected: (DataFrame, DataFrame, DataFrame)) {
        for (actual, expected) in [actual.0, actual.1, actual.2].into_iter().zip([expected.0, expected.1, expected.2]) {
            assert_eq!(actual.height(), expected.height());
            for expected in expected.columns() {
                let actual=actual.column(expected.name()).unwrap();
                assert_eq!(actual.dtype(),expected.dtype());
                assert!(actual.equals_missing(expected), "different column {}", actual.name());
            }
        }
    }

    #[test]
    fn empty_columns_preserve_legacy_schema() {
        assert_frames_equal(EmittedRows::with_capacity(0).frames().unwrap(), legacy_frames(&[]));
    }

    #[test]
    fn emitted_columns_match_legacy_rows_and_tombstones() {
        let mut rows = EmittedRows::with_capacity(1);
        let mut node = Node::new("entity:16385".into(), "player", 2);
        node.class = Some("a-long-class-name-that-exceeds-inline-string-storage".into());
        node.id = Some((1, 1));
        node.owner = Some(u64::MAX);
        node.team = Some(3);
        node.hero = Some(42);
        node.ability = Some(77);
        node.observer = Some(2);
        node.source_time = Some(0.0);
        node.map_time = Some(1.0);
        node.health_time = Some(2.0);
        node.world_time = Some(3.0);
        node.position_observation_time = Some(4.0);
        node.position_observation_live = true;
        node.position_emitted_time = Some(5.0);
        for (j, value) in node.values.iter_mut().enumerate() {
            *value = (j % 3 != 0).then_some(j as f64 - 0.5);
        }
        node.extras.insert("z-last".into(), -0.0);
        node.extras.insert("a-first".into(), 9.0);
        node.relations = vec![("target-z".into(), 2), ("target-a".into(), 1), ("target-z".into(), 2)].into();
        rows.push((-1, None, node.clone()));
        rows.push((0, Some(1.25), Node::new("null-row".into(), "", 0)));
        node.active = false;
        node.values = [None; 66];
        node.extras.clear();
        node.relations.clear();
        node.source_time = None;
        rows.push((1, None, node));
        assert_eq!(rows.len(), 3);
        assert!(!rows[2].2.active);
        let expected = legacy_frames(&rows.records);
        assert_frames_equal(rows.frames().unwrap(), expected);
    }
}
