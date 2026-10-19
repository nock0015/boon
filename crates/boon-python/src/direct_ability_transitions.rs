//! Exact completed-source-tick ability/item state transitions. These are not
//! cast labels. Every transition carries a complete typed replacement state,
//! allowing both pre-window state lookup and bounded event-window selection.
use super::*;
const CANONICAL:[(&str,&str);7]=[("cooldown_start","m_flCooldownStart"),("cooldown_end","m_flCooldownEnd"),("remaining_charges","m_iRemainingCharges"),("legacy_charges","m_nCharges"),("charge_recharge_start","m_flChargeRechargeStart"),("charge_recharge_end","m_flChargeRechargeEnd"),("ability_slot","m_eAbilitySlot")];
fn eligible(class:&str)->bool{let _method_span=crate::direct_method_profile::span(67);class.contains("Ability")||class.contains("Item")}
// Class IDs are stable during playback. Retain the name as well so a reused
// ID in a synthetic/reloaded schema cannot reuse a stale eligibility result.
#[derive(Default)]
struct Eligibility {classes:HashMap<i32,(std::sync::Arc<str>,bool)>}
impl Eligibility {
 fn get(&mut self,id:i32,name:&std::sync::Arc<str>)->bool{
  if let Some((cached,value))=self.classes.get(&id){if std::sync::Arc::ptr_eq(cached,name)||cached==name{return *value}}
  let value=eligible(name);self.classes.insert(id,(name.clone(),value));value
 }
}
struct State {class:String,active:bool,owner_handle:Option<i64>,owner:Option<u64>,team:Option<i64>,token:Option<i64>,values:[Option<f64>;7],present:[bool;7]}
impl Clone for State {fn clone(&self)->Self{let _method_span=crate::direct_method_profile::span(68);Self{class:self.class.clone(),active:self.active.clone(),owner_handle:self.owner_handle.clone(),owner:self.owner.clone(),team:self.team.clone(),token:self.token.clone(),values:self.values.clone(),present:self.present.clone()}}}
impl PartialEq for State {fn eq(&self,other:&Self)->bool{let _method_span=crate::direct_method_profile::span(69);self.class==other.class&&self.active==other.active&&self.owner_handle==other.owner_handle&&self.owner==other.owner&&self.team==other.team&&self.token==other.token&&self.values==other.values&&self.present==other.present}}

impl State {
 fn observe(ctx:&Context,e:&Entity,bindings:&Bindings)->Self{let _method_span=crate::direct_method_profile::span(70);
  let owner_handle=i(ctx,e,"m_hOwnerEntity").filter(|h|*h>=0);
  let qualified=owner_handle.and_then(|handle|bindings.current.values().find(|b|b.index==(handle&16383)as i32&&b.serial as i64==handle>>14));
  let raw=CANONICAL.map(|(_,name)|field(ctx,e,name));
  Self{class:e.class_name.to_string(),active:e.active,owner_handle,owner:qualified.map(|b|b.steam),team:qualified.map(|b|b.team),token:i(ctx,e,"m_nSubclassID").filter(|v|*v>=0),values:raw.map(numeric),present:raw.map(|v|v.is_some())}
 }
}
struct Row {tick:i32,time:Option<f64>,id:Id,event:&'static str,state:State,postgame:bool,closure:bool}
#[derive(Default)]
pub(super) struct AbilityTransitions {
 current:BTreeMap<Id,State>,bindings:BTreeMap<(u64,i32,u32),Binding>,rows:Vec<Row>,
 eligibility:Eligibility,indices:Vec<i32>,removal_indices:Vec<i32>,missing:Vec<Id>,
 replaced:HashSet<Id>,slot_count:usize,schema_dirty:bool,
}
impl AbilityTransitions {
 pub(super) fn invalidate_schema(&mut self){self.schema_dirty=true;self.eligibility=Eligibility::default();}
 pub(super) fn len(&self)->usize{let _method_span=crate::direct_method_profile::span(71);self.rows.len()}
 pub(super) fn collect(&mut self,ctx:&Context,time:Option<f64>,bindings:&Bindings,postgame:bool){let _method_span=crate::direct_method_profile::span(72);
  let binding_changed=self.bindings!=bindings.current||self.schema_dirty;
  self.prepare(ctx.entities(),binding_changed);
  // All removals precede all observations, in the original Id order. A delete
  // followed by recreation of the same generation must survive this check:
  // the final entity, rather than the intermediate wire event, is authoritative.
  for id in self.missing.drain(..){let mut state=self.current.remove(&id).unwrap();state.active=false;self.rows.push(Row{tick:ctx.tick(),time,id,event:"removed",state,postgame,closure:false});}
  for &index in &self.indices{
   let Some(e)=ctx.entities().get(index)else{continue};
   if !self.eligibility.get(e.class_id,&e.class_name){continue}
   let id=(e.index,e.serial);let state=State::observe(ctx,e,bindings);
   let event=match self.current.get(&id){
    None=>"initial",
    Some(_) if self.replaced.contains(&id)=>"replaced",
    Some(old) if *old==state=>continue,
    Some(old) if old.active&&!state.active=>"inactive",
    Some(old) if !old.active&&state.active=>"reactivated",
    Some(old) if old.owner_handle!=state.owner_handle||old.owner!=state.owner=>"owner_changed",
    Some(_)=>"changed",
   };
   if let Some(old)=self.current.get(&id){
    if old.active&&(self.replaced.contains(&id)||(old.owner.is_some()&&(old.owner!=state.owner||old.owner_handle!=state.owner_handle))){
     let mut closing=old.clone();closing.active=false;
     self.rows.push(Row{tick:ctx.tick(),time,id,event,state:closing,postgame,closure:true});
    }
   }
   self.rows.push(Row{tick:ctx.tick(),time,id,event,state:state.clone(),postgame,closure:false});self.current.insert(id,state);
  }
  if binding_changed{self.bindings=bindings.current.clone();}
 }
 // Playback creates/replacements (including filtered-out replacements), deletes,
 // updates and PVS changes are all journaled. Only touched slots can invalidate
 // a tracked generation. Sign-on entities retain the original bootstrap rule:
 // they are first observed through updated_indices or an existing binding refresh.
 // No sampling is introduced: prepare runs at every completed source callback.
 fn prepare(&mut self,entities:&boon_parser::EntityContainer,binding_changed:bool){
  self.indices.clear();self.removal_indices.clear();self.missing.clear();self.replaced.clear();
  self.indices.extend_from_slice(entities.updated_indices());
  if std::mem::take(&mut self.schema_dirty){self.indices.extend(entities.iter().map(|(index,_)|index));self.removal_indices.extend(self.current.keys().map(|id|id.0));}
  for change in entities.entity_changes(){
   if matches!(change.kind,boon_parser::EntityChangeKind::Created|boon_parser::EntityChangeKind::Deleted){self.removal_indices.push(change.index);}
   if self.eligibility.get(change.class_id,&change.class_name){
    self.indices.push(change.index);
    if change.kind==boon_parser::EntityChangeKind::Created{self.replaced.insert((change.index,change.serial));}
   }
  }
  // reserve_slots can discard a tail without journal entries. Playback does
  // not call it, but cover observable shrinkage without scanning unaffected Ids.
  if entities.slot_count()<self.slot_count{
   self.removal_indices.extend(self.current.range((entities.slot_count() as i32,0)..).map(|(id,_)|id.0));
  }
  self.slot_count=entities.slot_count();
  self.removal_indices.sort_unstable();self.removal_indices.dedup();
  for &index in &self.removal_indices{
   for (&id,_) in self.current.range((index,0)..=(index,u32::MAX)){
    let present=entities.get(index).is_some_and(|e|e.serial==id.1&&self.eligibility.get(e.class_id,&e.class_name));
    if !present{self.missing.push(id);}
   }
  }
  if binding_changed{self.indices.extend(self.current.keys().filter(|id|self.missing.binary_search(id).is_err()).map(|id|id.0));}
  self.indices.sort_unstable();self.indices.dedup();
 }
 pub(super) fn frame(self)->PolarsResult<DataFrame>{let _method_span=crate::direct_method_profile::span(73);
  let r=self.rows;macro_rules! c{($name:expr,$f:expr)=>{Column::new($name.into(),r.iter().map($f).collect::<Vec<_>>())};}
  let mut cols=vec![Column::new("occurrence".into(),(0..r.len()as u64).collect::<Vec<_>>()),c!("tick",|r|r.tick),c!("match_seconds",|r|r.time),c!("source_game_time_known",|r|r.time.is_some()),c!("is_postgame",|r|r.postgame),c!("node_key",|r|physical(r.id)),c!("entity_index",|r|r.id.0),c!("entity_serial",|r|r.id.1),c!("entity_class",|r|r.state.class.clone()),c!("active",|r|r.state.active),c!("event",|r|r.event),c!("state_closure",|r|r.closure),c!("owner_handle",|r|r.state.owner_handle),c!("owner_entity_index",|r|r.state.owner_handle.map(|h|(h&16383)as i32)),c!("owner_entity_serial",|r|r.state.owner_handle.and_then(|h|u32::try_from(h>>14).ok())),c!("owner_steam_id",|r|r.state.owner),c!("team",|r|r.state.team),c!("ability_id",|r|r.state.token)];
  for(j,(name,_))in CANONICAL.iter().enumerate(){cols.push(Column::new((*name).into(),r.iter().map(|r|r.state.values[j]).collect::<Vec<_>>()));cols.push(Column::new(format!("{name}_known").into(),r.iter().map(|r|r.state.values[j].is_some()).collect::<Vec<_>>()));cols.push(Column::new(format!("{name}_present").into(),r.iter().map(|r|r.state.present[j]).collect::<Vec<_>>()));}
  df_from_columns(cols)
 }
}
#[cfg(test)]mod tests{
 use super::*;
 fn state()->State{State{class:"CCitadel_Ability_Test".into(),active:true,owner_handle:Some(4+(7<<14)),owner:Some(123),team:Some(2),token:Some(99),values:[Some(0.),None,Some(0.),None,None,None,Some(2.)],present:[true,false,true,true,false,false,true]}}
 #[test]fn full_state_null_zero_presence_and_identity_remain_distinct(){let c=AbilityTransitions{rows:vec![Row{tick:10,time:Some(0.),id:(17,8),event:"initial",state:state(),postgame:false,closure:false}],..Default::default()};let frame=c.frame().unwrap();assert_eq!(frame.column("cooldown_start").unwrap().f64().unwrap().get(0),Some(0.));assert_eq!(frame.column("legacy_charges").unwrap().null_count(),1);assert_eq!(frame.column("legacy_charges_present").unwrap().bool().unwrap().get(0),Some(true));assert_eq!(frame.column("legacy_charges_known").unwrap().bool().unwrap().get(0),Some(false));assert_eq!(frame.column("owner_entity_serial").unwrap().u32().unwrap().get(0),Some(7));}
 #[test]fn removed_rows_are_observations_not_casts_or_inferred_deaths(){let mut s=state();s.active=false;let frame=AbilityTransitions{rows:vec![Row{tick:11,time:None,id:(17,8),event:"removed",state:s,postgame:false,closure:false}],..Default::default()}.frame().unwrap();assert_eq!(frame.column("event").unwrap().str().unwrap().get(0),Some("removed"));assert_eq!(frame.column("match_seconds").unwrap().null_count(),1);assert_eq!(frame.column("active").unwrap().bool().unwrap().get(0),Some(false));}
 fn entity(index:i32,serial:u32,class_id:i32,class:&str)->Entity{Entity::from_fields(index,serial,class_id,class,true,Default::default()).unwrap()}
 fn assert_prepare_matches_original(c:&mut AbilityTransitions,entities:&boon_parser::EntityContainer,binding_changed:bool){
  let missing:Vec<_>=c.current.keys().copied().filter(|(index,serial)|!entities.get(*index).is_some_and(|e|e.serial==*serial&&eligible(&e.class_name))).collect();
  let mut indices:std::collections::BTreeSet<_>=entities.updated_indices().iter().copied().collect();
  let mut replaced=HashSet::new();
  for change in entities.entity_changes(){if eligible(&change.class_name){indices.insert(change.index);if change.kind==boon_parser::EntityChangeKind::Created{replaced.insert((change.index,change.serial));}}}
  if binding_changed{indices.extend(c.current.keys().filter(|id|!missing.contains(id)).map(|id|id.0));}
  c.prepare(entities,binding_changed);
  assert_eq!(c.missing,missing);assert_eq!(c.indices,indices.into_iter().collect::<Vec<_>>());assert_eq!(c.replaced,replaced);
 }
 #[test]fn schema_replacement_refreshes_existing_abilities_without_changing_normal_bootstrap(){
  let mut entities=boon_parser::EntityContainer::new();entities.insert(entity(17,8,1,"AbilityTest")).unwrap();entities.clear_tick_changes();let mut c=AbilityTransitions::default();c.prepare(&entities,false);assert!(c.indices.is_empty());
  c.current.insert((17,8),state());c.invalidate_schema();c.prepare(&entities,true);assert_eq!(c.indices,vec![17]);assert!(c.missing.is_empty());assert!(!c.schema_dirty);
  c.prepare(&entities,false);assert!(c.indices.is_empty());
 }
 #[test]fn eligibility_cache_checks_reused_class_ids(){
  let mut cache=Eligibility::default();
  for name in ["AbilityTest","Unrelated","ItemTest","Unrelated","AbilityTest"]{let name:std::sync::Arc<str>=name.into();assert_eq!(cache.get(3,&name),eligible(&name));assert_eq!(cache.get(3,&name),eligible(&name));}
 }
 #[test]fn preparation_preserves_same_generation_replacement_and_sorted_removals(){
  let mut entities=boon_parser::EntityContainer::new();let mut c=AbilityTransitions::default();
  for index in [9,2,5]{entities.insert(entity(index,7,1,"AbilityTest")).unwrap();c.current.insert((index,7),state());}
  entities.clear_tick_changes();c.prepare(&entities,false);
  entities.insert(entity(9,8,1,"AbilityTest")).unwrap();
  entities.insert(entity(5,7,2,"Unrelated")).unwrap();
  entities.insert(entity(2,7,2,"Unrelated")).unwrap();
  entities.insert(entity(2,7,1,"AbilityTest")).unwrap();
  assert_prepare_matches_original(&mut c,&entities,false);
  assert_eq!(c.missing,vec![(5,7),(9,7)]);assert!(c.replaced.contains(&(2,7)));
 }
 #[test]fn preparation_keeps_bootstrap_and_binding_refresh_cadence(){
  let mut entities=boon_parser::EntityContainer::new();entities.insert(entity(17,8,1,"AbilityTest")).unwrap();entities.clear_tick_changes();
  let mut c=AbilityTransitions::default();assert_prepare_matches_original(&mut c,&entities,false);assert!(c.indices.is_empty());
  c.current.insert((17,8),state());assert_prepare_matches_original(&mut c,&entities,true);assert_eq!(c.indices,vec![17]);
  assert_prepare_matches_original(&mut c,&entities,false);assert!(c.indices.is_empty());assert!(c.replaced.is_empty());
 }
 #[test]fn preparation_covers_unjournaled_tail_shrink(){
  let mut entities=boon_parser::EntityContainer::new();let mut c=AbilityTransitions::default();
  for index in [1,7,12]{entities.insert(entity(index,3,1,"ItemTest")).unwrap();c.current.insert((index,3),state());}
  c.prepare(&entities,false);entities.clear_tick_changes();entities.reserve_slots(4).unwrap();
  assert_prepare_matches_original(&mut c,&entities,false);assert_eq!(c.missing,vec![(7,3),(12,3)]);
 }
 #[test]fn preparation_matches_original_across_completed_tick_journals(){
  let mut entities=boon_parser::EntityContainer::new();let mut c=AbilityTransitions::default();let mut random=17u32;
  for tick in 0..512{
   random=random.wrapping_mul(1664525).wrapping_add(1013904223);
   let index=((random>>8)%32)as i32;let serial=(random>>16)%5;
   if tick%19==0{entities.reserve_slots(index as usize).unwrap();}
   else{for delta in 0..=tick%3{let class=if (random+delta)%4==0{"Unrelated"}else if random%2==0{"AbilityTest"}else{"ItemTest"};entities.insert(entity(index,serial,delta as i32,class)).unwrap();}}
   assert_prepare_matches_original(&mut c,&entities,tick%7==0);
   for id in c.missing.drain(..){c.current.remove(&id);}
   for &index in &c.indices{if let Some(e)=entities.get(index).filter(|e|eligible(&e.class_name)){c.current.insert((e.index,e.serial),state());}}
   entities.clear_tick_changes();
  }
 }

}
