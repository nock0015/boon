//! Synthetic, bounded unit fixtures. No recorded replay or benchmark workload.
use super::*;
use boon_proto::proto::*;

fn packet(messages:&[(u32,Vec<u8>)])->Vec<u8> {
    let mut bits=Vec::new();
    let mut write=|value:u64,count:u32|bits.extend((0..count).map(|bit|value&(1<<bit)!=0));
    for (kind,body) in messages {
        match *kind {
            0..=15=>write(u64::from(*kind),6),
            16..=255=>{write(u64::from((*kind&15)|16),6);write(u64::from(*kind>>4),4);},
            256..=4095=>{write(u64::from((*kind&15)|32),6);write(u64::from(*kind>>4),8);},
            _=>{write(u64::from((*kind&15)|48),6);write(u64::from(*kind>>4),28);},
        }
        let mut length=Vec::new();prost::encoding::encode_varint(body.len()as u64,&mut length);
        for byte in length.into_iter().chain(body.iter().copied()) {write(u64::from(byte),8);}
    }
    CDemoPacket {data:Some(bits.chunks(8).map(|chunk|chunk.iter().enumerate().fold(0,|byte,(bit,&set)|byte|(u8::from(set)<<bit))).collect()),..Default::default()}.encode_to_vec()
}
fn source(malformed:bool)->Demo {
    let mut entity_bits=Vec::new();
    let mut write_entity=|value:u64,count:u32|entity_bits.extend((0..count).map(|bit|value&(1<<bit)!=0));
    write_entity(7,6);write_entity(2,2);write_entity(0,1);write_entity(9,17);write_entity(0,8);
    // Valve field-path finish code is 10 in source bit order. No fields follow.
    write_entity(1,1);write_entity(0,1);
    let entity_data=entity_bits.chunks(8).map(|chunk|chunk.iter().enumerate().fold(0,|byte,(bit,&set)|byte|(u8::from(set)<<bit))).collect();
    let mut first=vec![(SvcMessages::SvcPacketEntities as u32,CsvcMsgPacketEntities {max_entries:Some(16),updated_entries:Some(1),entity_data:Some(entity_data),..Default::default()}.encode_to_vec())];
    // More than one batch in a single source tick exercises the exact retained
    // source-tick allowance, row splitting, ordering, and final partial flush.
    for amount in 0..1025 {
        first.push((Msg::KEUserMsgDamage as u32,CCitadelUserMessageDamage {damage:Some(amount),ability_id:Some(0),entindex_victim:Some(7),..Default::default()}.encode_to_vec()));
        first.push((Msg::KEUserMsgCurrencyChanged as u32,CCitadelUserMsgCurrencyChanged {delta:Some(amount),currency_source:Some(999),victim:Some(7+(9<<14)),..Default::default()}.encode_to_vec()));
    }
    first.push((Msg::KEUserMsgImportantAbilityUsed as u32,CCitadelUserMessageImportantAbilityUsed {ability_name:Some("duplicate".into()),..Default::default()}.encode_to_vec()));
    first.push(first.last().unwrap().clone());
    first.push((crate::direct_breakable_messages::MESSAGE_TYPE,CCitadelEntityMsgBreakablePropSpawnDebris {entity_msg:Some(CEntityMsg {target_entity:Some(7+(9<<14))}),damage:Some(0.),..Default::default()}.encode_to_vec()));
    let mut second=vec![
        (Msg::KEUserMsgDamage as u32,if malformed {vec![0x80]}else{Vec::new()}),
        (Msg::KEUserMsgCurrencyChanged as u32,Vec::new()),
        (Msg::KEUserMsgGameOver as u32,CCitadelUserMessageGameOver {winning_team:Some(2),..Default::default()}.encode_to_vec()),
    ];
    // A large model-only message must not count against the event collector's
    // byte guard. Unknown protobuf fields remain valid skipped source payload.
    let mut model_only=Vec::new();prost::encoding::encode_varint((127<<3)|2,&mut model_only);
    prost::encoding::encode_varint(4*1024*1024,&mut model_only);model_only.resize(model_only.len()+4*1024*1024,0);
    second.push((crate::direct_breakable_messages::MESSAGE_TYPE,model_only));
    // A duplicate after the first bounded flush must remain a new occurrence.
    second.push((Msg::KEUserMsgImportantAbilityUsed as u32,CCitadelUserMessageImportantAbilityUsed {ability_name:Some("duplicate".into()),..Default::default()}.encode_to_vec()));
    let serializers=CsvcMsgFlattenedSerializer {symbols:vec!["CompanionUnknown".into()],serializers:vec![ProtoFlattenedSerializerT {serializer_name_sym:Some(0),..Default::default()}],..Default::default()}.encode_to_vec();
    let mut send_tables=Vec::new();prost::encoding::encode_varint(serializers.len()as u64,&mut send_tables);send_tables.extend(serializers);
    let class_info=CDemoClassInfo {classes:vec![c_demo_class_info::ClassT {class_id:Some(0),network_name:Some("CompanionUnknown".into()),..Default::default()}]}.encode_to_vec();
    let mut bytes=b"PBDEMS2\0".to_vec();bytes.extend([0;8]);
    for (command,tick,body) in [
        (EDemoCommands::DemSendTables as i32,0,CDemoSendTables {data:Some(send_tables)}.encode_to_vec()),
        (EDemoCommands::DemClassInfo as i32,0,class_info),
        (EDemoCommands::DemSyncTick as i32,0,Vec::new()),
        (EDemoCommands::DemPacket as i32,10,packet(&first)),
        (EDemoCommands::DemPacket as i32,11,packet(&second)),
        (EDemoCommands::DemPacket as i32,12,packet(&[(Msg::KEUserMsgCurrencyChanged as u32,Vec::new())])),
        (EDemoCommands::DemStop as i32,13,Vec::new()),
    ] {
        for value in [command as u64,tick,body.len()as u64] {prost::encoding::encode_varint(value,&mut bytes);}
        bytes.extend(body);
    }
    Demo {parser:boon_parser::Parser::from_bytes(bytes).without_prepared_index(),path:PathBuf::new(),build:0,map_name:String::new(),total_ticks:13,playback_time:0.,tick_rate:30,match_id:None,game_mode:0,paused_ticks:None,paused_ticks_disk:None,cached_datasets:DatasetCache::default(),cached_barriers:std::sync::OnceLock::new(),cached_summary:None,game_over:None,game_over_match_clock:None,game_over_match_clock_scanned:false,banned_hero_ids:None,always_events_scanned:false,cached_players:None}
}
fn table(root:&Path,name:&str)->Option<DataFrame> {
    let mut result:Option<DataFrame>=None;
    for line in std::fs::read_to_string(root.join("batches.jsonl")).unwrap().lines() {
        let item:serde_json::Value=serde_json::from_str(line).unwrap();
        if item["table"]!=name {continue}
        let frame=IpcReader::new(File::open(root.join(item["file"].as_str().unwrap())).unwrap()).finish().unwrap();
        if let Some(result)=result.as_mut(){result.vstack_mut(&frame).unwrap();}else{result=Some(frame);}
    }
    result
}
fn model_frame(py:Python<'_>,frames:&Py<PyDict>,name:&str)->DataFrame {
    frames.bind(py).get_item(name).unwrap().unwrap().extract::<PyDataFrame>().unwrap().0
}

#[test]
fn combined_pass_matches_existing_collectors_across_bounded_flushes_and_optional_modes() {
    Python::initialize();
    Python::attach(|py|for optional in [false,true] {
        let root=tempfile::tempdir().unwrap();let separate=root.path().join("separate");let combined=root.path().join("combined");
        let names=vec!["damage".into(),"abilities".into(),"resource_events".into()];
        let mut old=source(false);let old_frames=old.direct_model_columns(py,vec![10,11,12],None,false).unwrap();
        let old_receipt=old.bounded_dataset_spool(py,separate.to_str().unwrap(),names.clone(),None,optional).unwrap();
        let mut new=source(false);let (new_frames,new_receipt)=new.direct_model_columns_and_events(py,combined.to_str().unwrap(),vec![10,11,12],None,Some(names),None,optional,false).unwrap();
        let old_receipt:serde_json::Value=serde_json::from_str(&old_receipt).unwrap();let new_receipt:serde_json::Value=serde_json::from_str(&new_receipt).unwrap();
        for field in ["rows","batches","batchRows","batchBytes","sourceTickAllowance","decodeCohorts","optionalEventsContract","optionalEventCoverage","manifestSha256","ownerStatus","peakTableCollectorRows","peakTableCollectorBytes"] {assert_eq!(old_receipt[field],new_receipt[field],"receipt {field}");}
        #[cfg(feature="direct-method-profile")]
        for field in ["source_ticks","source_events"] {assert_eq!(old_receipt["eventProfile"][0][field],new_receipt["eventProfile"][0][field],"event profile count {field}");}
        #[cfg(not(feature="direct-method-profile"))]
        {assert!(old_receipt["eventProfile"].as_array().unwrap().is_empty());assert!(new_receipt["eventProfile"].as_array().unwrap().is_empty());}
        assert_eq!(new_receipt["sharedSourcePass"]["decodePasses"],1);
        #[cfg(feature="direct-method-profile")]
        assert!(new_receipt["eventProfile"][0]["shared_pass"].as_bool().unwrap());
        for name in ["damage","abilities","resource_events","soul_bag_pickups","breakable_events"] {
            match (table(&separate,name),table(&combined,name)) {
                (Some(left),Some(right))=>assert!(left.equals_missing(&right),"table {name}"),
                (None,None)=>{},_=>panic!("table presence differs: {name}"),
            }
        }
        for name in ["clock","entity_updates","extras","relations","projection_counts","player_bindings","ability_usage","breakable_messages"] {assert!(model_frame(py,&old_frames,name).equals_missing(&model_frame(py,&new_frames,name)),"model {name}");}
        let resource=table(&combined,"resource_events").unwrap();assert_eq!(resource.height(),1027);
        assert_eq!(resource.column("sequence").unwrap().u64().unwrap().get(1026),Some(1026));
        assert_eq!(resource.column("steam_id").unwrap().null_count(),1027);assert_eq!(resource.column("source_entity_serial").unwrap().null_count(),2);
        assert_eq!(resource.column("source_entity_serial").unwrap().u32().unwrap().get(0),Some(9));
        let damage=table(&combined,"damage").unwrap();assert_eq!(damage.height(),1026);assert_eq!(damage.column("damage_available").unwrap().bool().unwrap().get(1025),Some(false));
        assert_eq!(table(&combined,"abilities").unwrap().height(),3);
        let debris=model_frame(py,&new_frames,"breakable_messages");assert_eq!(debris.height(),2);assert_eq!(debris.column("target_entity_serial").unwrap().null_count(),2);
        assert_eq!(model_frame(py,&new_frames,"entity_updates").height(),0);
        let clock=model_frame(py,&new_frames,"clock");
        for row in 0..clock.height() {let tick=clock.column("tick").unwrap().i32().unwrap().get(row).unwrap();assert_eq!(clock.column("is_postgame").unwrap().bool().unwrap().get(row),Some(tick>=11));}
        assert!(!new.parser.has_prepared_index());
    });
}

#[test]
fn empty_model_selection_still_collects_events_and_bad_cohorts_do_not_create_output() {
    Python::initialize();Python::attach(|py|{
        let root=tempfile::tempdir().unwrap();let path=root.path().join("empty");let mut demo=source(false);
        let (frames,receipt)=demo.direct_model_columns_and_events(py,path.to_str().unwrap(),Vec::new(),None,None,None,true,false).unwrap();
        assert_eq!(model_frame(py,&frames,"clock").height(),0);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&receipt).unwrap()["rows"]["damage"],1026);
        for names in [Vec::new(),vec!["player_ticks".into()],vec!["urn".into()],vec!["abilities".into()]] {
            let path=root.path().join("invalid");assert!(demo.direct_model_columns_and_events(py,path.to_str().unwrap(),vec![10],None,Some(names),None,false,false).is_err());assert!(!path.exists());
        }
    });
}

#[test]
fn observer_cancellation_and_malformed_events_never_publish_completion() {
    Python::initialize();Python::attach(|py|{
        let root=tempfile::tempdir().unwrap();let path=root.path().join("cancelled");let mut demo=source(false);let mut spool=Spool::new(&path,None).unwrap();
        let mut seen=0;let mut visit=|_ctx:&boon_parser::Context,_events:&[boon_parser::GameEvent]|{seen+=1;Err(aborted())};
        assert!(demo.collect_datasets_with_observer(py,&[Dataset::Damage],Some(&mut spool),None,None,Some(&mut visit)).is_err());
        assert_eq!(seen,1);assert!(!path.join("complete.json").exists());assert!(std::fs::read_to_string(path.join("batches.jsonl")).unwrap().is_empty());
        let path=root.path().join("malformed");let mut demo=source(true);
        assert!(demo.direct_model_columns_and_events(py,path.to_str().unwrap(),vec![10,11,12],None,None,None,true,false).is_err());assert!(!path.join("complete.json").exists());
    });
}
