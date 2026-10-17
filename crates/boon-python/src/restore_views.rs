//! Complete team-known entity projection over bounded evidence blocks.
//! This ports the existing visibility policy, not gameplay recognition rules.
use flate2::{Compression, write::ZlibEncoder};
use indexmap::IndexMap;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use serde::Serialize;
use serde_json::{Map as M, Value as V, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::Write,
    sync::{
        Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};
type Id = (i64, i64);
type Rows = IndexMap<i64, M<String, V>>;
fn obj() -> M<String, V> {
    M::new()
}
fn get<'a>(m: &'a M<String, V>, k: &str) -> &'a V {
    m.get(k).unwrap_or(&V::Null)
}
fn int(v: &V) -> Option<i128> {
    v.as_i64()
        .map(i128::from)
        .or_else(|| v.as_u64().map(i128::from))
}
fn num(v: &V) -> bool {
    v.as_f64().is_some_and(f64::is_finite)
}
fn text(v: &V) -> &str {
    v.as_str().unwrap_or("")
}
fn display(v: &V) -> String {
    match v {
        V::Null => "None".into(),
        V::Bool(true) => "True".into(),
        V::Bool(false) => "False".into(),
        V::String(s) => s.clone(),
        V::Array(a) => format!("[{}]", a.iter().map(display).collect::<Vec<_>>().join(", ")),
        _ => v.to_string(),
    }
}
fn eq(a: &V, b: &V) -> bool {
    if a == b {
        return true;
    }
    let number = |v: &V| {
        v.as_f64()
            .or_else(|| v.as_bool().map(|b| if b { 1. } else { 0. }))
    };
    match (number(a), number(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}
fn key(id: Id) -> String {
    format!("entity:{}", i128::from(id.0) + (i128::from(id.1) << 14))
}
fn normal(s: &str) -> String {
    s.split('.')
        .enumerate()
        .map(|(i, p)| {
            if i > 0 && p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty() {
                let q = p.trim_start_matches('0');
                if q.is_empty() { "0" } else { q }
            } else {
                p
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}
fn decoded(f: &V) -> V {
    let d = &f["data"];
    let v = &d["value"];
    match text(&d["type"]) {
        "i64" | "u64" => {
            if let Some(s) = v.as_str() {
                if let Ok(n) = s.trim().parse::<i64>() {
                    return json!(n);
                }
                if let Ok(n) = s.trim().parse::<u64>() {
                    return json!(n);
                }
            }
            if let Some(n) = int(v) {
                return if n < 0 {
                    json!(n as i64)
                } else {
                    json!(n as u64)
                };
            }
            if let Some(n) = v
                .as_f64()
                .filter(|n| n.is_finite() && *n >= i64::MIN as f64 && *n < u64::MAX as f64)
            {
                return if n < 0. {
                    json!(n.trunc() as i64)
                } else {
                    json!(n.trunc() as u64)
                };
            }
            if let Some(b) = v.as_bool() {
                return json!(i32::from(b));
            }
            V::Null
        }
        "bytes" => {
            let Some(a) = v.as_array() else {
                return V::Null;
            };
            let Some(bytes) = a
                .iter()
                .map(|n| n.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect::<Option<Vec<_>>>()
            else {
                return V::Null;
            };
            String::from_utf8(bytes)
                .map(|s| json!(s.trim_end_matches('\0')))
                .unwrap_or(V::Null)
        }
        _ => v.clone(),
    }
}
fn arrays(f: &M<String, V>, prefix: &str) -> Rows {
    let mut out = Rows::new();
    let prefix = format!("{prefix}.");
    for (k, v) in f {
        if let Some(t) = k.strip_prefix(&prefix) {
            if let Some((slot, tail)) = t.split_once('.') {
                if !slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()) {
                    if let Ok(i) = slot.parse::<i64>() {
                        out.entry(i).or_default().insert(tail.into(), v.clone());
                    }
                }
            }
        }
    }
    out
}
fn position(f: &M<String, V>) -> M<String, V> {
    for base in [
        "CBodyComponent.m_skeletonInstance.m_vecOrigin.",
        "CBodyComponent.",
    ] {
        let cells = ["X", "Y", "Z"].map(|a| get(f, &format!("{base}m_cell{a}")));
        let offsets = ["X", "Y", "Z"].map(|a| get(f, &format!("{base}m_vec{a}")));
        if cells.iter().chain(offsets.iter()).all(|v| num(v)) {
            let mut p = obj();
            for (i, a) in ["x", "y", "z"].iter().enumerate() {
                let v = match (int(cells[i]), int(offsets[i])) {
                    (Some(c), Some(o)) => {
                        let n = c * 512 - 16384 + o;
                        if let Ok(n) = i64::try_from(n) {
                            json!(n)
                        } else {
                            json!(
                                cells[i].as_f64().unwrap() * 512. - 16384.
                                    + offsets[i].as_f64().unwrap()
                            )
                        }
                    }
                    _ => json!(
                        cells[i].as_f64().unwrap() * 512. - 16384. + offsets[i].as_f64().unwrap()
                    ),
                };
                p.insert(format!("world_{a}"), v);
            }
            return p;
        }
    }
    obj()
}
fn kind(class: &str) -> &'static str {
    match class {
        "CCitadelPlayerPawn" => "player",
        "CNPC_TrooperBoss" => "guardian",
        "CNPC_Boss_Tier2" => "walker",
        "CCitadel_KothCashIn" => "rift",
        "CNPC_Trooper" => "lane_creep",
        "CNPC_TrooperNeutral" | "CNPC_Neutral_Bug" => "neutral",
        "CCitadel_BreakableProp" => "breakable",
        "CNPC_Neutral_SinnersSacrifice" | "CNPC_Neutral_SinnersSacrifice_Hideout" => "sacrifice",
        "CItemXP" => "orb",
        "CCitadel_Pickup_Gold" | "CCitadel_Pickup_AssignedGold" => "pickup",
        "CNPC_MidBoss" => "midboss",
        "CNPC_Boss_Tier3" => "patron",
        "CNPC_BarrackBoss" => "barracks",
        "CCitadel_Destroyable_Building" => "shrine",
        "CCitadelZipLineNode" => "zipline",
        _ => "unknown",
    }
}
fn objective(class: &str) -> Option<&'static str> {
    Some(match class {
        "CNPC_TrooperBoss" => "CLASS_TROOPER_BOSS",
        "CNPC_Boss_Tier2" => "CLASS_BOSS_TIER_2",
        "CNPC_Boss_Tier3" => "CLASS_BOSS_TIER_3",
        "CNPC_BarrackBoss" => "CLASS_BOSS_BARRACKS",
        "CCitadel_Destroyable_Building" => "CLASS_DESTROYABLE_BUILDING",
        "CNPC_MidBoss" => "CLASS_MID_BOSS",
        _ => return None,
    })
}
fn title(s: &str) -> String {
    s.split('_')
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|a| a.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("_")
}
fn side(v: &V) -> &'static str {
    if eq(v, &json!(2)) {
        "Hidden King (Amber)"
    } else if eq(v, &json!(3)) {
        "Archmother (Sapphire)"
    } else {
        "Neutral"
    }
}
fn name(k: &str, team: &V, lane: &V) -> String {
    format!(
        "{} · {}{}",
        title(k),
        side(team),
        if int(lane).is_some() {
            format!(" · lane {}", display(lane))
        } else {
            String::new()
        }
    )
}
fn lane_facts(lane: &V) -> V {
    if let Some(n) = int(lane) {
        let mut m = obj();
        m.insert(format!("lane-code:{n}"), json!(1.));
        V::Object(m)
    } else {
        json!({})
    }
}
fn pubmut(n: &mut V) -> &mut M<String, V> {
    n["public"].as_object_mut().unwrap()
}
#[derive(Clone)]
struct Entity {
    class: String,
    active: bool,
    names: HashMap<String, String>,
    owners: HashMap<String, IndexMap<String, V>>,
    fields: M<String, V>,
}
impl Entity {
    fn remove(&mut self, k: &str) {
        if let Some(n) = self.names.remove(k) {
            if let Some(owners) = self.owners.get_mut(&n) {
                owners.shift_remove(k);
                if let Some((_, v)) = owners.last() {
                    self.fields.insert(n, v.clone());
                } else {
                    self.owners.remove(&n);
                    self.fields.shift_remove(&n);
                }
            }
        }
    }
}
#[derive(Default)]
struct State {
    entities: IndexMap<Id, Entity>,
    teams: HashMap<String, V>,
    heroes: M<String, V>,
    classes: M<String, V>,
    counts: IndexMap<String, u64>,
    last_seen: HashMap<(u8, Id), (f64, M<String, V>)>,
    observed: [IndexMap<String, (f64, V)>; 2],
    camp_memory: HashMap<String, (f64, M<String, V>)>,
    objectives: IndexMap<Id, (f64, V)>,
    markers: HashSet<Id>,
    pending: VecDeque<V>,
    frames: u64,
    peak: usize,
    coverage: BTreeMap<String, BTreeMap<String, BTreeMap<String, u64>>>,
}
impl State {
    fn count(&mut self, k: &str) {
        *self.counts.entry(k.into()).or_default() += 1;
    }
    fn classification(&self, v: &V) -> Option<&str> {
        int(v)
            .and_then(|n| self.classes.get(&n.to_string()))
            .and_then(V::as_str)
    }
    fn handle(&self, v: &V) -> Option<Id> {
        let h = int(v)?;
        if h < 0 {
            return None;
        }
        let id = ((h & 16383) as i64, i64::try_from(h >> 14).ok()?);
        self.entities.contains_key(&id).then_some(id)
    }
    fn apply(&mut self, row: &V) -> Result<(), String> {
        if row.get("movementPolicy").is_some() || row.get("movement_samples").is_some() {
            return Err("Sampled movement evidence requires the qualified backend or reference projector".into());
        }
        let id = (
            row["entity_index"].as_i64().ok_or("Invalid entity index")?,
            row["entity_serial"]
                .as_i64()
                .ok_or("Invalid entity serial")?,
        );
        if row["kind"] == "absent" {
            self.entities.shift_remove(&id);
            return Ok(());
        }
        if row["kind"] == "state" {
            self.entities.insert(
                id,
                Entity {
                    class: text(&row["entity_class"]).into(),
                    active: false,
                    names: HashMap::new(),
                    owners: HashMap::new(),
                    fields: obj(),
                },
            );
        }
        let e = self
            .entities
            .get_mut(&id)
            .ok_or("Entity delta without state")?;
        e.active = row["active"].as_bool().ok_or("Invalid activity")?;
        if let Some(a) = row["removed"].as_array() {
            for k in a {
                e.remove(text(k));
            }
        }
        let fields = row["fields"].as_object().ok_or("Invalid entity fields")?;
        for (k, f) in fields {
            let n = f["name"].as_str().map(normal);
            if e.names.get(k) != n.as_ref() {
                e.remove(k);
            }
            if let Some(n) = n {
                e.names.insert(k.clone(), n.clone());
                let owners = e.owners.entry(n.clone()).or_default();
                owners.insert(k.clone(), decoded(f));
                e.fields.insert(n, owners.last().unwrap().1.clone());
            }
        }
        Ok(())
    }
    fn at(&mut self, tick: i64, time: f64) -> [Vec<V>; 2] {
        let mut bindings = Vec::new();
        let mut count_ambiguous = 0;
        for (id, e) in &self.entities {
            if e.class != "CCitadelPlayerController" || !e.active {
                continue;
            }
            let f = &e.fields;
            let sid = display(get(f, "m_steamID"));
            let team = self.teams.get(&sid).cloned().unwrap_or(V::Null);
            if !eq(&team, &json!(2)) && !eq(&team, &json!(3)) {
                continue;
            }
            let mut candidates = HashSet::new();
            for field in ["m_hPawn", "m_hHeroPawn"] {
                if let Some(p) = self.handle(get(f, field)) {
                    if self.entities[&p].class == "CCitadelPlayerPawn" {
                        candidates.insert(p);
                    }
                }
            }
            if candidates.len() != 1 {
                count_ambiguous += 1;
                continue;
            }
            bindings.push((sid, team, *candidates.iter().next().unwrap(), *id));
        }
        for _ in 0..count_ambiguous {
            self.count("ambiguousPlayerPawn")
        }
        let mut sc = HashMap::new();
        let mut pc = HashMap::new();
        for (s, _, p, _) in &bindings {
            *sc.entry(s.clone()).or_insert(0) += 1;
            *pc.entry(*p).or_insert(0) += 1;
        }
        let mut players = IndexMap::<String, V>::new();
        let mut pawn_sid = IndexMap::<Id, String>::new();
        let mut controllers = IndexMap::new();
        let mut stat_entries = Vec::new();
        for (sid, team, pawn, id) in bindings {
            if sc[&sid] != 1 || pc[&pawn] != 1 {
                self.count("duplicatePlayerBinding");
                continue;
            }
            let f = &self.entities[&id].fields;
            let pf = &self.entities[&pawn].fields;
            let hero = get(pf, "m_CCitadelHeroComponent.m_spawnedHero.m_nHeroID");
            let mut public = obj();
            let mut private = obj();
            for (dest, source) in [
                ("gold_net_worth", "m_iGoldNetWorth"),
                ("level", "m_iLevel"),
                ("kills", "m_iPlayerKills"),
                ("deaths", "m_iDeaths"),
                ("assists", "m_iPlayerAssists"),
                ("is_alive", "m_bAlive"),
                ("respawn_source_time", "m_flRespawnTime"),
            ] {
                public.insert(
                    dest.into(),
                    get(f, &format!("m_PlayerDataGlobal.{source}")).clone(),
                );
            }
            for (dest, source) in [
                ("health_regen", "m_flHealthRegen"),
                ("ap_net_worth", "m_iAPNetWorth"),
                ("boon_count", "m_iBoonCount"),
                ("is_alive", "m_bAlive"),
                ("has_rebirth", "m_bHasRebirth"),
                ("has_rejuvenator", "m_bHasRejuvenator"),
                ("has_ultimate_trained", "m_bUltimateTrained"),
                ("ultimate_cooldown_start", "m_flUltimateCooldownStart"),
                ("ultimate_cooldown_end", "m_flUltimateCooldownEnd"),
                ("denies", "m_iDenies"),
                ("last_hits", "m_iLastHits"),
                ("hero_damage", "m_iHeroDamage"),
                ("objective_damage", "m_iObjectiveDamage"),
                ("hero_healing", "m_iHeroHealing"),
                ("self_healing", "m_iSelfHealing"),
                ("kill_streak", "m_iKillStreak"),
            ] {
                private.insert(
                    dest.into(),
                    get(f, &format!("m_PlayerDataGlobal.{source}")).clone(),
                );
            }
            for alias in [
                "m_PlayerDataGlobal.m_iBoonCount",
                "m_PlayerDataGlobal.m_nBoonCount",
                "m_iBoonCount",
                "m_nBoonCount",
            ] {
                if int(get(f, alias)).is_some() {
                    private.insert("boon_count".into(), get(f, alias).clone());
                    break;
                }
            }
            for (dest, source) in [
                ("health", "m_iHealth"),
                ("max_health", "m_iMaxHealth"),
                ("lifestate", "m_lifeState"),
                ("souls", "m_nCurrencies.m_nCurrencies"),
                ("spent_souls", "m_nSpentCurrencies.m_nSpentCurrencies"),
                ("respawn_source_time", "m_flRespawnTime"),
                ("death_source_time", "m_flDeathTime"),
                ("last_spawn_source_time", "m_flLastSpawnTime"),
                ("in_regen_zone", "m_bInRegenerationZone"),
                ("in_item_shop", "m_bInItemShopZone"),
            ] {
                private.insert(dest.into(), get(pf, source).clone());
            }
            private.extend(position(pf));
            for (slot, entry) in arrays(f, "m_PlayerDataGlobal.m_vecStatViewerModifierValues") {
                let code = get(&entry, "m_eValType");
                let amount = get(&entry, "m_flValue");
                if int(code).is_some() && num(amount) {
                    let modifier = get(&entry, "m_SourceModifierID");
                    let ability = get(&entry, "m_SourceAbilityID");
                    let mut facts = obj();
                    facts.insert(
                        format!("stat-contribution-type:{}", display(code)),
                        amount.clone(),
                    );
                    stat_entries.push(json!({"key":format!("player:{sid}:stat:{slot}:type:{}:modifier:{}:ability:{}",display(code),display(modifier),display(ability)),"kind":"upgrade","owner":sid,"public":{},"private":{},"privateFacts":facts,"entityClass":if int(modifier).is_some(){format!("stat-source:{}",display(modifier))}else{"stat-source-unknown".into()},"ability":if int(ability).is_some_and(|n|n!=0){json!(format!("id:{}",display(ability)))}else{V::Null},"sourceModifierId":modifier,"sourceAbilityId":ability,"statType":code,"statSlot":slot,"sourceTick":tick,"evidence":"recorded-stat-viewer-contribution"}));
                }
            }
            let hero_id = int(hero).filter(|h| *h > 0).map(|h| h.to_string());
            let hero_name = if let Some(s) = hero.as_str() {
                self.heroes.get(s.trim())
            } else {
                hero_id.as_ref().and_then(|h| self.heroes.get(h))
            }
            .filter(|v| {
                !text(v).to_lowercase().starts_with("hero_")
                    && !text(v).to_lowercase().starts_with("npc_")
            })
            .cloned()
            .unwrap_or(V::Null);
            players.insert(sid.clone(),json!({"key":format!("player:{sid}"),"kind":"player","owner":sid,"team":team,"heroId":hero_id,"heroName":hero_name,"entityClass":self.entities[&pawn].class,"public":public,"private":private,"privateFacts":{},"sourceTick":tick}));
            pawn_sid.insert(pawn, sid.clone());
            controllers.insert(sid, id);
        }
        let nodes = players
            .values()
            .cloned()
            .chain(stat_entries)
            .collect::<Vec<_>>();
        let mut result = [nodes.clone(), nodes];
        let mut by_index = HashMap::<i64, Vec<Id>>::new();
        let mut channels: [Vec<Id>; 2] = Default::default();
        for (id, e) in &self.entities {
            if e.active {
                by_index.entry(id.0).or_default().push(*id);
                if e.class == "CCitadelTeam" {
                    for (t, channel) in channels.iter_mut().enumerate() {
                        if eq(get(&e.fields, "m_iTeamNum"), &json!(t + 2)) {
                            channel.push(*id);
                        }
                    }
                }
            }
        }
        let mut camp_rows = IndexMap::<String, Vec<(String, M<String, V>)>>::new();
        let mut qualified = IndexMap::<u8, (i128, Rows)>::new();
        let mut objective_rows = HashMap::<Id, Vec<M<String, V>>>::new();
        for (t, channel) in channels.iter().enumerate() {
            let team = (t + 2) as u8;
            if channel.len() != 1 {
                self.count("missingOrAmbiguousTeamChannel");
                continue;
            }
            let f = &self.entities[&channel[0]].fields;
            let Some(count) = int(get(f, "m_vecFOWEntities")).filter(|n| (0..=255).contains(n))
            else {
                continue;
            };
            let entries: Rows = arrays(f, "m_vecFOWEntities")
                .into_iter()
                .filter(|(i, _)| i128::from(*i) < count)
                .collect();
            let mut divisors = Vec::new();
            for d in [1, 2] {
                let mut anchors = 0;
                let mut valid = true;
                for row in entries.values() {
                    if !eq(get(row, "m_eClass"), &json!(1)) {
                        continue;
                    }
                    let Some(raw) = int(get(row, "m_nEntIndex")).filter(|r| *r >= 0 && r % d == 0)
                    else {
                        valid = false;
                        break;
                    };
                    let matches = pawn_sid
                        .iter()
                        .filter(|(p, s)| {
                            i128::from(p.0) == raw / d
                                && eq(&players[*s]["team"], get(row, "m_nTeam"))
                        })
                        .count();
                    if matches > 1 {
                        valid = false;
                        break;
                    }
                    anchors += matches;
                }
                if valid && anchors > 0 {
                    divisors.push(d);
                }
            }
            if divisors.len() != 1 {
                self.count("unqualifiedFowIndexEncoding");
                continue;
            }
            let d = divisors[0];
            let mut multiplicity = HashMap::<i128, usize>::new();
            for row in entries.values() {
                if let Some(raw) = int(get(row, "m_nEntIndex")) {
                    *multiplicity.entry(raw).or_default() += 1;
                }
            }
            if entries.len() == count as usize
                && (0..count as i64).all(|i| entries.contains_key(&i))
                && entries.values().all(|r| {
                    int(get(r, "m_nEntIndex")).is_some_and(|raw| {
                        raw >= 0
                            && ((raw % d == 0 && multiplicity.get(&raw) == Some(&1))
                                || (d == 2
                                    && raw == 1
                                    && self.classification(get(r, "m_eClass")).is_some()))
                    }) && int(get(r, "m_eClass")).is_some()
                        && int(get(r, "m_nTeam")).is_some()
                        && get(r, "m_bVisibleOnMap").is_boolean()
                })
            {
                qualified.insert(team, (d, entries.clone()));
            }
            let mut seen = HashSet::new();
            for row in entries.values() {
                let Some(raw) = int(get(row, "m_nEntIndex"))
                    .filter(|r| *r >= 0 && r % d == 0 && multiplicity.get(r) == Some(&1))
                else {
                    continue;
                };
                let classification = self.classification(get(row, "m_eClass")).map(str::to_owned);
                if classification.as_deref().is_some_and(|c| {
                    [
                        "CLASS_WEAK_NEUTRAL_CAMP",
                        "CLASS_MEDIUM_NEUTRAL_CAMP",
                        "CLASS_STRONG_NEUTRAL_CAMP",
                        "CLASS_VAULT_NEUTRAL_CAMP",
                        "CLASS_SUPER_NEUTRAL_CAMP",
                    ]
                    .contains(&c)
                }) {
                    let c = classification.unwrap();
                    camp_rows
                        .entry(format!("minimap-camp:{}:{c}", raw / d))
                        .or_default()
                        .push((c, row.clone()));
                    continue;
                }
                let Some(candidates) = i64::try_from(raw / d)
                    .ok()
                    .and_then(|i| by_index.get(&i))
                    .filter(|ids| ids.len() == 1)
                else {
                    continue;
                };
                let id = candidates[0];
                let e = &self.entities[&id];
                if e.class == "CInfoTrooperNeutralCamp" {
                    self.count("unqualifiedCampClass");
                    continue;
                }
                let expected = match e.class.as_str() {
                    "CCitadelPlayerPawn" => Some(1),
                    "CNPC_TrooperBoss" => Some(5),
                    "CNPC_Boss_Tier2" => Some(29),
                    "CCitadel_KothCashIn" => Some(92),
                    _ => None,
                };
                if expected.is_some_and(|c| !eq(get(row, "m_eClass"), &json!(c))) {
                    self.count("fowClassMismatch");
                    continue;
                }
                let sid = pawn_sid.get(&id);
                let entity_team = sid
                    .map(|s| players[s]["team"].clone())
                    .unwrap_or_else(|| get(&e.fields, "m_iTeamNum").clone());
                if !eq(&entity_team, get(row, "m_nTeam")) {
                    continue;
                }
                if !seen.insert(id) {
                    self.count("duplicateFowIdentity");
                    continue;
                }
                if let Some(expected_class) = objective(&e.class) {
                    if classification.as_deref() == Some(expected_class)
                        || expected.is_some_and(|c| eq(get(row, "m_eClass"), &json!(c)))
                    {
                        objective_rows.entry(id).or_default().push(row.clone());
                        if get(row, "m_bVisibleOnMap") == &V::Bool(true) {
                            self.markers.insert(id);
                        }
                    }
                }
                let shown = get(row, "m_bVisibleOnMap");
                let memory = (team, id);
                let vals = if shown == &V::Bool(true) {
                    let mut vals = obj();
                    vals.insert("visible".into(), json!(true));
                    let x = get(row, "m_nPositionX");
                    let y = get(row, "m_nPositionY");
                    if [x, y]
                        .iter()
                        .all(|v| int(v).is_some_and(|n| (0..=255).contains(&n)))
                    {
                        vals.insert("map_x".into(), x.clone());
                        vals.insert("map_y".into(), y.clone());
                        vals.insert("map_age".into(), json!(0.));
                    }
                    let hp = get(row, "m_nHealthPercent");
                    if int(hp).is_some_and(|n| (0..=255).contains(&n)) {
                        vals.insert("health_byte".into(), hp.clone());
                        vals.insert("health_age".into(), json!(0.));
                    }
                    if get(row, "m_bBackdoorProtectionActive").is_boolean() {
                        vals.insert(
                            "protected".into(),
                            get(row, "m_bBackdoorProtectionActive").clone(),
                        );
                    }
                    self.last_seen.insert(memory, (time, vals.clone()));
                    vals
                } else if let Some((at, old)) = self.last_seen.get(&memory) {
                    let mut vals = old.clone();
                    vals.shift_remove("visible");
                    vals.shift_remove("protected");
                    if vals.contains_key("map_x") {
                        vals.insert("map_age".into(), json!(time - at));
                    }
                    if vals.contains_key("health_byte") {
                        vals.insert("health_age".into(), json!(time - at));
                    }
                    if shown.is_boolean() {
                        vals.insert("visible".into(), shown.clone());
                    }
                    vals
                } else {
                    continue;
                };
                if let Some(sid) = sid {
                    let target = result[t]
                        .iter_mut()
                        .find(|n| n["key"] == format!("player:{sid}"))
                        .unwrap();
                    pubmut(target).extend(vals);
                } else {
                    let lane = get(row, "m_iLane");
                    let k = kind(&e.class);
                    let display_name = get(row, "m_strCSSClass");
                    let display_name = if display_name.is_null() || display_name == "" {
                        json!(name(k, &entity_team, lane))
                    } else {
                        display_name.clone()
                    };
                    result[t].push(json!({"key":key(id),"kind":k,"entityClass":e.class,"team":entity_team,"public":vals,"private":{},"displayName":display_name,"lane":lane,"publicFacts":lane_facts(lane),"sourceTick":tick,"evidence":"team-minimap"}));
                }
            }
        }
        for (identity, entries) in camp_rows {
            let classes: HashSet<_> = entries.iter().map(|(c, _)| c).collect();
            let flags: HashSet<_> = entries
                .iter()
                .filter_map(|(_, r)| get(r, "m_bVisibleOnMap").as_bool())
                .collect();
            if classes.len() != 1 || flags.len() != 1 {
                self.count("conflictingPublicCampObservations");
                continue;
            }
            let available = *flags.iter().next().unwrap();
            let mut pos = obj();
            if available {
                let mut positions = Vec::new();
                for (_, r) in &entries {
                    if get(r, "m_bVisibleOnMap") == &json!(true) {
                        let xy = (
                            get(r, "m_nPositionX").clone(),
                            get(r, "m_nPositionY").clone(),
                        );
                        if !positions.contains(&xy) {
                            positions.push(xy);
                        }
                    }
                }
                if positions.len() == 1 {
                    let (x, y) = &positions[0];
                    if [x, y]
                        .iter()
                        .all(|v| int(v).is_some_and(|n| (0..=255).contains(&n)))
                    {
                        pos.insert("map_x".into(), x.clone());
                        pos.insert("map_y".into(), y.clone());
                        pos.insert("map_age".into(), json!(0.));
                        self.camp_memory
                            .insert(identity.clone(), (time, pos.clone()));
                    }
                }
            }
            if pos.is_empty() {
                if let Some((at, old)) = self.camp_memory.get(&identity) {
                    pos = old.clone();
                    pos.insert("map_age".into(), json!(time - at));
                }
            }
            for nodes in &mut result {
                nodes.push(json!({"key":identity,"kind":"neutral","entityClass":"camp-minimap-marker","public":pos,"private":{},"publicFacts":{"camp_available":available},"sourceTick":tick,"displayName":entries[0].0,"evidence":"public-camp-minimap","identityScope":"minimap-marker, not an individual creature or entity serial","campTotalSoulValue":null,"campValueStatus":"No qualified minimap total in this source row"}));
            }
        }
        for (t, nodes) in result.iter_mut().enumerate() {
            for (id, sid) in &pawn_sid {
                let Some((at, old)) = self.last_seen.get(&((t + 2) as u8, *id)) else {
                    continue;
                };
                let Some(n) = nodes
                    .iter_mut()
                    .find(|n| n["key"] == format!("player:{sid}"))
                else {
                    continue;
                };
                let p = pubmut(n);
                if p.contains_key("map_age") {
                    continue;
                }
                let mut memory = old.clone();
                memory.shift_remove("visible");
                memory.shift_remove("protected");
                for field in ["map_age", "health_age"] {
                    if memory.contains_key(field) {
                        memory.insert(field.into(), json!(time - at));
                    }
                }
                p.extend(memory);
            }
        }
        for (t, nodes) in result.iter_mut().enumerate() {
            let current: HashSet<_> = nodes.iter().map(|n| text(&n["key"]).to_owned()).collect();
            for n in nodes.iter() {
                if n["evidence"] == "team-minimap" {
                    self.observed[t].insert(text(&n["key"]).into(), (time, n.clone()));
                }
            }
            for (id, (at, old)) in &self.observed[t] {
                if current.contains(id) {
                    continue;
                }
                let mut n = old.clone();
                let p = pubmut(&mut n);
                p.shift_remove("visible");
                p.shift_remove("protected");
                for field in ["map_age", "health_age"] {
                    if let Some(v) = p.get(field).and_then(V::as_f64) {
                        p.insert(field.into(), json!(v + (time - at)));
                    }
                }
                nodes.push(n);
            }
        }
        self.public_objectives(&mut result, tick, time, &qualified, &objective_rows);
        self.public_ziplines(&mut result, tick);
        for (owner, id) in controllers {
            if !players.contains_key(&owner) {
                continue;
            }
            let f = &self.entities[&id].fields;
            let prefix = "m_PlayerDataGlobal.m_vecUpgrades";
            let Some(count) = int(get(f, prefix)).filter(|n| (0..=64).contains(n)) else {
                self.count("unknownInventory");
                continue;
            };
            let ap: HashMap<_, _> = arrays(f, "m_PlayerDataGlobal.m_vecAbilityUpgradeState")
                .values()
                .filter_map(|e| {
                    int(get(e, "m_ItemID")).map(|id| (id, get(e, "m_nUpgradeInfo").clone()))
                })
                .collect();
            let mut unknown = 0;
            for slot in 0..count {
                let token = get(f, &format!("{prefix}.{slot}"));
                let Some(token) = int(token).filter(|n| *n >= 0) else {
                    unknown += 1;
                    continue;
                };
                let mut facts = obj();
                if let Some(packed) = ap.get(&token).and_then(int).filter(|n| *n >= 0) {
                    for bit in 0..15 {
                        facts.insert(
                            format!("upgrade_bit:{bit}"),
                            json!(if ((packed >> 17) & (1 << bit)) != 0 {
                                1.
                            } else {
                                0.
                            }),
                        );
                    }
                }
                for nodes in &mut result {
                    nodes.push(json!({"key":format!("player:{owner}:upgrade:{slot}"),"kind":"upgrade","owner":owner,"ability":format!("id:{token}"),"publicInventory":true,"public":{},"private":{},"privateFacts":facts,"sourceTick":tick,"evidence":"recorded-current-upgrade-vector"}));
                }
            }
            for _ in 0..unknown {
                self.count("unknownInventoryEntry")
            }
        }
        let rules: Vec<_> = self
            .entities
            .iter()
            .filter(|(_, e)| e.active && e.class == "CCitadelGameRulesProxy")
            .map(|(id, _)| *id)
            .collect();
        if rules.len() == 1 {
            let f = &self.entities[&rules[0]].fields;
            let base = "m_pGameRules.";
            let names = [
                "m_vecNeutralCampTimerOrigins",
                "m_vecNeutralCampNextSpawnTimes",
                "m_vecNeutralCampTimerIsMidBoss",
            ];
            let lengths = names.map(|n| int(get(f, &format!("{base}{n}"))));
            if lengths
                .iter()
                .all(|n| n.is_some_and(|n| (0..=256).contains(&n)))
                && lengths[0] == lengths[1]
                && lengths[0] == lengths[2]
            {
                let mut incomplete = 0;
                for slot in 0..lengths[0].unwrap() {
                    let values = names.map(|n| get(f, &format!("{base}{n}.{slot}")));
                    let position = values[0];
                    let spawn = values[1];
                    let midboss = values[2];
                    let Some(p) = position
                        .as_array()
                        .filter(|a| a.len() == 3 && a.iter().all(num))
                    else {
                        incomplete += 1;
                        continue;
                    };
                    if !num(spawn) || !midboss.is_boolean() {
                        incomplete += 1;
                        continue;
                    }
                    let mut public = obj();
                    for (i, a) in ["x", "y", "z"].iter().enumerate() {
                        public.insert(format!("world_{a}"), p[i].clone());
                    }
                    for nodes in &mut result {
                        nodes.push(json!({"key":format!("camp-timer:{slot}:{}",display(position)),"kind":"neutral","entityClass":"camp-timer-record","public":public,"private":{},"publicFacts":{"camp_next_spawn_source_clock":spawn,"camp_timer_is_midboss":midboss},"sourceTick":tick,"evidence":"public-camp-timer","identityScope":"timer array entry and recorded origin; no inferred creature membership"}));
                    }
                }
                for _ in 0..incomplete {
                    self.count("incompletePublicCampTimer")
                }
            }
        }
        let rifts: Vec<_> = self
            .entities
            .iter()
            .filter(|(_, e)| e.active && e.class == "CCitadel_KothCashIn")
            .map(|(id, _)| *id)
            .collect();
        if rules.len() == 1 && rifts.len() == 1 {
            let f = &self.entities[&rules[0]].fields;
            let start = get(f, "m_pGameRules.m_timeKothCashInStarted");
            if num(start) && start.as_f64().unwrap() > 0. {
                let mut values = obj();
                values.insert("capture_active".into(), json!(true));
                for (side, e) in arrays(f, "m_pGameRules.m_vecTeamKothStates") {
                    let frac = get(&e, "m_flCaptureProgressFrac");
                    if (side == 0 || side == 1)
                        && num(frac)
                        && (0.0..=1.0).contains(&frac.as_f64().unwrap())
                    {
                        values.insert(format!("capture_fraction_state{side}"), frac.clone());
                    }
                }
                for nodes in &mut result {
                    if let Some(n) = nodes.iter_mut().find(|n| n["key"] == key(rifts[0])) {
                        pubmut(n).extend(values.clone());
                    } else {
                        nodes.push(json!({"key":key(rifts[0]),"kind":"rift","public":values,"private":{},"sourceTick":tick,"evidence":"current-public-rift-rules"}));
                    }
                }
            }
        }
        for (id, e) in &self.entities {
            if !e.active {
                continue;
            }
            let f = &e.fields;
            let Some(owner) = self
                .handle(get(f, "m_hOwnerEntity"))
                .and_then(|p| pawn_sid.get(&p))
            else {
                continue;
            };
            let Some(token) = int(get(f, "m_nSubclassID")) else {
                continue;
            };
            if !e.class.contains("Ability") && !e.class.contains("Item") {
                continue;
            }
            let mut private = obj();
            for (source, dest) in [
                ("m_flCooldownEnd", "cooldown_source_time"),
                ("m_nAbilityUpgradeBits", "upgrade_bits"),
                ("m_nCharges", "charges"),
                ("m_iClip1", "ammo"),
            ] {
                let v = get(f, source);
                if num(v) {
                    private.insert(dest.into(), v.clone());
                }
            }
            let extras: M<String, V> = f
                .iter()
                .filter(|(n, v)| n.starts_with("m_nUpgradeInfo.") && num(v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for nodes in &mut result {
                nodes.push(json!({"key":key(*id),"kind":"upgrade","owner":owner,"entityClass":e.class,"ability":format!("id:{token}"),"public":{},"private":private,"privateFacts":extras,"sourceTick":tick,"evidence":"own-ability-entity"}));
            }
        }
        result
    }
    fn public_objectives(
        &mut self,
        result: &mut [Vec<V>; 2],
        tick: i64,
        time: f64,
        channels: &IndexMap<u8, (i128, Rows)>,
        markers: &HashMap<Id, Vec<M<String, V>>>,
    ) {
        for (id, e) in &self.entities {
            if objective(&e.class).is_none() || !e.active {
                continue;
            }
            let f = &e.fields;
            let old = self.objectives.get(id);
            let mut public = position(f);
            for (source, dest) in [
                ("m_iHealth", "health"),
                ("m_iMaxHealth", "max_health"),
                ("m_lifeState", "lifestate"),
            ] {
                let v = get(f, source);
                if num(v) {
                    public.insert(dest.into(), v.clone());
                }
            }
            let hp = get(&public, "health");
            let life = get(&public, "lifestate");
            let alive = if num(hp) {
                Some(hp.as_f64().unwrap() > 0. && (life.is_null() || eq(life, &json!(0))))
            } else {
                int(life).map(|l| l == 0)
            };
            if let Some(alive) = alive {
                public.insert("is_alive".into(), json!(alive));
            }
            public.insert("health_age".into(), json!(0.));
            let k = kind(&e.class);
            let lane = get(f, "m_iLane");
            let team = get(f, "m_iTeamNum");
            let mut n = json!({"key":key(*id),"kind":k,"entityClass":e.class,"team":team,"public":public,"private":{},"sourceTick":tick,"lane":lane,"displayName":name(k,team,lane),"publicFacts":lane_facts(lane),"evidence":"public-objective-state","lifeEvidence":"recorded-objective-state"});
            if let Some((_, old)) = old {
                n["displayName"] = old["displayName"].clone();
                if old["lifeEvidence"] == "missing-objective-marker" {
                    let p = pubmut(&mut n);
                    p.shift_remove("health");
                    p.shift_remove("lifestate");
                    p.insert("is_alive".into(), json!(false));
                    n["lifeEvidence"] = json!("missing-objective-marker");
                }
            }
            self.objectives.insert(*id, (time, n));
        }
        let ids: Vec<_> = self.objectives.keys().copied().collect();
        for id in ids {
            let (at, saved) = &self.objectives[&id];
            let at = *at;
            let mut n = saved.clone();
            if let Some(rows) = markers.get(&id) {
                for (source, dest) in [
                    ("m_bBackdoorProtectionActive", "protected"),
                    ("m_nHealthPercent", "health_byte"),
                ] {
                    let mut values = Vec::<V>::new();
                    for r in rows {
                        if let Some(v) = r.get(source) {
                            if !values.iter().any(|x| eq(x, v)) {
                                values.push(v.clone());
                            }
                        }
                    }
                    if values.len() == 1 {
                        pubmut(&mut n).insert(dest.into(), values[0].clone());
                    }
                }
                let mut names = Vec::new();
                for r in rows {
                    if let Some(s) = get(r, "m_strCSSClass").as_str().filter(|s| !s.is_empty()) {
                        if !names.contains(&s) {
                            names.push(s);
                        }
                    }
                }
                if names.len() == 1 {
                    n["displayName"] = json!(names[0]);
                }
            }
            let mut statuses = Vec::new();
            if self.markers.contains(&id) && channels.len() == 2 {
                for (d, entries) in channels.values() {
                    let matches: Vec<_> = entries
                        .values()
                        .filter(|r| {
                            int(get(r, "m_nEntIndex"))
                                .is_some_and(|raw| raw / d == i128::from(id.0))
                        })
                        .collect();
                    if matches.is_empty() {
                        statuses.push(Some(false));
                        continue;
                    }
                    let current: Vec<_> = self
                        .entities
                        .iter()
                        .filter(|(i, e)| i.0 == id.0 && e.active)
                        .map(|(i, _)| *i)
                        .collect();
                    if current != vec![id] {
                        statuses.push(None);
                        continue;
                    }
                    let row = matches[0];
                    if self.classification(get(row, "m_eClass"))
                        != objective(text(&n["entityClass"]))
                    {
                        statuses.push(None);
                        continue;
                    }
                    statuses.push(get(row, "m_bVisibleOnMap").as_bool());
                }
            }
            if statuses == [Some(false), Some(false)] {
                let p = pubmut(&mut n);
                p.insert("is_alive".into(), json!(false));
                p.insert("visible".into(), json!(false));
                for field in ["health", "lifestate", "health_byte", "protected"] {
                    p.shift_remove(field);
                }
                n["lifeEvidence"] = json!("missing-objective-marker");
            } else if statuses == [Some(true), Some(true)]
                && n["lifeEvidence"] == "missing-objective-marker"
            {
                if let Some(hp) = self
                    .entities
                    .get(&id)
                    .and_then(|e| e.fields.get("m_iHealth"))
                    .filter(|v| num(v))
                {
                    let p = pubmut(&mut n);
                    p.insert("health".into(), hp.clone());
                    p.insert("is_alive".into(), json!(hp.as_f64().unwrap() > 0.));
                    n["lifeEvidence"] = json!("recorded-objective-state");
                }
            }
            if at < time {
                pubmut(&mut n).insert("health_age".into(), json!(time - at));
            }
            self.objectives.insert(id, (at, n.clone()));
            for nodes in result.iter_mut() {
                if let Some(i) = nodes.iter().position(|x| x["key"] == n["key"]) {
                    for k in ["map_x", "map_y", "map_age"] {
                        if let Some(v) = nodes[i]["public"].get(k) {
                            pubmut(&mut n).entry(k).or_insert_with(|| v.clone());
                        }
                    }
                    nodes.remove(i);
                }
                nodes.push(n.clone());
            }
        }
    }
    fn public_ziplines(&self, result: &mut [Vec<V>; 2], tick: i64) {
        for (id, e) in &self.entities {
            if e.class != "CCitadelZipLineNode" || !e.active {
                continue;
            }
            let f = &e.fields;
            let pos = position(f);
            if pos.is_empty() {
                continue;
            }
            let owner = get(f, "m_iTeamNum");
            let lane = get(f, "m_iPrimaryLane");
            let mut facts = obj();
            for (source, dest) in [
                ("m_flCumulativeDistance", "zipline_route_distance"),
                ("m_iNodeIndex", "zipline_node_index"),
                ("m_bCapturable", "zipline_capturable"),
                ("m_bDisableZippingToByPlayers", "zipline_disabled"),
                ("m_eCaptureState", "zipline_capture_state"),
            ] {
                let v = get(f, source);
                if num(v) || v.is_boolean() {
                    facts.insert(dest.into(), v.clone());
                }
            }
            if let Some(n) = int(lane) {
                facts.insert(format!("lane-code:{n}"), json!(1.));
            }
            let count = int(get(f, "m_vecConnections"));
            let mut complete = count.is_some_and(|n| (0..=32).contains(&n));
            let mut connections = Vec::new();
            if complete {
                for slot in 0..count.unwrap() {
                    let other = self.handle(get(f, &format!("m_vecConnections.{slot}")));
                    if let Some(other) = other.filter(|i| {
                        self.entities[i].class == "CCitadelZipLineNode" && self.entities[i].active
                    }) {
                        connections.push(other);
                    } else {
                        complete = false
                    }
                }
            }
            if complete && (eq(owner, &json!(2)) || eq(owner, &json!(3))) {
                let eligible: Vec<_> = connections
                    .iter()
                    .map(|i| &self.entities[i].fields)
                    .filter(|f| eq(get(f, "m_iPrimaryLane"), lane))
                    .collect();
                if !eligible.is_empty()
                    && eligible.iter().all(|f| {
                        [0, 2, 3]
                            .iter()
                            .any(|n| eq(get(f, "m_iTeamNum"), &json!(n)))
                    })
                {
                    facts.insert(
                        "zipline_frontier".into(),
                        json!(eligible.iter().any(|f| !eq(get(f, "m_iTeamNum"), owner))),
                    );
                }
            }
            let n = json!({"key":key(*id),"kind":"zipline","team":owner,"entityClass":e.class,"public":pos,"private":{},"publicFacts":facts,"sourceTick":tick,"lane":lane,"connections":connections.into_iter().map(key).collect::<Vec<_>>(),"connectionsComplete":complete,"evidence":"public-zipline-ownership","displayName":format!("Zipline · team {} · lane {}",display(owner),display(lane))});
            for nodes in result.iter_mut() {
                nodes.push(n.clone());
            }
        }
    }
    fn record(&mut self, views: &[Vec<V>; 2]) {
        self.frames += 1;
        self.peak = self.peak.max(views.iter().map(Vec::len).max().unwrap_or(0));
        for (t, nodes) in views.iter().enumerate() {
            for n in nodes {
                let kind = text(&n["kind"]);
                if !self.coverage.contains_key(kind) {
                    self.coverage.insert(kind.to_owned(), BTreeMap::new());
                }
                let scopes = self.coverage.get_mut(kind).unwrap();
                for scope in ["public", "publicFacts", "private", "privateFacts"] {
                    if (scope == "private" || scope == "privateFacts") && t != 0 {
                        continue;
                    }
                    if let Some(fields) = n[scope].as_object() {
                        if !scopes.contains_key(scope) {
                            scopes.insert(scope.to_owned(), BTreeMap::new());
                        }
                        let counts = scopes.get_mut(scope).unwrap();
                        for (field, value) in fields {
                            if num(value) || value.is_boolean() {
                                if let Some(count) = counts.get_mut(field.as_str()) {
                                    *count += 1;
                                } else {
                                    counts.insert(field.clone(), 1);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
// Serialize existing ordered nodes directly instead of cloning a second JSON
// object graph just to put the two team arrays in their output envelope.
#[derive(Serialize)]
struct Teams<'a> {
    #[serde(rename = "2")]
    amber: &'a [V],
    #[serde(rename = "3")]
    sapphire: &'a [V],
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Timings {
    feed_seconds: f64,
    advance_seconds: f64,
    projection_seconds: f64,
    coverage_seconds: f64,
    encoding_seconds: f64,
    compression_seconds: f64,
    compression_wait_seconds: f64,
}
type Compressed = Result<(i64, Vec<u8>, f64), String>;
// Only one snapshot may be in flight. The coordinator can prepare the next
// frame while this worker compresses the previous one, without replay-sized
// queues, reordering observations, or multiplying the configured thread limit.
struct Compressor {
    input: Option<SyncSender<(i64, Vec<u8>)>>,
    output: Mutex<Receiver<Compressed>>,
    thread: Option<JoinHandle<()>>,
    pending: bool,
}
impl Compressor {
    fn new() -> Result<Self, String> {
        let (input, requests) = mpsc::sync_channel::<(i64, Vec<u8>)>(1);
        let (results, output) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("boon-view-compress".into())
            .spawn(move || {
                while let Ok((tick, bytes)) = requests.recv() {
                    let result =
                        compress(&bytes).map(|(payload, seconds)| (tick, payload, seconds));
                    if results.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            input: Some(input),
            output: Mutex::new(output),
            thread: Some(thread),
            pending: false,
        })
    }
    fn receive(&mut self) -> Compressed {
        self.pending = false;
        self.output
            .lock()
            .map_err(|e| e.to_string())?
            .recv()
            .map_err(|e| e.to_string())?
    }
}
impl Drop for Compressor {
    fn drop(&mut self) {
        self.input.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn compress(encoded: &[u8]) -> Result<(Vec<u8>, f64), String> {
    let start = Instant::now();
    let mut zip = ZlibEncoder::new(Vec::new(), Compression::default());
    zip.write_all(encoded).map_err(|e| e.to_string())?;
    let payload = zip.finish().map_err(|e| e.to_string())?;
    Ok((payload, start.elapsed().as_secs_f64()))
}
#[pyclass]
pub struct KnownViews {
    state: State,
    timings: Timings,
    parallel: bool,
    compressor: Option<Compressor>,
}
impl KnownViews {
    fn encode(&mut self, tick: i64, time: f64) -> Result<Vec<u8>, String> {
        let start = Instant::now();
        let views = self.state.at(tick, time);
        self.timings.projection_seconds += start.elapsed().as_secs_f64();
        let start = Instant::now();
        self.state.record(&views);
        self.timings.coverage_seconds += start.elapsed().as_secs_f64();
        let start = Instant::now();
        let encoded = serde_json::to_vec(&Teams {
            amber: &views[0],
            sapphire: &views[1],
        })
        .map_err(|e| e.to_string())?;
        self.timings.encoding_seconds += start.elapsed().as_secs_f64();
        Ok(encoded)
    }
    fn receive(&mut self) -> Result<Option<(i64, Vec<u8>)>, String> {
        let Some(worker) = self.compressor.as_mut().filter(|w| w.pending) else {
            return Ok(None);
        };
        let start = Instant::now();
        let (tick, payload, seconds) = worker.receive()?;
        self.timings.compression_wait_seconds += start.elapsed().as_secs_f64();
        self.timings.compression_seconds += seconds;
        Ok(Some((tick, payload)))
    }
}
fn error(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}
#[pymethods]
impl KnownViews {
    #[new]
    fn new(config: &str) -> PyResult<Self> {
        let config: V = serde_json::from_str(config).map_err(error)?;
        if config.get("movementPolicy").is_some() {
            return Err(error("Sampled movement evidence requires the qualified backend or reference projector"));
        }
        let mut state = State::default();
        state.teams = config["teams"]
            .as_object()
            .ok_or_else(|| error("Missing teams"))?
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        state.heroes = config["heroes"].as_object().cloned().unwrap_or_default();
        state.classes = config["classes"].as_object().cloned().unwrap_or_default();
        let parallel = config["threads"].as_u64().is_some_and(|n| n >= 2);
        Ok(Self {
            state,
            parallel,
            timings: Timings::default(),
            compressor: None,
        })
    }
    /// Supply the same exact-version catalog used by the reference projector.
    /// The production restore sets it once in the constructor.
    fn set_classes(&mut self, data: &str) -> PyResult<()> {
        self.state.classes = serde_json::from_str(data).map_err(error)?;
        Ok(())
    }
    /// One bounded, already validated/decompressed evidence block. Never a replay.
    fn feed(&mut self, py: Python<'_>, data: Vec<u8>) -> PyResult<()> {
        py.detach(|| {
            let start = Instant::now();
            if !self.state.pending.is_empty() {
                return Err(error(
                    "Consume pending evidence before feeding another block",
                ));
            }
            for line in data.split(|b| *b == b'\n').filter(|s| !s.is_empty()) {
                let row: V=serde_json::from_slice(line).map_err(error)?;
                if row.get("movementPolicy").is_some() || row.get("movement_samples").is_some() {
                    return Err(error("Sampled movement evidence requires the qualified backend or reference projector"));
                }
                self.state.pending.push_back(row);
            }
            self.timings.feed_seconds += start.elapsed().as_secs_f64();
            Ok(())
        })
    }
    /// False means a future record is held; True means another block may be read.
    fn advance(&mut self, py: Python<'_>, tick: i64) -> PyResult<bool> {
        py.detach(|| {
            let start = Instant::now();
            while self
                .state
                .pending
                .front()
                .is_some_and(|r| r["tick"].as_i64().is_some_and(|t| t <= tick))
            {
                let row = self.state.pending.pop_front().unwrap();
                self.state.apply(&row).map_err(error)?;
            }
            self.timings.advance_seconds += start.elapsed().as_secs_f64();
            Ok(self.state.pending.is_empty())
        })
    }
    fn apply_json(&mut self, py: Python<'_>, data: &str) -> PyResult<()> {
        py.detach(|| {
            let row = serde_json::from_str(data).map_err(error)?;
            self.state.apply(&row).map_err(error)
        })
    }
    fn at_json(&mut self, py: Python<'_>, tick: i64, time: f64) -> PyResult<String> {
        py.detach(|| {
            let [a, b] = self.state.at(tick, time);
            serde_json::to_string(&Teams {
                amber: &a,
                sapphire: &b,
            })
            .map_err(error)
        })
    }
    fn project<'py>(
        &mut self,
        py: Python<'py>,
        tick: i64,
        time: f64,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let payload = py.detach(|| -> PyResult<Vec<u8>> {
            let encoded = self.encode(tick, time).map_err(error)?;
            let (payload, seconds) = compress(&encoded).map_err(error)?;
            self.timings.compression_seconds += seconds;
            Ok(payload)
        })?;
        Ok(PyBytes::new(py, &payload))
    }
    /// Output is the previous completed frame when compression is overlapped.
    /// With one configured thread, it is the current frame synchronously.
    fn submit<'py>(
        &mut self,
        py: Python<'py>,
        tick: i64,
        time: f64,
    ) -> PyResult<Option<(i64, Bound<'py, PyBytes>)>> {
        let result = py
            .detach(|| -> Result<Option<(i64, Vec<u8>)>, String> {
                let encoded = self.encode(tick, time)?;
                if !self.parallel {
                    let (payload, seconds) = compress(&encoded)?;
                    self.timings.compression_seconds += seconds;
                    return Ok(Some((tick, payload)));
                }
                if self.compressor.is_none() {
                    self.compressor = Some(Compressor::new()?);
                }
                let previous = self.receive()?;
                let worker = self.compressor.as_mut().unwrap();
                worker
                    .input
                    .as_ref()
                    .ok_or("Compressor is closed")?
                    .send((tick, encoded))
                    .map_err(|e| e.to_string())?;
                worker.pending = true;
                Ok(previous)
            })
            .map_err(error)?;
        Ok(result.map(|(tick, payload)| (tick, PyBytes::new(py, &payload))))
    }
    fn finish<'py>(&mut self, py: Python<'py>) -> PyResult<Option<(i64, Bound<'py, PyBytes>)>> {
        let result = py
            .detach(|| {
                let result = self.receive();
                self.compressor.take();
                result
            })
            .map_err(error)?;
        Ok(result.map(|(tick, payload)| (tick, PyBytes::new(py, &payload))))
    }
    fn timings(&self) -> PyResult<String> {
        let mut value = serde_json::to_value(&self.timings).map_err(error)?;
        value["parallelCompression"] = json!(self.parallel);
        value["maximumInFlightSnapshots"] = json!(1);
        serde_json::to_string(&value).map_err(error)
    }
    fn statistics(&self) -> PyResult<String> {
        let mut coverage = Vec::new();
        for (kind, scopes) in &self.state.coverage {
            for (scope, fields) in scopes {
                for (field, count) in fields {
                    coverage.push(json!({"entityKind":kind,"scope":scope,"field":field,"observedValues":count}));
                }
            }
        }
        let counts: M<String, V> = self
            .state
            .counts
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        serde_json::to_string(&json!({"frames":self.state.frames,"peakEntitiesPerTeam":self.state.peak,"exclusions":counts,"fieldCoverage":coverage,"currentEntities":self.state.entities.len()})).map_err(error)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_keeps_alias_owner_order() {
        let mut s = State::default();
        let r = json!({"kind":"state","entity_index":1,"entity_serial":2,"entity_class":"CCitadelTeam","active":true,"fields":{"1":{"name":"a.0001.x","data":{"type":"u64","value":"18446744073709551615"}},"2":{"name":"a.1.x","data":{"type":"i32","value":7}}}});
        s.apply(&r).unwrap();
        assert_eq!(s.entities[&(1, 2)].fields["a.1.x"], 7);
        s.apply(&json!({"kind":"delta","entity_index":1,"entity_serial":2,"active":true,"removed":["2"],"fields":{}})).unwrap();
        assert_eq!(s.entities[&(1, 2)].fields["a.1.x"], json!(u64::MAX));
    }
    #[test]
    fn pending_does_not_leak_future_state() {
        let mut s = State::default();
        s.pending.push_back(json!({"tick":20}));
        assert!(
            !s.pending
                .front()
                .is_some_and(|r| r["tick"].as_i64().is_some_and(|t| t <= 19))
        );
        assert_eq!(normal("a.0000.004.x"), "a.0.4.x");
    }
}
