//! Opt-in, source-time evidence. PVS departure is never a destruction claim.
use crate::*;
use boon_parser::{Context, Entity, FieldValue};
pub(super) const CONTRACT: &str = "boon-optional-causal-events-v1";
pub(super) const BREAKABLE: &str = "CCitadel_BreakableProp";

fn key(ctx: &Context, entity: &Entity, name: &str) -> Option<u64> {
    ctx.serializers().get(&entity.class_name)?.resolve_field_key(name)
}
fn uint(ctx: &Context, entity: &Entity, name: &str) -> Option<u32> {
    match entity.field_value(key(ctx, entity, name))? {
        FieldValue::U32(v) => Some(*v), FieldValue::U64(v) => (*v).try_into().ok(),
        _ => None,
    }
}

pub(super) fn exact_generation(tick:i32,current_tick:i32,index:i32,serial:u32,changes:impl Iterator<Item=(i32,u32)>)->bool {
    tick==current_tick && !changes.into_iter().any(|(changed_index,changed_serial)|changed_index==index && changed_serial!=serial)
}

pub(super) struct CurrentController { serial:u32, steam:u64, pawns:[Option<boon_parser::EntityId>;2] }
pub(super) trait ActorLookup { fn current_controller(&self,index:i32)->Option<CurrentController>; }
impl ActorLookup for Context {
    fn current_controller(&self,index:i32)->Option<CurrentController> {
        let controller=self.entities().get(index).filter(|e|e.class_name.as_ref()=="CCitadelPlayerController")?;
        let FieldValue::U64(steam)=controller.field_value(key(self,controller,"m_steamID"))? else{return None;};
        if *steam==0{return None;}
        let pawns=["m_hHeroPawn","m_hPawn"].map(|name|controller.get_handle(key(self,controller,name))
            .and_then(|handle|self.entities().get_by_handle(handle)).map(Entity::id));
        Some(CurrentController{serial:controller.serial,steam:*steam,pawns})
    }
}

#[derive(Clone)]
pub(super) struct Row {
    pub tick:i32,pub sequence:u64,pub event:&'static str,
    pub entity_index:i32,pub entity_serial:Option<u32>,pub subclass_id:Option<u32>,
    pub actor_steam_id:Option<u64>,pub actor_index:Option<i32>,pub actor_serial:Option<u32>,
    pub damage:Option<i32>,pub health_after:Option<i32>,pub is_melee:Option<bool>,
}
#[derive(Default)]
pub(super) struct Collector { pub rows:Vec<Row>, sequence:u64, pub class_available:bool, actors:HashMap<i32,(u32,u64)> }
impl Collector {
    fn push(&mut self,mut row:Row) { row.sequence=self.sequence;self.sequence+=1;self.rows.push(row); }
    /// Revalidate cached controller candidates against the source-time entity
    /// state. Only current controller slots are examined, never all entities.
    /// Missing/new controllers remain unknown until collected, never guessed.
    pub fn verified_actor(&self,ctx:&impl ActorLookup,pawn:&Entity)->Option<u64> {
        if pawn.class_name.as_ref()!="CCitadelPlayerPawn" {return None;}
        let mut found=None;
        for (index,(serial,steam)) in &self.actors {
            let Some(current)=ctx.current_controller(*index) else{continue;};
            if current.serial!=*serial || current.steam!=*steam || !current.pawns.contains(&Some(pawn.id())){continue;}
            if found.is_some_and(|old|old!=*steam){return None;}
            found=Some(*steam);
        }
        found
    }
    pub fn lifecycle(&mut self,ctx:&Context) {
        for &index in ctx.entities().updated_indices() {
            let Some(controller)=ctx.entities().get(index) else{continue;};
            self.actors.remove(&index);
            if controller.class_name.as_ref()!="CCitadelPlayerController"{continue;}
            let Some(FieldValue::U64(steam))=controller.field_value(key(ctx,controller,"m_steamID")) else{continue;};
            if *steam==0{continue;}
            self.actors.insert(index,(controller.serial,*steam));
        }
        self.actors.retain(|index,(serial,_)|ctx.entities().get(*index).is_some_and(|entity|entity.serial==*serial));
        self.class_available |= ctx.serializers().get(BREAKABLE).is_some();
        for change in ctx.entities().entity_changes() {
            if change.class_name.as_ref()!=BREAKABLE {continue;}
            use boon_parser::entity::EntityChangeKind as K;
            let event=match change.kind {K::Created=>"created",K::Reactivated=>"reactivated",K::LeftPvs=>"left_pvs",K::Deleted=>"deleted",_=>continue};
            // Full-packet same-generation rebuild is not a lifecycle outcome.
            if matches!(change.kind,K::Deleted) && ctx.entities().get(change.index).is_some_and(|e|e.id()==change.id()) {continue;}
            let entity=ctx.entities().get(change.index).filter(|e|e.id()==change.id());
            self.push(Row {tick:ctx.tick(),sequence:0,event,entity_index:change.index,entity_serial:Some(change.serial),
                subclass_id:entity.and_then(|e|uint(ctx,e,"m_nSubclassID")),actor_steam_id:None,actor_index:None,actor_serial:None,
                damage:None,health_after:None,is_melee:None});
        }
    }
    pub fn damage(&mut self,ctx:&Context,tick:i32,msg:&boon_proto::proto::CCitadelUserMessageDamage)->Option<u64> {
        let victim=ctx.entities().get(msg.entindex_victim?).filter(|e|e.class_name.as_ref()==BREAKABLE)?;
        let attacker=msg.entindex_attacker.and_then(|i|ctx.entities().get(i));
        let sequence=self.sequence;
        // Damage carries indices, not generation handles. A same-tick slot
        // replacement cannot safely identify which lifetime was hit/attacking.
        let exact=|entity:&Entity|exact_generation(tick,ctx.tick(),entity.index,entity.serial,ctx.entities().entity_changes().iter().map(|change|(change.index,change.serial)));
        let victim_exact=exact(victim);
        let attacker=attacker.filter(|entity|exact(entity));
        let melee=msg.citadel_type.zip(msg.flags).map(|(t,f)|crate::datasets::classify_melee_damage(t,f).0);
        self.push(Row {tick,sequence,event:"damage",entity_index:victim.index,entity_serial:victim_exact.then_some(victim.serial),
            subclass_id:victim_exact.then(||uint(ctx,victim,"m_nSubclassID")).flatten(),actor_steam_id:if victim_exact {attacker.and_then(|e|self.verified_actor(ctx,e))}else{None},
            actor_index:attacker.map(|e|e.index),actor_serial:attacker.map(|e|e.serial),damage:msg.damage,
            health_after:msg.victim_health_new,is_melee:melee});
        Some(sequence)
    }
    pub fn frame(&mut self)->PolarsResult<DataFrame> {
        let rows=std::mem::take(&mut self.rows);
        macro_rules! c {($name:expr,$get:expr)=>{Column::new($name.into(),rows.iter().map($get).collect::<Vec<_>>())};}
        df_from_columns(vec![c!("tick",|r|r.tick),c!("sequence",|r|r.sequence),c!("event",|r|r.event),
            c!("entity_index",|r|r.entity_index),c!("entity_serial",|r|r.entity_serial),c!("subclass_id",|r|r.subclass_id),
            c!("actor_steam_id",|r|r.actor_steam_id),c!("actor_entity_index",|r|r.actor_index),c!("actor_entity_serial",|r|r.actor_serial),
            c!("damage",|r|r.damage),c!("health_after",|r|r.health_after),c!("is_melee",|r|r.is_melee)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn message_indices_require_same_tick_and_unambiguous_generation() {
        assert!(exact_generation(10,10,5,2,[(5,2),(6,1)].into_iter()));
        assert!(!exact_generation(9,10,5,2,[].into_iter()));
        assert!(!exact_generation(10,10,5,2,[(5,1),(5,2)].into_iter()));
    }
    #[test]
    fn raw_purchase_enum_respects_replay_version_boundary_and_unknowns() {
        use boon_parser::demo::ability_change_name;
        assert_eq!(ability_change_name(Some(4),Some("/games/citadel_v6710/citadel")),"failure");
        assert_eq!(ability_change_name(Some(4),Some("/games/citadel_v6711/citadel")),"leveled_up");
        assert_eq!(ability_change_name(Some(4),None),"unknown");
        assert_eq!(ability_change_name(Some(999),Some("/games/citadel_v6774/citadel")),"unknown");
        assert_eq!(ability_change_name(None,None),"unknown");
    }
    #[test]
    fn verified_ownership_rejects_current_controller_reuse_reassignment_and_conflicts() {
        struct View(HashMap<i32,CurrentController>);
        impl ActorLookup for View { fn current_controller(&self,index:i32)->Option<CurrentController>{
            self.0.get(&index).map(|v|CurrentController{serial:v.serial,steam:v.steam,pawns:v.pawns})
        }}
        let pawn=Entity::from_fields(14,7,0,"CCitadelPlayerPawn",true,Default::default()).unwrap();
        let next=Entity::from_fields(14,8,0,"CCitadelPlayerPawn",true,Default::default()).unwrap();
        let prop=Entity::from_fields(14,7,0,BREAKABLE,true,Default::default()).unwrap();
        let mut collector=Collector::default();let mut view=View(HashMap::new());
        collector.actors.insert(1,(3,76561198000000001));
        assert_eq!(collector.verified_actor(&view,&pawn),None);
        view.0.insert(1,CurrentController{serial:3,steam:76561198000000001,pawns:[Some(pawn.id()),None]});
        assert_eq!(collector.verified_actor(&view,&pawn),Some(76561198000000001));
        assert_eq!(collector.verified_actor(&view,&next),None);
        assert_eq!(collector.verified_actor(&view,&prop),None);
        view.0.get_mut(&1).unwrap().serial=4;
        assert_eq!(collector.verified_actor(&view,&pawn),None);
        view.0.get_mut(&1).unwrap().serial=3;view.0.get_mut(&1).unwrap().steam=76561198000000002;
        assert_eq!(collector.verified_actor(&view,&pawn),None);
        view.0.get_mut(&1).unwrap().steam=76561198000000001;view.0.get_mut(&1).unwrap().pawns=[Some(next.id()),None];
        assert_eq!(collector.verified_actor(&view,&pawn),None);
        view.0.get_mut(&1).unwrap().pawns=[Some(pawn.id()),None];
        collector.actors.insert(2,(1,76561198000000002));
        view.0.insert(2,CurrentController{serial:1,steam:76561198000000002,pawns:[Some(pawn.id()),None]});
        assert_eq!(collector.verified_actor(&view,&pawn),None);
    }
    #[test]
    fn bounded_flush_keeps_source_sequence_nulls_and_no_destruction_inference() {
        let mut collector=Collector::default();
        let row=Row {tick:10,sequence:0,event:"left_pvs",entity_index:9,entity_serial:Some(4),subclass_id:None,
            actor_steam_id:None,actor_index:None,actor_serial:None,damage:None,health_after:None,is_melee:None};
        collector.push(row.clone());let frame=collector.frame().unwrap();
        assert_eq!(frame.height(),1);assert_eq!(frame.column("actor_steam_id").unwrap().null_count(),1);
        assert_eq!(frame.column("subclass_id").unwrap().null_count(),1);
        assert_eq!(frame.column("event").unwrap().str().unwrap().get(0),Some("left_pvs"));
        assert!(collector.rows.is_empty());assert_eq!(collector.frame().unwrap().height(),0);
        collector.push(row);let frame=collector.frame().unwrap();
        assert_eq!(frame.column("sequence").unwrap().u64().unwrap().get(0),Some(1));
    }
}
