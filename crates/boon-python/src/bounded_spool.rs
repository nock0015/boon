//! Bounded native data ingress. Each file is an independently owned typed IPC batch.
//! Legacy collectors and source clock arithmetic remain the oracle; no complete
//! dataset or snapshot output is installed in Demo.cached_datasets.
use crate::*;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write, Read, BufRead};
use sha2::{Digest,Sha256};
use std::path::{Path, PathBuf};

struct HashWriter<W> {inner:W,digest:Sha256,bytes:u64}
impl<W:Write> HashWriter<W> {fn new(inner:W)->Self {Self {inner,digest:Sha256::new(),bytes:0}}}
impl<W:Write> Write for HashWriter<W> {
    fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {let count=self.inner.write(bytes)?;self.digest.update(&bytes[..count]);self.bytes+=count as u64;Ok(count)}
    fn flush(&mut self)->std::io::Result<()> {self.inner.flush()}
}
fn digest_file(path:&Path)->PyResult<String> {
    let mut file=File::open(path)?;let mut digest=Sha256::new();let mut buffer=[0u8;65536];
    loop {let count=file.read(&mut buffer)?;if count==0 {break;}digest.update(&buffer[..count]);}
    Ok(format!("{:x}",digest.finalize()))
}
fn verified_bytes(path:&Path,expected:&str,bytes:u64)->PyResult<Vec<u8>> {
    if expected.len()!=64 || std::fs::metadata(path)?.len()!=bytes {return Err(InvalidDemoError::new_err("native spool byte count/hash identity mismatch"));}
    let mut data=Vec::new();File::open(path)?.take(bytes.saturating_add(1)).read_to_end(&mut data)?;
    if data.len() as u64!=bytes || format!("{:x}",Sha256::digest(&data))!=expected {return Err(InvalidDemoError::new_err("native spool SHA256 mismatch"));}
    Ok(data)
}

pub(super) struct Spool {
    root: PathBuf,
    manifest: HashWriter<BufWriter<File>>,
    auxiliary_hashes:HashMap<String,String>,
    counts: HashMap<String, u64>,
    ordinal: u64,
    last_signal_check: std::time::Instant,
    peak_batch_bytes:usize,
    peak_batch_rows:usize,
    peak_collector_rows:usize,
    peak_collector_bytes:usize,
    owner_status:Option<serde_json::Value>,
    progress:Option<Py<PyAny>>,
    written_bytes:u64,
    nonfinite_lifecycle:bool,
    decode_cohorts:Vec<Vec<String>>,
    shared_model_columns:bool,
    pub(super) event_profile:Vec<serde_json::Value>,
    pub(super) optional_events:bool,
    pub(super) optional_coverage:Option<serde_json::Value>,
}
impl Spool {
    pub(super) fn new(root: &Path,progress:Option<Py<PyAny>>) -> PyResult<Self> {
        std::fs::create_dir(root)?;
        Ok(Self { root: root.to_owned(), manifest: HashWriter::new(BufWriter::new(OpenOptions::new().write(true).create_new(true).open(root.join("batches.jsonl"))?)),auxiliary_hashes:HashMap::new(), counts: HashMap::new(), ordinal: 0, last_signal_check:std::time::Instant::now(),peak_batch_bytes:0,peak_batch_rows:0,peak_collector_rows:0,peak_collector_bytes:0,owner_status:None,progress,written_bytes:0,nonfinite_lifecycle:false,decode_cohorts:Vec::new(),shared_model_columns:false,optional_events:false,optional_coverage:None,event_profile:Vec::new() })
    }
    pub(super) fn root(&self)->&Path {&self.root}
    pub(super) fn check_cancel(&mut self)->PyResult<()> {
        if self.last_signal_check.elapsed().as_millis()>=100 {
            Python::attach(|py|py.check_signals())?;
            self.report_pressure(0)?;
            self.last_signal_check=std::time::Instant::now();
        }
        Ok(())
    }
    fn report_pressure(&self,future_bytes:u64)->PyResult<()> {
        if let Some(progress)=&self.progress {
            let auxiliary=["barrier-events.bin","barrier-current.sqlite3","minimap-history.sqlite3","objective-history.sqlite3","batches.jsonl","snapshot-ticks.json"].iter().filter_map(|name|std::fs::metadata(self.root.join(name)).ok()).map(|info|info.len()).sum::<u64>();
            Python::attach(|py|progress.call1(py,(self.written_bytes+auxiliary+future_bytes,self.ordinal)).map(|_|()))?;
        }
        Ok(())
    }
    pub(super) fn write(&mut self, table: &str, frame: DataFrame) -> PyResult<()> {
        self.check_cancel()?;
        self.report_pressure(frame.estimated_size() as u64+65536)?;
        if table=="player_ticks" {
            for field in ["hero_id","last_spawn_time","death_time","deaths","lifestate","is_alive"] {
                if let Ok(column)=frame.column(field) {
                    let bad=match column.dtype() {
                        DataType::Float32=>column.f32().is_ok_and(|values|values.into_iter().flatten().any(|value|!value.is_finite())),
                        DataType::Float64=>column.f64().is_ok_and(|values|values.into_iter().flatten().any(|value|!value.is_finite())),
                        _=>false,
                    };
                    self.nonfinite_lifecycle|=bad;
                }
            }
        }
        // The producer has at most its threshold plus one source tick's rows.
        // Split wide batches by both row and represented-byte budgets. A single
        // valid oversized row is explicit and never truncated.
        let height = frame.height();
        self.peak_collector_rows=self.peak_collector_rows.max(height);
        self.peak_collector_bytes=self.peak_collector_bytes.max(frame.estimated_size());
        if height == 0 { return Ok(()); }
        let mut start=0;
        while start<height {
            let mut count=1024.min(height-start);
            let mut batch=frame.slice(start as i64,count);
            while count>1 && batch.estimated_size()>4*1024*1024 {
                count=count.div_ceil(2);batch=frame.slice(start as i64,count);
            }
            self.peak_batch_bytes=self.peak_batch_bytes.max(batch.estimated_size());
            self.peak_batch_rows=self.peak_batch_rows.max(batch.height());
            let name = format!("{:016}.arrow", self.ordinal);
            let file = OpenOptions::new().write(true).create_new(true).open(self.root.join(&name))?;
            let mut writer=HashWriter::new(BufWriter::new(file));
            IpcWriter::new(&mut writer).finish(&mut batch).map_err(|e| InvalidDemoError::new_err(e.to_string()))?;
            writer.flush()?;
            let bytes=writer.bytes;let hash=format!("{:x}",writer.digest.clone().finalize());
            self.written_bytes+=bytes;
            let row = serde_json::json!({"table":table,"file":name,"rows":batch.height(),"ordinal":self.ordinal,"bytes":bytes,"sha256":hash,"representedBytes":batch.estimated_size(),"oversizedRow":batch.height()==1 && batch.estimated_size()>4*1024*1024});
            serde_json::to_writer(&mut self.manifest,&row).map_err(|e| InvalidDemoError::new_err(e.to_string()))?;
            self.manifest.write_all(b"\n")?;
            *self.counts.entry(table.to_owned()).or_default() += batch.height() as u64;
            self.ordinal += 1;
            start+=count;
        }
        Ok(())
    }
    fn write_auxiliary(&mut self,name:&str,value:&impl serde::Serialize)->PyResult<()> {
        let mut writer=HashWriter::new(BufWriter::new(File::create(self.root.join(name))?));
        serde_json::to_writer(&mut writer,value).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
        writer.flush()?;self.auxiliary_hashes.insert(name.to_owned(),format!("{:x}",writer.digest.finalize()));Ok(())
    }
    pub(super) fn kill_identities(&mut self,identities:&HashMap<i32,i64>)->PyResult<()> {self.write_auxiliary("kill-identities.json",identities)}
    fn finish(mut self) -> PyResult<String> {
        self.manifest.flush()?;
        let mut receipt=serde_json::json!({"schema":"boon-bounded-spool-v1","eventProfile":self.event_profile,"rows":self.counts,"batches":self.ordinal,"batchRows":1024,"batchBytes":4*1024*1024,"sourceTickAllowance":true,"kills":"final-entity-hero-map","input":"read-only-mmap","preparedIndex":"disabled-cold-sequential-sessions","peakNativeBatchBytes":self.peak_batch_bytes,"peakNativeBatchRows":self.peak_batch_rows,"peakTableCollectorRows":self.peak_collector_rows,"peakTableCollectorBytes":self.peak_collector_bytes,"ownerStatus":self.owner_status,"nonfiniteLifecycle":self.nonfinite_lifecycle,"decodeCohorts":self.decode_cohorts,"integrity":"sha256-batches-manifest-aux-v1","manifestSha256":format!("{:x}",self.manifest.digest.clone().finalize()),"auxiliarySha256":self.auxiliary_hashes});
        if self.shared_model_columns {
            receipt["sharedSourcePass"]=serde_json::json!({"modelColumns":true,"recordedEvents":true,"decodePasses":1,"passScope":"completed source ticks; header and schema initialization excluded","collectors":"retained loader collectors","dispatch":"model observer, retained entity/event collectors, bounded flush"});
        }
        if self.optional_events {
            receipt["optionalEventsContract"]=serde_json::json!(crate::optional_events::CONTRACT);
            receipt["optionalEventCoverage"]=self.optional_coverage.ok_or_else(||InvalidDemoError::new_err("Missing optional event collection coverage"))?;
        }
        let encoded=serde_json::to_string(&receipt).map_err(|e| InvalidDemoError::new_err(e.to_string()))?;
        let mut file=OpenOptions::new().write(true).create_new(true).open(self.root.join("complete.json"))?;
        file.write_all(encoded.as_bytes())?;
        Ok(encoded)
    }
}

fn combined_event_datasets(names:&[String])->PyResult<Vec<Dataset>> {
    let allowed=["damage","abilities","resource_events","soul_bag_pickups"];
    if !names.iter().any(|name|matches!(name.as_str(),"damage"|"resource_events"|"soul_bag_pickups"))||names.iter().any(|name|!allowed.contains(&name.as_str())) {
        return Err(pyo3::exceptions::PyValueError::new_err("combined model pass requires damage or resource events, with optional abilities; other cohorts are unsupported"));
    }
    names.iter().map(|name|name.parse()).collect()
}

struct SharedModelSource<'a> {demo:&'a mut Demo,datasets:&'a [Dataset],spool:&'a mut Spool}
impl crate::direct_model_columns::ModelSourcePass for SharedModelSource<'_> {
    fn initialize(&mut self)->boon_parser::Result<boon_parser::Context> {self.demo.parser.parse_init()}
    fn scan(&mut self,classes:&HashSet<&str>,events:&HashSet<u32>,visit:&mut crate::direct_model_columns::ModelSourceCallback<'_>)->PyResult<f64> {
        Python::attach(|py|self.demo.collect_datasets_with_observer(py,self.datasets,Some(&mut *self.spool),Some(classes),Some(events),Some(visit)))
    }
    fn collect_without_model(&mut self)->PyResult<()> {
        Python::attach(|py|self.demo.collect_datasets(py,self.datasets,Some(&mut *self.spool)))
    }
}

#[pymethods]
impl Demo {
    /// Write source-shaped typed batches directly from native bounded collectors.
    /// The destination must not exist. A completion receipt is written last.
    /// Snapshot cadence is sampled from every completed source tick, exactly as
    /// snapshots(every=...) does; tables with zero rows do not create frames.
    #[pyo3(signature = (path, *, every, progress=None, optional_events=false))]
    pub(crate) fn bounded_replay_spool(&mut self, py: Python<'_>, path: &str, every:i32,progress:Option<Py<PyAny>>,optional_events:bool) -> PyResult<String> {
        if every <= 0 { return Err(pyo3::exceptions::PyValueError::new_err("every must be positive")); }
        let mut spool=Spool::new(Path::new(path),progress)?;
        spool.optional_events=optional_events;
        py.detach(|| self.spool_snapshots(&mut spool,every))?;
        let mut names=vec![Dataset::Objectives,Dataset::Damage,Dataset::Kills,Dataset::Abilities,Dataset::AbilityUpgrades,Dataset::ResourceEvents,Dataset::SoulBagPickups,Dataset::ActiveModifiers,Dataset::AbilityTicks,Dataset::SinnersSacrifice,Dataset::Neutrals,Dataset::MidBoss];
        if optional_events {names.extend([Dataset::ItemPurchases,Dataset::Urn]);}
        // Exact damage/resource endpoints and intrinsic modifier deletions
        // already require every entity class. Collect ability/NPC changes in
        // that same source pass instead of decoding those classes again.
        // The aggregate row/byte guard and source-tick allowance are unchanged.
        spool.decode_cohorts.push(names.iter().map(|dataset|dataset.as_str().to_owned()).collect());
        self.collect_datasets(py,&names,Some(&mut spool))?;
        let ticks=spool.root.join("snapshot-ticks.json");
        let ticks:Vec<i32>=serde_json::from_reader(File::open(ticks)?).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
        py.detach(|| self.spool_minimap(&mut spool,&ticks))?;
        spool.owner_status=Some(self.bounded_owner_status());
        spool.finish()
    }

    #[pyo3(signature = (path, *, ticks, progress=None))]
    pub(crate) fn bounded_target_spool(&self, py: Python<'_>, path:&str, ticks:Vec<i32>,progress:Option<Py<PyAny>>) -> PyResult<String> {
        let mut spool=Spool::new(Path::new(path),progress)?;
        py.detach(||self.spool_targets(&mut spool,ticks))?;
        spool.owner_status=Some(self.bounded_owner_status());
        spool.finish()
    }
    /// Independently qualify narrow bounded event cohorts using the same
    /// collectors as bounded_replay_spool and the legacy dataset APIs.
    #[pyo3(signature = (path, *, datasets, progress=None, optional_events=false))]
    pub(crate) fn bounded_dataset_spool(&mut self,py:Python<'_>,path:&str,datasets:Vec<String>,progress:Option<Py<PyAny>>,optional_events:bool)->PyResult<String> {
        let allowed=["objectives","damage","kills","abilities","ability_upgrades","resource_events","soul_bag_pickups","active_modifiers","ability_ticks","sinners_sacrifice","neutrals","mid_boss","rift","item_purchases"];
        if datasets.iter().any(|name|!allowed.contains(&name.as_str())) {return Err(pyo3::exceptions::PyValueError::new_err("unsupported bounded event cohort"));}
        let datasets:Vec<Dataset>=datasets.iter().map(|name|name.parse()).collect::<PyResult<_>>()?;
        let mut spool=Spool::new(Path::new(path),progress)?;
        spool.optional_events=optional_events;
        spool.decode_cohorts.push(datasets.iter().map(|dataset|dataset.as_str().to_owned()).collect());
        self.collect_datasets(py,&datasets,Some(&mut spool))?;
        spool.owner_status=Some(self.bounded_owner_status());
        spool.finish()
    }
    /// Collect full model columns and bounded recorded events in one source pass.
    /// Event rows use the same collectors as bounded_dataset_spool. The event
    /// destination must not exist. Its completion receipt is written last.
    #[pyo3(signature = (path, *, ticks, entity_classes=None, datasets=None, progress=None, optional_events=false, method_profile=false))]
    pub(crate) fn direct_model_columns_and_events(
        &mut self,py:Python<'_>,path:&str,ticks:Vec<i32>,entity_classes:Option<HashMap<i64,String>>,
        datasets:Option<Vec<String>>,progress:Option<Py<PyAny>>,optional_events:bool,method_profile:bool,
    )->PyResult<(Py<PyDict>,String)> {
        if method_profile&&!crate::direct_method_profile::AVAILABLE {
            return Err(pyo3::exceptions::PyValueError::new_err("method profiling requires an artifact built with direct-method-profile"));
        }
        let names=datasets.unwrap_or_else(||vec!["damage".into(),"abilities".into(),"resource_events".into()]);
        let datasets=combined_event_datasets(&names)?;
        let mut spool=Spool::new(Path::new(path),progress)?;
        spool.optional_events=optional_events;
        spool.shared_model_columns=true;
        spool.decode_cohorts.push(datasets.iter().map(|dataset|dataset.as_str().to_owned()).collect());
        let source=SharedModelSource {demo:self,datasets:&datasets,spool:&mut spool};
        let frames=crate::direct_model_columns::collect_model_columns(py,ticks,entity_classes,method_profile,source)?;
        spool.owner_status=Some(self.bounded_owner_status());
        Ok((frames,spool.finish()?))
    }
    pub(crate) fn bounded_memory_status(&self)->String {self.bounded_owner_status().to_string()}
    pub(crate) fn release_import_outputs(&mut self) {
        self.cached_summary=None;
        self.cached_datasets=DatasetCache::default();
        self.cached_barriers=std::sync::OnceLock::new();
    }


}

impl Demo {
    fn bounded_owner_status(&self)->serde_json::Value {
        serde_json::json!({"boundedSessions":self.parser.has_bounded_sessions(),"preparedIndexRetained":self.parser.has_prepared_index(),"cachedOutputTables":Dataset::ALL.iter().filter(|&&dataset|self.cached_datasets[dataset].is_some()).count(),"cachedOutputRows":Dataset::ALL.iter().filter_map(|&dataset|self.cached_datasets[dataset].as_ref().map(DataFrame::height)).sum::<usize>(),"barrierTimelineHeap":self.cached_barriers.get().is_some(),"pauseTicksHeap":self.paused_ticks.as_ref().map_or(0,Vec::len),"pauseTicksDisk":self.paused_ticks_disk.as_ref().map_or(0,|disk|disk.lock().expect("source clock spool lock poisoned").1)})
    }
    fn spool_snapshots(&self, sink:&mut Spool, every:i32)->PyResult<()> {
        let wants=SnapWants {player_ticks:true,world_ticks:true,troopers:true};
        let barriers=spool_barriers(&self.parser,sink)?;
        let init=self.parser.parse_init().map_err(to_py_err)?;
        let keys=SnapKeys {ammo:AmmoKeys::resolve(&init),pt:PtKeys::resolve(&init),wk:WkKeys::resolve(&init),tk:TkKeys::resolve(&init)};
        drop(init);
        let mut classes=vec!["CCitadelPlayerPawn","CCitadelPlayerController","CCitadelGameRulesProxy","CNPC_Trooper","CNPC_TrooperBoss"];
        classes.extend(keys.ammo.classes());
        let filter:HashSet<&str>=classes.into_iter().collect();
        let mut columns=SegSnap::default();
        let selected:HashSet<i32>=self.parser.bounded_sample_ticks(every,60000).map_err(to_py_err)?.into_iter().collect();
        let mut ticks=Vec::new();
        let mut error=None;
        self.parser.try_decode_segment(None,i32::MAX,&filter,|ctx| {
            if error.is_some() { return Err(aborted()); }
            if let Err(e)=sink.check_cancel() {error=Some(e);return Err(aborted());}
            let tick=ctx.tick();
            if selected.contains(&tick) {
                let before=columns.pt.tick.len()+columns.wt.tick.len()+columns.tr.tick.len();
                columns.collect_tick(ctx,&keys,wants,Some(&barriers));
                if columns.pt.tick.len()+columns.wt.tick.len()+columns.tr.tick.len()>before { ticks.push(tick); }
                if ticks.len()>60000 { error=Some(InvalidDemoError::new_err("Selected-tick count exceeds the full-reader bound")); return Err(aborted()); }
                if columns.pt.tick.len()+columns.wt.tick.len()+columns.tr.tick.len()>=1024 {
                    error=flush_snapshots(sink,&mut columns,wants).err();
                }
            }
            if error.is_some() {Err(aborted())} else {Ok(())}
        }).map_err(|e|error.take().unwrap_or_else(||to_py_err(e)))?;
        if let Some(error)=error { return Err(error); }
        flush_snapshots(sink,&mut columns,wants)?;
        sink.write_auxiliary("snapshot-ticks.json",&ticks)?;
        Ok(())
    }
}
fn flush_snapshots(sink:&mut Spool,columns:&mut SegSnap,wants:SnapWants)->PyResult<()> {
    let (pt,wt,tr)=std::mem::take(columns).into_frames(wants)?;
    for (name,frame) in [("player_ticks",pt),("world_ticks",wt),("troopers",tr)] { if let Some(frame)=frame { sink.write(name,frame)?; } }
    Ok(())
}


// Barrier trackers deliberately use packet deltas without keyframe modifier
// snapshots. Preserve that independent timeline on disk, then merge its cursor
// with sampled pawn ticks. SQLite owns the generation-indexed current values;
// neither history nor inactive generations grow the native heap.
pub(super) struct DiskBarrier {
    events: std::io::BufReader<File>,
    current: rusqlite::Connection,
    pending: Option<(i32,u32,u32)>,
    last_tick: i32,
}
impl DiskBarrier {
    fn next(&mut self)->Result<Option<(i32,u32,u32)>,Box<dyn std::error::Error>> {
        use std::io::Read;
        let mut row=[0u8;12];
        if self.events.read(&mut row[..1])?==0 {return Ok(None);}
        self.events.read_exact(&mut row[1..])?;
        Ok(Some((i32::from_le_bytes(row[..4].try_into()?),u32::from_le_bytes(row[4..8].try_into()?),u32::from_le_bytes(row[8..].try_into()?))))
    }
    pub(super) fn remaining(&mut self,tick:i32,parent:u32)->Result<f32,Box<dyn std::error::Error>> {
        use rusqlite::OptionalExtension;
        if tick<self.last_tick {return Err("barrier spool cursor moved backwards".into());}
        self.last_tick=tick;
        while self.pending.is_some_and(|row|row.0<=tick) {
            let (_,owner,bits)=self.pending.take().unwrap();
            self.current.prepare_cached("INSERT OR REPLACE INTO current VALUES (?,?)")?.execute(rusqlite::params![owner,bits])?;
            self.pending=self.next()?;
        }
        let bits:Option<u32>=self.current.prepare_cached("SELECT bits FROM current WHERE parent=?")?.query_row([parent],|row|row.get(0)).optional()?;
        Ok(f32::from_bits(bits.unwrap_or_default()))
    }
}
fn spool_barriers(parser:&boon_parser::Parser,sink:&mut Spool)->PyResult<BarrierTimeline> {
    let path=sink.root.clone();
    let mut events=BufWriter::new(File::create(path.join("barrier-events.bin"))?);
    let mut state=BarrierTimeline::default();
    let mut serials=HashMap::new();
    let mut error:Option<PyErr>=None;
    parser.try_visit_modifier_ticks(|tick,changes| {
        if let Err(e)=sink.check_cancel() {error=Some(e);return Err(aborted());}
        for change in changes {state.apply(tick,change,&mut serials);}
        for (parent,values) in state.values.drain() {
            for (tick,remaining) in values {
                for bytes in [tick.to_le_bytes(),parent.to_le_bytes(),remaining.to_bits().to_le_bytes()] {
                    if let Err(e)=events.write_all(&bytes) {error=Some(e.into());return Err(aborted());}
                }
            }
        }
        Ok(())
    }).map_err(|e|error.take().unwrap_or_else(||to_py_err(e)))?;
    events.flush()?;
    let current=rusqlite::Connection::open(path.join("barrier-current.sqlite3")).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
    current.execute_batch("PRAGMA cache_size=-1024; PRAGMA journal_mode=OFF; CREATE TABLE current(parent INTEGER PRIMARY KEY,bits INTEGER NOT NULL); BEGIN").map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
    let mut cursor=DiskBarrier {events:std::io::BufReader::new(File::open(path.join("barrier-events.bin"))?),current,pending:None,last_tick:i32::MIN};
    cursor.pending=cursor.next().map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
    Ok(BarrierTimeline {values:HashMap::new(),disk:Some(std::sync::Mutex::new(cursor))})
}

pub(super) fn aborted()->boon_parser::Error {boon_parser::Error::Parse {context:"bounded spool consumer aborted".into()}}

// Direct typed handoff to the backend semantic owner. No Python row graph or
// JSON representation is constructed between IPC and the versioned boundary.
#[pyclass]
pub(super) struct NativeSpoolReader {
    root:PathBuf,
    kill_ids:HashMap<i32,i64>,
    cache:std::sync::Mutex<BatchCache>,
    index:Option<std::sync::Mutex<rusqlite::Connection>>,
    closed:bool,
}
#[derive(Default)]
struct BatchCache {frames:HashMap<String,(u64,DataFrame)>,bytes:usize}

fn length(out:&mut Vec<u8>,n:usize)->PyResult<()> {
    let n=u32::try_from(n).map_err(|_|InvalidDemoError::new_err("typed value count exceeds u32"))?;
    out.extend(n.to_be_bytes());Ok(())
}
fn text_value(out:&mut Vec<u8>,value:&str)->PyResult<()> {out.push(5);length(out,value.len())?;out.extend(value.as_bytes());Ok(())}
fn integer_value(out:&mut Vec<u8>,value:impl std::fmt::Display)->PyResult<()> {let text=value.to_string();out.push(3);length(out,text.len())?;out.extend(text.as_bytes());Ok(())}
fn float_value(out:&mut Vec<u8>,value:f64) {if !value.is_finite() {out.push(0);} else {out.push(4);out.extend(value.to_bits().to_be_bytes());}}
fn scalar_i64(value:AnyValue<'_>)->Option<i64> {match value {AnyValue::Int32(n)=>Some(n.into()),AnyValue::Int64(n)=>Some(n),AnyValue::UInt32(n)=>Some(n.into()),AnyValue::UInt64(n)=>i64::try_from(n).ok(),_=>None}}
fn cell_value(out:&mut Vec<u8>,value:AnyValue<'_>)->PyResult<()> {
    match value {
        AnyValue::Null=>out.push(0),
        AnyValue::Boolean(value)=>out.push(if value {2} else {1}),
        AnyValue::String(value)=>text_value(out,value)?,AnyValue::StringOwned(value)=>text_value(out,value.as_str())?,
        AnyValue::Int8(value)=>integer_value(out,value)?,AnyValue::Int16(value)=>integer_value(out,value)?,AnyValue::Int32(value)=>integer_value(out,value)?,AnyValue::Int64(value)=>integer_value(out,value)?,AnyValue::Int128(value)=>integer_value(out,value)?,
        AnyValue::UInt8(value)=>integer_value(out,value)?,AnyValue::UInt16(value)=>integer_value(out,value)?,AnyValue::UInt32(value)=>integer_value(out,value)?,AnyValue::UInt64(value)=>integer_value(out,value)?,AnyValue::UInt128(value)=>integer_value(out,value)?,
        AnyValue::Float32(value)=>float_value(out,f64::from(value)),AnyValue::Float64(value)=>float_value(out,value),
        AnyValue::List(values)=>{out.push(6);length(out,values.len())?;for index in 0..values.len(){cell_value(out,values.get(index).map_err(|e|InvalidDemoError::new_err(e.to_string()))?)?;}},
        other=>return Err(InvalidDemoError::new_err(format!("unsupported typed evidence cell {:?}; original IPC batch is retained",other.dtype()))),
    }
    Ok(())
}
fn source_clock(demo:&mut Demo,tick:i32)->Option<f64> {demo.tick_to_match_seconds(tick).ok().flatten().filter(|value|value.is_finite())}

#[pymethods]
impl Demo {
    #[pyo3(signature=(path, *, receipt=None))]
    pub(crate) fn native_spool_reader(&self,path:&str,receipt:Option<String>)->PyResult<NativeSpoolReader> {
        let root=PathBuf::from(path);
        let receipt=if let Some(receipt)=receipt {receipt} else {let mut text=String::new();File::open(root.join("complete.json"))?.take(65537).read_to_string(&mut text)?;text};
        if receipt.len()>65536 {return Err(InvalidDemoError::new_err("native spool receipt exceeds bound"));}
        let receipt:serde_json::Value=serde_json::from_str(&receipt).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
        if receipt["schema"].as_str()!=Some("boon-bounded-spool-v1") || receipt["integrity"].as_str()!=Some("sha256-batches-manifest-aux-v1") {return Err(InvalidDemoError::new_err("unsupported native spool integrity receipt"));}
        let expected=receipt["manifestSha256"].as_str().ok_or_else(||InvalidDemoError::new_err("native spool manifest integrity receipt missing"))?;
        if digest_file(&root.join("batches.jsonl"))?!=expected {return Err(InvalidDemoError::new_err("native spool manifest SHA256 mismatch"));}
        let file=root.join("kill-identities.json");
        let kill_ids=if file.exists() {
            let expected=receipt["auxiliarySha256"]["kill-identities.json"].as_str().ok_or_else(||InvalidDemoError::new_err("native kill identity integrity receipt missing"))?;
            let bytes=std::fs::metadata(&file)?.len();
            if bytes>16*1024*1024 {return Err(InvalidDemoError::new_err("native kill identity metadata exceeds bound"));}
            let data=verified_bytes(&file,expected,bytes)?;
            serde_json::from_slice(&data).map_err(|e|InvalidDemoError::new_err(e.to_string()))?
        } else {HashMap::new()};
        let index=root.parent().unwrap_or(&root).join("order.sqlite3");
        let index=if index.exists() {let connection=rusqlite::Connection::open_with_flags(index,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;connection.execute_batch("PRAGMA cache_size=-1024; PRAGMA query_only=ON").map_err(|e|InvalidDemoError::new_err(e.to_string()))?;Some(std::sync::Mutex::new(connection))} else {None};
        Ok(NativeSpoolReader {root,kill_ids,cache:std::sync::Mutex::new(BatchCache::default()),index,closed:false})
    }
}
impl NativeSpoolReader {
    fn integrity(&self,ordinal:u64)->PyResult<(String,u64)> {
        if let Some(index)=&self.index {
            return index.lock().map_err(|_|InvalidDemoError::new_err("native spool index lock poisoned"))?.query_row("SELECT sha256,bytes FROM batch_integrity WHERE ordinal=?",[ordinal],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|e|InvalidDemoError::new_err(e.to_string()));
        }
        let mut reader=std::io::BufReader::new(File::open(self.root.join("batches.jsonl"))?);
        loop {
            let mut line=String::new();let count=Read::by_ref(&mut reader).take(8193).read_line(&mut line)?;
            if count==0 {break;}if count>8192 {return Err(InvalidDemoError::new_err("native spool manifest row exceeds bound"));}
            let row:serde_json::Value=serde_json::from_str(&line).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
            if row["ordinal"].as_u64()==Some(ordinal) {return Ok((row["sha256"].as_str().ok_or_else(||InvalidDemoError::new_err("native spool batch hash missing"))?.to_owned(),row["bytes"].as_u64().ok_or_else(||InvalidDemoError::new_err("native spool batch byte count missing"))?));}
        }
        Err(InvalidDemoError::new_err("native spool batch identity missing"))
    }
}
#[pymethods]
impl NativeSpoolReader {
    fn close(&mut self) {
        self.index=None;
        if let Ok(cache)=self.cache.get_mut() {cache.frames.clear();cache.bytes=0;}
        self.closed=true;
    }
    /// selections contain only table/batch/row occurrence locations. Payload
    /// cells, source event clocks and remembered minimap clocks stay native.
    fn frame(&self,py:Python<'_>,mut demo:PyRefMut<'_,Demo>,tick:i32,selections:Vec<(String,Vec<(u64,Vec<usize>)>)>)->PyResult<(Py<pyo3::types::PyBytes>,Py<pyo3::types::PyBytes>)> {
        if self.closed {return Err(InvalidDemoError::new_err("native spool reader is closed"));}
        let mut tables=b"BTS66\0".to_vec();let mut times=b"BTS66\0".to_vec();
        tables.push(7);length(&mut tables,selections.len())?;
        let event_count=selections.iter().filter(|(name,_)|!matches!(name.as_str(),"player_ticks"|"world_ticks"|"troopers"|"enemy_minimap_ticks")).count();
        times.push(7);length(&mut times,event_count)?;
        for (name,parts) in selections {
            text_value(&mut tables,&name)?;tables.push(6);let row_count=parts.iter().map(|(_,rows)|rows.len()).sum();length(&mut tables,row_count)?;
            let is_event=!matches!(name.as_str(),"player_ticks"|"world_ticks"|"troopers"|"enemy_minimap_ticks");
            if is_event {text_value(&mut times,&name)?;times.push(6);length(&mut times,row_count)?;}
            for (ordinal,rows) in parts {
                let batch={
                    let mut cache=self.cache.lock().map_err(|_|InvalidDemoError::new_err("native IPC cursor cache poisoned"))?;
                    if let Some((at,batch))=cache.frames.get(&name).filter(|(at,_)|*at==ordinal) {let _=at;batch.clone()}
                    else {
                        let (hash,bytes)=self.integrity(ordinal)?;
                        let data=verified_bytes(&self.root.join(format!("{ordinal:016}.arrow")),&hash,bytes)?;
                        let batch=IpcReader::new(std::io::Cursor::new(data)).finish().map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
                        let bytes=batch.estimated_size();
                        if let Some((_,old))=cache.frames.remove(&name) {cache.bytes=cache.bytes.saturating_sub(old.estimated_size());}
                        if cache.bytes+bytes>64*1024*1024 || cache.frames.len()>=32 {cache.frames.clear();cache.bytes=0;}
                        if bytes<=64*1024*1024 {cache.frames.insert(name.clone(),(ordinal,batch.clone()));cache.bytes+=bytes;}
                        batch
                    }
                };
                for row in rows {
                    if row>=batch.height() {return Err(InvalidDemoError::new_err("typed row location is outside its native batch"));}
                    let observed=if name=="enemy_minimap_ticks" {batch.column("last_seen_tick").ok().and_then(|column|column.get(row).ok()).and_then(scalar_i64).and_then(|at|i32::try_from(at).ok()).filter(|&at|at<=tick)} else {None};
                    tables.push(7);length(&mut tables,batch.width()+usize::from(observed.is_some()))?;
                    for column in batch.columns() {
                        let field=column.name().as_str();text_value(&mut tables,field)?;
                        let value=column.get(row).map_err(|e|InvalidDemoError::new_err(e.to_string()))?;
                        if name=="kills" && matches!(field,"victim_hero_id"|"attacker_hero_id") {
                            let hero=scalar_i64(value).and_then(|index|i32::try_from(index).ok()).and_then(|index|self.kill_ids.get(&index).copied()).unwrap_or_default();integer_value(&mut tables,hero)?;
                        } else if name=="kills" && field=="assister_hero_ids" {
                            if let AnyValue::List(values)=value {
                                let mapped:Vec<i64>=(0..values.len()).filter_map(|index|values.get(index).ok().and_then(scalar_i64)).filter_map(|index|i32::try_from(index).ok()).filter_map(|index|self.kill_ids.get(&index).copied()).collect();
                                tables.push(6);length(&mut tables,mapped.len())?;for hero in mapped {integer_value(&mut tables,hero)?;}
                            } else {cell_value(&mut tables,value)?;}
                        } else {cell_value(&mut tables,value)?;}
                    }
                    if let Some(observed)=observed {text_value(&mut tables,"last_seen_match_time")?;if let Some(at)=source_clock(&mut demo,observed){float_value(&mut tables,at);}else{tables.push(0);}}
                    if is_event {
                        let at=batch.column("tick").ok().and_then(|column|column.get(row).ok()).and_then(scalar_i64).and_then(|tick|i32::try_from(tick).ok()).and_then(|tick|source_clock(&mut demo,tick));
                        if let Some(at)=at {float_value(&mut times,at);} else {times.push(0);}
                    }
                }
            }
        }
        Ok((pyo3::types::PyBytes::new(py,&tables).unbind(),pyo3::types::PyBytes::new(py,&times).unbind()))
    }
}

#[cfg(test)]
#[path="shared_source_pass_tests.rs"]
mod shared_source_pass_tests;
