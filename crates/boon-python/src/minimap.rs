//! Team-specific minimap observations recorded on `CCitadelTeam`.
//! World positions are deliberately never used as a fallback for hidden icons.

use crate::*;
use boon_parser::{Context, Entity, FieldValue, Serializer};
use crate::entity_evidence::movement_policy::boundary as movement_sample_boundary;

#[derive(Clone, Copy, Default)]
struct ControllerKeys {
    steam_id: Option<u64>,
    pawn: Option<u64>,
    hero_pawn: Option<u64>,
}

impl ControllerKeys {
    fn resolve(ctx: &Context) -> Self {
        let serializer = ctx.serializers().get("CCitadelPlayerController");
        let key = |name| serializer.and_then(|s| s.resolve_field_key(name));
        Self {
            steam_id: key("m_steamID"),
            pawn: key("m_hPawn"),
            hero_pawn: key("m_hHeroPawn"),
        }
    }
}

/// Bind only controllers whose current serial-bearing pawn handles agree.
/// More than one controller for a pawn makes its Steam ID ambiguous.
fn controller_steam_ids(
    ctx: &Context,
    keys: ControllerKeys,
) -> HashMap<boon_parser::EntityId, Option<u64>> {
    let mut by_pawn = HashMap::new();
    for (_, controller) in ctx.entities().iter().filter(|(_, entity)| {
        entity.active && entity.class_name.as_ref() == "CCitadelPlayerController"
    }) {
        let mut pawns = Vec::new();
        for key in [keys.pawn, keys.hero_pawn] {
            let Some(pawn) = controller
                .get_handle(key)
                .and_then(|handle| ctx.entities().get_by_handle(handle))
                .filter(|entity| entity.class_name.as_ref() == "CCitadelPlayerPawn")
            else {
                continue;
            };
            if !pawns.contains(&pawn.id()) {
                pawns.push(pawn.id());
            }
        }
        if pawns.len() != 1 {
            continue;
        }
        let steam_id = match controller.field_value(keys.steam_id) {
            Some(FieldValue::U64(id)) if *id != 0 => Some(*id),
            _ => None,
        };
        match by_pawn.entry(pawns[0]) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(steam_id);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    by_pawn
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
    // Indexed network vectors have used both path renderings in replay tools.
    entity
        .get_by_name(&format!("m_vecFOWEntities.{index}.{name}"), serializer)
        .or_else(|| entity.get_by_name(&format!("m_vecFOWEntities.{index:04}.{name}"), serializer))
}

fn entry_i64(entity: &Entity, serializer: &Serializer, index: usize, name: &str) -> Option<i64> {
    integer(entry_value(entity, serializer, index, name)?)
}

/// Resolve the FOW row's pawn index conservatively. Some builds encode the
/// index directly and others expose its unsigned-zigzag alias. If both resolve
/// to different pawns, the row remains unbound.
fn pawn_for_fow<'a>(
    ctx: &'a Context,
    raw_index: i64,
    target_team: i64,
    pawn_team_key: Option<u64>,
) -> Option<&'a Entity> {
    let team_key = pawn_team_key?;
    let direct = i32::try_from(raw_index).ok();
    let half = (raw_index >= 0 && raw_index % 2 == 0)
        .then(|| i32::try_from(raw_index / 2).ok())
        .flatten();
    let mut matches = Vec::new();
    for index in [direct, half].into_iter().flatten() {
        if matches.iter().any(|&(seen, _)| seen == index) {
            continue;
        }
        if let Some(pawn) = ctx.entities().get(index)
            && pawn.class_name.as_ref() == "CCitadelPlayerPawn"
            && pawn.get_i64(Some(team_key)) == target_team
        {
            matches.push((index, pawn));
        }
    }
    (matches.len() == 1).then(|| matches[0].1)
}

#[derive(Default)]
struct MinimapSlot {
    identity: Option<(i64,i64,Option<(i32,u32)>)>,
    shown: Option<bool>,
    sample: Option<(i32,i64,i64)>,
}
impl MinimapSlot {
    fn observe(&mut self, identity:(i64,i64,Option<(i32,u32)>), shown:Option<bool>,
        cell:Option<(i64,i64)>,tick:i32)->(Option<(i32,i64,i64)>,bool) {
        let identity_changed=self.identity!=Some(identity);
        if identity_changed {self.sample=None;}
        let show=shown==Some(true) && self.shown!=Some(true);
        let refresh=shown==Some(true) && cell.is_some() &&
            (identity_changed || show || movement_sample_boundary(tick) || self.sample.is_none());
        if refresh {let (x,y)=cell.unwrap();self.sample=Some((tick,x,y));}
        if shown==Some(true) && cell.is_none() {self.sample=None;}
        self.identity=Some(identity);self.shown=shown;
        (self.sample,refresh)
    }
}
#[derive(Default)]
struct MinimapCols {
    tick: Vec<i32>,
    observer_team: Vec<i64>,
    target_team: Vec<i64>,
    target_hero_id: Vec<Option<i64>>,
    target_steam_id: Vec<Option<u64>>,
    target_entity_index: Vec<Option<i32>>,
    target_entity_serial: Vec<Option<u32>>,
    fow_entity_index_raw: Vec<i64>,
    visible_on_map: Vec<Option<bool>>,
    map_x: Vec<Option<i64>>,
    map_y: Vec<Option<i64>>,
    tick_hidden_raw: Vec<Option<i64>>,
    last_seen_tick: Vec<Option<i32>>,
    last_seen_x: Vec<Option<i64>>,
    last_seen_y: Vec<Option<i64>>,
    last_seen: HashMap<(i64, i32, u32), (i32, i64, i64)>,
    last_seen_disk: Option<rusqlite::Connection>,
    map_samples: HashMap<(i32,u32,usize),MinimapSlot>,
}

impl MinimapCols {
    fn collect(
        &mut self,
        ctx: &Context,
        hero_key: Option<u64>,
        pawn_team_key: Option<u64>,
        controller_keys: ControllerKeys,
        emit: bool,
    ) {
        let Some(serializer) = ctx.serializers().get("CCitadelTeam") else {
            return;
        };
        let steam_ids = emit.then(|| controller_steam_ids(ctx, controller_keys));
        let mut live_slots=HashSet::new();
        for (_, team) in ctx
            .entities()
            .iter()
            .filter(|(_, e)| e.active && e.class_name.as_ref() == "CCitadelTeam")
        {
            let Some(observer_team) = field_i64(team, serializer, "m_iTeamNum") else {
                continue;
            };
            if observer_team != 2 && observer_team != 3 {
                continue;
            }
            let Some(count) = field_i64(team, serializer, "m_vecFOWEntities") else {
                continue;
            };
            // pbdems2 stores a dynamic-array index in one byte.
            if !(0..=255).contains(&count) {
                continue;
            }
            for i in 0..count as usize {
                // Hero icon class 1 is the recorded team FOW hero entry.
                if entry_i64(team, serializer, i, "m_eClass") != Some(1) {
                    continue;
                }
                let Some(target_team) = entry_i64(team, serializer, i, "m_nTeam") else {
                    continue;
                };
                if (target_team != 2 && target_team != 3) || target_team == observer_team {
                    continue;
                }
                let Some(raw_index) = entry_i64(team, serializer, i, "m_nEntIndex") else {
                    continue;
                };
                let pawn = pawn_for_fow(ctx, raw_index, target_team, pawn_team_key);
                let identity = pawn.map(Entity::id);
                let shown = match entry_value(team, serializer, i, "m_bVisibleOnMap") {
                    Some(FieldValue::Bool(value)) => Some(*value),
                    _ => None,
                };
                let cell = if shown == Some(true) {
                    match (
                        entry_i64(team, serializer, i, "m_nPositionX"),
                        entry_i64(team, serializer, i, "m_nPositionY"),
                    ) {
                        (Some(x), Some(y)) if (0..=255).contains(&x) && (0..=255).contains(&y) => {
                            Some((x, y))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let slot_key=(team.index,team.serial,i);
                live_slots.insert(slot_key);
                let (sample,refresh)=self.map_samples.entry(slot_key).or_default().observe(
                    (raw_index,target_team,identity.map(|id|(id.index,id.serial))),shown,cell,ctx.tick());
                let cell=if shown==Some(true) {sample.map(|(_,x,y)|(x,y))} else {None};
                let id_key = identity.map(|id| (observer_team, id.index, id.serial));
                if let (true,Some(key), Some((x, y))) = (refresh,id_key, cell) {
                    if let Some(disk)=&self.last_seen_disk {
                        disk.prepare_cached("INSERT OR REPLACE INTO last_seen VALUES (?,?,?,?,?,?)").expect("minimap history statement failed").execute(rusqlite::params![key.0,key.1,key.2,ctx.tick(),x,y]).expect("minimap history write failed");
                    } else {self.last_seen.insert(key, (ctx.tick(), x, y));}
                }
                if !emit {
                    continue;
                }
                let remembered = id_key.and_then(|key| {
                    if let Some(disk)=&self.last_seen_disk {
                        use rusqlite::OptionalExtension;
                        disk.prepare_cached("SELECT tick,x,y FROM last_seen WHERE team=? AND entity=? AND serial=?").expect("minimap history statement failed").query_row(rusqlite::params![key.0,key.1,key.2],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().expect("minimap history read failed")
                    } else {self.last_seen.get(&key).copied()}
                });
                self.tick.push(ctx.tick());
                self.observer_team.push(observer_team);
                self.target_team.push(target_team);
                self.target_hero_id.push(
                    pawn.and_then(|p| hero_key.map(|key| p.get_i64(Some(key))))
                        .filter(|&id| id > 0),
                );
                self.target_steam_id.push(
                    identity
                        .and_then(|id| steam_ids.as_ref()?.get(&id))
                        .copied()
                        .flatten(),
                );
                self.target_entity_index.push(identity.map(|id| id.index));
                self.target_entity_serial.push(identity.map(|id| id.serial));
                self.fow_entity_index_raw.push(raw_index);
                self.visible_on_map.push(shown);
                self.map_x.push(cell.map(|(x, _)| x));
                self.map_y.push(cell.map(|(_, y)| y));
                self.tick_hidden_raw
                    .push(entry_i64(team, serializer, i, "m_nTickHidden"));
                self.last_seen_tick
                    .push(remembered.map(|(tick, _, _)| tick));
                self.last_seen_x.push(remembered.map(|(_, x, _)| x));
                self.last_seen_y.push(remembered.map(|(_, _, y)| y));
            }
        }
        // Prune slots on shrink, class/identity invalidation, inactivity or team
        // generation removal. The last-visible identity history is separate.
        self.map_samples.retain(|key,_|live_slots.contains(key));
    }

    fn into_frame(self) -> PyResult<DataFrame> {
        df_from_columns(vec![
            Column::new("tick".into(), self.tick),
            Column::new("observer_team".into(), self.observer_team),
            Column::new("target_team".into(), self.target_team),
            Column::new("target_hero_id".into(), self.target_hero_id),
            Column::new("target_steam_id".into(), self.target_steam_id),
            Column::new("target_entity_index".into(), self.target_entity_index),
            Column::new("target_entity_serial".into(), self.target_entity_serial),
            Column::new("fow_entity_index_raw".into(), self.fow_entity_index_raw),
            Column::new("visible_on_map".into(), self.visible_on_map),
            Column::new("map_x".into(), self.map_x),
            Column::new("map_y".into(), self.map_y),
            Column::new("tick_hidden_raw".into(), self.tick_hidden_raw),
            Column::new("last_seen_tick".into(), self.last_seen_tick),
            Column::new("last_seen_x".into(), self.last_seen_x),
            Column::new("last_seen_y".into(), self.last_seen_y),
        ])
        .map_err(|error| {
            InvalidDemoError::new_err(format!("Failed to create minimap DataFrame: {error}"))
        })
    }
}

#[pymethods]
impl Demo {
    /// Recorded enemy hero icons on each team's minimap.
    ///
    /// `observer_team` is the shared team information channel. Coordinates are
    /// native 0..255 map bytes and are populated only while the icon is shown.
    /// Position samples use every tenth source tick plus immediate visibility
    /// and identity transitions. `last_seen_tick` is the actual sample tick.
    /// Last-seen coordinates come only from a prior shown row with the same
    /// pawn entity identity; unresolved identities keep those fields null.
    /// `target_steam_id` requires one active controller bound to that pawn.
    /// Pass `ticks` to sample exact replay ticks, or `every` for a tick stride.
    #[pyo3(signature = (*, ticks=None, every=None))]
    pub(crate) fn enemy_minimap_ticks(
        &self,
        py: Python<'_>,
        ticks: Option<Vec<i32>>,
        every: Option<i32>,
    ) -> PyResult<PyDataFrame> {
        if every.is_some_and(|step| step <= 0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "every must be positive",
            ));
        }
        py.detach(|| {
            let selected = ticks.map(|items| items.into_iter().collect::<HashSet<_>>());
            let init = self.parser.parse_init().map_err(to_py_err)?;
            let hero_key = init.serializers().get("CCitadelPlayerPawn").and_then(|s| {
                s.resolve_field_key("m_CCitadelHeroComponent.m_spawnedHero.m_nHeroID")
            });
            let pawn_team_key = init
                .serializers()
                .get("CCitadelPlayerPawn")
                .and_then(|s| s.resolve_field_key("m_iTeamNum"));
            let controller_keys = ControllerKeys::resolve(&init);
            let filter = HashSet::from([
                "CCitadelTeam",
                "CCitadelPlayerPawn",
                "CCitadelPlayerController",
            ]);
            let mut cols = MinimapCols::default();
            let mut last_sample: Option<i32> = None;
            self.parser
                .run_to_end_filtered(&filter, |ctx| {
                    let tick = ctx.tick();
                    let selected_tick = selected.as_ref().is_none_or(|set| set.contains(&tick));
                    let stride_tick =
                        every.is_none_or(|step| last_sample.is_none_or(|last| tick - last >= step));
                    let emit = selected_tick && stride_tick;
                    if emit && every.is_some() {
                        last_sample = Some(tick);
                    }
                    cols.collect(ctx, hero_key, pawn_team_key, controller_keys, emit);
                })
                .map_err(to_py_err)?;
            cols.into_frame().map(PyDataFrame)
        })
    }
}

impl Demo {
    pub(super) fn spool_minimap(&self, sink:&mut crate::bounded_spool::Spool, ticks:&[i32])->PyResult<()> {
        let selected:HashSet<i32>=ticks.iter().copied().collect();
        let init=self.parser.parse_init().map_err(to_py_err)?;
        let hero_key=init.serializers().get("CCitadelPlayerPawn").and_then(|s|s.resolve_field_key("m_CCitadelHeroComponent.m_spawnedHero.m_nHeroID"));
        let pawn_team_key=init.serializers().get("CCitadelPlayerPawn").and_then(|s|s.resolve_field_key("m_iTeamNum"));
        let controller_keys=ControllerKeys::resolve(&init);
        drop(init);
        let filter=HashSet::from(["CCitadelTeam","CCitadelPlayerPawn","CCitadelPlayerController"]);
        let disk=rusqlite::Connection::open(sink.root().join("minimap-history.sqlite3")).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
        disk.execute_batch("PRAGMA cache_size=-1024; PRAGMA journal_mode=OFF; CREATE TABLE last_seen(team INTEGER,entity INTEGER,serial INTEGER,tick INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(team,entity,serial)) WITHOUT ROWID; BEGIN").map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
        let mut cols=MinimapCols {last_seen_disk:Some(disk),..Default::default()};
        let mut error=None;
        self.parser.try_run_to_end_filtered(&filter,|ctx| {
            if error.is_some() {return Err(crate::bounded_spool::aborted());}
            if let Err(e)=sink.check_cancel() {error=Some(e);return Err(crate::bounded_spool::aborted());}
            cols.collect(ctx,hero_key,pawn_team_key,controller_keys,selected.contains(&ctx.tick()));
            if cols.tick.len()>=1024 {
                // Only output columns transfer; the source-observed last-seen
                // state must survive every batch boundary.
                let remembered=std::mem::take(&mut cols.last_seen);
                let disk=cols.last_seen_disk.take();
                let map_samples=std::mem::take(&mut cols.map_samples);
                let output=std::mem::replace(&mut cols,MinimapCols {last_seen:remembered,last_seen_disk:disk,map_samples,..Default::default()});
                error=output.into_frame().and_then(|frame|sink.write("enemy_minimap_ticks",frame)).err();
            }
            if error.is_some() {Err(crate::bounded_spool::aborted())} else {Ok(())}
        }).map_err(|e|error.take().unwrap_or_else(||to_py_err(e)))?;
        if let Some(error)=error {return Err(error);}
        sink.write("enemy_minimap_ticks",cols.into_frame()?)
    }
}


#[cfg(test)]
mod movement_sample_tests {
    use super::*;
    #[test]
    fn minimap_same_schema_uses_causal_source_sample_ticks() {
        let mut s=MinimapSlot::default();let id=(148,3,Some((74,2)));
        assert_eq!(s.observe(id,Some(true),Some((1,2)),3),(Some((3,1,2)),true));
        assert_eq!(s.observe(id,Some(true),Some((9,9)),9),(Some((3,1,2)),false));
        assert_eq!(s.observe(id,Some(true),Some((3,4)),10),(Some((10,3,4)),true));
        assert_eq!(s.observe(id,Some(false),None,11),(Some((10,3,4)),false));
        assert_eq!(s.observe(id,Some(false),None,20),(Some((10,3,4)),false));
        assert_eq!(s.observe(id,Some(true),Some((5,6)),21),(Some((21,5,6)),true));
    }
    #[test]
    fn minimap_slot_reuse_and_serial_changes_do_not_reuse_coordinates() {
        let mut s=MinimapSlot::default();let id=(148,3,Some((74,2)));
        s.observe(id,Some(true),Some((1,2)),3);
        let newer=(148,3,Some((74,3)));
        assert_eq!(s.observe(newer,Some(false),None,4),(None,false));
        assert_eq!(s.observe(newer,Some(true),Some((5,6)),5),(Some((5,5,6)),true));
        assert_eq!(s.observe(newer,Some(true),None,6),(None,false));
        assert_eq!(s.observe(newer,Some(true),Some((7,8)),7),(Some((7,7,8)),true));
        assert_eq!(s.observe(newer,Some(true),Some((9,10)),10),(Some((10,9,10)),true));
        assert_eq!(s.observe(newer,Some(true),Some((11,12)),10),(Some((10,11,12)),true));
    }
}
