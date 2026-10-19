//! Bounded borrowed-plan tests with the unchanged scalar getters as oracle.
use super::*;
use boon_proto::proto::*;

fn context(names:&[String])->Context {
    let mut symbols=vec!["GetterOracle".to_owned(),"uint32".to_owned()];
    let mut fields=Vec::new();
    for name in names {
        let (prefix,leaf)=name.rsplit_once('.').unwrap_or(("",name));
        let leaf_sym=symbols.len()as i32;symbols.push(leaf.to_owned());
        let send_node_sym=if prefix.is_empty(){None}else{let index=symbols.len()as i32;symbols.push(prefix.to_owned());Some(index)};
        fields.push(ProtoFlattenedSerializerFieldT {var_type_sym:Some(1),var_name_sym:Some(leaf_sym),send_node_sym,..Default::default()});
    }
    let flattened=CsvcMsgFlattenedSerializer {symbols,serializers:vec![ProtoFlattenedSerializerT {serializer_name_sym:Some(0),fields_index:(0..fields.len()as i32).collect(),..Default::default()}],fields,..Default::default()}.encode_to_vec();
    let mut data=Vec::new();prost::encoding::encode_varint(flattened.len()as u64,&mut data);data.extend(flattened);
    let class_info=CDemoClassInfo {classes:vec![c_demo_class_info::ClassT {class_id:Some(1),network_name:Some("GetterOracle".into()),..Default::default()}]}.encode_to_vec();
    let mut bytes=b"PBDEMS2\0".to_vec();bytes.extend([0;8]);
    for (command,body) in [
        (EDemoCommands::DemSendTables as u64,CDemoSendTables {data:Some(data)}.encode_to_vec()),
        (EDemoCommands::DemClassInfo as u64,class_info),
        (EDemoCommands::DemSyncTick as u64,Vec::new()),
        (EDemoCommands::DemStop as u64,Vec::new()),
    ] {for value in [command,0,body.len()as u64]{prost::encoding::encode_varint(value,&mut bytes);}bytes.extend(body);}
    boon_parser::Parser::from_bytes(bytes).without_prepared_index().parse_init().unwrap()
}
fn entity(ctx:&Context,values:impl IntoIterator<Item=(String,FieldValue)>)->Entity {
    let serializer=ctx.serializers().get("GetterOracle").unwrap();
    let fields=values.into_iter().map(|(name,value)|(serializer.resolve_field_key(&name).expect("fixture key"),value)).collect();
    Entity::from_fields(7,9,1,"GetterOracle",true,fields).unwrap()
}
fn copy_oracle(ctx:&Context,e:&Entity,prefix:&str,fields:&[(&str,&str)])->Node {
    let mut node=Node::new("oracle".into(),"test",0);
    for (destination,source) in fields {node.set(destination,n(ctx,e,&format!("{prefix}{source}")));}node
}

#[test]
fn every_copy_group_matches_scalar_oracle_with_typed_missing_and_zero_values() {
    let objective=&[("health","m_iHealth"),("max_health","m_iMaxHealth"),("lifestate","m_lifeState")][..];
    for (group,prefix,fields) in [
        (CopyGroup::PublicPlayer,"m_PlayerDataGlobal.",PUBLIC_PLAYER),
        (CopyGroup::PublicPawn,"",PUBLIC_PAWN),
        (CopyGroup::TeamPlayer,"m_PlayerDataGlobal.",TEAM_PLAYER),
        (CopyGroup::TeamPawn,"",TEAM_PAWN),
        (CopyGroup::Objective,"",objective),
    ] {
        let names:Vec<_>=fields.iter().map(|(_,source)|format!("{prefix}{source}")).collect();
        // Last source deliberately has no serializer key. Earlier missing values
        // still have keys, proving cached absence differs from unresolved keys.
        let ctx=context(&names[..names.len()-1]);
        for mode in 0..6 {
            let _scope=FieldKeyScope::enter();
            let values=names[..names.len()-1].iter().enumerate().filter_map(|(index,name)|{
                if mode==3&&index%2==0{return None}
                let boolean=name.rsplit('.').next().unwrap().starts_with("m_b");
                let value=match mode {
                    0=>if boolean{FieldValue::Bool(false)}else{FieldValue::I32(0)},
                    1=>if boolean{FieldValue::Bool(true)}else{FieldValue::F32(-0.)},
                    2=>if boolean{FieldValue::I32(1)}else{FieldValue::Bool(true)},
                    4=>FieldValue::F32(f32::NAN),
                    5=>FieldValue::F32(f32::INFINITY),
                    _=>FieldValue::I32((index+7)as i32),
                };Some((name.clone(),value))
            });
            let e=entity(&ctx,values);let expected=copy_oracle(&ctx,&e,prefix,fields);
            for _ in 0..2 {let mut actual=Node::new("oracle".into(),"test",0);actual.copy_fields(&ctx,&e,group,prefix,fields);assert_eq!(actual.values,expected.values);for (a,b)in actual.values.iter().zip(expected.values){assert_eq!(a.map(f64::to_bits),b.map(f64::to_bits));}}
        }
    }
}

const POSITION_NAMES:[[&str;6];2]=[
 ["CBodyComponent.m_skeletonInstance.m_vecOrigin.m_cellX","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_cellY","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_cellZ","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecX","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecY","CBodyComponent.m_skeletonInstance.m_vecOrigin.m_vecZ"],
 ["CBodyComponent.m_cellX","CBodyComponent.m_cellY","CBodyComponent.m_cellZ","CBodyComponent.m_vecX","CBodyComponent.m_vecY","CBodyComponent.m_vecZ"]];
fn position_oracle(ctx:&Context,e:&Entity)->Option<[f64;3]> {
    for names in POSITION_NAMES {let values=names.map(|name|n(ctx,e,name));if values.iter().all(Option::is_some){let v=values.map(Option::unwrap);return Some([v[0]*512.-16384.+v[3],v[1]*512.-16384.+v[4],v[2]*512.-16384.+v[5]])}}None
}
#[test]
fn both_position_layouts_and_missing_value_fallback_match_scalar_oracle() {
    let all:Vec<_>=POSITION_NAMES.into_iter().flatten().map(str::to_owned).collect();
    for primary_keys in [true,false] {
        let ctx=context(if primary_keys{&all}else{&all[6..]});
        for mode in 0..6 {
            let _scope=FieldKeyScope::enter();
            let values=all.iter().enumerate().filter_map(|(index,name)|{
                if !primary_keys&&index<6||mode==1&&index==0||mode==5&&index%6==0{return None}
                let value=if mode==2&&index==0{FieldValue::Bool(true)}else if mode==3&&index==0{FieldValue::F32(f32::NAN)}else if mode==4&&index==0{FieldValue::F32(f32::INFINITY)}else{FieldValue::F32((index+1)as f32)};
                Some((name.clone(),value))
            });
            let e=entity(&ctx,values);let expected=position_oracle(&ctx,&e);
            for _ in 0..2{let actual=position(&ctx,&e);assert_eq!(actual,expected);assert_eq!(actual.map(|v|v.map(f64::to_bits)),expected.map(|v|v.map(f64::to_bits)));}
        }
    }
}
#[test]
fn schema_reset_replaces_positive_and_negative_copy_and_position_plans() {
    let _scope=FieldKeyScope::enter();
    let none=context(&[]);let empty=entity(&none,[]);let fields=&[("health","m_iHealth")][..];
    let mut node=Node::new("oracle".into(),"test",0);node.copy_fields(&none,&empty,CopyGroup::Objective,"",fields);assert_eq!(node.get("health"),None);assert_eq!(position(&none,&empty),None);
    let mut names:Vec<_>=POSITION_NAMES.into_iter().flatten().map(str::to_owned).collect();names.push("m_iHealth".into());
    let next=context(&names);let e=entity(&next,names.iter().enumerate().map(|(index,name)|(name.clone(),FieldValue::I32(index as i32+1))));
    reset_getter_plans();node.copy_fields(&next,&e,CopyGroup::Objective,"",fields);assert_eq!(node.get("health"),Some(13.));assert_eq!(position(&next,&e),position_oracle(&next,&e));
    // Reordered wire fields change packed keys without changing class ID/name.
    names.reverse();let reordered=context(&names);let e=entity(&reordered,names.iter().enumerate().map(|(index,name)|(name.clone(),FieldValue::I32(if name=="m_iHealth"{77}else{100+index as i32}))));
    reset_getter_plans();node.copy_fields(&reordered,&e,CopyGroup::Objective,"",fields);assert_eq!(node.get("health"),Some(77.));assert_eq!(position(&reordered,&e),position_oracle(&reordered,&e));
    reset_getter_plans();node.copy_fields(&none,&empty,CopyGroup::Objective,"",fields);assert_eq!(node.get("health"),None);assert_eq!(position(&none,&empty),None);
}
