//! Exact native debris messages, distinct from PVS or inferred destruction.
use crate::*;
use boon_parser::{Context,GameEvent};
use boon_proto::proto::CCitadelEntityMsgBreakablePropSpawnDebris;

pub(crate) const MESSAGE_TYPE:u32=500;
#[derive(Default)]
pub(crate) struct DebrisColumns {
 tick:Vec<i32>,target:Vec<Option<u32>>,damage:Vec<Option<f32>>,
 index:Vec<Option<i32>>,serial:Vec<Option<u32>>,class:Vec<Option<String>>,
 active:Vec<Option<bool>>,qualified:Vec<bool>,
}
impl DebrisColumns {
 pub(crate) fn collect(&mut self,ctx:&Context,events:&[GameEvent])->boon_parser::Result<()> {
  self.collect_with_visibility(ctx,events,&mut crate::direct_model_columns::ModelEntityView::default())
 }
 pub(crate) fn collect_with_visibility(&mut self,ctx:&Context,events:&[GameEvent],model_entities:&mut crate::direct_model_columns::ModelEntityView)->boon_parser::Result<()> {let _method_span=crate::direct_method_profile::span(78);
  for event in events.iter().filter(|e|e.msg_type==MESSAGE_TYPE){
   let message=CCitadelEntityMsgBreakablePropSpawnDebris::decode(event.payload.as_slice())?;
   let target=message.entity_msg.and_then(|m|m.target_entity).filter(|v|*v!=16777215&&*v!=u32::MAX);
   let entity=target.filter(|_|event.tick==ctx.tick()).and_then(|h|model_entities.get_by_handle(ctx.entities(),h));
   self.tick.push(event.tick);self.target.push(target);self.damage.push(message.damage.filter(|x|x.is_finite()));
   self.index.push(entity.map(|e|e.index));self.serial.push(entity.map(|e|e.serial));self.class.push(entity.map(|e|e.class_name.to_string()));
   self.active.push(entity.map(|e|e.active));self.qualified.push(entity.is_some_and(|e|e.class_name.as_ref()=="CCitadel_BreakableProp"));
  }Ok(())
 }
 pub(crate) fn frame(self)->PolarsResult<DataFrame>{let _method_span=crate::direct_method_profile::span(79);df_from_columns(vec![
  Column::new("tick".into(),self.tick),Column::new("target_handle_raw".into(),self.target),
  Column::new("damage".into(),self.damage),Column::new("target_entity_index".into(),self.index),
  Column::new("target_entity_serial".into(),self.serial),Column::new("target_entity_class".into(),self.class),
  Column::new("target_active".into(),self.active),Column::new("breakable_identity_qualified".into(),self.qualified),
 ])}
}
#[cfg(test)]mod tests {
 use super::*;
 #[test]fn debris_payload_preserves_zero_and_absent_damage(){
  let a=CCitadelEntityMsgBreakablePropSpawnDebris{entity_msg:Some(boon_proto::proto::CEntityMsg{target_entity:Some(7+(9<<14))}),damage:Some(0.),..Default::default()};
  let b=CCitadelEntityMsgBreakablePropSpawnDebris::decode(a.encode_to_vec().as_slice()).unwrap();
  assert_eq!(b.damage,Some(0.));assert_eq!(b.entity_msg.unwrap().target_entity,Some(7+(9<<14)));
  let empty=CCitadelEntityMsgBreakablePropSpawnDebris::decode([].as_slice()).unwrap();assert_eq!(empty.damage,None);
 }
}
