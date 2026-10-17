use pbdems2::{DemoParser, PlaybackCheckpoint, PreparedPlayback};

use crate::entity::{ClassInfo, SerializerContainer};
use crate::error::{Error, Result};

use super::{CitadelAdapter, Context, GameEvent, HEADER_SIZE, Parser};

const DEFAULT_TICK_INTERVAL: f32 = 1.0 / 30.0;

impl Parser {
    pub(crate) fn replay_identity(&self) -> &std::sync::Arc<()> {
        &self.replay_identity
    }

    pub(crate) fn stat_checkpoint(
        &self,
        previous: Option<&PlaybackCheckpoint<CitadelAdapter>>,
        tick: i32,
        classes: &std::collections::HashSet<&str>,
        mut on_tick: impl FnMut(&Context),
    ) -> Result<PlaybackCheckpoint<CitadelAdapter>> {
        let parser = self.demo_parser()?;
        let visit = |ctx: &Context, _: &mut CitadelAdapter| {
            on_tick(ctx);
            Ok(())
        };
        if let Some(previous) = previous {
            previous.replay_through(parser, tick, visit)
        } else {
            let mut session = self.prepared()?.session(parser)?;
            session.adapter_mut().skip_modifier_snapshots();
            session.checkpoint_through(tick, classes, visit)
        }
    }
    fn demo_parser(&self) -> Result<DemoParser<'_>> {
        DemoParser::new(self.data()).map_err(Error::from)
    }

    fn prepared(&self) -> Result<&PreparedPlayback<CitadelAdapter>> {
        if let Some(prepared) = self.prepared_cache.get() {
            return Ok(prepared);
        }

        let _guard = self.prepared_lock.lock().map_err(|_| Error::Parse {
            context: "prepared-playback cache lock poisoned".into(),
        })?;
        if self.prepared_cache.get().is_none() {
            let prepared = self
                .demo_parser()?
                .prepare(CitadelAdapter::default(), DEFAULT_TICK_INTERVAL)?;
            self.prepared_cache.set(prepared).unwrap_or_else(|_| {
                unreachable!("prepared-playback cache initialized while locked")
            });
        }
        Ok(self
            .prepared_cache
            .get()
            .expect("prepared-playback cache populated"))
    }

    /// Parse send tables from the signon stream.
    pub fn parse_send_tables(&self) -> Result<SerializerContainer> {
        if self.bounded {return Ok(self.parse_init()?.serializers().clone());}
        Ok(self.prepared()?.initial_state().serializers().clone())
    }

    /// Parse network class information from the signon stream.
    pub fn parse_class_info(&self) -> Result<ClassInfo> {
        if self.bounded {return Ok(self.parse_init()?.class_info().clone());}
        Ok(self.prepared()?.initial_state().class_info().clone())
    }

    /// Parse all initialization data through `DEM_SyncTick`.
    pub fn parse_init(&self) -> Result<Context> {
        if self.bounded { return self.demo_parser()?.initial_state(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL); }
        Ok(self.prepared()?.initial_state().clone())
    }

    /// Parse the demo to a specific tick, restoring the nearest full packet.
    pub fn parse_to_tick(&self, target_tick: i32) -> Result<Context> {
        let parser = self.demo_parser()?;
        if self.bounded {
            // Scan framed headers with constant memory, preserving the exact
            // nearest-keyframe seek semantics of the prepared API.
            let mut start=None;
            let mut past_sync=false;
            for frame in parser.demo().commands() {
                let frame=frame.map_err(Error::from)?;
                if frame.header().cmd==super::dem::STOP {break;}
                if frame.header().cmd==boon_proto::proto::EDemoCommands::DemSyncTick as i32 {past_sync=true;continue;}
                if past_sync && frame.header().cmd==super::dem::FULL_PACKET && frame.header().tick<=target_tick { start=Some(frame.offset()); }
            }
            let init=self.parse_init()?;
            let names:Vec<String>=init.serializers().iter().map(|(name,_)|name.to_string()).collect();
            let classes=names.iter().map(String::as_str).collect();
            return parser.decode_segment(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL,start,target_tick.saturating_add(1),&classes, |_| {});
        }
        self.prepared()?.session(parser)?.parse_to_tick(target_tick)
    }

    /// Parse the entire demo, calling a callback once per completed tick.
    pub fn run_to_end<F>(&self, on_tick: F) -> Result<()>
    where
        F: FnMut(&Context),
    {
        let parser = self.demo_parser()?;
        if self.bounded { parser.run_to_end(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL,on_tick)?; return Ok(()); }
        self.prepared()?.session(parser)?.run_to_end(on_tick)?;
        Ok(())
    }

    /// Parse with a fallible tick callback; stop immediately if the consumer fails.
    pub fn try_run_to_end<F>(&self, on_tick: F) -> Result<()>
    where F: FnMut(&Context) -> Result<()>,
    {
        let parser = self.demo_parser()?;
        if self.bounded { parser.try_run_to_end(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL,on_tick)?; return Ok(()); }
        self.prepared()?.session(parser)?.try_run_to_end(on_tick)?;
        Ok(())
    }

    /// Parse the entire demo. Create only the selected entity classes.
    pub fn run_to_end_filtered<F>(
        &self,
        class_filter: &std::collections::HashSet<&str>,
        on_tick: F,
    ) -> Result<()>
    where
        F: FnMut(&Context),
    {
        let parser = self.demo_parser()?;
        if self.bounded { parser.run_to_end_filtered(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL,class_filter,on_tick)?; return Ok(()); }
        self.prepared()?
            .session(parser)?
            .run_to_end_filtered(class_filter, on_tick)?;
        Ok(())
    }

    /// Byte offsets relative to the post-header stream and ticks of full packets.
    pub fn full_packet_offsets(&self) -> Result<Vec<(usize, i32)>> {
        Ok(self
            .prepared()?
            .index()
            .full_packets()
            .iter()
            .map(|position| (position.offset() - HEADER_SIZE, position.tick()))
            .collect())
    }

    /// Every distinct post-signon tick in ascending stream order.
    pub fn distinct_ticks(&self) -> Result<Vec<i32>> {
        Ok(self.prepared()?.index().distinct_ticks().to_vec())
    }

    /// Decode one full-packet-bounded segment for parallel consumers.
    pub fn decode_segment<F>(
        &self,
        start: Option<usize>,
        end_tick: i32,
        class_filter: &std::collections::HashSet<&str>,
        on_tick: F,
    ) -> Result<()>
    where
        F: FnMut(&Context),
    {
        let parser = self.demo_parser()?;
        if self.bounded { parser.decode_segment(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL,start.map(|offset|offset+HEADER_SIZE),end_tick,class_filter,on_tick)?; return Ok(()); }
        self.prepared()?.session(parser)?.decode_segment(
            start.map(|offset| offset + HEADER_SIZE),
            end_tick,
            class_filter,
            on_tick,
        )?;
        Ok(())
    }

    /// Read modifier changes from the packet stream, without keyframe snapshots.
    pub(crate) fn decode_stat_ticks<F>(
        &self,
        end_tick: i32,
        classes: &std::collections::HashSet<&str>,
        on_tick: F,
    ) -> Result<()>
    where
        F: FnMut(&Context),
    {
        if self.bounded {
            let mut adapter=CitadelAdapter::default(); adapter.skip_modifier_snapshots();
            self.demo_parser()?.decode_segment(&mut adapter,DEFAULT_TICK_INTERVAL,None,end_tick,classes,on_tick)?;
            return Ok(());
        }
        let mut session = self.prepared()?.session(self.demo_parser()?)?;
        // Relay keyframes can contain server-side modifier state ahead of the
        // recorded entities. Replay deltas from signon to keep both at one time.
        session.adapter_mut().skip_modifier_snapshots();
        session.decode_segment(None, end_tick, classes, on_tick)?;
        Ok(())
    }

    /// Visit merged modifier changes at each recorded tick.
    ///
    /// Relay keyframe tables can have a different capture time from entities.
    /// This reads packet deltas from signon and does not decode entities.
    /// # Errors
    /// Returns an error if the replay or a packet cannot be decoded.
    pub fn visit_modifier_changes(
        &self,
        mut visit: impl FnMut(i32, crate::ModifierChange),
    ) -> Result<()> {
        let mut state = crate::ModifierState::default();
        self.decode_stat_ticks(i32::MAX, &std::collections::HashSet::new(), |ctx| {
            for change in state.update(ctx) {
                visit(ctx.tick(), change);
            }
        })
    }

    /// Parse entities and only selected final event message types in one pass.
    pub fn run_to_end_with_event_types_filtered<F>(
        &self,
        class_filter: &std::collections::HashSet<&str>,
        event_types: &std::collections::HashSet<u32>,
        mut on_tick: F,
    ) -> Result<()>
    where
        F: FnMut(&Context, &[GameEvent]),
    {
        let parser = self.demo_parser()?;
        if self.bounded {
            let mut adapter=CitadelAdapter::default(); adapter.enable_event_types(event_types);
            parser.run_to_end_filtered_with_adapter(&mut adapter,DEFAULT_TICK_INTERVAL,class_filter,|state,adapter| {on_tick(state,adapter.tick_events());adapter.clear_tick_events();})?;
            return Ok(());
        }
        let mut session = self.prepared()?.session(parser)?;
        session.adapter_mut().enable_event_types(event_types);
        session.run_to_end_filtered_with_adapter(class_filter, |state, adapter| {
            on_tick(state, adapter.tick_events());
            adapter.clear_tick_events();
        })?;
        Ok(())
    }

    /// Parse entities and game events in one filtered pass.
    pub fn run_to_end_with_events_filtered<F>(
        &self,
        class_filter: &std::collections::HashSet<&str>,
        mut on_tick: F,
    ) -> Result<()>
    where
        F: FnMut(&Context, &[GameEvent]),
    {
        let parser = self.demo_parser()?;
        if self.bounded {
            let mut adapter=CitadelAdapter::default(); adapter.enable_events();
            parser.run_to_end_filtered_with_adapter(&mut adapter,DEFAULT_TICK_INTERVAL,class_filter,|state,adapter| {on_tick(state,adapter.tick_events());adapter.clear_tick_events();})?;
            return Ok(());
        }
        let mut session = self.prepared()?.session(parser)?;
        session.adapter_mut().enable_events();
        session.run_to_end_filtered_with_adapter(class_filter, |state, adapter| {
            on_tick(state, adapter.tick_events());
            adapter.clear_tick_events();
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_queries_skip_modifier_snapshots_but_keep_other_tables() {
        use boon_proto::proto::{
            CDemoFullPacket, CDemoPacket, CDemoSendTables, CDemoStringTables,
            CsvcMsgCreateStringTable, EDemoCommands, SvcMessages,
            c_demo_string_tables::{ItemsT, TableT},
        };
        use prost::Message;

        let mut bits = Vec::new();
        let mut write_bits = |value: u64, count: u32| {
            bits.extend((0..count).map(|bit| value & (1 << bit) != 0));
        };
        let message_type = SvcMessages::SvcCreateStringTable as u64;
        assert!((16..256).contains(&message_type));
        for name in ["ActiveModifiers", "test_table"] {
            let body = CsvcMsgCreateStringTable {
                name: Some(name.into()),
                ..Default::default()
            }
            .encode_to_vec();
            // Source 2 frames use a UBitVar type followed by a varint length.
            write_bits((message_type & 15) | 16, 6);
            write_bits(message_type >> 4, 4);
            let mut length = Vec::new();
            prost::encoding::encode_varint(body.len() as u64, &mut length);
            for byte in length.into_iter().chain(body) {
                write_bits(u64::from(byte), 8);
            }
        }
        let signon = CDemoPacket {
            data: Some(
                bits.chunks(8)
                    .map(|chunk| {
                        chunk
                            .iter()
                            .enumerate()
                            .fold(0, |byte, (bit, &set)| byte | (u8::from(set) << bit))
                    })
                    .collect(),
            ),
        };
        let tables = CDemoStringTables {
            tables: ["ActiveModifiers", "test_table"]
                .into_iter()
                .map(|name| TableT {
                    table_name: Some(name.into()),
                    items: vec![ItemsT {
                        str: Some("entry".into()),
                        data: Some(vec![1]),
                    }],
                    ..Default::default()
                })
                .collect(),
        };
        let packet = CDemoFullPacket {
            string_table: Some(tables),
            ..Default::default()
        };
        let mut bytes = b"PBDEMS2\0".to_vec();
        bytes.extend([0; 8]);
        for (command, tick, body) in [
            (
                EDemoCommands::DemSendTables as i32,
                0,
                CDemoSendTables {
                    data: Some(vec![0]),
                }
                .encode_to_vec(),
            ),
            (EDemoCommands::DemClassInfo as i32, 0, Vec::new()),
            (
                EDemoCommands::DemSignonPacket as i32,
                0,
                signon.encode_to_vec(),
            ),
            (EDemoCommands::DemSyncTick as i32, 0, Vec::new()),
            (
                super::super::command::dem::FULL_PACKET,
                10,
                packet.encode_to_vec(),
            ),
            (super::super::command::dem::STOP, 11, Vec::new()),
        ] {
            for value in [command as u64, tick, body.len() as u64] {
                prost::encoding::encode_varint(value, &mut bytes);
            }
            bytes.extend(body);
        }
        let parser = Parser::from_bytes(bytes);
        let snapshot = parser.parse_to_tick(10).unwrap();
        assert_eq!(
            snapshot
                .string_tables()
                .find_table("ActiveModifiers")
                .unwrap()
                .entries()
                .len(),
            1
        );
        let mut checked = false;
        parser
            .decode_stat_ticks(11, &Default::default(), |ctx| {
                if ctx.tick() == 10 {
                    assert!(
                        ctx.string_tables()
                            .find_table("ActiveModifiers")
                            .unwrap()
                            .entries()
                            .is_empty()
                    );
                    assert_eq!(
                        ctx.string_tables()
                            .find_table("test_table")
                            .unwrap()
                            .entries()
                            .len(),
                        1
                    );
                    checked = true;
                }
            })
            .unwrap();
        assert!(checked);
    }
}

impl Parser {
    /// Fallible bounded-consumer callback; cancellation and output failures stop
    /// decoding immediately rather than retaining a failed producer to EOF.
    pub fn try_run_to_end_filtered<F>(&self,classes:&std::collections::HashSet<&str>,visit:F)->Result<()> where F:FnMut(&Context)->Result<()> {
        let parser=self.demo_parser()?;
        if self.bounded {parser.try_run_to_end_filtered(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL,classes,visit)?;}
        else {self.prepared()?.session(parser)?.try_run_to_end_filtered(classes,visit)?;}
        Ok(())
    }
    pub fn try_run_to_end_with_event_types_filtered<F>(&self,classes:&std::collections::HashSet<&str>,events:&std::collections::HashSet<u32>,mut visit:F)->Result<()> where F:FnMut(&Context,&[GameEvent])->Result<()> {
        let parser=self.demo_parser()?;
        let callback=|state:&Context,adapter:&mut CitadelAdapter| {let result=visit(state,adapter.tick_events());adapter.clear_tick_events();result};
        if self.bounded {let mut adapter=CitadelAdapter::default();adapter.enable_event_types(events);parser.try_run_to_end_filtered_with_adapter(&mut adapter,DEFAULT_TICK_INTERVAL,classes,callback)?;}
        else {let mut session=self.prepared()?.session(parser)?;session.adapter_mut().enable_event_types(events);session.try_run_to_end_filtered_with_adapter(classes,callback)?;}
        Ok(())
    }
    pub fn try_decode_segment<F>(&self,start:Option<usize>,end_tick:i32,classes:&std::collections::HashSet<&str>,visit:F)->Result<()> where F:FnMut(&Context)->Result<()> {
        let parser=self.demo_parser()?;
        if self.bounded {parser.try_decode_segment(&mut CitadelAdapter::default(),DEFAULT_TICK_INTERVAL,start.map(|value|value+HEADER_SIZE),end_tick,classes,visit)?;}
        else {self.prepared()?.session(parser)?.try_decode_segment(start.map(|value|value+HEADER_SIZE),end_tick,classes,visit)?;}
        Ok(())
    }
}

impl Parser {
    /// Sample header ticks without constructing the complete command index.
    /// STOP callbacks are intentionally excluded, matching distinct_ticks().
    pub fn bounded_sample_ticks(&self,every:i32,maximum:usize)->Result<Vec<i32>> {
        let parser=self.demo_parser()?;
        let mut ticks=Vec::new();let mut past_sync=false;let mut last=None;
        for frame in parser.demo().commands() {
            let frame=frame.map_err(Error::from)?;let header=frame.header();
            if header.cmd==super::dem::STOP {break;}
            if header.cmd==boon_proto::proto::EDemoCommands::DemSyncTick as i32 {past_sync=true;continue;}
            if past_sync && header.tick>=0 && last.is_none_or(|tick|header.tick-tick>=every) {
                if ticks.len()>=maximum {return Err(Error::Parse {context:"Selected-tick count exceeds the full-reader bound".into()});}
                ticks.push(header.tick);last=Some(header.tick);
            }
        }
        Ok(ticks)
    }
}

impl Parser {
    pub fn try_visit_modifier_ticks(&self,mut visit:impl FnMut(i32,Vec<crate::ModifierChange>)->Result<()>)->Result<()> {
        let mut state=crate::ModifierState::default();
        let mut adapter=CitadelAdapter::default();adapter.skip_modifier_snapshots();
        let classes=std::collections::HashSet::new();
        let callback=|ctx:&Context| {visit(ctx.tick(),state.update(ctx))};
        let parser=self.demo_parser()?;
        if self.bounded {parser.try_decode_segment(&mut adapter,DEFAULT_TICK_INTERVAL,None,i32::MAX,&classes,callback)?;}
        else {let mut session=self.prepared()?.session(parser)?;session.adapter_mut().skip_modifier_snapshots();session.try_decode_segment(None,i32::MAX,&classes,callback)?;}
        Ok(())
    }
}
