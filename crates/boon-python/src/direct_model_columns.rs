//! Direct, typed, source-qualified model columns. No JSON, archive, database,
//! restore view, or retrospective match-clock conversion participates here.
use crate::*;
use boon_parser::{Context,Entity,FieldValue};
use std::collections::BTreeMap;
use std::time::Instant;

// All diagnostic clocks compile out of production. Only signal polling uses Instant.
#[derive(Clone,Copy)]
struct FineInstant {
 #[cfg(any(feature="direct-method-profile",test))] start:Instant,
}
impl FineInstant {
 #[inline]fn now()->Self{Self{#[cfg(any(feature="direct-method-profile",test))] start:Instant::now()}}
 #[inline]fn elapsed(self)->std::time::Duration{
  #[cfg(any(feature="direct-method-profile",test))]{self.start.elapsed()}
  #[cfg(not(any(feature="direct-method-profile",test)))]{std::time::Duration::ZERO}
 }
}

pub(super) const FIELDS: [&str;66] = ["world_x","world_y","world_z","map_x","map_y","health","max_health","barrier","souls","spent_souls","gold_net_worth","ap_net_worth","level","kills","deaths","assists","last_hits","hero_damage","objective_damage","damage","amount","age","team_amber","team_sapphire","same_team","visible","is_alive","health_byte","capture_active","disabled","source_lane","source_camp","source_box","source_sacrifice","source_player","source_objective","map_age","health_age","world_age","currency_souls","currency_other","is_melee","protected","health_regen","boon_count","has_rebirth","has_rejuvenator","has_ultimate_trained","ultimate_cooldown_start","ultimate_cooldown_end","denies","hero_healing","self_healing","kill_streak","lifestate","respawn_source_time","death_source_time","last_spawn_source_time","in_regen_zone","in_item_shop","cooldown_source_time","upgrade_bits","charges","ammo","capture_fraction_state0","capture_fraction_state1"];

// Stable numeric feature IDs; match dispatch replaces repeated 66-name scans.
#[inline]
fn feature_index(name:&str)->Option<usize>{match name{"world_x"=>Some(0),"world_y"=>Some(1),"world_z"=>Some(2),"map_x"=>Some(3),"map_y"=>Some(4),"health"=>Some(5),"max_health"=>Some(6),"barrier"=>Some(7),"souls"=>Some(8),"spent_souls"=>Some(9),"gold_net_worth"=>Some(10),"ap_net_worth"=>Some(11),"level"=>Some(12),"kills"=>Some(13),"deaths"=>Some(14),"assists"=>Some(15),"last_hits"=>Some(16),"hero_damage"=>Some(17),"objective_damage"=>Some(18),"damage"=>Some(19),"amount"=>Some(20),"age"=>Some(21),"team_amber"=>Some(22),"team_sapphire"=>Some(23),"same_team"=>Some(24),"visible"=>Some(25),"is_alive"=>Some(26),"health_byte"=>Some(27),"capture_active"=>Some(28),"disabled"=>Some(29),"source_lane"=>Some(30),"source_camp"=>Some(31),"source_box"=>Some(32),"source_sacrifice"=>Some(33),"source_player"=>Some(34),"source_objective"=>Some(35),"map_age"=>Some(36),"health_age"=>Some(37),"world_age"=>Some(38),"currency_souls"=>Some(39),"currency_other"=>Some(40),"is_melee"=>Some(41),"protected"=>Some(42),"health_regen"=>Some(43),"boon_count"=>Some(44),"has_rebirth"=>Some(45),"has_rejuvenator"=>Some(46),"has_ultimate_trained"=>Some(47),"ultimate_cooldown_start"=>Some(48),"ultimate_cooldown_end"=>Some(49),"denies"=>Some(50),"hero_healing"=>Some(51),"self_healing"=>Some(52),"kill_streak"=>Some(53),"lifestate"=>Some(54),"respawn_source_time"=>Some(55),"death_source_time"=>Some(56),"last_spawn_source_time"=>Some(57),"in_regen_zone"=>Some(58),"in_item_shop"=>Some(59),"cooldown_source_time"=>Some(60),"upgrade_bits"=>Some(61),"charges"=>Some(62),"ammo"=>Some(63),"capture_fraction_state0"=>Some(64),"capture_fraction_state1"=>Some(65),_=>None}}

type Id=(i32,u32);
type Scope=(String,i32,Option<i64>);
fn physical(id:Id)->String {let _method_span=crate::direct_method_profile::span(0);format!("entity:{}",u64::from(id.1)*16384+id.0 as u64)}
fn integer(v:Option<&FieldValue>)->Option<i64>{let _method_span=crate::direct_method_profile::span(1);match v? {FieldValue::I32(v)=>Some((*v).into()),FieldValue::U32(v)=>Some((*v).into()),FieldValue::I64(v)=>Some(*v),FieldValue::U64(v)=>i64::try_from(*v).ok(),_=>None}}
fn number(v:Option<&FieldValue>)->Option<f64>{let _method_span=crate::direct_method_profile::span(2);match v? {FieldValue::F32(v) if v.is_finite()=>Some((*v).into()),FieldValue::Bool(v)=>Some(u8::from(*v) as f64),other=>integer(Some(other)).map(|v|v as f64)}}
fn numeric(v:Option<&FieldValue>)->Option<f64>{let _method_span=crate::direct_method_profile::span(3);match v?{FieldValue::Bool(_)=>None,value=>number(Some(value))}}
fn boolean(v:Option<&FieldValue>)->Option<bool>{let _method_span=crate::direct_method_profile::span(4);match v? {FieldValue::Bool(v)=>Some(*v),_=>None}}
// Getter-plan reuse is confined to this call and thread. Values, observations,
// clocks, and parser state are never cached here; null key resolutions are too.
#[derive(Clone,Copy)]
struct CopyField {destination:usize,key:Option<u64>,boolean:bool}
#[derive(Clone,Copy)]
enum CopyGroup {PublicPlayer,PublicPawn,TeamPlayer,TeamPawn,Objective}
impl CopyGroup {fn index(self)->usize{self as usize}}
struct ClassFieldPlan {fixed:Vec<Option<Option<u64>>>,dynamic:HashMap<String,Option<u64>>,copies:[Option<Vec<CopyField>>;5],position:Option<[[Option<u64>;6];2]>}
impl Default for ClassFieldPlan{fn default()->Self{Self{fixed:vec![None;105],dynamic:HashMap::new(),copies:Default::default(),position:None}}}
type FieldKeys=HashMap<i32,ClassFieldPlan>;
#[inline]
fn static_field_index(name:&str)->Option<usize>{match name{"CBodyComponent.m_skeletonInstance.m_vecOrigin."=>Some(0),"m_CCitadelHeroComponent.m_spawnedHero.m_nHeroID"=>Some(1),"m_ItemID"=>Some(2),"m_PlayerDataGlobal."=>Some(3),"m_PlayerDataGlobal.m_iBoonCount"=>Some(4),"m_PlayerDataGlobal.m_iHealthMax"=>Some(5),"m_PlayerDataGlobal.m_nBoonCount"=>Some(6),"m_PlayerDataGlobal.m_vecAbilityUpgradeState"=>Some(7),"m_PlayerDataGlobal.m_vecStatViewerModifierValues"=>Some(8),"m_PlayerDataGlobal.m_vecUpgrades"=>Some(9),"m_SourceAbilityID"=>Some(10),"m_SourceModifierID"=>Some(11),"m_b"=>Some(12),"m_bAlive"=>Some(13),"m_bBackdoorProtectionActive"=>Some(14),"m_bCapturable"=>Some(15),"m_bDisableZippingToByPlayers"=>Some(16),"m_bDisabled"=>Some(17),"m_bHasRebirth"=>Some(18),"m_bHasRejuvenator"=>Some(19),"m_bInItemShopZone"=>Some(20),"m_bInRegenerationZone"=>Some(21),"m_bInReload"=>Some(22),"m_bReloading"=>Some(23),"m_bUltimateTrained"=>Some(24),"m_bVisibleOnMap"=>Some(25),"m_cellX"=>Some(26),"m_cellY"=>Some(27),"m_cellZ"=>Some(28),"m_eAbilitySlot"=>Some(29),"m_eCaptureState"=>Some(30),"m_eClass"=>Some(31),"m_eValType"=>Some(32),"m_flAmmoFrac"=>Some(33),"m_flCaptureProgressFrac"=>Some(34),"m_flChargeRechargeEnd"=>Some(35),"m_flChargeRechargeStart"=>Some(36),"m_flCooldownEnd"=>Some(37),"m_flCooldownStart"=>Some(38),"m_flCumulativeDistance"=>Some(39),"m_flDeathTime"=>Some(40),"m_flHealthRegen"=>Some(41),"m_flLastSpawnTime"=>Some(42),"m_flRespawnTime"=>Some(43),"m_flUltimateCooldownEnd"=>Some(44),"m_flUltimateCooldownStart"=>Some(45),"m_flValue"=>Some(46),"m_hHeroPawn"=>Some(47),"m_hOwnerEntity"=>Some(48),"m_hPawn"=>Some(49),"m_iAPNetWorth"=>Some(50),"m_iBoonCount"=>Some(51),"m_iClip1"=>Some(52),"m_iDeaths"=>Some(53),"m_iDenies"=>Some(54),"m_iGoldNetWorth"=>Some(55),"m_iHealth"=>Some(56),"m_iHeroDamage"=>Some(57),"m_iHeroHealing"=>Some(58),"m_iKillStreak"=>Some(59),"m_iLane"=>Some(60),"m_iLastHits"=>Some(61),"m_iLevel"=>Some(62),"m_iMaxHealth"=>Some(63),"m_iNodeIndex"=>Some(64),"m_iObjectiveDamage"=>Some(65),"m_iPlayerAssists"=>Some(66),"m_iPlayerKills"=>Some(67),"m_iPrimaryLane"=>Some(68),"m_iRemainingCharges"=>Some(69),"m_iSelfHealing"=>Some(70),"m_iTeamNum"=>Some(71),"m_lifeState"=>Some(72),"m_nAbilityUpgradeBits"=>Some(73),"m_nBoonCount"=>Some(74),"m_nCharges"=>Some(75),"m_nCurrencies.m_nCurrencies"=>Some(76),"m_nEntIndex"=>Some(77),"m_nHealthPercent"=>Some(78),"m_nPositionX"=>Some(79),"m_nPositionY"=>Some(80),"m_nSpentCurrencies.m_nSpentCurrencies"=>Some(81),"m_nSubclassID"=>Some(82),"m_nTeam"=>Some(83),"m_nTickBase"=>Some(84),"m_nUpgradeInfo"=>Some(85),"m_nUpgradeInfo."=>Some(86),"m_pGameRules.m_bGamePaused"=>Some(87),"m_pGameRules.m_bServerPaused"=>Some(88),"m_pGameRules.m_flMatchClockAtLastUpdate"=>Some(89),"m_pGameRules.m_nMatchClockUpdateTick"=>Some(90),"m_pGameRules.m_nTotalPausedTicks"=>Some(91),"m_pGameRules.m_tNextMidBossSpawnTime"=>Some(92),"m_pGameRules.m_timeKothCashInStarted"=>Some(93),"m_pGameRules.m_vKothCashInCurrentLocation"=>Some(94),"m_pGameRules.m_vecNeutralCampNextSpawnTimes"=>Some(95),"m_pGameRules.m_vecNeutralCampTimerIsMidBoss"=>Some(96),"m_pGameRules.m_vecNeutralCampTimerOrigins"=>Some(97),"m_pGameRules.m_vecTeamKothStates"=>Some(98),"m_steamID"=>Some(99),"m_vecConnections"=>Some(100),"m_vecFOWEntities"=>Some(101),"m_vecX"=>Some(102),"m_vecY"=>Some(103),"m_vecZ"=>Some(104),_=>None}}

thread_local!{static FIELD_KEYS:std::cell::RefCell<Option<FieldKeys>>=const{std::cell::RefCell::new(None)};}
// Retain the immutable serializer allocations so replacement cannot reuse an
// old address. Playback accepts SEND_TABLES after initialization. A replacement
// reparses every serializer; one retained anchor plus length detects it in O(1).
#[derive(Default)]
struct SchemaGuard { serializers:Option<boon_parser::entity::SerializerContainer>, classes:Option<boon_parser::entity::ClassInfo> }
impl SchemaGuard {
 fn refresh(&mut self,ctx:&Context)->bool{self.refresh_parts(ctx.serializers(),ctx.class_info())}
 fn refresh_parts(&mut self,serializers:&boon_parser::entity::SerializerContainer,classes:&boon_parser::entity::ClassInfo)->bool{
  let same_serializers=self.serializers.as_ref().is_some_and(|old|old.len()==serializers.len()&&serializers.iter().next().is_none_or(|(name,s)|old.get(name).is_some_and(|old|std::ptr::eq(old,s))));
  // ClassInfo clones retain each network-name Arc. Adapter installations create
  // fresh names, even for an otherwise identical class table.
  let same_classes=self.classes.as_ref().is_some_and(|old|old.len()==classes.len()&&classes.classes().first().is_none_or(|entry|old.classes().first().is_some_and(|old|std::sync::Arc::ptr_eq(&old.network_name,&entry.network_name)&&old.class_id==entry.class_id&&old.table_name==entry.table_name)));
  if same_serializers&&same_classes{return false}
  let replaced=self.serializers.is_some()||self.classes.is_some();self.serializers=Some(serializers.clone());self.classes=Some(classes.clone());replaced
 }
}
fn reset_getter_plans(){FIELD_KEYS.with(|keys|{if let Some(keys)=keys.borrow_mut().as_mut(){keys.clear();}});ARRAY_PLANS.with(|plans|{if let Some(plans)=plans.borrow_mut().as_mut(){plans.clear();}});CANDIDATE_INDEXES.with(|indexes|{if let Some(indexes)=indexes.borrow_mut().as_mut(){*indexes=CandidateIndexes::default();}});}
struct FieldKeyScope {previous:Option<FieldKeys>,arrays:Option<ArrayPlans>,indexes:Option<CandidateIndexes>}
impl FieldKeyScope {fn enter()->Self{let _method_span=crate::direct_method_profile::span(5);Self{previous:FIELD_KEYS.with(|keys|keys.replace(Some(HashMap::new()))),arrays:ARRAY_PLANS.with(|plans|plans.replace(Some(HashMap::new()))),indexes:CANDIDATE_INDEXES.with(|indexes|indexes.replace(Some(CandidateIndexes::default())))}}}
impl Drop for FieldKeyScope {fn drop(&mut self){let _method_span=crate::direct_method_profile::span(6);FIELD_KEYS.with(|keys|{keys.replace(self.previous.take());});ARRAY_PLANS.with(|plans|{plans.replace(self.arrays.take());});CANDIDATE_INDEXES.with(|indexes|{indexes.replace(self.indexes.take());});}}
fn cached_field_key(class:i32,name:&str,resolve:impl FnOnce()->Option<u64>)->Option<u64>{let _method_span=crate::direct_method_profile::span(7);
 FIELD_KEYS.with(|keys|{let mut keys=keys.borrow_mut();let Some(keys)=keys.as_mut()else{return resolve()};let plan=keys.entry(class).or_default();if let Some(index)=static_field_index(name){return *plan.fixed[index].get_or_insert_with(resolve)}if let Some(key)=plan.dynamic.get(name){return *key}let key=resolve();plan.dynamic.insert(name.to_owned(),key);key})
}
fn field<'a>(ctx:&Context,e:&'a Entity,name:&str)->Option<&'a FieldValue>{let _method_span=crate::direct_method_profile::span(8);
 let key=cached_field_key(e.class_id,name,||ctx.serializers().get(&e.class_name).and_then(|s|{let _span=crate::direct_method_profile::span(87);s.resolve_field_key(name)}));{let _span=crate::direct_method_profile::span(89);e.field_value(key)}
}
fn n(ctx:&Context,e:&Entity,name:&str)->Option<f64>{let _method_span=crate::direct_method_profile::span(9);if name.rsplit('.').next().is_some_and(|p|p.starts_with("m_b")){boolean(field(ctx,e,name)).map(|v|v as u8 as f64)}else{numeric(field(ctx,e,name))}}
fn i(ctx:&Context,e:&Entity,name:&str)->Option<i64>{let _method_span=crate::direct_method_profile::span(10);integer(field(ctx,e,name))}
fn b(ctx:&Context,e:&Entity,name:&str)->Option<bool>{let _method_span=crate::direct_method_profile::span(11);boolean(field(ctx,e,name))}
fn count(ctx:&Context,e:&Entity,name:&str,max:i64)->Option<usize>{let _method_span=crate::direct_method_profile::span(12);i(ctx,e,name).filter(|v|(0..=max).contains(v)).map(|v|v as usize)}
fn resolved<'a>(ctx:&'a Context,e:&Entity,name:&str)->Option<&'a Entity>{let _method_span=crate::direct_method_profile::span(13);let raw=i(ctx,e,name)?;if raw<0{return None} let target=ctx.entities().get((raw&16383) as i32)?;((raw>>14)==target.serial as i64).then_some(target)}
fn normal(s:&str)->String{let _method_span=crate::direct_method_profile::span(14);s.split('.').enumerate().map(|(j,p)|if j>0&&!p.is_empty()&&p.bytes().all(|v|v.is_ascii_digit()){let t=p.trim_start_matches('0');if t.is_empty(){"0"}else{t}}else{p}).collect::<Vec<_>>().join(".")}
#[derive(Default)]
struct PrefixPlan {keys:Vec<(u64,String)>,known:HashSet<u64>,normalized:HashMap<String,u64>,collides:bool}
#[derive(Default)]
struct FieldNames {names:HashMap<(i32,u64),Option<String>>,upgrades:HashMap<i32,PrefixPlan>,seeded:bool}
impl std::ops::Deref for FieldNames{type Target=HashMap<(i32,u64),Option<String>>;fn deref(&self)->&Self::Target{&self.names}}
impl std::ops::DerefMut for FieldNames{fn deref_mut(&mut self)->&mut Self::Target{&mut self.names}}
impl FieldNames{
 fn observe(&mut self,ctx:&Context){
  let entities:Vec<&Entity>=if self.seeded{ctx.entities().updated_indices().iter().filter_map(|index|ctx.entities().get(*index)).collect()}else{ctx.entities().iter().map(|(_,e)|e).collect()};
  for e in entities{if !(e.class_name.contains("Ability")||e.class_name.contains("Item")){continue}let Some(serializer)=ctx.serializers().get(&e.class_name)else{continue};
   let keys:Vec<u64>=if self.seeded{e.changed_field_keys().to_vec()}else{e.fields.keys().copied().collect()};
   for key in keys{
    let plan=self.upgrades.entry(e.class_id).or_default();if !plan.known.insert(key){continue}
    let name=self.names.entry((e.class_id,key)).or_insert_with(||serializer.field_name_for_key(key).map(|s|normal(&s)));
    if let Some(name)=name.as_ref().filter(|name|name.starts_with("m_nUpgradeInfo.")){
     if plan.normalized.insert(name.clone(),key).is_some(){plan.collides=true;}
     plan.keys.push((key,name.clone()));
    }
   }
  }
  self.seeded=true;
 }
}
fn named_numeric(ctx:&Context,e:&Entity,prefix:&str,names:&mut FieldNames)->Vec<(String,f64)>{let _method_span=crate::direct_method_profile::span(15);
 if prefix=="m_nUpgradeInfo."&&names.seeded{
  if let Some(plan)=names.upgrades.get(&e.class_id){if !plan.collides{return plan.keys.iter().filter_map(|(key,name)|number(e.field_value(Some(*key))).map(|value|(name.clone(),value))).collect()}}
 }
 let Some(serializer)=ctx.serializers().get(&e.class_name)else{return vec![]};
 e.fields.iter().filter_map(|(key,value)|{
  let name=names.entry((e.class_id,*key)).or_insert_with(||{let _span=crate::direct_method_profile::span(88);serializer.field_name_for_key(*key)}.map(|s|normal(&s))).as_deref()?;
  if !name.starts_with(prefix){return None}number(Some(value)).map(|v|(name.to_owned(),v))
 }).collect()
}
fn array_slots(ctx:&Context,e:&Entity,prefix:&str,max:i64,names:&mut FieldNames)->Vec<usize>{let _method_span=crate::direct_method_profile::span(16);
 if let Some(len)=count(ctx,e,prefix,max){return (0..len).collect();}
 let Some(serializer)=ctx.serializers().get(&e.class_name)else{return vec![]};
 let start=format!("{prefix}.");let mut slots=std::collections::BTreeSet::new();
 for key in e.fields.keys(){let name=names.entry((e.class_id,*key)).or_insert_with(||{let _span=crate::direct_method_profile::span(88);serializer.field_name_for_key(*key)}.map(|s|normal(&s)));let Some(name)=name.as_deref()else{continue};let Some(tail)=name.strip_prefix(&start)else{continue};if let Some(slot)=tail.split('.').next().and_then(|v|v.parse::<usize>().ok()).filter(|v|(*v as i64)<max){slots.insert(slot);}}
 slots.into_iter().collect()
}
type ArrayPlans=HashMap<i32,HashMap<String,HashMap<String,Vec<Option<(Option<u64>,Option<u64>)>>>>>;
thread_local!{static ARRAY_PLANS:std::cell::RefCell<Option<ArrayPlans>>=const{std::cell::RefCell::new(None)};}
fn arr<'a>(ctx:&Context,e:&'a Entity,prefix:&str,slot:usize,tail:&str)->Option<&'a FieldValue>{let _method_span=crate::direct_method_profile::span(17);
 let keys=ARRAY_PLANS.with(|plans|{
  let mut plans=plans.borrow_mut();let Some(plans)=plans.as_mut()else{let serializer=ctx.serializers().get(&e.class_name);let suffix=if tail.is_empty(){String::new()}else{format!(".{tail}")};return (serializer.and_then(|s|s.resolve_field_key(&format!("{prefix}.{slot}{suffix}"))),serializer.and_then(|s|s.resolve_field_key(&format!("{prefix}.{slot:04}{suffix}"))))};
  let by_prefix=plans.entry(e.class_id).or_default();
  if !by_prefix.contains_key(prefix){by_prefix.insert(prefix.to_owned(),HashMap::new());}
  let by_tail=by_prefix.get_mut(prefix).unwrap();
  if !by_tail.contains_key(tail){by_tail.insert(tail.to_owned(),Vec::new());}
  let slots=by_tail.get_mut(tail).unwrap();if slots.len()<=slot{slots.resize(slot+1,None);}
  *slots[slot].get_or_insert_with(||{
   let serializer=ctx.serializers().get(&e.class_name);let suffix=if tail.is_empty(){String::new()}else{format!(".{tail}")};
   (serializer.and_then(|s|s.resolve_field_key(&format!("{prefix}.{slot}{suffix}"))),serializer.and_then(|s|s.resolve_field_key(&format!("{prefix}.{slot:04}{suffix}"))))
  })
 });
 planned_pair_value(e,keys)
}
fn planned_pair_value(e:&Entity,keys:(Option<u64>,Option<u64>))->Option<&FieldValue>{e.field_value(keys.0).or_else(||e.field_value(keys.1))}

fn ai(ctx:&Context,e:&Entity,prefix:&str,slot:usize,tail:&str)->Option<i64>{let _method_span=crate::direct_method_profile::span(18);integer(arr(ctx,e,prefix,slot,tail))}
fn an(ctx:&Context,e:&Entity,prefix:&str,slot:usize,tail:&str)->Option<f64>{let _method_span=crate::direct_method_profile::span(19);numeric(arr(ctx,e,prefix,slot,tail))}
fn ab(ctx:&Context,e:&Entity,prefix:&str,slot:usize,tail:&str)->Option<bool>{let _method_span=crate::direct_method_profile::span(20);boolean(arr(ctx,e,prefix,slot,tail))}
fn position(ctx:&Context,e:&Entity)->Option<[f64;3]>{let _method_span=crate::direct_method_profile::span(21);
 const PLANS:[[&str;6];2]=[
 ["CBodyComponent.m_skeletonInstance.m_vecOrigin.m_cellX","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_cellY","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_cellZ","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecX","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecY","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecZ"],
 ["CBodyComponent.m_cellX","CBodyComponent.m_cellY","CBodyComponent.m_cellZ","CBodyComponent.m_vecX","CBodyComponent.m_vecY","CBodyComponent.m_vecZ"]];
 let resolve=||PLANS.map(|names|names.map(|name|ctx.serializers().get(&e.class_name).and_then(|s|s.resolve_field_key(name))));
 FIELD_KEYS.with(|plans|{let mut plans=plans.borrow_mut();let local;let plans=if let Some(plans)=plans.as_mut(){plans.entry(e.class_id).or_default().position.get_or_insert_with(resolve)}else{local=resolve();&local};
  for keys in plans.iter(){let values=keys.map(|key|numeric(e.field_value(key)));if values.iter().all(Option::is_some){let v=values.map(Option::unwrap);return Some([v[0]*512.-16384.+v[3],v[1]*512.-16384.+v[4],v[2]*512.-16384.+v[5]])}}None
 })
}
fn kind(class:&str)->&'static str {let _method_span=crate::direct_method_profile::span(22);match class {"CCitadelPlayerPawn"=>"player","CNPC_TrooperBoss"=>"guardian","CNPC_Boss_Tier2"=>"walker","CNPC_Boss_Tier3"=>"patron","CNPC_BarrackBoss"=>"barracks","CCitadel_Destroyable_Building"=>"shrine","CNPC_MidBoss"=>"midboss","CCitadel_KothCashIn"=>"rift","CNPC_Trooper"=>"lane_creep","CNPC_TrooperNeutral"|"CNPC_Neutral_Bug"=>"neutral","CCitadel_BreakableProp"=>"breakable","CNPC_Neutral_SinnersSacrifice"|"CNPC_Neutral_SinnersSacrifice_Hideout"=>"sacrifice","CItemXP"=>"orb","CCitadel_Pickup_Gold"|"CCitadel_Pickup_AssignedGold"=>"pickup","CCitadelZipLineNode"=>"zipline",_=>"unknown"}}
fn objective(class:&str)->bool{let _method_span=crate::direct_method_profile::span(23);matches!(kind(class),"guardian"|"walker"|"patron"|"barracks"|"shrine"|"midboss")}
fn objective_code(class:&str)->&'static str{let _method_span=crate::direct_method_profile::span(24);match class{"CNPC_TrooperBoss"=>"CLASS_TROOPER_BOSS","CNPC_Boss_Tier2"=>"CLASS_BOSS_TIER_2","CNPC_Boss_Tier3"=>"CLASS_BOSS_TIER_3","CNPC_BarrackBoss"=>"CLASS_BOSS_BARRACKS","CCitadel_Destroyable_Building"=>"CLASS_DESTROYABLE_BUILDING","CNPC_MidBoss"=>"CLASS_MID_BOSS",_=>""}}

include!("direct_shared_payload.rs");
struct Node {key:String,kind:String,class:Option<String>,id:Option<Id>,owner:Option<u64>,team:Option<i64>,hero:Option<i64>,ability:Option<i64>,audience:i32,observer:Option<i64>,active:bool,values:[Option<f64>;66],extras:SharedExtras,relations:SharedRelations,source_time:Option<f64>,map_time:Option<f64>,health_time:Option<f64>,world_time:Option<f64>,position_observation_time:Option<f64>,position_observation_live:bool,position_emitted_time:Option<f64>}
impl Clone for Node {fn clone(&self)->Self{let _method_span=crate::direct_method_profile::span(25);Self{key:self.key.clone(),kind:self.kind.clone(),class:self.class.clone(),id:self.id.clone(),owner:self.owner.clone(),team:self.team.clone(),hero:self.hero.clone(),ability:self.ability.clone(),audience:self.audience.clone(),observer:self.observer.clone(),active:self.active.clone(),values:self.values.clone(),extras:self.extras.clone(),relations:self.relations.clone(),source_time:self.source_time.clone(),map_time:self.map_time.clone(),health_time:self.health_time.clone(),world_time:self.world_time.clone(),position_observation_time:self.position_observation_time.clone(),position_observation_live:self.position_observation_live.clone(),position_emitted_time:self.position_emitted_time.clone()}}}
impl PartialEq for Node {fn eq(&self,other:&Self)->bool{let _method_span=crate::direct_method_profile::span(26);self.key==other.key&&self.kind==other.kind&&self.class==other.class&&self.id==other.id&&self.owner==other.owner&&self.team==other.team&&self.hero==other.hero&&self.ability==other.ability&&self.audience==other.audience&&self.observer==other.observer&&self.active==other.active&&self.values==other.values&&self.extras==other.extras&&self.relations==other.relations&&self.source_time==other.source_time&&self.map_time==other.map_time&&self.health_time==other.health_time&&self.world_time==other.world_time&&self.position_observation_time==other.position_observation_time&&self.position_observation_live==other.position_observation_live&&self.position_emitted_time==other.position_emitted_time}}

impl Node {
 // Exact append equality after four observation timestamps are normalized.
 fn same_observation_content(&self,other:&Self)->bool{[self.map_time,self.health_time,self.world_time,self.position_observation_time].into_iter().all(|v|v.is_none_or(|v|!v.is_nan()))&&self.key==other.key&&self.kind==other.kind&&self.class==other.class&&self.id==other.id&&self.owner==other.owner&&self.team==other.team&&self.hero==other.hero&&self.ability==other.ability&&self.audience==other.audience&&self.observer==other.observer&&self.active==other.active&&self.values==other.values&&self.extras==other.extras&&self.relations==other.relations&&self.source_time==other.source_time&&self.position_observation_live==other.position_observation_live&&self.position_emitted_time==other.position_emitted_time}

 fn new(key:String,kind:&str,audience:i32)->Self{let _method_span=crate::direct_method_profile::span(27);Self{key,kind:kind.into(),class:None,id:None,owner:None,team:None,hero:None,ability:None,audience,observer:None,active:true,values:[None;66],extras:SharedExtras::default(),relations:SharedRelations::default(),source_time:None,map_time:None,health_time:None,world_time:None,position_observation_time:None,position_observation_live:false,position_emitted_time:None}}
 fn entity(e:&Entity,audience:i32)->Self{let _method_span=crate::direct_method_profile::span(28);let mut node=Self::new(physical((e.index,e.serial)),kind(&e.class_name),audience);node.id=Some((e.index,e.serial));node.class=Some(e.class_name.to_string());node}
 fn scope(&self)->Scope{let _method_span=crate::direct_method_profile::span(29);(self.key.clone(),self.audience,self.observer)}
 fn set(&mut self,name:&str,v:Option<f64>){let _method_span=crate::direct_method_profile::span(30);if let Some(j)=feature_index(name){self.values[j]=v.filter(|v|v.is_finite());}}
 fn get(&self,name:&str)->Option<f64>{let _method_span=crate::direct_method_profile::span(31);self.values[feature_index(name).unwrap()]}
 fn xyz(&mut self,p:Option<[f64;3]>){let _method_span=crate::direct_method_profile::span(32);if let Some(p)=p{self.set("world_age",Some(0.));for(k,v)in ["world_x","world_y","world_z"].iter().zip(p){self.set(k,Some(v));}}}
 fn copy_fields(&mut self,ctx:&Context,e:&Entity,group:CopyGroup,prefix:&str,fields:&[(&str,&str)]){let _method_span=crate::direct_method_profile::span(33);
  // One invocation-local class/group borrow replaces per-field TLS, formatting,
  // name dispatch and feature-name dispatch. Schema replacement clears it.
  let resolve=||fields.iter().filter_map(|(dest,src)|feature_index(dest).map(|destination|{
   let name=format!("{prefix}{src}");let key=ctx.serializers().get(&e.class_name).and_then(|s|s.resolve_field_key(&name));
   CopyField{destination,key,boolean:name.rsplit('.').next().is_some_and(|part|part.starts_with("m_b"))}
  })).collect::<Vec<_>>();
  FIELD_KEYS.with(|plans|{let mut plans=plans.borrow_mut();let local;let plan=if let Some(plans)=plans.as_mut(){plans.entry(e.class_id).or_default().copies[group.index()].get_or_insert_with(resolve)}else{local=resolve();&local};
   for field in plan.iter(){let value=e.field_value(field.key);self.values[field.destination]=if field.boolean{boolean(value).map(|v|v as u8 as f64)}else{numeric(value)}.filter(|v|v.is_finite());}
  });
 }
 fn owned(&mut self,owner:&Player){let _method_span=crate::direct_method_profile::span(34);self.owner=Some(owner.steam);self.team=Some(owner.team);self.relations.push((format!("player:{}",owner.steam),1));}
}
#[derive(Clone,Copy)]
struct PlayerIdentity{steam:u64,team:i64,pawn:Id,controller:Id,hero:Option<i64>}
#[derive(Default)]
struct CandidateIndexes {qualified:Option<Vec<PlayerIdentity>>,binding_dirty:bool,binding_fields:HashMap<(i32,u64),bool>,seeded:bool,slots:HashMap<i32,i32>,classes:HashMap<i32,std::collections::BTreeSet<i32>>,names:HashMap<String,i32>}
thread_local!{static CANDIDATE_INDEXES:std::cell::RefCell<Option<CandidateIndexes>>=const{std::cell::RefCell::new(None)};}
fn binding_field(name:&str)->bool{matches!(name,"m_steamID"|"m_hPawn"|"m_hHeroPawn"|"m_iTeamNum"|"m_CCitadelHeroComponent.m_spawnedHero.m_nHeroID")}
fn observe_candidates(ctx:&Context){CANDIDATE_INDEXES.with(|state|{
 let mut state=state.borrow_mut();let Some(state)=state.as_mut()else{return};
 if !state.seeded||ctx.entities().entity_changes().iter().any(|change|change.kind!=boon_parser::EntityChangeKind::Updated){state.binding_dirty=true;}
 for &index in ctx.entities().updated_indices(){if let Some(e)=ctx.entities().get(index){
  if matches!(e.class_name.as_ref(),"CCitadelPlayerController"|"CCitadelPlayerPawn"){
   let serializer=ctx.serializers().get(&e.class_name);
   if e.changed_field_keys().iter().any(|&key|*state.binding_fields.entry((e.class_id,key)).or_insert_with(||serializer.and_then(|s|s.field_name_for_key(key)).as_deref().is_none_or(binding_field))){state.binding_dirty=true;}
  }
 }}
 let touched:Vec<i32>=if state.seeded{ctx.entities().updated_indices().iter().copied().chain(ctx.entities().entity_changes().iter().map(|c|c.index)).collect()}else{state.seeded=true;ctx.entities().iter().map(|(index,_)|index).collect()};
 for index in touched{
  let current=ctx.entities().get(index);let class=current.map(|e|e.class_id);
  if state.slots.get(&index).copied()==class{continue}
  if let Some(old)=state.slots.remove(&index){if let Some(indices)=state.classes.get_mut(&old){indices.remove(&index);}}
  if let Some(e)=current{state.slots.insert(index,e.class_id);state.classes.entry(e.class_id).or_default().insert(index);if !state.names.contains_key(e.class_name.as_ref()){state.names.insert(e.class_name.to_string(),e.class_id);}}
 }
});}
fn class_candidates<'a>(ctx:&'a Context,class:&str)->Vec<&'a Entity>{CANDIDATE_INDEXES.with(|state|{
 let state=state.borrow();match state.as_ref().filter(|s|s.seeded){
  Some(state)=>state.names.get(class).and_then(|id|state.classes.get(id)).into_iter().flatten().filter_map(|index|ctx.entities().get(*index)).collect(),
  None=>ctx.entities().iter().filter(|(_,e)|e.class_name.as_ref()==class).map(|(_,e)|e).collect()
 }
})}

struct Player<'a>{steam:u64,team:i64,pawn:&'a Entity,controller:&'a Entity,hero:Option<i64>}
fn players(ctx:&Context)->Vec<Player<'_>>{
 let cached=CANDIDATE_INDEXES.with(|state|state.borrow().as_ref().filter(|state|state.seeded&&!state.binding_dirty).and_then(|state|state.qualified.clone()));
 if let Some(cached)=cached{return cached.into_iter().map(|identity|{
  let pawn=ctx.entities().get(identity.pawn.0).filter(|e|e.serial==identity.pawn.1).expect("cached qualified pawn lost generation without binding invalidation");
  let controller=ctx.entities().get(identity.controller.0).filter(|e|e.serial==identity.controller.1).expect("cached qualified controller lost generation without binding invalidation");
  Player{steam:identity.steam,team:identity.team,pawn,controller,hero:identity.hero}
 }).collect()}
 let rows=players_uncached(ctx);
 CANDIDATE_INDEXES.with(|state|{if let Some(state)=state.borrow_mut().as_mut().filter(|state|state.seeded){state.qualified=Some(rows.iter().map(|p|PlayerIdentity{steam:p.steam,team:p.team,pawn:(p.pawn.index,p.pawn.serial),controller:(p.controller.index,p.controller.serial),hero:p.hero}).collect());state.binding_dirty=false;}});
 rows
}

fn players_uncached(ctx:&Context)->Vec<Player<'_>>{let _method_span=crate::direct_method_profile::span(35);
 let mut rows=vec![];
 for c in class_candidates(ctx,"CCitadelPlayerController").into_iter().filter(|e|e.active){
  let Some(FieldValue::U64(steam))=field(ctx,c,"m_steamID") else{continue};if *steam==0{continue}
  let mut pawns=vec![];for name in ["m_hPawn","m_hHeroPawn"]{if let Some(p)=resolved(ctx,c,name).filter(|p|p.active&&p.class_name.as_ref()=="CCitadelPlayerPawn"){if !pawns.iter().any(|q:&&Entity|q.id()==p.id()){pawns.push(p)}}}
  if pawns.len()!=1{continue}let p=pawns[0];let Some(team)=i(ctx,p,"m_iTeamNum").or_else(||i(ctx,c,"m_iTeamNum")).filter(|v|*v==2||*v==3)else{continue};
  rows.push(Player{steam:*steam,team,pawn:p,controller:c,hero:i(ctx,p,"m_CCitadelHeroComponent.m_spawnedHero.m_nHeroID").filter(|v|*v>0)});
 }
 let steam_counts=rows.iter().fold(HashMap::new(),|mut m,p|{*m.entry(p.steam).or_insert(0)+=1;m});let pawn_counts=rows.iter().fold(HashMap::new(),|mut m,p|{*m.entry(p.pawn.id()).or_insert(0)+=1;m});rows.retain(|p|steam_counts[&p.steam]==1&&pawn_counts[&p.pawn.id()]==1);rows
}

struct Binding {steam:u64,team:i64,index:i32,serial:u32,hero:Option<i64>,hero_known:bool}
impl Clone for Binding {fn clone(&self)->Self{let _method_span=crate::direct_method_profile::span(36);Self{steam:self.steam.clone(),team:self.team.clone(),index:self.index.clone(),serial:self.serial.clone(),hero:self.hero.clone(),hero_known:self.hero_known.clone()}}}
impl PartialEq for Binding {fn eq(&self,other:&Self)->bool{let _method_span=crate::direct_method_profile::span(37);self.steam==other.steam&&self.team==other.team&&self.index==other.index&&self.serial==other.serial&&self.hero==other.hero&&self.hero_known==other.hero_known}}

#[derive(Default)]
struct Bindings {current:BTreeMap<(u64,i32,u32),Binding>,rows:Vec<(i32,bool,Binding)>}
impl Bindings {
 fn collect(&mut self,ctx:&Context){let _method_span=crate::direct_method_profile::span(38);
  let ps=players(ctx);let mut counts=HashMap::new();for p in &ps{if let Some(hero)=p.hero{*counts.entry(hero).or_insert(0)+=1;}}
  let mut next=BTreeMap::new();for p in ps{let binding=Binding{steam:p.steam,team:p.team,index:p.pawn.index,serial:p.pawn.serial,hero:p.hero,hero_known:p.hero.is_some_and(|h|counts[&h]==1)};next.insert((binding.steam,binding.index,binding.serial),binding);}
  for(k,old)in &self.current{if next.get(k)!=Some(old){self.rows.push((ctx.tick(),false,old.clone()));}}
  for(k,new)in &next{if self.current.get(k)!=Some(new){self.rows.push((ctx.tick(),true,new.clone()));}}
  self.current=next;
 }
 fn frame(self)->PolarsResult<DataFrame>{let _method_span=crate::direct_method_profile::span(39);let r=self.rows;macro_rules! c{($name:expr,$f:expr)=>{Column::new($name.into(),r.iter().map($f).collect::<Vec<_>>())};}df_from_columns(vec![c!("tick",|r|r.0),c!("active",|r|r.1),c!("steam_id",|r|r.2.steam),c!("team",|r|r.2.team),c!("entity_index",|r|r.2.index),c!("entity_serial",|r|r.2.serial),c!("hero_id",|r|r.2.hero),c!("hero_known",|r|r.2.hero_known)])}
}

// Attribution is a boundary measurement, not a stock/full-parity benchmark.
// Original helper calls induced by these projections stay in integration cost.
// Parser residual includes native dispatch/setup plus retained and fork code.
fn profile_row_unit(name:&str)->&'static str{let _method_span=crate::direct_method_profile::span(40);match name{
 "projection.shared_entity_walks"=>"selected_candidate_entities",
 "projection.player_binding_plan"=>"qualified_player_bindings",
 "projection.ammo"=>"touched_scopes",
 "projection.channel_count_receipt"|"append.position_policy"=>"input_scopes",
 "append.comparison"=>"compared_scopes","append.comparable_copy"=>"copied_comparables","append.output_copy"=>"emitted_state_rows","append.lifecycle_and_replacement"=>"emitted_tombstones",
 "integration.source_bindings"=>"emitted_binding_rows","integration.source_clock"=>"clock_samples","integration.modifier_updates"=>"modifier_changes","integration.ability_transitions"=>"emitted_ability_transition_rows","integration.debris_messages"=>"decoded_debris_messages",
 "integration.projection_unresolved"=>"selected_source_ticks","integration.callback_unresolved"|"parser.scan_dispatch_setup_shared_residual"=>"source_ticks",
 "parser.initial_parse"=>"initial_parse_calls","integration.class_filter_plan"=>"selected_classes","integration.dataframe_builds"=>"entity_update_rows",
 _ if name.starts_with("projection.")=>"materialized_scopes",_=>"phase_specific_rows"}}
trait ProfileClock {fn seconds(self)->f64;}
impl ProfileClock for Instant {fn seconds(self)->f64{self.elapsed().as_secs_f64()}}
impl ProfileClock for FineInstant {fn seconds(self)->f64{self.elapsed().as_secs_f64()}}
#[cfg(any(feature="direct-method-profile",test))]
#[derive(Default)]
struct Profile {groups:BTreeMap<&'static str,(f64,u64,u64)>}
#[cfg(any(feature="direct-method-profile",test))]
impl Profile {
 fn add(&mut self,name:&'static str,seconds:f64,rows:usize){
  if !cfg!(any(feature="direct-method-profile",test)){return}
let _method_span=crate::direct_method_profile::span(41);let entry=self.groups.entry(name).or_default();entry.0+=seconds;entry.1+=1;entry.2+=rows as u64;}
 fn summary(&mut self,name:&'static str,seconds:f64,calls:u64,rows:u64){if !cfg!(any(feature="direct-method-profile",test)){return}let _method_span=crate::direct_method_profile::span(42);self.groups.insert(name,(seconds,calls,rows));}
 fn elapsed(&mut self,name:&'static str,start:impl ProfileClock,rows:usize){let _method_span=crate::direct_method_profile::span(43);self.add(name,start.seconds(),rows);}
 fn sum_prefix(&self,prefix:&str)->f64{let _method_span=crate::direct_method_profile::span(44);self.groups.iter().filter(|(k,_)|k.starts_with(prefix)).map(|(_,v)|v.0).sum()}
 fn frame(self)->PolarsResult<DataFrame>{let _method_span=crate::direct_method_profile::span(45);let rows=self.groups.into_iter().collect::<Vec<_>>();df_from_columns(vec![Column::new("phase".into(),rows.iter().map(|r|r.0).collect::<Vec<_>>()),Column::new("seconds".into(),rows.iter().map(|r|r.1.0).collect::<Vec<_>>()),Column::new("calls".into(),rows.iter().map(|r|r.1.1).collect::<Vec<_>>()),Column::new("rows".into(),rows.iter().map(|r|r.1.2).collect::<Vec<_>>()),Column::new("exclusive".into(),vec![true;rows.len()]),Column::new("row_unit".into(),rows.iter().map(|r|profile_row_unit(r.0)).collect::<Vec<_>>()),Column::new("attribution".into(),rows.iter().map(|r|if r.0.starts_with("parser."){"shared_parser_boundary"}else{"direct_integration_including_induced_helpers"}).collect::<Vec<_>>()),Column::new("over_account_seconds".into(),rows.iter().map(|r|(-r.1.0).max(0.)).collect::<Vec<_>>())])}
}

#[cfg(not(any(feature="direct-method-profile",test)))]
#[derive(Default)]struct Profile;
#[cfg(not(any(feature="direct-method-profile",test)))]
impl Profile {
 #[inline(always)]fn add(&mut self,_name:&'static str,_seconds:f64,_rows:usize){}
 #[inline(always)]fn elapsed(&mut self,_name:&'static str,_start:impl ProfileClock,_rows:usize){}
}

struct PositionAnchor {point:[f64;3],life:Option<[Option<f64>;4]>,id:Option<Id>,emitted:Option<f64>,value_observed:Option<f64>,observed:Option<f64>,gap:bool}
impl Clone for PositionAnchor {fn clone(&self)->Self{let _method_span=crate::direct_method_profile::span(46);Self{point:self.point.clone(),life:self.life.clone(),id:self.id.clone(),emitted:self.emitted.clone(),value_observed:self.value_observed.clone(),observed:self.observed.clone(),gap:self.gap.clone()}}}

// Only these primitive fields are normalized by append/position policy.
// Raw projection rows retain original observations between selected ticks.
#[derive(Clone)]
struct RawObservation {position:[Option<f64>;4],times:[Option<f64>;6],live:bool,extras:SharedExtras}
impl RawObservation {
 fn save(node:&Node)->Self{Self{position:[node.values[0],node.values[1],node.values[2],node.values[38]],times:[node.source_time,node.map_time,node.health_time,node.world_time,node.position_observation_time,node.position_emitted_time],live:node.position_observation_live,extras:node.extras.clone()}}
 fn restore(self,node:&mut Node){for(index,value)in [0,1,2,38].into_iter().zip(self.position){node.values[index]=value;}let [source,map,health,world,observed,emitted]=self.times;node.source_time=source;node.map_time=map;node.health_time=health;node.world_time=world;node.position_observation_time=observed;node.position_emitted_time=emitted;node.position_observation_live=self.live;node.extras=self.extras;}
}
#[derive(Default)]
struct Collector {projection_restore:Vec<RawObservation>,projection_cache:projection::ProjectionCache,profile:Profile,zip_homes:zipline_reach::Homes,projection_counts:Vec<(i32,[u64;8])>,rows:rows::EmittedRows,previous:BTreeMap<Scope,Node>,objective_markers:HashSet<Id>,clock_samples:Vec<bool>,clock_terminal:Vec<bool>,clock_ticks:Vec<i32>,clock_seconds:Vec<Option<f64>>,clock_observed_ticks:Vec<Option<i32>>,last_clock:Option<(f64,i32)>,modifiers:boon_parser::ModifierState,modifier_times:HashMap<u32,Option<f64>>,positions:BTreeMap<Scope,PositionAnchor>,last_projection_time:Option<f64>,field_names:FieldNames,camp_positions:BTreeMap<String,([f64;2],Option<f64>)>}
impl Collector {
 fn count_projection(&mut self,tick:i32,nodes:&projection::DenseNodes){let _method_span=crate::direct_method_profile::span(47);let start=FineInstant::now();let mut counts=[0u64;8];counts[0]=nodes.len()as u64;for node in nodes.values(){let index=match node.audience{0=>1,1=>2,3=>3,2 if node.observer==Some(2)=>4,2 if node.observer==Some(3)=>5,_=>6};counts[index]+=1;if node.audience==0&&node.kind=="player"{counts[7]+=1;}}self.projection_counts.push((tick,counts));self.profile.elapsed("projection.channel_count_receipt",start,nodes.len());}
 #[cfg(test)]
 fn position_policy(&mut self,time:Option<f64>,current:&mut BTreeMap<Scope,Node>){let _method_span=crate::direct_method_profile::span(48);
  let gap=match(self.last_projection_time,time){(Some(old),Some(now))=>now-old>0.375,_=>false};
  if gap{for anchor in self.positions.values_mut(){anchor.gap=true;}}
  if time.is_some(){self.last_projection_time=time;}
  let lives:HashMap<String,[Option<f64>;4]>=current.values().filter(|node|node.audience==0).map(|node|(node.key.clone(),["is_alive","lifestate","last_spawn_source_time","respawn_source_time"].map(|name|node.get(name)))).collect();
  let mut present=HashSet::new();
  for(scope,node)in current.iter_mut(){
   let values=[node.get("world_x"),node.get("world_y"),node.get("world_z")];
   if !values.iter().all(Option::is_some){self.positions.remove(scope);continue}
   let point=values.map(Option::unwrap);present.insert(scope.clone());
   let life=lives.get(&node.key).copied();
   let fresh=node.get("world_age")==Some(0.)&&node.get("visible")!=Some(0.);
   let observed=if fresh{time}else{node.position_observation_time.or(node.world_time)};
   let anchor=match self.positions.entry(scope.clone()){
    std::collections::btree_map::Entry::Vacant(entry)=>entry.insert(PositionAnchor{point,life,id:node.id,emitted:observed,value_observed:observed,observed,gap:!fresh}),
    std::collections::btree_map::Entry::Occupied(entry)=>{
     let anchor=entry.into_mut();let emit=anchor.life!=life||anchor.id!=node.id||anchor.gap||anchor.point.iter().zip(point).map(|(a,b)|(a-b)*(a-b)).sum::<f64>()>4.;
     if fresh&&emit{*anchor=PositionAnchor{point,life,id:node.id,emitted:observed,value_observed:observed,observed,gap:false};}
     else{if fresh{anchor.observed=observed;if point==anchor.point{anchor.value_observed=observed;}}anchor.gap=!fresh;}
     anchor
    }
   };
   node.xyz(Some(anchor.point));node.world_time=anchor.value_observed;node.position_observation_time=anchor.observed;node.position_observation_live=fresh;node.position_emitted_time=anchor.emitted;
   node.set("world_age",if fresh&&anchor.value_observed==time{Some(0.)}else{None});
   if fresh&&anchor.observed.is_some(){node.extras.insert("position-observation-age".into(),0.);}else{node.extras.remove("position-observation-age");}
  }
  self.positions.retain(|scope,_|present.contains(scope));
 }
 fn position_policy_dense(&mut self,time:Option<f64>,current:&mut projection::DenseNodes){let _method_span=crate::direct_method_profile::span(48);
  let gap=match(self.last_projection_time,time){(Some(old),Some(now))=>now-old>0.375,_=>false};
  if gap{for anchor in self.positions.values_mut(){anchor.gap=true;}}
  if time.is_some(){self.last_projection_time=time;}
  let lives:HashMap<String,[Option<f64>;4]>=current.values().filter(|node|node.audience==0).map(|node|(node.key.clone(),["is_alive","lifestate","last_spawn_source_time","respawn_source_time"].map(|name|node.get(name)))).collect();
  let mut present=HashSet::new();
  current.for_each_mut(|scope,node|{
   let values=[node.get("world_x"),node.get("world_y"),node.get("world_z")];
   if !values.iter().all(Option::is_some){self.positions.remove(scope);return}
   let point=values.map(Option::unwrap);present.insert(scope.clone());
   let life=lives.get(&node.key).copied();
   let fresh=node.get("world_age")==Some(0.)&&node.get("visible")!=Some(0.);
   let observed=if fresh{time}else{node.position_observation_time.or(node.world_time)};
   let anchor=match self.positions.entry(scope.clone()){
    std::collections::btree_map::Entry::Vacant(entry)=>entry.insert(PositionAnchor{point,life,id:node.id,emitted:observed,value_observed:observed,observed,gap:!fresh}),
    std::collections::btree_map::Entry::Occupied(entry)=>{
     let anchor=entry.into_mut();let emit=anchor.life!=life||anchor.id!=node.id||anchor.gap||anchor.point.iter().zip(point).map(|(a,b)|(a-b)*(a-b)).sum::<f64>()>4.;
     if fresh&&emit{*anchor=PositionAnchor{point,life,id:node.id,emitted:observed,value_observed:observed,observed,gap:false};}
     else{if fresh{anchor.observed=observed;if point==anchor.point{anchor.value_observed=observed;}}anchor.gap=!fresh;}
     anchor
    }
   };
   node.xyz(Some(anchor.point));node.world_time=anchor.value_observed;node.position_observation_time=anchor.observed;node.position_observation_live=fresh;node.position_emitted_time=anchor.emitted;
   node.set("world_age",if fresh&&anchor.value_observed==time{Some(0.)}else{None});
   if fresh&&anchor.observed.is_some(){node.extras.insert("position-observation-age".into(),0.);}else{node.extras.remove("position-observation-age");}
  });
  self.positions.retain(|scope,_|present.contains(scope));
 }
 #[cfg(test)]
 fn append(&mut self,tick:i32,time:Option<f64>,mut current:BTreeMap<Scope,Node>){let _method_span=crate::direct_method_profile::span(49);
  let started=FineInstant::now();self.position_policy(time,&mut current);self.profile.elapsed("append.position_policy",started,current.len());
  for(scope,node)in &mut current{
   let started=FineInstant::now();let copied=0.;let explicit_source=node.source_time;let modifier=node.class.as_deref().is_some_and(|c|c.starts_with("modifier:"));
   let changed=if let Some(old)=self.previous.get(scope).filter(|old|old.id==node.id&&old.owner==node.owner&&old.hero==node.hero&&old.team==node.team){
    node.map_time=if node.get("map_age")==Some(0.){time}else{node.map_time.or(old.map_time)};
    node.health_time=if node.get("health_age")==Some(0.){time}else{node.health_time.or(old.health_time)};
    node.world_time=if node.get("world_age")==Some(0.){time}else{node.world_time.or(old.world_time)};
    node.source_time=if modifier{explicit_source}else{explicit_source.or(old.source_time)};
    !node.same_observation_content(old)
   }else{node.source_time=if modifier{explicit_source}else{explicit_source.or(time)};if node.get("map_age")==Some(0.){node.map_time=time};if node.get("health_age")==Some(0.){node.health_time=time};if node.get("world_age")==Some(0.){node.world_time=time};true};
   if changed{node.source_time=if modifier{explicit_source}else{explicit_source.or(time)};}
   self.profile.add("append.comparison",started.elapsed().as_secs_f64()-copied,1);
   if copied>0.{self.profile.add("append.comparable_copy",copied,1);}
   if changed{let copy=FineInstant::now();self.rows.push((tick,time,node.clone()));self.profile.elapsed("append.output_copy",copy,1);}
  }
  let _lifecycle_method=crate::direct_method_profile::span(86);let copy=FineInstant::now();let before=self.rows.len();
  for(scope,mut n)in std::mem::take(&mut self.previous){if !current.contains_key(&scope){n.active=false;n.values=[None;66];n.extras.clear();n.relations.clear();n.source_time=time;self.rows.push((tick,time,n));}}
  self.previous=current;self.profile.elapsed("append.lifecycle_and_replacement",copy,self.rows.len()-before);
 }
 // Runtime path: persistent raw projection and normalized previous records.
 // Unchanged observations refresh their timestamps without cloning a Node.
 fn append_projected(&mut self,tick:i32,time:Option<f64>,mut current:projection::DenseNodes){let _method_span=crate::direct_method_profile::span(49);
  let mut restore=std::mem::take(&mut self.projection_restore);restore.clear();restore.extend(current.values().map(RawObservation::save));
  let started=FineInstant::now();self.position_policy_dense(time,&mut current);self.profile.elapsed("append.position_policy",started,current.len());
  current.for_each_mut(|scope,node|{
   let started=FineInstant::now();let explicit_source=node.source_time;let modifier=node.class.as_deref().is_some_and(|c|c.starts_with("modifier:"));
   let changed=if let Some(old)=self.previous.get(scope).filter(|old|old.id==node.id&&old.owner==node.owner&&old.hero==node.hero&&old.team==node.team){
    node.map_time=if node.get("map_age")==Some(0.){time}else{node.map_time.or(old.map_time)};
    node.health_time=if node.get("health_age")==Some(0.){time}else{node.health_time.or(old.health_time)};
    node.world_time=if node.get("world_age")==Some(0.){time}else{node.world_time.or(old.world_time)};
    node.source_time=if modifier{explicit_source}else{explicit_source.or(old.source_time)};
    !node.same_observation_content(old)
   }else{node.source_time=if modifier{explicit_source}else{explicit_source.or(time)};if node.get("map_age")==Some(0.){node.map_time=time};if node.get("health_age")==Some(0.){node.health_time=time};if node.get("world_age")==Some(0.){node.world_time=time};true};
   self.profile.elapsed("append.comparison",started,1);
   if changed{
    node.source_time=if modifier{explicit_source}else{explicit_source.or(time)};
    let copy=FineInstant::now();self.rows.push((tick,time,node.clone()));self.profile.elapsed("append.output_copy",copy,1);
    self.previous.insert(scope.clone(),node.clone());
   }else if let Some(old)=self.previous.get_mut(scope){// Equality intentionally treats signed zeros as equal, but the original
    // whole-map replacement retained the latest bits for later remembered rows.
    old.values=node.values;old.extras=node.extras.clone();
    old.source_time=node.source_time;old.map_time=node.map_time;old.health_time=node.health_time;old.world_time=node.world_time;old.position_observation_time=node.position_observation_time;old.position_emitted_time=node.position_emitted_time;}
  });
  let _lifecycle_method=crate::direct_method_profile::span(86);let started=FineInstant::now();let before=self.rows.len();let removed:Vec<_>=self.previous.keys().filter(|scope|!current.contains_key(*scope)).cloned().collect();
  for scope in removed{let mut node=self.previous.remove(&scope).unwrap();node.active=false;node.values=[None;66];node.extras.clear();node.relations.clear();node.source_time=time;self.rows.push((tick,time,node));}
  self.profile.elapsed("append.lifecycle_and_replacement",started,self.rows.len()-before);
  let mut saved=restore.iter().cloned();current.for_each_mut(|_,node|saved.next().expect("raw restoration count").restore(node));
  restore.clear();self.projection_restore=restore;self.projection_cache.output=current;
 }
 fn frames(self)->PolarsResult<Vec<(&'static str,DataFrame)>>{let _method_span=crate::direct_method_profile::span(50);
  let mut count_cols=vec![Column::new("tick".into(),self.projection_counts.iter().map(|r|r.0).collect::<Vec<_>>())];
  for(index,name)in ["total_nodes","public_nodes","owner_nodes","team_private_nodes","observer_team2_nodes","observer_team3_nodes","other_scope_nodes","public_player_nodes"].iter().enumerate(){count_cols.push(Column::new((*name).into(),self.projection_counts.iter().map(|r|r.1[index]).collect::<Vec<_>>()));}
  let count_frame=df_from_columns(count_cols)?;let(updates,extras,relations)=self.rows.frames()?;
  Ok(vec![("clock",df_from_columns(vec![Column::new("tick".into(),self.clock_ticks),Column::new("is_sample".into(),self.clock_samples),Column::new("is_postgame".into(),self.clock_terminal),Column::new("match_seconds".into(),self.clock_seconds),Column::new("clock_observed_tick".into(),self.clock_observed_ticks)])?),("entity_updates",updates),("extras",extras),("relations",relations),("projection_counts",count_frame)])
 }

}

// Linux ru_maxrss is KiB and ru_utime/ru_stime are process CPU seconds.
// Unsupported platforms or syscall errors remain null, never zero-filled.
fn process_usage()->(Option<f64>,Option<u64>){let _method_span=crate::direct_method_profile::span(51);
 #[cfg(target_os="linux")]{
  let mut usage=std::mem::MaybeUninit::<libc::rusage>::uninit();
  if unsafe{libc::getrusage(libc::RUSAGE_SELF,usage.as_mut_ptr())}!=0{return (None,None)}
  let usage=unsafe{usage.assume_init()};let cpu=usage.ru_utime.tv_sec as f64+usage.ru_utime.tv_usec as f64/1_000_000.+usage.ru_stime.tv_sec as f64+usage.ru_stime.tv_usec as f64/1_000_000.;
  return (cpu.is_finite().then_some(cpu),u64::try_from(usage.ru_maxrss).ok());
 }
 #[cfg(not(target_os="linux"))]{(None,None)}
}
#[derive(Default)]
struct Diagnostics {source_ticks:u64,selected_ticks:u64,projection_seconds:f64,bindings_seconds:f64,clock_seconds:f64,modifier_seconds:f64,usage_seconds:f64,debris_seconds:f64,scan_seconds:f64,frames_seconds:f64,scan_cpu_seconds:Option<f64>,process_cpu_seconds:Option<f64>,process_peak_rss_kib:Option<u64>,callback_seconds:f64}
impl Diagnostics {
 fn frame(mut self)->PolarsResult<DataFrame>{let _method_span=crate::direct_method_profile::span(52);
  #[cfg(not(feature="direct-method-profile"))]
  return df_from_columns(vec![Column::new("source_ticks".into(),[self.source_ticks]),Column::new("selected_ticks".into(),[self.selected_ticks]),Column::new("profiling_enabled".into(),[false])]);
  #[cfg(feature="direct-method-profile")]
  { (self.process_cpu_seconds,self.process_peak_rss_kib)=process_usage();
  macro_rules! c {($name:ident)=>{Column::new(stringify!($name).into(),vec![self.$name])};}
  df_from_columns(vec![c!(source_ticks),c!(selected_ticks),c!(projection_seconds),c!(bindings_seconds),c!(clock_seconds),c!(modifier_seconds),c!(usage_seconds),c!(debris_seconds),c!(scan_seconds),c!(frames_seconds),c!(scan_cpu_seconds),c!(process_cpu_seconds),c!(process_peak_rss_kib),c!(callback_seconds)])
  }
 }
}

#[path="direct_model_visibility.rs"] mod visibility;
pub(super) use visibility::ModelEntityView;
#[path="direct_model_rows.rs"] mod rows;
#[path="direct_model_projection.rs"] mod projection;
#[path="direct_zipline_reach.rs"] mod zipline_reach;
#[path="direct_ability_transitions.rs"] mod ability_transitions;

#[pymethods]
impl Demo {
 /// Bounded instrumentation-overhead calibration, with no replay parse.
 #[pyo3(signature=(*,iterations=100000))]
 pub(crate) fn direct_method_profile_probe(&self,iterations:usize)->PyResult<PyDataFrame>{
  if iterations==0||iterations>1_000_000{return Err(pyo3::exceptions::PyValueError::new_err("iterations must be 1..1000000"))}
  if !crate::direct_method_profile::AVAILABLE{return Err(pyo3::exceptions::PyValueError::new_err("method profiling requires an artifact built with direct-method-profile"))}
  crate::direct_method_profile::probe(iterations).map(PyDataFrame).map_err(|e|InvalidDemoError::new_err(e.to_string()))
 }
 /// Exact replay client patch parsed from the header directory. This is not
 /// the engine build number. Missing or conflicting client paths stay unknown.
 pub(crate) fn direct_client_version(&self)->PyResult<Option<String>>{let _method_span=crate::direct_method_profile::span(53);
  let header=self.parser.file_header().map_err(to_py_err)?;
  let Some(directory)=header.game_directory else{return Ok(None)};
  let mut versions=std::collections::BTreeSet::new();
  for part in directory.split(['/', '\\']){if let Some(version)=part.strip_prefix("citadel_v"){if !version.is_empty()&&version.bytes().all(|c|c.is_ascii_digit()){versions.insert(version.to_owned());}}}
  Ok((versions.len()==1).then(||versions.into_iter().next().unwrap()))
 }
 /// Full-null replacement columns for each stable node/audience/observer scope.
 /// Public=0, owner-only=1, observed-team=2, own/allied ability state=3.
 /// Only requested completed source ticks are admitted. Repeated states are
 /// change-compressed. Supply a carry-in sample before the window start.
 /// Match seconds are the CURRENT replicated HUD clock; never future game-over.
 #[pyo3(signature=(*,ticks,entity_classes=None,method_profile=false))]
 pub(crate) fn direct_model_columns(&self,py:Python<'_>,ticks:Vec<i32>,entity_classes:Option<HashMap<i64,String>>,method_profile:bool)->PyResult<Py<PyDict>>{
  collect_model_columns(py,ticks,entity_classes,method_profile,StandaloneModelSource(&self.parser))
 }
}


/// A source pass supplies the same completed-tick context to every consumer.
/// Companion collectors return their exclusive setup, callback, and flush time.
pub(super) trait ModelSourcePass: Send {
 fn initialize(&mut self)->boon_parser::Result<Context>;
 fn scan(&mut self,classes:&HashSet<&str>,events:&HashSet<u32>,visit:&mut ModelSourceCallback<'_>)->PyResult<f64>;
 fn collect_without_model(&mut self)->PyResult<()> {Ok(())}
}
pub(super) type ModelSourceCallback<'a>=dyn FnMut(&Context,&[boon_parser::GameEvent])->boon_parser::Result<()>+Send+'a;
struct StandaloneModelSource<'a>(&'a boon_parser::Parser);
impl ModelSourcePass for StandaloneModelSource<'_> {
 fn initialize(&mut self)->boon_parser::Result<Context>{self.0.parse_init()}
 fn scan(&mut self,classes:&HashSet<&str>,events:&HashSet<u32>,visit:&mut ModelSourceCallback<'_>)->PyResult<f64>{
  self.0.try_run_to_end_with_event_types_filtered(classes,events,visit).map_err(to_py_err)?;Ok(0.)
 }
}
pub(super) fn collect_model_columns<S:ModelSourcePass>(py:Python<'_>,ticks:Vec<i32>,entity_classes:Option<HashMap<i64,String>>,method_profile:bool,mut source:S)->PyResult<Py<PyDict>>{
  if method_profile&&!crate::direct_method_profile::AVAILABLE{return Err(pyo3::exceptions::PyValueError::new_err("method profiling requires an artifact built with direct-method-profile"))}

  let frames=py.detach(|| -> PyResult<Vec<(&'static str,DataFrame)>> {
   let method_session=crate::direct_method_profile::Session::begin(method_profile);
   let result=(|| -> PyResult<Vec<(&'static str,DataFrame)>> {let _method_span=crate::direct_method_profile::span(54);
   let _field_keys=FieldKeyScope::enter();
   let selected:HashSet<i32>=ticks.into_iter().collect();let mut out=Collector::default();let mut diagnostics=Diagnostics::default();if selected.is_empty(){source.collect_without_model()?;let mut frames=out.frames().map_err(|e|InvalidDemoError::new_err(e.to_string()))?;frames.push(("player_bindings",Bindings::default().frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));frames.push(("breakable_messages",crate::direct_breakable_messages::DebrisColumns::default().frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));frames.push(("ability_usage",ability_transitions::AbilityTransitions::default().frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));#[cfg(feature="direct-method-profile")]frames.push(("profile",Profile::default().frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));frames.push(("diagnostics",diagnostics.frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));return Ok(frames);}
   let classes=entity_classes.unwrap_or_default();let mut error=None;let mut check=std::time::Instant::now();
   let initial=FineInstant::now();let init={let _span=crate::direct_method_profile::span(80);source.initialize().map_err(to_py_err)?};#[cfg(any(feature="direct-method-profile",test))]{out.profile.elapsed("parser.initial_parse",initial,1);};let plan=FineInstant::now();
   let class_plan_span=crate::direct_method_profile::span(85);let names:Vec<String>=init.serializers().iter().filter(|(name,s)|kind(name)!="unknown"||name.contains("Ability")||name.contains("Item")||{let _span=crate::direct_method_profile::span(87);s.resolve_field_key("m_flAmmoFrac")}.is_some()||matches!(name.as_ref(),"CCitadelPlayerController"|"CCitadelTeam"|"CCitadelGameRulesProxy")).map(|(name,_)|name.to_string()).collect();
   drop(class_plan_span);drop(init);let mut model_entities=ModelEntityView::restricted(names.iter().map(String::as_str));let filter=names.iter().map(String::as_str).collect();#[cfg(any(feature="direct-method-profile",test))]{out.profile.elapsed("integration.class_filter_plan",plan,names.len());};
   let event_types=HashSet::from([Msg::KEUserMsgGameOver as u32,crate::direct_breakable_messages::MESSAGE_TYPE]);let mut ended=false;let mut debris=crate::direct_breakable_messages::DebrisColumns::default();let mut bindings=Bindings::default();let mut usage=ability_transitions::AbilityTransitions::default();
   let mut schema=SchemaGuard::default();let mut anchor:Option<(f64,i64,i64)>=None;let scan_start=FineInstant::now();#[cfg(feature="direct-method-profile")]let scan_cpu_start=process_usage().0;#[cfg(feature="direct-method-profile")]let progress=std::env::var("BOON_DIRECT_PROGRESS").as_deref()==Ok("1");
   // Native source-time event dispatch establishes postgame causally. No
   // pre-scanned future end tick is used to convert earlier input clocks.
   let scan_method=crate::direct_method_profile::span(81);let shared_collector_seconds=source.scan(&filter,&event_types,&mut |ctx,events|{let _callback_method=crate::direct_method_profile::span(82);
    let callback=FineInstant::now();if check.elapsed().as_millis()>=100{if let Err(e)=Python::attach(|py|py.check_signals()){error=Some(e);return Err(crate::bounded_spool::aborted())}check=std::time::Instant::now();}
    diagnostics.source_ticks+=1;if schema.refresh(ctx){model_entities.invalidate_schema();reset_getter_plans();out.field_names=FieldNames::default();out.projection_cache.invalidate_schema();usage.invalidate_schema();}out.field_names.observe(ctx);observe_candidates(ctx);out.projection_cache.observe(ctx,&mut model_entities);
    let stage=FineInstant::now();let modifier_changes={let _span=crate::direct_method_profile::span(83);out.modifiers.update(ctx)};let elapsed=stage.elapsed().as_secs_f64();diagnostics.modifier_seconds+=elapsed;#[cfg(any(feature="direct-method-profile",test))]{out.profile.add("integration.modifier_updates",elapsed,modifier_changes.len());};
    let stage=FineInstant::now();let before=bindings.rows.len();bindings.collect(ctx);let elapsed=stage.elapsed().as_secs_f64();diagnostics.bindings_seconds+=elapsed;#[cfg(any(feature="direct-method-profile",test))]{out.profile.add("integration.source_bindings",elapsed,bindings.rows.len()-before);};
    let stage=FineInstant::now();debris.collect_with_visibility(ctx,events,&mut model_entities)?;let elapsed=stage.elapsed().as_secs_f64();diagnostics.debris_seconds+=elapsed;#[cfg(any(feature="direct-method-profile",test))]{out.profile.add("integration.debris_messages",elapsed,events.iter().filter(|e|e.msg_type==crate::direct_breakable_messages::MESSAGE_TYPE).count());};
    let stage=FineInstant::now();
    let clock_method=crate::direct_method_profile::span(84);let rules:Vec<_>=class_candidates(ctx,"CCitadelGameRulesProxy").into_iter().filter(|e|e.active).collect();
    if events.iter().any(|event|event.msg_type==Msg::KEUserMsgGameOver as u32){ended=true;}
    let time=if rules.len()==1{
     let rule=rules[0];let raw=n(ctx,rule,"m_pGameRules.m_flMatchClockAtLastUpdate");
     let anchor_tick=i(ctx,rule,"m_pGameRules.m_nMatchClockUpdateTick");
     let paused=i(ctx,rule,"m_pGameRules.m_nTotalPausedTicks");
     match(raw,anchor_tick,paused){
      (Some(seconds),Some(server_anchor),Some(paused))=>{
       if anchor.is_none_or(|(s,t,_)|s!=seconds||t!=server_anchor){
        // The initial paused-counter baseline is unqualified if nonzero.
        // A newly observed anchor is qualified with its contemporaneous count.
        if anchor.is_some()||paused==0 {anchor=Some((seconds,server_anchor,paused));out.last_clock=Some((seconds,ctx.tick()));}
       }
       if ended{Some(seconds)}else if b(ctx,rule,"m_pGameRules.m_bGamePaused")==Some(true)||b(ctx,rule,"m_pGameRules.m_bServerPaused")==Some(true){None}else{
        let base=class_candidates(ctx,"CCitadelPlayerController").into_iter().filter(|e|e.active)
         .filter(|e|["m_hPawn","m_hHeroPawn"].iter().any(|name|resolved(ctx,e,name).is_some_and(|p|p.active&&p.class_name.as_ref()=="CCitadelPlayerPawn")))
         .filter_map(|e|i(ctx,e,"m_nTickBase")).max();
        match(anchor,base){(Some((s,t,p)),Some(base)) if paused>=p&&base>=t=>{let result=s+(base-t-(paused-p))as f64*f64::from(ctx.tick_interval());result.is_finite().then_some(result)},_=>None}
       }
      },_=>None
     }
    }else{None};
    let elapsed=stage.elapsed().as_secs_f64();drop(clock_method);diagnostics.clock_seconds+=elapsed;#[cfg(any(feature="direct-method-profile",test))]{out.profile.add("integration.source_clock",elapsed,1);};let stage=FineInstant::now();let before=usage.len();usage.collect(ctx,time,&bindings,ended);let elapsed=stage.elapsed().as_secs_f64();diagnostics.usage_seconds+=elapsed;#[cfg(any(feature="direct-method-profile",test))]{out.profile.add("integration.ability_transitions",elapsed,usage.len()-before);};
    for change in modifier_changes{out.projection_cache.modifier_changed(change.serial);if change.kind==boon_parser::ModifierChangeKind::Removed{out.modifier_times.remove(&change.serial);}else{out.modifier_times.insert(change.serial,time);}}
    let sample=selected.contains(&ctx.tick());out.clock_ticks.push(ctx.tick());out.clock_samples.push(sample);out.clock_terminal.push(ended);out.clock_seconds.push(time);out.clock_observed_ticks.push(time.and(out.last_clock.map(|v|v.1)));
    if sample&&!ended{diagnostics.selected_ticks+=1;let stage=FineInstant::now();let nodes=out.project(ctx,time,&classes,&mut model_entities);out.count_projection(ctx.tick(),&nodes);out.append_projected(ctx.tick(),time,nodes);diagnostics.projection_seconds+=stage.elapsed().as_secs_f64();}
    #[cfg(feature="direct-method-profile")]if progress&&diagnostics.source_ticks%10_000==0{let(cpu,rss)=process_usage();let scan_cpu=cpu.zip(scan_cpu_start).map(|(now,start)|now-start);eprintln!("boon_direct tick={} elapsed={:.3}s selected={} state_rows={} projection={:.3}s usage={:.3}s scan_cpu_seconds={:?} process_cpu_seconds={:?} process_peak_rss_kib={:?}",ctx.tick(),scan_start.elapsed().as_secs_f64(),diagnostics.selected_ticks,out.rows.len(),diagnostics.projection_seconds,diagnostics.usage_seconds,scan_cpu,cpu,rss);}
    diagnostics.callback_seconds+=callback.elapsed().as_secs_f64();Ok(())
   }).map_err(|e|error.take().unwrap_or(e))?;
   drop(scan_method);diagnostics.scan_seconds=scan_start.elapsed().as_secs_f64();#[cfg(feature="direct-method-profile")]{diagnostics.scan_cpu_seconds=process_usage().0.zip(scan_cpu_start).map(|(now,start)|now-start);}let mut profile=std::mem::take(&mut out.profile);let frame_rows=out.rows.len();let stage=FineInstant::now();let mut frames=out.frames().map_err(|e|InvalidDemoError::new_err(e.to_string()))?;frames.push(("breakable_messages",debris.frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));frames.push(("player_bindings",bindings.frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));frames.push(("ability_usage",usage.frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));diagnostics.frames_seconds=stage.elapsed().as_secs_f64();
   #[cfg(any(feature="direct-method-profile",test))]let projection_detail=profile.sum_prefix("projection.")+profile.sum_prefix("append.");
   #[cfg(any(feature="direct-method-profile",test))]{profile.summary("integration.projection_unresolved",diagnostics.projection_seconds-projection_detail,diagnostics.selected_ticks,diagnostics.selected_ticks);};

   #[cfg(any(feature="direct-method-profile",test))]let qualified=diagnostics.projection_seconds+diagnostics.bindings_seconds+diagnostics.clock_seconds+diagnostics.modifier_seconds+diagnostics.usage_seconds+diagnostics.debris_seconds;
   #[cfg(any(feature="direct-method-profile",test))]{profile.summary("integration.callback_unresolved",diagnostics.callback_seconds-qualified,diagnostics.source_ticks,diagnostics.source_ticks);};
   #[cfg(any(feature="direct-method-profile",test))]{profile.add("parser.scan_dispatch_setup_shared_residual",diagnostics.scan_seconds-diagnostics.callback_seconds-shared_collector_seconds,diagnostics.source_ticks as usize);};
   if shared_collector_seconds!=0. {#[cfg(any(feature="direct-method-profile",test))]{profile.add("shared_pass.recorded_event_collectors",shared_collector_seconds,diagnostics.source_ticks as usize);};}
   #[cfg(any(feature="direct-method-profile",test))]{profile.add("integration.dataframe_builds",diagnostics.frames_seconds,frame_rows);};
   #[cfg(feature="direct-method-profile")]frames.push(("profile",profile.frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));frames.push(("diagnostics",diagnostics.frame().map_err(|e|InvalidDemoError::new_err(e.to_string()))?));Ok(frames)
   })();
   let mut frames=result?;frames.extend(method_session.finish_frames().map_err(|e|InvalidDemoError::new_err(e.to_string()))?);Ok(frames)
  })?;
  let dict=PyDict::new(py);for(name,frame)in frames{dict.set_item(name,PyDataFrame(frame))?;}Ok(dict.unbind())
}

#[cfg(test)]
mod tests {
 use super::*;
 #[test]
 fn schema_guard_detects_replacements_but_preserves_initial_bootstrap_and_clones(){
  use boon_parser::entity::{SerializerContainer,ClassInfo,ClassEntry,FlattenedSerializer,FlattenedSerializerDefinition,FlattenedField,DecodeProfile,BareCharEncoding,PreciseQAngleMode};
  let serializers=||SerializerContainer::parse(FlattenedSerializer::new(vec![FlattenedSerializerDefinition::new(Some(0),vec![0])],vec!["Test".into(),"uint32".into(),"value".into()],vec![FlattenedField::new(Some(1),Some(2))]),DecodeProfile::new(BareCharEncoding::NullTerminatedString,PreciseQAngleMode::Centered)).unwrap();
  let classes=||ClassInfo::try_from_entries([ClassEntry::new(0,String::from("Test"),"")]).unwrap();
  let mut guard=SchemaGuard::default();let first=serializers();let names=classes();
  assert!(!guard.refresh_parts(&first,&names));assert!(!guard.refresh_parts(&first.clone(),&names.clone()));
  let next=serializers();assert!(guard.refresh_parts(&next,&names));assert!(!guard.refresh_parts(&next,&names));
  let remapped=classes();assert!(guard.refresh_parts(&next,&remapped));assert!(!guard.refresh_parts(&next,&remapped));
  assert!(guard.refresh_parts(&SerializerContainer::default(),&ClassInfo::empty()));assert!(!guard.refresh_parts(&SerializerContainer::default(),&ClassInfo::empty()));
 }
 #[test]
 fn schema_reset_discards_positive_and_negative_getter_plans(){
  let _scope=FieldKeyScope::enter();assert_eq!(cached_field_key(1,"m_iHealth",||None),None);assert_eq!(cached_field_key(2,"m_iHealth",||Some(3)),Some(3));
  reset_getter_plans();assert_eq!(cached_field_key(1,"m_iHealth",||Some(9)),Some(9));assert_eq!(cached_field_key(2,"m_iHealth",||None),None);
 }
 #[test]
 fn persistent_append_matches_reference_and_restores_raw_observations(){
  let mut reference=Collector::default();let mut persistent=Collector::default();
  for tick in 0..128{
   let time=if tick%17==0{None}else{Some((tick as f64)*0.25)};
   let mut nodes=BTreeMap::new();
   for index in 0..4{if tick%13==index+1{continue}
    let mut node=Node::new(format!("node:{index}"),"test",if index==0{0}else{2});node.id=Some((index,tick as u32/41));node.observer=Some(2);node.owner=Some(index as u64);node.team=Some(2);node.hero=Some((tick/37)as i64);
    node.xyz(Some([tick as f64*0.4+index as f64,2.,3.]));node.set("health",Some((tick/9)as f64));node.set("health_age",Some(0.));node.set("visible",Some(if tick%7<4{1.}else{0.}));node.map_time=Some(-1.);node.position_observation_time=Some(-2.);
    if tick%7>=4{node.set("world_age",None);node.set("health_age",None);}
    if index==3{node.class=Some("modifier:1".into());node.source_time=if tick%3==0{None}else{Some((tick/11)as f64)};}
    nodes.insert(node.scope(),node);
   }
   let oracle:BTreeMap<_,_>=nodes.iter().map(|(scope,node)|(scope.clone(),node.clone())).collect();
   reference.append(tick,time,oracle);persistent.append_projected(tick,time,nodes.into());
   assert!(reference.previous==persistent.previous,"previous differs at tick {tick}");
   assert_eq!(reference.rows.len(),persistent.rows.len());
   assert!(reference.rows.iter().zip(persistent.rows.iter()).all(|(a,b)|a.0==b.0&&a.1==b.1&&a.2==b.2),"emitted rows differ at tick {tick}");
   // Raw node coordinates/time evidence survive position anchoring unchanged.
   for node in persistent.projection_cache.output.values(){assert_eq!(node.get("world_x"),Some(tick as f64*0.4+node.id.unwrap().0 as f64));assert_eq!(node.map_time,Some(-1.));assert_eq!(node.position_observation_time,Some(-2.));assert!(!node.extras.contains_key("position-observation-age"));}
  }
 }
 #[test]
 fn binding_dependency_plan_covers_exact_qualification_fields(){
  for field in ["m_steamID","m_hPawn","m_hHeroPawn","m_iTeamNum","m_CCitadelHeroComponent.m_spawnedHero.m_nHeroID"]{assert!(binding_field(field));}
  for field in ["m_nTickBase","m_PlayerDataGlobal.m_iHealthMax","m_iHealth","m_vecFOWEntities"]{assert!(!binding_field(field));}
 }
 #[test]
 fn prebound_pair_falls_back_on_missing_value_not_only_missing_key(){
  let fields=[(3,FieldValue::I32(9)),(5,FieldValue::Bool(false))].into_iter().collect();let entity=Entity::from_fields(1,2,3,"test",true,fields).unwrap();
  assert_eq!(integer(planned_pair_value(&entity,(Some(0),Some(3)))),Some(9));assert_eq!(integer(planned_pair_value(&entity,(None,Some(3)))),Some(9));assert_eq!(boolean(planned_pair_value(&entity,(Some(5),Some(3)))),Some(false));assert!(planned_pair_value(&entity,(Some(0),None)).is_none());
 }
 #[test]
 fn fixed_field_plans_cache_resolved_absence_and_remain_call_scoped(){
  {let _scope=FieldKeyScope::enter();assert_eq!(cached_field_key(1,"m_iHealth",||Some(0)),Some(0));assert_eq!(cached_field_key(1,"m_iHealth",||panic!("re-resolved fixed key")),Some(0));assert_eq!(cached_field_key(2,"m_iHealth",||None),None);assert_eq!(cached_field_key(2,"m_iHealth",||panic!("re-resolved fixed absence")),None);}
  let _scope=FieldKeyScope::enter();assert_eq!(cached_field_key(1,"m_iHealth",||Some(8)),Some(8));
 }
 #[test]
 fn indexed_features_preserve_names_order_missing_and_finite_contract(){
  let mut node=Node::new("indexed".into(),"test",0);
  for(index,name)in FIELDS.iter().enumerate(){assert_eq!(feature_index(name),Some(index));node.set(name,Some(index as f64));assert_eq!(node.values[index],Some(index as f64));assert_eq!(node.get(name),Some(index as f64));}
  assert_eq!(feature_index("missing"),None);node.set("missing",Some(9.));node.set("ammo",Some(f64::NAN));assert_eq!(node.get("ammo"),None);node.set("ammo",Some(f64::INFINITY));assert_eq!(node.get("ammo"),None);
 }
 #[test]
 fn borrowed_comparison_is_exact_timestamp_normalized_clone_oracle(){
  let mut old=Node::new("compare".into(),"test",2);old.id=Some((1,2));old.observer=Some(2);old.source_time=Some(1.);old.set("health",Some(5.));old.extras.insert("a".into(),1.);old.relations.push(("player:1".into(),1));
  let mut changed=old.clone();changed.map_time=Some(2.);changed.health_time=Some(3.);changed.world_time=Some(4.);changed.position_observation_time=Some(5.);
  let check=|new:&Node|{let mut normalized=old.clone();normalized.map_time=new.map_time;normalized.health_time=new.health_time;normalized.world_time=new.world_time;normalized.position_observation_time=new.position_observation_time;assert_eq!(new.same_observation_content(&old),*new==normalized);};
  check(&changed);assert!(changed.same_observation_content(&old));
  let mut new=changed.clone();new.position_observation_live=true;check(&new);assert!(!new.same_observation_content(&old));
  let mut new=changed.clone();new.position_emitted_time=Some(6.);check(&new);assert!(!new.same_observation_content(&old));
  let mut new=changed.clone();new.source_time=None;check(&new);assert!(!new.same_observation_content(&old));
  let mut new=changed.clone();new.values[0]=Some(9.);check(&new);assert!(!new.same_observation_content(&old));
  let mut new=changed.clone();new.extras.insert("b".into(),2.);check(&new);assert!(!new.same_observation_content(&old));
  let mut new=changed.clone();new.relations.push(("player:2".into(),2));check(&new);assert!(!new.same_observation_content(&old));
 }
 #[test]
 fn normalized_comparison_retains_nan_inequality_and_latest_signed_zero_bits(){
  let old=Node::new("zero".into(),"test",0);let mut nan=old.clone();nan.map_time=Some(f64::NAN);let mut normalized=old.clone();normalized.map_time=nan.map_time;assert!(!(nan==normalized));assert!(!nan.same_observation_content(&old));
  let mut c=Collector::default();let mut first=old;first.values[5]=Some(0.);first.extras.insert("zero".into(),0.);c.append_projected(1,Some(0.),[(first.scope(),first.clone())].into_iter().collect());
  first.values[5]=Some(-0.);first.extras.insert("zero".into(),-0.);c.append_projected(2,Some(1.),[(first.scope(),first)].into_iter().collect());
  assert_eq!(c.rows.len(),1);let saved=c.previous.values().next().unwrap();assert_eq!(saved.values[5].unwrap().to_bits(),(-0_f64).to_bits());assert_eq!(saved.extras["zero"].to_bits(),(-0_f64).to_bits());
 }
 #[test]
 fn borrowed_comparison_preserves_changed_then_sorted_tombstone_order(){
  let mut collector=Collector::default();let a=Node::new("a".into(),"test",0);let b=Node::new("b".into(),"test",0);let c=Node::new("c".into(),"test",0);
  collector.append(1,Some(0.),[a.clone(),b,c.clone()].into_iter().map(|n|(n.scope(),n)).collect());
  let mut changed=c;changed.set("health",Some(1.));collector.append(2,Some(1.),[(changed.scope(),changed)].into_iter().collect());
  let output=&collector.rows[3..];assert_eq!(output.iter().map(|r|r.2.key.as_str()).collect::<Vec<_>>(),["c","a","b"]);assert!(output[0].2.active);assert!(!output[1].2.active);assert!(!output[2].2.active);
 }
 #[test]
 fn profile_and_channel_receipts_count_scopes_without_viewer_copies(){let mut c=Collector::default();let public=Node::new("player:1".into(),"player",0);let mut team=public.clone();team.audience=3;let mut a=public.clone();a.audience=2;a.observer=Some(2);let mut b=a.clone();b.observer=Some(3);let nodes=[public,team,a,b].into_iter().map(|n|(n.scope(),n)).collect();c.count_projection(1,&nodes);assert_eq!(c.projection_counts[0].1,[4,1,0,1,1,1,0,1]);let mut profile=Profile::default();profile.add("projection.test",1.,2);profile.add("projection.test",2.,3);assert_eq!(profile.groups["projection.test"],(3.,2,5));assert_eq!(profile.sum_prefix("projection."),3.);}
 #[test]
 fn getter_keys_are_call_scoped_class_specific_and_preserve_missing(){
  {let _scope=FieldKeyScope::enter();assert_eq!(cached_field_key(1,"value",||Some(0)),Some(0));assert_eq!(cached_field_key(1,"value",||panic!("resolved a cached key")),Some(0));assert_eq!(cached_field_key(2,"value",||Some(8)),Some(8));assert_eq!(cached_field_key(1,"missing",||None),None);assert_eq!(cached_field_key(1,"missing",||panic!("resolved a cached absence")),None);}
  let _scope=FieldKeyScope::enter();assert_eq!(cached_field_key(1,"value",||Some(99)),Some(99));assert_eq!(cached_field_key(1,"missing",||Some(17)),Some(17));
 }
 #[test]
 fn unavailable_is_not_zero_and_float_integers_are_not_handles(){
  assert_eq!(number(None),None);assert_eq!(number(Some(&FieldValue::F32(f32::NAN))),None);
  assert_eq!(number(Some(&FieldValue::U32(0))),Some(0.));assert_eq!(number(Some(&FieldValue::Bool(false))),Some(0.));
  assert_eq!(integer(Some(&FieldValue::F32(1.))),None);assert_eq!(integer(Some(&FieldValue::U64(u64::MAX))),None);
 }
 #[test]
 fn direct_full_replacements_preserve_nulls_scope_and_tombstones(){
  let mut c=Collector::default();let mut a=Node::new("player:1".into(),"player",0);a.owner=Some(1);a.set("health",Some(0.));let mut b=a.clone();b.audience=3;b.values=[None;66];b.set("ammo",Some(0.));
  c.append(1,Some(0.),[(a.scope(),a.clone()),(b.scope(),b.clone())].into_iter().collect());
  c.append(2,Some(1.),[(a.scope(),a.clone()),(b.scope(),b.clone())].into_iter().collect());assert_eq!(c.rows.len(),2);
  c.append(3,Some(2.),[(a.scope(),a)].into_iter().collect());assert_eq!(c.rows.len(),3);assert!(!c.rows[2].2.active);assert!(c.rows[2].2.values.iter().all(Option::is_none));assert_eq!(c.rows[0].2.get("health"),Some(0.));assert_eq!(c.rows[0].2.get("ammo"),None);
  let frames=c.frames().unwrap();let states=&frames.iter().find(|(k,_)|*k=="entity_updates").unwrap().1;assert_eq!(states.height(),3);assert_eq!(states.column("health").unwrap().null_count(),2);assert_eq!(states.column("ammo").unwrap().null_count(),2);
 }
 #[test]
 fn unchanged_visible_observation_is_compressed_but_hidden_keeps_last_observed_clock(){
  let mut c=Collector::default();let mut node=Node::new("entity:2".into(),"neutral",2);node.observer=Some(2);node.set("visible",Some(1.));node.set("map_x",Some(0.));node.set("map_y",Some(5.));node.set("map_age",Some(0.));
  c.append(1,Some(0.),[(node.scope(),node.clone())].into_iter().collect());c.append(2,Some(1.),[(node.scope(),node.clone())].into_iter().collect());assert_eq!(c.rows.len(),1);
  node.set("visible",Some(0.));node.set("map_age",None);c.append(3,Some(2.),[(node.scope(),node)].into_iter().collect());assert_eq!(c.rows.len(),2);assert_eq!(c.rows[1].2.map_time,Some(1.));assert_eq!(c.rows[1].2.get("map_age"),None);
 }
 #[test]
 fn class_map_includes_all_six_public_objective_types(){for class in ["CNPC_TrooperBoss","CNPC_Boss_Tier2","CNPC_Boss_Tier3","CNPC_BarrackBoss","CCitadel_Destroyable_Building","CNPC_MidBoss"]{assert!(objective(class));assert!(!objective_code(class).is_empty())}assert!(!objective("CCitadel_BreakableProp"));}
}

#[cfg(test)]
mod position_tests {
 use super::*;
 #[test]
 fn strict_two_unit_anchor_separates_value_age_and_observation_age(){
  let mut c=Collector::default();let mut node=Node::new("player:1".into(),"player",1);node.id=Some((1,2));node.xyz(Some([0.,0.,0.]));
  c.append(1,Some(0.),[(node.scope(),node.clone())].into_iter().collect());
  node.xyz(Some([1.,0.,0.]));c.append(17,Some(0.25),[(node.scope(),node.clone())].into_iter().collect());
  assert_eq!(c.rows.len(),2);assert_eq!(c.rows[1].2.get("world_x"),Some(0.));assert_eq!(c.rows[1].2.get("world_age"),None);assert_eq!(c.rows[1].2.world_time,Some(0.));assert_eq!(c.rows[1].2.position_observation_time,Some(0.25));assert!(c.rows[1].2.position_observation_live);
  node.xyz(Some([2.,0.,0.]));c.append(33,Some(0.5),[(node.scope(),node.clone())].into_iter().collect());assert_eq!(c.rows.len(),2);
  node.xyz(Some([2.01,0.,0.]));c.append(49,Some(0.75),[(node.scope(),node.clone())].into_iter().collect());assert_eq!(c.rows.len(),3);assert_eq!(c.rows[2].2.get("world_x"),Some(2.01));assert_eq!(c.rows[2].2.get("world_age"),Some(0.));
  c.append(65,Some(1.),[(node.scope(),node.clone())].into_iter().collect());assert_eq!(c.rows.len(),3);assert_eq!(c.previous[&node.scope()].world_time,Some(1.));
 }
 #[test]
 fn qualified_coverage_gap_forces_new_anchor_without_future_information(){
  let mut c=Collector::default();let mut node=Node::new("entity:1".into(),"guardian",0);node.xyz(Some([0.,0.,0.]));c.append(1,Some(0.),[(node.scope(),node.clone())].into_iter().collect());node.xyz(Some([1.,0.,0.]));c.append(64,Some(1.),[(node.scope(),node)].into_iter().collect());assert_eq!(c.rows[1].2.get("world_x"),Some(1.));
 }
}

#[cfg(all(test,not(feature="direct-method-profile")))]
mod production_api_tests {
 use super::*;
 use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
 struct EmptySource(Arc<AtomicUsize>);
 impl ModelSourcePass for EmptySource {
  fn initialize(&mut self)->boon_parser::Result<Context>{panic!("empty/rejected request must not initialize parser")}
  fn scan(&mut self,_classes:&HashSet<&str>,_events:&HashSet<u32>,_visit:&mut ModelSourceCallback<'_>)->PyResult<f64>{panic!("empty/rejected request must not scan parser")}
  fn collect_without_model(&mut self)->PyResult<()>{self.0.fetch_add(1,Ordering::SeqCst);Ok(())}
 }
 #[test]
 fn empty_source_keeps_functional_tables_without_profiling_tables(){
  Python::initialize();Python::attach(|py|{
   let calls=Arc::new(AtomicUsize::new(0));
   let result=collect_model_columns(py,vec![],None,false,EmptySource(calls.clone())).unwrap();
   let tables=result.bind(py);
   for name in ["clock","entity_updates","extras","relations","projection_counts","player_bindings","breakable_messages","ability_usage","diagnostics"]{assert!(tables.contains(name).unwrap(),"missing functional table {name}");}
   for name in ["profile","method_profile","method_inventory","method_profile_diagnostics"]{assert!(!tables.contains(name).unwrap(),"unexpected profiling table {name}");}
   assert_eq!(calls.load(Ordering::SeqCst),1);
   let diagnostics=Diagnostics::default().frame().unwrap();
   assert_eq!(diagnostics.width(),3);
   assert!(diagnostics.column("source_ticks").is_ok());assert!(diagnostics.column("selected_ticks").is_ok());
   assert_eq!(diagnostics.column("profiling_enabled").unwrap().bool().unwrap().get(0),Some(false));
  });
 }
 #[test]
 fn profiling_request_is_rejected_before_initialization_or_empty_collection(){
  Python::initialize();Python::attach(|py|{
   let calls=Arc::new(AtomicUsize::new(0));
   for ticks in [vec![],vec![1]]{
    let error=collect_model_columns(py,ticks,None,true,EmptySource(calls.clone())).unwrap_err();
    assert!(error.is_instance_of::<pyo3::exceptions::PyValueError>(py));
    assert!(error.to_string().contains("requires an artifact built with direct-method-profile"));
   }
   assert_eq!(calls.load(Ordering::SeqCst),0);
  });
 }
}
