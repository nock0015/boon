use super::*;
const PUBLIC_PLAYER:&[(&str,&str)]=&[("gold_net_worth","m_iGoldNetWorth"),("level","m_iLevel"),("kills","m_iPlayerKills"),("deaths","m_iDeaths"),("assists","m_iPlayerAssists"),("is_alive","m_bAlive"),("respawn_source_time","m_flRespawnTime"),("has_rebirth","m_bHasRebirth"),("has_rejuvenator","m_bHasRejuvenator"),("has_ultimate_trained","m_bUltimateTrained")];
const TEAM_PLAYER:&[(&str,&str)]=&[("health_regen","m_flHealthRegen"),("ap_net_worth","m_iAPNetWorth"),("boon_count","m_iBoonCount"),("ultimate_cooldown_start","m_flUltimateCooldownStart"),("ultimate_cooldown_end","m_flUltimateCooldownEnd"),("denies","m_iDenies"),("last_hits","m_iLastHits"),("hero_damage","m_iHeroDamage"),("objective_damage","m_iObjectiveDamage"),("hero_healing","m_iHeroHealing"),("self_healing","m_iSelfHealing"),("kill_streak","m_iKillStreak")];
const TEAM_PAWN:&[(&str,&str)]=&[("souls","m_nCurrencies.m_nCurrencies"),("spent_souls","m_nSpentCurrencies.m_nSpentCurrencies"),("in_regen_zone","m_bInRegenerationZone"),("in_item_shop","m_bInItemShopZone")];
const PUBLIC_PAWN:&[(&str,&str)]=&[("health","m_iHealth"),("max_health","m_iMaxHealth"),("lifestate","m_lifeState"),("respawn_source_time","m_flRespawnTime"),("death_source_time","m_flDeathTime"),("last_spawn_source_time","m_flLastSpawnTime")];
const ABILITY_FIELDS:&[(&str,&str)]=&[("cooldown_start","m_flCooldownStart"),("cooldown_end","m_flCooldownEnd"),("remaining_charges","m_iRemainingCharges"),("legacy_charges","m_nCharges"),("charge_recharge_start","m_flChargeRechargeStart"),("charge_recharge_end","m_flChargeRechargeEnd"),("ability_slot","m_eAbilitySlot")];
#[derive(Clone)]
struct Slot{raw:i64,class:i64,team:i64,visible:bool,x:Option<f64>,y:Option<f64>,health:Option<f64>,protected:Option<bool>}
#[derive(Clone)]
struct Channel{team:i64,divisor:i64,complete:bool,slots:Vec<Slot>}
// Every callback contributes to these journals, including unsampled ticks.
// The complete output map retains raw projected records; append temporarily
// normalizes then restores primitive observation/position metadata each sample.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
struct Binding{steam:u64,team:i64,hero:Option<i64>,pawn:Id,controller:Id}
impl Binding{fn of(p:&Player<'_>)->Self{Self{steam:p.steam,team:p.team,hero:p.hero,pawn:(p.pawn.index,p.pawn.serial),controller:(p.controller.index,p.controller.serial)}}}
#[derive(Clone,Copy)]
struct Expiry{at:f32,serial:u32,version:u64}
impl PartialEq for Expiry{fn eq(&self,other:&Self)->bool{self.cmp(other)==std::cmp::Ordering::Equal}}
impl Eq for Expiry{}
impl PartialOrd for Expiry{fn partial_cmp(&self,other:&Self)->Option<std::cmp::Ordering>{Some(self.cmp(other))}}
impl Ord for Expiry{fn cmp(&self,other:&Self)->std::cmp::Ordering{other.at.total_cmp(&self.at).then_with(||other.serial.cmp(&self.serial)).then_with(||other.version.cmp(&self.version))}}
struct ModifierEffect{deadline:Option<f32>,aura:bool,effective:bool,version:u64}
impl ModifierEffect{fn effective_at(&self,now:Option<f32>)->bool{self.aura&&match(now,self.deadline){(Some(now),Some(at))=>now<at,_=>true}}}
// Scope IDs are invocation-local, stable handles. Ordering remains the exact
// lexicographic Scope order in `output`; IDs never determine emitted row order.
pub(super) type ScopeId=usize;
// A compaction is the only operation that changes a live scope handle.
// History consumes this exact mapping before using the next projection IDs.
pub(super) struct ScopeRemap{from_generation:u64,to_generation:u64,old_to_new:Vec<Option<ScopeId>>}
#[derive(Default)]
struct ScopeRegistry{ids:HashMap<Scope,ScopeId>,scopes:Vec<Scope>,seen:Vec<bool>,generation:u64}
impl ScopeRegistry{
 fn intern(&mut self,scope:Scope)->ScopeId{if let Some(&id)=self.ids.get(&scope){return id}let id=self.scopes.len();self.ids.insert(scope.clone(),id);self.scopes.push(scope);self.seen.push(false);id}
 fn begin(&mut self){self.seen.fill(false);}
}
// Custom dual index: stable dense payload slots and a lexical emission index.
// Payload access by cached ScopeId never hashes or compares replay strings.
// The ordered index changes only on membership changes; IDs are never order.
#[derive(Default)]
pub(super) struct DenseNodes {slots:Vec<Option<Node>>,order:BTreeMap<Scope,ScopeId>,generation:u64}
impl DenseNodes {
 pub(super) fn len(&self)->usize{self.order.len()}
 pub(super) fn keys(&self)->impl Iterator<Item=&Scope>{self.order.keys()}
 pub(super) fn iter(&self)->impl Iterator<Item=(&Scope,&Node)>{self.order.iter().map(|(scope,&id)|(scope,self.slots[id].as_ref().expect("live dense scope")))}
 pub(super) fn scope_ids(&self)->impl Iterator<Item=(&Scope,ScopeId)>{self.order.iter().map(|(scope,&id)|(scope,id))}
 pub(super) fn generation(&self)->u64{self.generation}
 #[cfg(test)]pub(super) fn slots_len_for_test(&self)->usize{self.slots.len()}
 pub(super) fn values(&self)->impl Iterator<Item=&Node>{self.order.values().map(|&id|self.slots[id].as_ref().expect("live dense scope"))}
 pub(super) fn get(&self,scope:&Scope)->Option<&Node>{self.order.get(scope).and_then(|&id|self.get_id(id))}
 pub(super) fn get_mut(&mut self,scope:&Scope)->Option<&mut Node>{let id=*self.order.get(scope)?;self.get_id_mut(id)}
 pub(super) fn get_id(&self,id:ScopeId)->Option<&Node>{self.slots.get(id).and_then(Option::as_ref)}
 pub(super) fn get_id_mut(&mut self,id:ScopeId)->Option<&mut Node>{self.slots.get_mut(id).and_then(Option::as_mut)}
 pub(super) fn contains_key(&self,scope:&Scope)->bool{self.order.contains_key(scope)}
 pub(super) fn insert_at(&mut self,id:ScopeId,scope:Scope,node:Node){if self.slots.len()<=id{self.slots.resize_with(id+1,||None);}self.slots[id]=Some(node);self.order.insert(scope,id);}
 pub(super) fn for_each_mut(&mut self,mut f:impl FnMut(&Scope,&mut Node)){self.for_each_id_mut(|scope,_,node|f(scope,node));}
 pub(super) fn for_each_id_mut(&mut self,mut f:impl FnMut(&Scope,ScopeId,&mut Node)){let slots=&mut self.slots;for(scope,&id)in &self.order{f(scope,id,slots[id].as_mut().expect("live dense scope"));}}
 pub(super) fn remove(&mut self,scope:&Scope)->Option<Node>{let id=self.order.remove(scope)?;self.slots[id].take()}
 pub(super) fn migrate_history(&mut self,remap:ScopeRemap,retired:&mut BTreeMap<Scope,Node>){
  assert_eq!(self.generation,remap.from_generation,"history scope generation");
  let mut old=std::mem::take(self);self.generation=remap.to_generation;
  self.slots.resize_with(remap.old_to_new.iter().flatten().copied().max().map_or(0,|id|id+1),||None);
  for(scope,id)in old.order{let node=old.slots[id].take().expect("live history scope");
   if let Some(next)=remap.old_to_new[id]{self.insert_at(next,scope,node);}else{retired.insert(scope,node);}
  }
 }
 pub(super) fn retain(&mut self,mut keep:impl FnMut(ScopeId)->bool){let slots=&mut self.slots;self.order.retain(|_,id|if keep(*id){true}else{slots[*id]=None;false});}
 pub(super) fn remap(&mut self,registry:&ScopeRegistry){let mut old=std::mem::take(self);self.generation=registry.generation;self.slots.resize_with(registry.scopes.len(),||None);for(scope,id)in old.order{let next=registry.ids[&scope];self.slots[next]=old.slots[id].take();self.order.insert(scope,next);}}
}
impl std::ops::Index<&Scope> for DenseNodes{type Output=Node;fn index(&self,scope:&Scope)->&Node{self.get(scope).expect("missing dense scope")}}
#[cfg(test)]impl From<BTreeMap<Scope,Node>> for DenseNodes{fn from(nodes:BTreeMap<Scope,Node>)->Self{let mut out=Self::default();for(scope,node)in nodes{out.insert_at(out.slots.len(),scope,node);}out}}
#[cfg(test)]impl FromIterator<(Scope,Node)> for DenseNodes{fn from_iter<T:IntoIterator<Item=(Scope,Node)>>(nodes:T)->Self{BTreeMap::from_iter(nodes).into()}}
struct ProjectedMap{output:DenseNodes,registry:ScopeRegistry,current_len:usize}
impl ProjectedMap{
 fn new(output:DenseNodes,mut registry:ScopeRegistry)->Self{assert_eq!(output.generation,registry.generation,"projection scope generation");registry.begin();Self{output,registry,current_len:0}}
 fn mark(&mut self,id:ScopeId){if !self.registry.seen[id]{self.registry.seen[id]=true;self.current_len+=1;}}
 fn has_all(&self,ids:&[ScopeId])->bool{ids.iter().all(|&id|self.output.get_id(id).is_some())}
 fn keep(&mut self,ids:&[ScopeId]){for &id in ids{debug_assert!(self.output.get_id(id).is_some());self.mark(id);}}
 fn insert(&mut self,node:Node)->ScopeId{let scope=node.scope();let id=self.registry.intern(scope.clone());self.mark(id);self.output.insert_at(id,scope,node);id}
 fn len(&self)->usize{self.current_len}
 fn contains_key(&self,scope:&Scope)->bool{self.registry.ids.get(scope).is_some_and(|&id|self.registry.seen[id])}
 fn get_mut(&mut self,scope:&Scope)->Option<&mut Node>{let id=*self.registry.ids.get(scope)?;self.get_id_mut(id)}
 fn get_id_mut(&mut self,id:ScopeId)->Option<&mut Node>{if self.registry.seen.get(id).copied()==Some(true){self.output.get_id_mut(id)}else{None}}
 fn get_or_insert(&mut self,node:Node)->&mut Node{let scope=node.scope();let id=self.registry.intern(scope.clone());if !self.registry.seen[id]||self.output.get_id(id).is_none(){self.output.insert_at(id,scope,node);}self.mark(id);self.output.get_id_mut(id).unwrap()}
 fn for_each_mut(&mut self,mut f:impl FnMut(&mut Node)){let seen=&self.registry.seen;let slots=&mut self.output.slots;for &id in self.output.order.values(){if seen[id]{f(slots[id].as_mut().unwrap());}}}
 fn finish(mut self)->(DenseNodes,ScopeRegistry){let seen=&self.registry.seen;self.output.retain(|id|seen[id]);(self.output,self.registry)}
}
#[derive(Default)]
pub(super) struct ProjectionCache{
 pub(super) output:DenseNodes,scope_registry:ScopeRegistry,pub(super) history_remap:Option<ScopeRemap>,
 seeded:bool,dirty:HashSet<i32>,dirty_groups:HashMap<i32,u8>,field_groups:HashMap<(i32,u64),u8>,candidates:[std::collections::BTreeSet<i32>;7],
 rows:HashMap<(u8,Id),(Binding,Option<u32>,Vec<ScopeId>)>,objective_rows:HashMap<Id,(ScopeId,[Option<f64>;66])>,
 upgrades:HashMap<Id,(Binding,Vec<(usize,i64,Option<u32>)>,Option<Vec<(usize,i64)>>)>,
 channel_bindings:Vec<Binding>,channel_ids:Vec<Id>,channels:Option<Vec<Channel>>,channel_slots:HashMap<Id,Vec<Option<Slot>>>,dirty_slots:HashMap<i32,Option<HashSet<usize>>>,slot_fields:HashMap<(i32,u64),(bool,Option<usize>)>,
 zip:zipline_reach::Cache,zip_revision:Option<u64>,zip_scopes:Vec<ScopeId>,
 rule_scopes:HashMap<Id,Vec<ScopeId>>,remembered_scopes:HashSet<ScopeId>,
 modifier_rows:HashMap<u32,(Binding,ScopeId)>,effects:HashMap<u32,ModifierEffect>,expiry:std::collections::BinaryHeap<Expiry>,effect_clock:Option<f32>,effect_version:u64,
}
impl ProjectionCache{
 pub(super) fn invalidate_schema(&mut self){
  // Keep qualified historical observations and scope handles, but discard every
  // derived key/value/topology plan. Original home identities live in Collector.
  let output=std::mem::take(&mut self.output);let scope_registry=std::mem::take(&mut self.scope_registry);let remembered_scopes=std::mem::take(&mut self.remembered_scopes);let history_remap=self.history_remap.take();
  *self=Self{output,scope_registry,remembered_scopes,history_remap,..Self::default()};
 }

 fn finish(&mut self,out:ProjectedMap)->DenseNodes{
  let (mut output,mut registry)=out.finish();
  if registry.ids.len()>output.len().saturating_mul(2).saturating_add(64){
   let mut compact=ScopeRegistry{generation:registry.generation.checked_add(1).expect("scope generation overflow"),..ScopeRegistry::default()};for scope in output.keys(){compact.intern(scope.clone());}
   assert!(self.history_remap.is_none(),"unconsumed history scope remap");
   self.history_remap=Some(ScopeRemap{from_generation:registry.generation,to_generation:compact.generation,old_to_new:registry.scopes.iter().map(|scope|compact.ids.get(scope).copied()).collect()});
   self.remembered_scopes=self.remembered_scopes.iter().filter_map(|&id|compact.ids.get(&registry.scopes[id]).copied()).collect();
   self.rows.clear();self.modifier_rows.clear();self.rule_scopes.clear();self.objective_rows.clear();self.zip_scopes.clear();self.zip_revision=None;
   output.remap(&compact);registry=compact;
  }
  self.scope_registry=registry;output
 }
 #[cfg(test)]pub(super) fn project_test_nodes(&mut self,nodes:impl IntoIterator<Item=Node>)->DenseNodes{
  let mut out=ProjectedMap::new(std::mem::take(&mut self.output),std::mem::take(&mut self.scope_registry));for node in nodes{out.insert(node);}self.finish(out)
 }
 #[cfg(test)]pub(super) fn registry_len(&self)->usize{self.scope_registry.scopes.len()}
 pub(super) fn modifier_changed(&mut self,serial:u32){self.modifier_rows.remove(&serial);self.effects.remove(&serial);}
 fn modifier_clock(&mut self,now:Option<f32>){
  if self.expiry.len()>self.effects.len().saturating_mul(2).saturating_add(64){self.expiry.retain(|expiry|self.effects.get(&expiry.serial).is_some_and(|effect|effect.version==expiry.version));}
  let now=now.filter(|v|v.is_finite());
  if !matches!((self.effect_clock,now),(Some(old),Some(next)) if next>=old){
   // Unknown and backward clocks can resurrect a duration-filtered record.
   self.expiry.clear();
   for (&serial,effect) in &mut self.effects{effect.effective=effect.effective_at(now);if let Some(at)=effect.deadline.filter(|at|at.is_finite()){if effect.effective{self.expiry.push(Expiry{at,serial,version:effect.version});}}}
  }else if let Some(now)=now{
   while self.expiry.peek().is_some_and(|expiry|expiry.at<=now){let expiry=self.expiry.pop().unwrap();if let Some(effect)=self.effects.get_mut(&expiry.serial).filter(|effect|effect.version==expiry.version){effect.effective=false;}}
  }
  self.effect_clock=now;
 }
 fn modifier_effective(&mut self,serial:u32,aura:Option<bool>,applied:Option<f32>,duration:Option<f32>)->bool{
  if let Some(effect)=self.effects.get(&serial){return effect.effective}
  self.effect_version=self.effect_version.wrapping_add(1);
  let deadline=applied.filter(|v|v.is_finite()).zip(duration.filter(|v|v.is_finite()&&*v>0.)).map(|(applied,duration)|applied+duration);
  let mut effect=ModifierEffect{deadline,aura:aura!=Some(false),effective:false,version:self.effect_version};effect.effective=effect.effective_at(self.effect_clock);
  if effect.effective{if let Some(at)=deadline.filter(|v|v.is_finite()){self.expiry.push(Expiry{at,serial,version:effect.version});}}
  let effective=effect.effective;self.effects.insert(serial,effect);effective
 }
 pub(super) fn observe(&mut self,ctx:&Context,model_entities:&mut ModelEntityView){
  let mut touched=HashSet::new();
  if !self.seeded{for(index,_)in ctx.entities().iter(){touched.insert(index);self.dirty_groups.insert(index,31);self.dirty_slots.insert(index,None);}self.seeded=true;}
  for &index in ctx.entities().updated_indices(){
   if let Some(e)=ctx.entities().get(index){
    if !e.changed_field_keys().is_empty(){touched.insert(index);}
    if e.class_name.as_ref()=="CCitadelTeam"{
     for &key in e.changed_field_keys(){
      let (relevant,slot)=*self.slot_fields.entry((e.class_id,key)).or_insert_with(||{
       let name=ctx.serializers().get(&e.class_name).and_then(|serializer|serializer.field_name_for_key(key));
       (name.as_deref().is_none_or(|name|name.starts_with("m_vecFOWEntities")),name.as_deref().and_then(fow_slot))
      });
      if relevant{
       let dirty=self.dirty_slots.entry(index).or_insert_with(||Some(HashSet::new()));
       match slot{Some(slot)=>{if let Some(dirty)=dirty{dirty.insert(slot);}},None=>*dirty=None}
      }
     }
    }
    if matches!(e.class_name.as_ref(),"CCitadelPlayerController"|"CCitadelTeam"|"CCitadelGameRulesProxy"){
     for &key in e.changed_field_keys(){
      let mask=*self.field_groups.entry((e.class_id,key)).or_insert_with(||{
       let name=ctx.serializers().get(&e.class_name).and_then(|s|s.field_name_for_key(key));
       // Unknown field paths invalidate conservatively.
       let Some(name)=name else{return 31};
       (if name.starts_with("m_PlayerDataGlobal.m_vecStatViewerModifierValues"){1}else{0})|
       (if name.starts_with("m_PlayerDataGlobal.m_vecUpgrades")||name.starts_with("m_PlayerDataGlobal.m_vecAbilityUpgradeState"){2}else{0})|
       (if name.starts_with("m_vecFOWEntities")||name=="m_iTeamNum"{4}else{0})|
       (if matches!(name.as_str(),"m_pGameRules.m_bGamePaused"|"m_pGameRules.m_tNextMidBossSpawnTime")||name.starts_with("m_pGameRules.m_vecNeutralCamp"){8}else{0})
      });
      *self.dirty_groups.entry(index).or_default()|=mask;
     }
    }
   }
  }
  for change in ctx.entities().entity_changes(){
   touched.insert(change.index);
   if change.kind!=boon_parser::EntityChangeKind::Updated{
    self.dirty_groups.insert(change.index,31);self.dirty_slots.insert(change.index,None);self.channel_slots.retain(|id,_|id.0!=change.index);
    self.rows.retain(|(_,id),_|id.0!=change.index);self.objective_rows.retain(|id,_|id.0!=change.index);self.rule_scopes.retain(|id,_|id.0!=change.index);self.upgrades.retain(|id,_|id.0!=change.index);
   }
  }
  for index in touched{
   self.dirty.insert(index);
   for set in &mut self.candidates{set.remove(&index);}
   if let Some(e)=model_entities.get(ctx.entities(),index).filter(|e|e.active){
    let class=e.class_name.as_ref();
    let flags=[class=="CCitadelTeam",objective(class),class=="CCitadelGameRulesProxy",class=="CCitadel_KothCashIn",class.contains("Ability")||class.contains("Item"),field(ctx,e,"m_flAmmoFrac").is_some(),class=="CCitadelZipLineNode"];
    for (set,flag) in self.candidates.iter_mut().zip(flags){if flag{set.insert(index);}}
   }
  }
 }
 fn entities<'a>(&self,ctx:&'a Context,kind:usize)->Vec<&'a Entity>{self.candidates[kind].iter().filter_map(|&index|ctx.entities().get(index).filter(|e|e.active)).collect()}
 fn cached(&self,kind:u8,id:Id,binding:Binding,packed:Option<u32>)->Option<&[ScopeId]>{
  let (old,upgrade,rows)=self.rows.get(&(kind,id))?;
  (*old==binding&&*upgrade==packed&&if kind==0||kind==3{self.dirty_groups.get(&binding.controller.0).copied().unwrap_or(0)&if kind==0{1}else{2}==0}else if kind==2{!self.dirty.contains(&binding.pawn.0)&&!self.dirty.contains(&binding.controller.0)}else{!self.dirty.contains(&id.0)}).then_some(rows.as_slice())
 }
 fn store(&mut self,kind:u8,id:Id,binding:Binding,packed:Option<u32>,rows:Vec<ScopeId>){self.rows.insert((kind,id),(binding,packed,rows));}
}
fn fow_slot(name:&str)->Option<usize>{let rest=name.strip_prefix("m_vecFOWEntities.")?;let(index,field)=rest.split_once('.')?;(!field.is_empty()).then_some(())?;index.parse().ok()}
fn refresh_slot_records<T>(records:&mut Vec<Option<T>>,len:usize,dirty:Option<&Option<HashSet<usize>>>,mut decode:impl FnMut(usize)->Option<T>){
 let old_len=records.len();records.resize_with(len,||None);let all=dirty.is_some_and(Option::is_none);
 for j in 0..len{if all||j>=old_len||dirty.and_then(Option::as_ref).is_some_and(|slots|slots.contains(&j)){records[j]=decode(j);}}
}
fn decode_slot(ctx:&Context,e:&Entity,j:usize)->Option<Slot>{
 let(raw,class,team,visible)=(ai(ctx,e,"m_vecFOWEntities",j,"m_nEntIndex")?,ai(ctx,e,"m_vecFOWEntities",j,"m_eClass")?,ai(ctx,e,"m_vecFOWEntities",j,"m_nTeam")?,ab(ctx,e,"m_vecFOWEntities",j,"m_bVisibleOnMap")?);
 Some(Slot{raw,class,team,visible,x:an(ctx,e,"m_vecFOWEntities",j,"m_nPositionX").filter(|v|(0. ..=255.).contains(v)),y:an(ctx,e,"m_vecFOWEntities",j,"m_nPositionY").filter(|v|(0. ..=255.).contains(v)),health:an(ctx,e,"m_vecFOWEntities",j,"m_nHealthPercent").filter(|v|(0. ..=255.).contains(v)),protected:ab(ctx,e,"m_vecFOWEntities",j,"m_bBackdoorProtectionActive")})
}
fn channels(ctx:&Context,ps:&[Player<'_>],classes:&HashMap<i64,String>,teams:&[&Entity],cache:&mut HashMap<Id,Vec<Option<Slot>>>,dirty:&HashMap<i32,Option<HashSet<usize>>>)->Vec<Channel>{let _method_span=crate::direct_method_profile::span(55);
 let mut out=vec![];
 for team in [2,3]{let es:Vec<_>=teams.iter().copied().filter(|e|i(ctx,e,"m_iTeamNum")==Some(team)).collect();if es.len()!=1{continue}let e=es[0];let Some(len)=count(ctx,e,"m_vecFOWEntities",255)else{continue};
  let cached=cache.entry((e.index,e.serial)).or_default();refresh_slot_records(cached,len,dirty.get(&e.index),|j|decode_slot(ctx,e,j));
  let mut slots:Vec<_>=cached.iter().filter_map(Clone::clone).collect();
  let mut divisors=vec![];for d in [1,2]{let mut anchors=0;let mut valid=true;for s in slots.iter().filter(|s|s.class==1){if s.raw<0||s.raw%d!=0{valid=false;break}let matches=ps.iter().filter(|p|p.pawn.index as i64==s.raw/d&&p.team==s.team).count();if matches>1{valid=false;break}anchors+=matches;}if valid&&anchors>0{divisors.push(d)}}if divisors.len()!=1{continue}let d=divisors[0];let mut multiplicity=HashMap::new();for s in &slots{*multiplicity.entry(s.raw).or_insert(0)+=1;}
  let complete=slots.len()==len&&slots.iter().all(|s|s.raw>=0&&((s.raw%d==0&&multiplicity[&s.raw]==1)||(d==2&&s.raw==1&&classes.contains_key(&s.class))));
  slots.retain(|s|s.raw>=0&&s.raw%d==0&&multiplicity[&s.raw]==1);out.push(Channel{team,divisor:d,complete,slots});
 }
 out
}
fn packed_upgrade(value:Option<&FieldValue>)->Option<u32>{let _method_span=crate::direct_method_profile::span(56);match value?{FieldValue::U32(v)=>Some(*v),FieldValue::I32(v)=>Some(*v as u32),FieldValue::U64(v)=>u32::try_from(*v).ok(),FieldValue::I64(v)=>u32::try_from(*v).ok().or_else(||i32::try_from(*v).ok().map(|v|v as u32)),_=>None}}
fn apply_upgrade_bits(node:&mut Node,packed:u32){let _method_span=crate::direct_method_profile::span(57);let bits=packed>>17;node.set("upgrade_bits",Some(bits as f64));for bit in 0..15{node.extras.insert(format!("upgrade_bit:{bit}"),((bits&(1<<bit))!=0)as u8 as f64);}}
fn put(out:&mut ProjectedMap,node:Node)->ScopeId{let _method_span=crate::direct_method_profile::span(58);out.insert(node)}
fn public_player(p:&Player<'_>)->Node{let _method_span=crate::direct_method_profile::span(59);let mut node=Node::entity(p.pawn,0);node.key=format!("player:{}",p.steam);node.owner=Some(p.steam);node.team=Some(p.team);node.hero=p.hero;node}
fn allied_player(p:&Player<'_>,position:Option<[f64;3]>)->Node{let _method_span=crate::direct_method_profile::span(60);let mut node=public_player(p);node.audience=3;node.xyz(position);node}
fn controller_upgrade_for(states:&HashMap<(u64,i64),Option<u32>>,owner:u64,token:i64)->Option<u32>{let _method_span=crate::direct_method_profile::span(61);states.get(&(owner,token)).copied().flatten()}


fn memory_generation_matches(old:&Node,current:&Node)->bool{let _method_span=crate::direct_method_profile::span(62);old.id==current.id&&old.owner==current.owner&&old.team==current.team&&old.hero==current.hero}
fn player_memory_current(old:&Node,players:&[Player<'_>])->bool{let _method_span=crate::direct_method_profile::span(63);old.kind!="player"||players.iter().any(|p|old.owner==Some(p.steam)&&old.id==Some((p.pawn.index,p.pawn.serial))&&old.hero==p.hero&&old.team==Some(p.team))}
fn apply_objective_latch(node:&mut Node,was_missing:bool,statuses:&[Option<bool>]){let _method_span=crate::direct_method_profile::span(64);
 let missing=statuses==[Some(false),Some(false)];
 let revived=statuses==[Some(true),Some(true)]&&node.get("health").is_some();
 if missing||(was_missing&&!revived){
  for name in ["health","health_byte","protected","lifestate","health_age"]{node.set(name,None)}
  node.set("is_alive",Some(0.));node.set("visible",Some(0.));node.extras.insert("life-evidence:missing-objective-marker".into(),1.);
 }else if was_missing&&revived{node.extras.remove("life-evidence:missing-objective-marker");node.set("visible",Some(1.));}
}

impl Collector{
 fn camp_position(&mut self,node:&mut Node,observed:Option<[f64;2]>,time:Option<f64>){let _method_span=crate::direct_method_profile::span(65);
  if let Some(point)=observed{self.camp_positions.insert(node.key.clone(),(point,time));node.set("map_age",Some(0.));}
  if let Some((point,at))=self.camp_positions.get(&node.key){node.set("map_x",Some(point[0]));node.set("map_y",Some(point[1]));node.map_time=*at;}
 }
 pub(super) fn project(&mut self,ctx:&Context,time:Option<f64>,classes:&HashMap<i64,String>,model_entities:&mut ModelEntityView)->DenseNodes{let _method_span=crate::direct_method_profile::span(66);
  if !self.projection_cache.seeded{self.projection_cache.observe(ctx,model_entities);}
  let phase=FineInstant::now();let ps=players(ctx);self.profile.elapsed("projection.player_binding_plan",phase,ps.len());let mut out=ProjectedMap::new(std::mem::take(&mut self.projection_cache.output),std::mem::take(&mut self.projection_cache.scope_registry));let mut upgrade_states:HashMap<(u64,i64),Option<u32>>=HashMap::new();let mut upgrade_slots=vec![];let mut joined_upgrades=HashSet::new();let by_pawn:HashMap<_,_>=ps.iter().map(|p|(p.pawn.id(),p)).collect();
  // Ascending-index candidate registries preserve the original entity walks
  // while lifecycle callbacks avoid repeated whole-container classification.
  let phase=FineInstant::now();let team_entities=self.projection_cache.entities(ctx,0);self.profile.elapsed("projection.shared_entity_walks",phase,team_entities.len());
  let phase=FineInstant::now();let objective_entities=self.projection_cache.entities(ctx,1);self.profile.elapsed("projection.shared_entity_walks",phase,objective_entities.len());
  let phase=FineInstant::now();let rule_entities=self.projection_cache.entities(ctx,2);self.profile.elapsed("projection.shared_entity_walks",phase,rule_entities.len());
  let phase=FineInstant::now();let rift_entities=self.projection_cache.entities(ctx,3);self.profile.elapsed("projection.shared_entity_walks",phase,rift_entities.len());
  let phase=FineInstant::now();let ability_entities=self.projection_cache.entities(ctx,4);self.profile.elapsed("projection.shared_entity_walks",phase,ability_entities.len());
  let phase=FineInstant::now();let ammo_entities=self.projection_cache.entities(ctx,5);self.profile.elapsed("projection.shared_entity_walks",phase,ammo_entities.len());
  let phase=FineInstant::now();let zip_entities=self.projection_cache.entities(ctx,6);self.profile.elapsed("projection.shared_entity_walks",phase,zip_entities.len());
  for p in &ps{let phase=FineInstant::now();let before=out.len();
   let binding=Binding::of(p);
   if let Some(rows)=self.projection_cache.cached(2,binding.pawn,binding,None).filter(|rows|out.has_all(rows)){out.keep(rows);if let Some(node)=out.get_id_mut(rows[1]){node.set("barrier",None);}}else{
   let mut public=public_player(p);public.copy_fields(ctx,p.controller,CopyGroup::PublicPlayer,"m_PlayerDataGlobal.",PUBLIC_PLAYER);public.copy_fields(ctx,p.pawn,CopyGroup::PublicPawn,"",PUBLIC_PAWN);
   if let Some(hp)=i(ctx,p.controller,"m_PlayerDataGlobal.m_iHealthMax").filter(|v|*v>0){public.set("max_health",Some(hp as f64));}
   if public.get("health").is_some(){public.set("health_age",Some(0.));}
   if let Some(len)=count(ctx,p.controller,"m_PlayerDataGlobal.m_vecUpgrades",64){public.extras.insert("inventory-known".into(),1.);public.extras.insert("inventory-count".into(),len as f64);}else{public.extras.insert("inventory-known".into(),0.);}
   let public_scope=put(&mut out,public);
   let mut private=allied_player(p,position(ctx,p.pawn));private.copy_fields(ctx,p.controller,CopyGroup::TeamPlayer,"m_PlayerDataGlobal.",TEAM_PLAYER);private.copy_fields(ctx,p.pawn,CopyGroup::TeamPawn,"",TEAM_PAWN);
   for alias in ["m_PlayerDataGlobal.m_iBoonCount","m_PlayerDataGlobal.m_nBoonCount","m_iBoonCount","m_nBoonCount"]{if let Some(v)=i(ctx,p.controller,alias){private.set("boon_count",Some(v as f64));break}}
   // Current exact positions are authorized for the player's allied team.
   // Merge into the same team-private scope; no public/observer coordinate copy.
   let private_scope=put(&mut out,private);self.projection_cache.store(2,binding.pawn,binding,None,vec![public_scope,private_scope]);
   }
   self.profile.elapsed("projection.player_basic_zone",phase,out.len()-before);let phase=FineInstant::now();let before=out.len();
   let binding=Binding::of(p);let controller_id=binding.controller;
   if let Some(rows)=self.projection_cache.cached(0,controller_id,binding,None).filter(|rows|out.has_all(rows)){out.keep(rows);}
   else{let mut stat_rows=vec![];
   let prefix="m_PlayerDataGlobal.m_vecStatViewerModifierValues";
   for slot in 0..count(ctx,p.controller,prefix,2048).unwrap_or(crate::snapshots::STAT_VIEWER_SLOTS){let(Some(code),Some(amount))=(ai(ctx,p.controller,prefix,slot,"m_eValType"),an(ctx,p.controller,prefix,slot,"m_flValue"))else{continue};let modifier=ai(ctx,p.controller,prefix,slot,"m_SourceModifierID");let ability=ai(ctx,p.controller,prefix,slot,"m_SourceAbilityID");let mut node=Node::new(format!("player:{}:stat:{}:type:{}:modifier:{:?}:ability:{:?}",p.steam,slot,code,modifier,ability),"upgrade",3);node.owned(p);node.class=Some(modifier.map(|v|format!("stat-source:{v}")).unwrap_or("stat-source-unknown".into()));node.ability=ability.filter(|v|*v!=0);node.extras.insert(format!("stat-contribution-type:{code}"),amount);stat_rows.push(node);}
    let scopes=stat_rows.into_iter().map(|node|put(&mut out,node)).collect();self.projection_cache.store(0,controller_id,binding,None,scopes);
   }
   self.profile.elapsed("projection.stat_contributions",phase,out.len()-before);let phase=FineInstant::now();let before=out.len();
   let inv="m_PlayerDataGlobal.m_vecUpgrades";let upgrade="m_PlayerDataGlobal.m_vecAbilityUpgradeState";
   // The authoritative native loader enumerates eight fixed controller slots.
   // A real transmitted length overrides that bound; typed values are required.
   let refresh=self.projection_cache.upgrades.get(&controller_id).is_none_or(|(old,_,_)|*old!=binding)||self.projection_cache.dirty_groups.get(&controller_id.0).copied().unwrap_or(0)&2!=0;
   if refresh{
    let mut ability_slots=vec![];for slot in 0..count(ctx,p.controller,upgrade,128).unwrap_or(8){let Some(token)=ai(ctx,p.controller,upgrade,slot,"m_ItemID").filter(|v|*v>0)else{continue};ability_slots.push((slot,token,packed_upgrade(arr(ctx,p.controller,upgrade,slot,"m_nUpgradeInfo"))));}
    let inventory=count(ctx,p.controller,inv,64).map(|len|(0..len).filter_map(|slot|ai(ctx,p.controller,inv,slot,"").filter(|v|*v>=0).map(|token|(slot,token))).collect());
    self.projection_cache.upgrades.insert(controller_id,(binding,ability_slots,inventory));
   }
   let (_,ability_slots,inventory)=&self.projection_cache.upgrades[&controller_id];
   for &(slot,token,packed) in ability_slots{let key=(p.steam,token);if let Some(old)=upgrade_states.get_mut(&key){if *old!=packed{*old=None;}}else{upgrade_states.insert(key,packed);}upgrade_slots.push((p,slot,token,packed));}
   if let Some(rows)=self.projection_cache.cached(3,controller_id,binding,None).filter(|rows|out.has_all(rows)){
    out.keep(rows);for &(_,token,_) in ability_slots{if controller_upgrade_for(&upgrade_states,p.steam,token).is_some()&&inventory.as_ref().is_some_and(|inventory|inventory.iter().any(|&(_,item)|item==token)){joined_upgrades.insert((p.steam,token));}}
   }else{let mut scopes=vec![];
   if let Some(inventory)=inventory{for &(slot,token) in inventory{let mut node=Node::new(format!("player:{}:upgrade:{slot}",p.steam),"upgrade",0);node.owned(p);node.ability=Some(token);node.class=Some("public-inventory-item".into());if let Some(packed)=controller_upgrade_for(&upgrade_states,p.steam,token){apply_upgrade_bits(&mut node,packed);joined_upgrades.insert((p.steam,token));}scopes.push(put(&mut out,node));}}
    self.projection_cache.store(3,controller_id,binding,None,scopes);
   }
   self.profile.elapsed("projection.inventory_upgrades",phase,out.len()-before);
  }
  let phase=FineInstant::now();let before=out.len();let channel_bindings:Vec<_>=ps.iter().map(Binding::of).collect();let channel_ids:Vec<_>=team_entities.iter().map(|e|(e.index,e.serial)).collect();
  if self.projection_cache.channels.is_none()||self.projection_cache.channel_bindings!=channel_bindings||self.projection_cache.channel_ids!=channel_ids||channel_ids.iter().any(|id|self.projection_cache.dirty_groups.get(&id.0).copied().unwrap_or(0)&4!=0){
   self.projection_cache.channels=Some(channels(ctx,&ps,classes,&team_entities,&mut self.projection_cache.channel_slots,&self.projection_cache.dirty_slots));self.projection_cache.channel_bindings=channel_bindings;self.projection_cache.channel_ids=channel_ids;
  }
  // Take and return the cached slots instead of copying every channel.
  let chans=self.projection_cache.channels.take().unwrap();let mut camps:BTreeMap<(i64,String),Vec<&Slot>>=BTreeMap::new();let mut markers:HashMap<Id,Vec<&Slot>>=HashMap::new();
  for ch in &chans{for s in &ch.slots{
   let classification=classes.get(&s.class).map(String::as_str);
   if classification.is_some_and(|c|matches!(c,"CLASS_WEAK_NEUTRAL_CAMP"|"CLASS_MEDIUM_NEUTRAL_CAMP"|"CLASS_STRONG_NEUTRAL_CAMP"|"CLASS_VAULT_NEUTRAL_CAMP"|"CLASS_SUPER_NEUTRAL_CAMP")){camps.entry((s.raw/ch.divisor,classification.unwrap().into())).or_default().push(s);continue}
   let Some(e)=model_entities.get(ctx.entities(),(s.raw/ch.divisor)as i32).filter(|e|e.active)else{continue};if e.class_name.as_ref()=="CCitadelZipLineNode"{continue}let expected=match e.class_name.as_ref(){"CCitadelPlayerPawn"=>Some(1),"CNPC_TrooperBoss"=>Some(5),"CNPC_Boss_Tier2"=>Some(29),"CCitadel_KothCashIn"=>Some(92),_=>None};if expected.is_some_and(|x|x!=s.class){continue}
   if objective(&e.class_name){markers.entry((e.index,e.serial)).or_default().push(s);self.objective_markers.insert((e.index,e.serial));}
   let mut node=Node::entity(e,2);node.observer=Some(ch.team);node.team=Some(s.team);if let Some(p)=by_pawn.get(&e.id()){node.key=format!("player:{}",p.steam);node.owner=Some(p.steam);node.hero=p.hero;}
   let scope=node.scope();let id=out.registry.intern(scope.clone());
   if !out.output.get_id(id).is_some_and(|old|memory_generation_matches(old,&node)){out.insert(node);}else{out.mark(id);}
   self.projection_cache.remembered_scopes.insert(id);
   let node=out.output.get_id_mut(id).unwrap();node.set("protected",None);
   node.set("visible",Some(s.visible as u8 as f64));node.set("map_age",None);node.set("health_age",None);
   if s.visible{node.set("map_x",s.x);node.set("map_y",s.y);if s.x.is_some()&&s.y.is_some(){node.set("map_age",Some(0.));node.map_time=time;}if !matches!(e.class_name.as_ref(),"CNPC_Trooper"|"CNPC_TrooperNeutral"|"CNPC_Neutral_Bug"){node.set("health_byte",s.health);if s.health.is_some(){node.set("health_age",Some(0.));node.health_time=time;}}node.set("protected",s.protected.map(|v|v as u8 as f64));}

  }}
  // Retain last qualified observations when storage/PVS drops an entity. This
  // is memory, never a death or destruction inference. Generation is in key.
  self.projection_cache.remembered_scopes.retain(|&id|{
   let Some(old)=out.output.get_id_mut(id).filter(|old|player_memory_current(old,&ps))else{return false};
   if !out.registry.seen[id]{old.set("visible",None);old.set("protected",None);old.set("map_age",None);old.set("health_age",None);out.mark(id);}
   true
  });
  self.profile.elapsed("projection.minimap_memory",phase,out.len()-before);let phase=FineInstant::now();let before=out.len();
  for((index,class),rows)in camps{let flags:HashSet<_>=rows.iter().map(|s|s.visible).collect();if flags.len()!=1{continue}let available=rows[0].visible;let key=format!("minimap-camp:{index}:{class}");let mut node=Node::new(key,"neutral",0);node.class=Some("camp-minimap-marker".into());node.extras.insert("camp_available".into(),available as u8 as f64);let observed=if available&&rows.iter().all(|s|(s.x,s.y)==(rows[0].x,rows[0].y)){rows[0].x.zip(rows[0].y).map(|(x,y)|[x,y])}else{None};
   self.camp_position(&mut node,observed,time);put(&mut out,node);
  }
  self.profile.elapsed("projection.camps_respawn",phase,out.len()-before);let phase=FineInstant::now();let before=out.len();
  for e in objective_entities{
   let id=(e.index,e.serial);
   let scope=if let Some((scope,values))=self.projection_cache.objective_rows.get(&id).filter(|(scope,_)|!self.projection_cache.dirty.contains(&e.index)&&out.has_all(&[*scope])){
    let scope=*scope;out.keep(&[scope]);let node=out.output.get_id_mut(scope).unwrap();
    node.values=*values;node.extras.remove("life-evidence:missing-objective-marker");node.extras.remove("position-observation-age");
    node.source_time=None;node.map_time=None;node.health_time=None;node.world_time=None;node.position_observation_time=None;node.position_emitted_time=None;node.position_observation_live=false;
    scope
   }else{
    let mut node=Node::entity(e,0);node.team=i(ctx,e,"m_iTeamNum");node.xyz(position(ctx,e));if node.get("world_x").is_some(){node.set("world_age",Some(0.));}node.copy_fields(ctx,e,CopyGroup::Objective,"",&[("health","m_iHealth"),("max_health","m_iMaxHealth"),("lifestate","m_lifeState")]);
    if node.get("health").is_some(){node.set("health_age",Some(0.));}let alive=node.get("health").map(|hp|hp>0.&&node.get("lifestate").is_none_or(|life|life==0.)).or_else(||node.get("lifestate").map(|life|life==0.));node.set("is_alive",alive.map(|v|v as u8 as f64));if let Some(lane)=i(ctx,e,"m_iLane"){node.extras.insert(format!("lane-code:{lane}"),1.);}
    let values=node.values;let scope=put(&mut out,node);self.projection_cache.objective_rows.insert(id,(scope,values));scope
   };
   let node=out.output.get_id_mut(scope).unwrap();
   if let Some(rows)=markers.get(&(e.index,e.serial)){let p=rows[0].protected;if rows.iter().all(|s|s.protected==p){node.set("protected",p.map(|v|v as u8 as f64));}let hp=rows[0].health;if rows.iter().all(|s|s.health==hp){node.set("health_byte",hp);}}
  }
  // Public objectives remain remembered even if no active server entity exists.
  for(scope,old)in self.history.iter(){if old.class.as_deref().is_some_and(objective)&&old.audience==0&&!out.contains_key(scope){
   let id=out.registry.intern(scope.clone());
   if let Some(saved)=out.output.get_id_mut(id){saved.values=old.values;RawObservation::save(old).restore(saved);saved.set("health_age",None);saved.set("world_age",None);out.mark(id);}
   else{let mut saved=old.clone();saved.set("health_age",None);saved.set("world_age",None);put(&mut out,saved);}
  }}
  out.for_each_mut(|node|{if !(node.audience==0&&node.class.as_deref().is_some_and(objective)){return}
   let Some(id)=node.id else{return};let was_missing=self.history.get(&node.scope()).is_some_and(|old|old.extras.get("life-evidence:missing-objective-marker")==Some(&1.));
   let mut statuses=vec![];
   if self.objective_markers.contains(&id)&&chans.len()==2&&chans.iter().all(|c|c.complete){
    for ch in &chans{let found:Vec<_>=ch.slots.iter().filter(|s|s.raw/ch.divisor==id.0 as i64).collect();if found.is_empty(){statuses.push(Some(false));continue}let exact=model_entities.get(ctx.entities(),id.0).is_some_and(|e|e.active&&e.serial==id.1);let proper=classes.get(&found[0].class).is_some_and(|c|c==objective_code(node.class.as_deref().unwrap()));statuses.push((exact&&proper).then_some(found[0].visible));}
   }
   apply_objective_latch(node,was_missing,&statuses);
  });
  self.profile.elapsed("projection.objective_hp_protection",phase,out.len()-before);let phase=FineInstant::now();let before=out.len();
  let rules=&rule_entities;
  if rules.len()==1{let e=rules[0];let id=(e.index,e.serial);
   if self.projection_cache.dirty_groups.get(&e.index).copied().unwrap_or(0)&8==0&&self.projection_cache.rule_scopes.get(&id).is_some_and(|rows|out.has_all(rows)){out.keep(&self.projection_cache.rule_scopes[&id]);}
   else{let mut rule_rows=vec![];
   let mut world=Node::entity(e,0);world.key="world:rules".into();world.kind="unknown".into();
   if let Some(v)=b(ctx,e,"m_pGameRules.m_bGamePaused"){world.extras.insert("is_paused".into(),v as u8 as f64);}
   if let Some(v)=n(ctx,e,"m_pGameRules.m_tNextMidBossSpawnTime"){world.extras.insert("next_midboss".into(),v);}
   rule_rows.push(world);
   let origin="m_pGameRules.m_vecNeutralCampTimerOrigins";let spawn="m_pGameRules.m_vecNeutralCampNextSpawnTimes";let mid="m_pGameRules.m_vecNeutralCampTimerIsMidBoss";let lengths=[origin,spawn,mid].map(|p|count(ctx,e,p,256));if lengths[0].is_some()&&lengths.iter().all(|n|*n==lengths[0]){for slot in 0..lengths[0].unwrap(){let(Some(FieldValue::Vector3(pos)),Some(next),Some(is_mid))=(arr(ctx,e,origin,slot,""),an(ctx,e,spawn,slot,""),ab(ctx,e,mid,slot,""))else{continue};if !pos.iter().all(|v|v.is_finite()){continue}let mut node=Node::new(format!("camp-timer:{slot}:{:?}",pos),"neutral",0);node.class=Some("camp-timer-record".into());node.xyz(Some(pos.map(f64::from)));node.extras.insert("camp_next_spawn_source_clock".into(),next);node.extras.insert("camp_timer_is_midboss".into(),is_mid as u8 as f64);rule_rows.push(node);}}
    let scopes=rule_rows.into_iter().map(|node|put(&mut out,node)).collect();self.projection_cache.rule_scopes.insert(id,scopes);
   }
   self.profile.elapsed("projection.camps_respawn",phase,out.len()-before);
  }else{self.profile.elapsed("projection.camps_respawn",phase,0);}
  let phase=FineInstant::now();let before=out.len();if rules.len()==1{let e=rules[0];let rifts=&rift_entities;if rifts.len()==1&&n(ctx,e,"m_pGameRules.m_timeKothCashInStarted").is_some_and(|v|v>=0.){let active=n(ctx,e,"m_pGameRules.m_timeKothCashInStarted").is_some_and(|v|v>0.);let mut node=Node::entity(rifts[0],0);node.set("capture_active",Some(active as u8 as f64));node.set("disabled",b(ctx,rifts[0],"m_bDisabled").map(|v|v as u8 as f64));
    if let Some(FieldValue::Vector3(pos))=field(ctx,e,"m_pGameRules.m_vKothCashInCurrentLocation"){if active&&*pos!=[0.;3]&&pos.iter().all(|v|v.is_finite()&&v.abs()<1_000_000.){node.xyz(Some(pos.map(f64::from)));node.set("world_age",Some(0.));}}
    if active{for (side,dest) in ["capture_fraction_state0","capture_fraction_state1"].into_iter().enumerate(){node.set(dest,an(ctx,e,"m_pGameRules.m_vecTeamKothStates",side,"m_flCaptureProgressFrac").filter(|v|(0. ..=1.).contains(v)));}}put(&mut out,node);}
  }
  self.profile.elapsed("projection.rift",phase,out.len()-before);let phase=FineInstant::now();let before=out.len();
  for e in ability_entities{let Some(owner)=resolved(ctx,e,"m_hOwnerEntity").and_then(|p|by_pawn.get(&p.id())).copied()else{continue};let Some(token)=i(ctx,e,"m_nSubclassID")else{continue};
   let binding=Binding::of(owner);let id=(e.index,e.serial);let packed=controller_upgrade_for(&upgrade_states,owner.steam,token);if packed.is_some(){joined_upgrades.insert((owner.steam,token));}
   if let Some(rows)=self.projection_cache.cached(1,id,binding,packed).filter(|rows|out.has_all(rows)){out.keep(rows);continue}
   let mut public=Node::entity(e,0);public.kind="upgrade".into();public.owned(owner);public.ability=Some(token);public.set("upgrade_bits",n(ctx,e,"m_nAbilityUpgradeBits"));if let Some(packed)=packed{apply_upgrade_bits(&mut public,packed);}public.set("charges",n(ctx,e,"m_nCharges"));for(name,v)in named_numeric(ctx,e,"m_nUpgradeInfo.",&mut self.field_names){public.extras.insert(name,v);}
   let mut team=public.clone();team.audience=3;team.values=[None;66];team.extras.clear();team.set("cooldown_source_time",n(ctx,e,"m_flCooldownEnd"));team.set("ammo",n(ctx,e,"m_iClip1"));
   for(dest,source)in ABILITY_FIELDS{let v=field(ctx,e,source);let value=numeric(v);let dst=if matches!(*dest,"remaining_charges"|"legacy_charges"|"ability_slot"){&mut public}else{&mut team};dst.extras.insert(format!("ability-state-known:{dest}"),value.is_some()as u8 as f64);dst.extras.insert(format!("ability-state-present:{dest}"),v.is_some()as u8 as f64);if let Some(v)=value{dst.extras.insert(format!("ability-state:{dest}"),v);}}
   let scopes=vec![put(&mut out,public),put(&mut out,team)];self.projection_cache.store(1,id,binding,packed,scopes);
  }
  // Controller state remains usable even before a matching live ability is
  // materialized; it is never restricted to the purchased-item vector.
  for(owner,slot,token,packed)in upgrade_slots{if joined_upgrades.contains(&(owner.steam,token)){continue}let mut node=Node::new(format!("player:{}:ability-upgrade:{}:{}:{slot}",owner.steam,owner.controller.index,owner.controller.serial),"upgrade",0);node.owned(owner);node.hero=owner.hero;node.ability=Some(token);node.class=Some("public-controller-ability-upgrade".into());if upgrade_states.get(&(owner.steam,token))==Some(&packed){if let Some(packed)=packed{apply_upgrade_bits(&mut node,packed);}}put(&mut out,node);}
  self.profile.elapsed("projection.ability_state",phase,out.len()-before);let phase=FineInstant::now();
  // Ammo fraction is a separate qualified field, never substituted for a
  // clip count. Class discovery uses the replay's serializer, not hero names.
  let mut ammo_rows=0;for e in ammo_entities{
   let Some(owner)=resolved(ctx,e,"m_hOwnerEntity").and_then(|p|by_pawn.get(&p.id())).copied()else{continue};
   let mut node=Node::entity(e,3);node.kind="upgrade".into();node.owned(owner);node.ability=i(ctx,e,"m_nSubclassID");
   let node=out.get_or_insert(node);
   if let Some(v)=n(ctx,e,"m_flAmmoFrac").filter(|v|*v>=0.){node.extras.insert("ammo_fraction".into(),v);}
   for name in ["m_bInReload","m_bReloading"]{if let Some(v)=b(ctx,e,name){node.extras.insert(name.into(),v as u8 as f64);}}
   ammo_rows+=1;
  }
  self.profile.elapsed("projection.ammo",phase,ammo_rows);let phase=FineInstant::now();let before=out.len();
  self.projection_cache.zip.project(ctx,&zip_entities,&self.projection_cache.dirty,&mut self.zip_homes);
  let revision=self.projection_cache.zip.revision();
  if self.projection_cache.zip_revision!=Some(revision)||!out.has_all(&self.projection_cache.zip_scopes){
   self.projection_cache.zip_scopes=self.projection_cache.zip.rows().iter().map(|node|put(&mut out,node.clone())).collect();
   self.projection_cache.zip_revision=Some(revision);
  }else{out.keep(&self.projection_cache.zip_scopes);}
  self.profile.elapsed("projection.zipline_reach",phase,out.len()-before);let phase=FineInstant::now();let before=out.len();
  // Current modifier table entries retain precise parent generation. Expired
  // entries are filtered by the native simulation-time effectiveness predicate.
  let modifier_clock={let _span=crate::direct_method_profile::span(90);boon_parser::ModifierClock::resolve(ctx)};let game_time={let _span=crate::direct_method_profile::span(91);modifier_clock.game_time(ctx)};
  self.projection_cache.modifier_clock(game_time);
  for(serial,m)in self.modifiers.entries(){if !{let _span=crate::direct_method_profile::span(92);self.projection_cache.modifier_effective(*serial,m.in_aura_range,m.last_applied_time,m.duration)}{continue}let Some(raw)=m.parent else{continue};let Some(pawn)=ctx.entities().get((raw&16383)as i32).filter(|p|p.serial==raw>>14)else{continue};let Some(owner)=by_pawn.get(&pawn.id()).copied()else{continue};let Some(code)=m.modifier_subclass else{continue};
   if code==crate::snapshots::BARRIER_TRACKER_MODIFIER_ID{if let Some(v)=m.float2.filter(|v|v.is_finite()&&*v>=0.){if let Some(player)=out.get_mut(&(format!("player:{}",owner.steam),3,None)){player.set("barrier",Some(v as f64));}}}
   let binding=Binding::of(owner);
   if let Some((_,scope))=self.projection_cache.modifier_rows.get(serial).filter(|(old,scope)|*old==binding&&out.has_all(&[*scope])){out.keep(&[*scope]);continue}
   let mut node=Node::new(format!("modifier:{}:{serial}",owner.steam),"upgrade",3);node.owned(owner);node.class=Some(format!("modifier:{code}"));node.source_time=self.modifier_times.get(serial).copied().flatten();node.ability=m.ability_subclass.map(i64::from);if let Some(v)=m.stack_count{node.extras.insert("stacks".into(),v as f64);}if let Some(v)=m.duration.filter(|v|v.is_finite()){node.extras.insert("duration".into(),v as f64);}if let Some(v)=m.creation_time.filter(|v|v.is_finite()){node.extras.insert("modifier-creation-source-clock".into(),v as f64);}let scope=put(&mut out,node);self.projection_cache.modifier_rows.insert(*serial,(binding,scope));}
  self.profile.elapsed("projection.modifiers",phase,out.len()-before);
  self.projection_cache.channels=Some(chans);self.projection_cache.dirty.clear();self.projection_cache.dirty_groups.clear();self.projection_cache.dirty_slots.clear();self.projection_cache.finish(out)
 }
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn schema_invalidation_keeps_memory_but_drops_all_derived_plans(){
  let mut cache=ProjectionCache::default();cache.seeded=true;let mut node=Node::new("remembered".into(),"neutral",2);node.map_time=Some(3.);let id=cache.scope_registry.intern(node.scope());cache.output.insert_at(id,node.scope(),node);cache.remembered_scopes.insert(id);cache.dirty.insert(5);cache.field_groups.insert((1,2),3);cache.slot_fields.insert((1,2),(true,Some(1)));cache.zip_revision=Some(2);
  cache.invalidate_schema();assert!(!cache.seeded);assert!(cache.field_groups.is_empty());assert!(cache.slot_fields.is_empty());assert!(cache.zip_revision.is_none());assert_eq!(cache.output.values().next().unwrap().map_time,Some(3.));assert!(cache.remembered_scopes.contains(&id));assert_eq!(cache.scope_registry.scopes[id].0,"remembered");
 }
 #[test]fn fow_slot_cache_preserves_absence_and_only_decodes_affected_records(){
  let mut records:Vec<Option<usize>>=vec![];let mut reads=vec![];
  refresh_slot_records(&mut records,3,None,|j|{reads.push(j);(j!=1).then_some(j)});assert_eq!(reads,vec![0,1,2]);assert_eq!(records,vec![Some(0),None,Some(2)]);
  reads.clear();refresh_slot_records(&mut records,3,None,|j|{reads.push(j);Some(10+j)});assert!(reads.is_empty());assert_eq!(records[1],None);
  let dirty=Some(HashSet::from([1]));refresh_slot_records(&mut records,3,Some(&dirty),|j|{reads.push(j);Some(10+j)});assert_eq!(reads,vec![1]);assert_eq!(records,vec![Some(0),Some(11),Some(2)]);
  reads.clear();refresh_slot_records(&mut records,4,None,|j|{reads.push(j);Some(j)});assert_eq!(reads,vec![3]);
  reads.clear();refresh_slot_records(&mut records,2,None,|j|{reads.push(j);Some(j)});assert!(reads.is_empty());assert_eq!(records,vec![Some(0),Some(11)]);
  refresh_slot_records(&mut records,2,Some(&None),|j|{reads.push(j);None});assert_eq!(reads,vec![0,1]);assert_eq!(records,vec![None,None]);
 }
 #[test]fn fow_slot_invalidation_accepts_padded_and_unpadded_slots_and_falls_back_conservatively(){
  assert_eq!(fow_slot("m_vecFOWEntities.0007.m_nHealthPercent"),Some(7));assert_eq!(fow_slot("m_vecFOWEntities.7.m_bVisibleOnMap"),Some(7));
  for unknown in ["m_vecFOWEntities","m_vecFOWEntities.7","m_vecFOWEntities.bad.m_nEntIndex","m_vecFOWEntities.7.","m_vecOther.7.m_nEntIndex"]{assert_eq!(fow_slot(unknown),None);}
 }
 #[test]fn persistent_scopes_reuse_nodes_preserve_scope_order_and_drop_unseen(){
  let mut first=ProjectedMap::new(DenseNodes::default(),ScopeRegistry::default());
  let later=first.insert(Node::new("entity:2".into(),"upgrade",0));
  let earlier=first.insert(Node::new("entity:10".into(),"upgrade",0));
  let (output,registry)=first.finish();let pointer=output[&registry.scopes[later]].key.as_ptr();
  let mut next=ProjectedMap::new(output,registry);assert_eq!(next.len(),0);assert!(!next.contains_key(&next.registry.scopes[later]));
  next.keep(&[later,earlier]);assert_eq!(next.len(),2);next.keep(&[later]);assert_eq!(next.len(),2);
  let (output,registry)=next.finish();assert_eq!(output.keys().map(|scope|scope.0.as_str()).collect::<Vec<_>>(),vec!["entity:10","entity:2"]);
  assert_eq!(output[&registry.scopes[later]].key.as_ptr(),pointer);
  let mut next=ProjectedMap::new(output,registry);next.keep(&[earlier]);let(output,_)=next.finish();assert_eq!(output.len(),1);assert_eq!(output.keys().next().unwrap().0,"entity:10");
 }
 #[test]fn missing_cached_scopes_are_rebuilt_after_expiry_or_binding_absence(){
  let mut first=ProjectedMap::new(DenseNodes::default(),ScopeRegistry::default());let id=first.insert(Node::new("modifier:1".into(),"upgrade",3));let(output,registry)=first.finish();
  let next=ProjectedMap::new(output,registry);let(output,registry)=next.finish();let next=ProjectedMap::new(output,registry);assert!(!next.has_all(&[id]));
 }
 #[test]fn scope_compaction_bounds_history_and_remaps_remembered_handles(){
  let mut out=ProjectedMap::new(DenseNodes::default(),ScopeRegistry::default());let live=out.insert(Node::new("live".into(),"neutral",2));
  for generation in 0..100{out.registry.intern((format!("old:{generation}"),0,None));}
  let mut cache=ProjectionCache::default();cache.remembered_scopes.insert(live);cache.rows.insert((0,(1,1)),(Binding{steam:1,team:2,hero:None,pawn:(1,1),controller:(2,1)},None,vec![live]));
  let output=cache.finish(out);assert_eq!(output.len(),1);assert_eq!(cache.scope_registry.scopes.len(),1);assert_eq!(cache.remembered_scopes.len(),1);let id=*cache.remembered_scopes.iter().next().unwrap();assert_eq!(cache.scope_registry.scopes[id].0,"live");assert!(cache.rows.is_empty());
 }
 #[test]fn modifier_expiry_cache_matches_native_effectiveness_matrix(){
  let mut cache=ProjectionCache::default();
  for aura in [None,Some(false),Some(true)]{for applied in [None,Some(-2.),Some(0.),Some(2.),Some(f32::MAX),Some(f32::NAN)]{for duration in [None,Some(-1.),Some(0.),Some(1.),Some(2.),Some(f32::MAX),Some(f32::INFINITY),Some(f32::NAN)]{
   cache.modifier_changed(1);
   let entry=boon_proto::proto::CModifierTableEntry{in_aura_range:aura,last_applied_time:applied,duration,..Default::default()};
   for now in [Some(-2.),Some(0.),Some(1.),Some(2.),Some(4.),None,Some(1.),Some(f32::INFINITY),Some(f32::NAN),Some(f32::MAX),Some(0.)]{
    cache.modifier_clock(now);assert_eq!(cache.modifier_effective(1,aura,applied,duration),boon_parser::modifier_state::modifier_is_effective_at(&entry,now),"aura={aura:?} applied={applied:?} duration={duration:?} now={now:?}");
   }
  }}}
 }
 #[test]fn modifier_expiry_cache_tracks_boundaries_rollback_and_unknown_clocks(){
  let mut cache=ProjectionCache::default();
  cache.modifier_clock(Some(4.));assert!(cache.modifier_effective(1,None,Some(3.),Some(2.)));
  cache.modifier_clock(Some(4.999));assert!(cache.modifier_effective(1,None,Some(3.),Some(2.)));
  cache.modifier_clock(Some(5.));assert!(!cache.modifier_effective(1,None,Some(3.),Some(2.)));
  cache.modifier_clock(None);assert!(cache.modifier_effective(1,None,Some(3.),Some(2.)));
  cache.modifier_clock(Some(6.));assert!(!cache.modifier_effective(1,None,Some(3.),Some(2.)));
  cache.modifier_clock(Some(4.));assert!(cache.modifier_effective(1,None,Some(3.),Some(2.)));
  cache.modifier_clock(Some(5.));assert!(!cache.modifier_effective(1,None,Some(3.),Some(2.)));
 }
 #[test]fn modifier_refresh_rejects_old_deadlines_and_preserves_indefinite_aura_rules(){
  let mut cache=ProjectionCache::default();cache.modifier_clock(Some(1.));
  assert!(cache.modifier_effective(1,None,Some(0.),Some(2.)));cache.modifier_changed(1);
  assert!(cache.modifier_effective(1,None,Some(1.),Some(5.)));cache.modifier_clock(Some(2.));assert!(cache.modifier_effective(1,None,Some(1.),Some(5.)));
  for duration in [None,Some(0.),Some(-1.),Some(f32::NAN),Some(f32::INFINITY)]{
   cache.modifier_changed(2);assert!(cache.modifier_effective(2,None,Some(0.),duration));
  }
  assert!(!cache.modifier_effective(3,Some(false),Some(0.),Some(0.)));cache.modifier_clock(None);assert!(!cache.modifier_effective(3,Some(false),Some(0.),Some(0.)));
 }
 #[test]fn stat_cache_ignores_unrelated_controller_fields_but_never_binding_or_array_changes(){
  let mut cache=ProjectionCache::default();let binding=Binding{steam:1,team:2,hero:Some(3),pawn:(4,5),controller:(6,7)};
  cache.store(0,binding.controller,binding,None,vec![0]);
  cache.dirty.insert(6);cache.dirty_groups.insert(6,2);assert!(cache.cached(0,binding.controller,binding,None).is_some());
  cache.dirty_groups.insert(6,1);assert!(cache.cached(0,binding.controller,binding,None).is_none());cache.dirty_groups.clear();
  assert!(cache.cached(0,binding.controller,Binding{pawn:(4,8),..binding},None).is_none());
  assert!(cache.cached(0,binding.controller,Binding{team:3,..binding},None).is_none());
 }
 #[test]fn controller_hero_ability_upgrade_joins_exact_owner_without_inventory_membership(){let states=HashMap::from([((123,99),Some(5<<17))]);assert_eq!(controller_upgrade_for(&states,123,99),Some(5<<17));assert_eq!(controller_upgrade_for(&states,124,99),None);assert_eq!(controller_upgrade_for(&states,123,98),None);}
 #[test]fn exact_position_is_only_in_allied_team_scope(){let pawn=Entity::from_fields(4,8,0,"CCitadelPlayerPawn",true,Default::default()).unwrap();let controller=Entity::from_fields(1,1,0,"CCitadelPlayerController",true,Default::default()).unwrap();let p=Player{steam:123,team:2,pawn:&pawn,controller:&controller,hero:Some(7)};let public=public_player(&p);let allied=allied_player(&p,Some([1.,2.,3.]));assert_eq!(public.get("world_x"),None);assert_eq!(allied.audience,3);assert_eq!(allied.team,Some(2));assert_eq!(allied.get("world_x"),Some(1.));assert_eq!(allied.id,public.id);}
 #[test]fn packed_controller_upgrades_preserve_mask_zero_and_signed_wire_bits(){let mut node=Node::new("ability".into(),"upgrade",0);apply_upgrade_bits(&mut node,0);assert_eq!(node.get("upgrade_bits"),Some(0.));assert_eq!(node.extras["upgrade_bit:0"],0.);apply_upgrade_bits(&mut node,5<<17);assert_eq!(node.get("upgrade_bits"),Some(5.));assert_eq!(node.extras["upgrade_bit:0"],1.);assert_eq!(node.extras["upgrade_bit:1"],0.);assert_eq!(node.extras["upgrade_bit:2"],1.);assert_eq!(packed_upgrade(Some(&FieldValue::I32(i32::MIN))),Some(0x8000_0000));assert_eq!(packed_upgrade(None),None);assert_eq!(packed_upgrade(Some(&FieldValue::F32(0.))),None);}
 fn objective_node()->Node{let mut node=Node::new("entity:1".into(),"guardian",0);node.set("health",Some(100.));node.set("max_health",Some(100.));node.set("is_alive",Some(1.));node.set("lifestate",Some(0.));node.set("protected",Some(0.));node}
 #[test]fn missing_objective_latch_survives_unknown_channels_and_requires_actual_hp_to_revive(){
  let mut node=objective_node();apply_objective_latch(&mut node,false,&[Some(false),Some(false)]);assert_eq!(node.get("is_alive"),Some(0.));assert_eq!(node.get("health"),None);
  for statuses in [vec![],vec![Some(true),None],vec![Some(true),Some(false)]]{let mut raw=objective_node();apply_objective_latch(&mut raw,true,&statuses);assert_eq!(raw.get("health"),None);assert_eq!(raw.get("is_alive"),Some(0.));}
  let mut unknown=objective_node();unknown.set("health",None);apply_objective_latch(&mut unknown,true,&[Some(true),Some(true)]);assert_eq!(unknown.get("is_alive"),Some(0.));
  let mut revived=objective_node();apply_objective_latch(&mut revived,true,&[Some(true),Some(true)]);assert_eq!(revived.get("health"),Some(100.));assert_eq!(revived.get("is_alive"),Some(1.));assert!(!revived.extras.contains_key("life-evidence:missing-objective-marker"));
 }
 #[test]fn camp_position_survives_missing_sample_without_refreshing_observation_time(){
  let mut collector=Collector::default();let mut observed=Node::new("minimap-camp:1:weak".into(),"neutral",0);
  collector.camp_position(&mut observed,Some([12.,24.]),Some(3.));collector.append(1,Some(3.),[(observed.scope(),observed.clone())].into_iter().collect());
  collector.append(2,Some(4.),BTreeMap::new());assert!(!collector.rows.last().unwrap().2.active);
  let mut returned=Node::new(observed.key.clone(),"neutral",0);collector.camp_position(&mut returned,None,Some(5.));
  assert_eq!(returned.get("map_x"),Some(12.));assert_eq!(returned.get("map_age"),None);assert_eq!(returned.map_time,Some(3.));assert!(!returned.extras.contains_key("camp_available"));
 }
 #[test]fn same_steam_new_pawn_never_inherits_previous_generation_fow_memory(){
  let pawn=Entity::from_fields(4,8,0,"CCitadelPlayerPawn",true,Default::default()).unwrap();let controller=Entity::from_fields(1,1,0,"CCitadelPlayerController",true,Default::default()).unwrap();
  let p=Player{steam:123,team:2,pawn:&pawn,controller:&controller,hero:Some(7)};let mut old=Node::new("player:123".into(),"player",2);old.owner=Some(123);old.team=Some(2);old.hero=Some(7);old.id=Some((4,7));old.observer=Some(2);old.set("map_x",Some(17.));let mut current=old.clone();current.id=Some((4,8));
  assert!(!memory_generation_matches(&old,&current));assert!(!player_memory_current(&old,&[p]));
  let p=Player{steam:123,team:2,pawn:&pawn,controller:&controller,hero:Some(7)};assert!(player_memory_current(&current,&[p]));let mut switched=current.clone();switched.hero=Some(8);assert!(!memory_generation_matches(&current,&switched));
 }
}

#[cfg(test)]
#[path="direct_getter_plan_tests.rs"]
mod getter_plan_tests;
