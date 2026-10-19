//! Opt-in invocation-local method accounting. Standard-library work and
//! uninstrumented upstream helpers remain in the enclosing method's exclusive
//! time. Timing includes probe overhead and is not baseline throughput.
use crate::*;
#[cfg(feature="direct-method-profile")]
mod enabled {
use super::*;
use std::{cell::{Cell,RefCell},marker::PhantomData,rc::Rc,time::Instant};
include!("direct_method_inventory.rs");
#[derive(Clone,Default)]struct Counter {calls:u64,inclusive:u128,exclusive:i128}
struct Frame {id:usize,token:u64,child:u128}
struct Recorder {started:Option<Instant>,counters:Vec<Counter>,stack:Vec<Frame>,next_token:u64,roots:u128,errors:u64,max_depth:usize}
impl Recorder {fn new(enabled:bool)->Self{Self{started:enabled.then(Instant::now),counters:vec![Counter::default();INVENTORY.len()],stack:Vec::new(),next_token:0,roots:0,errors:0,max_depth:0}}
 fn exit(&mut self,token:u64,elapsed:u128){
  if !self.stack.last().is_some_and(|f|f.token==token){self.errors+=1;return}
  let frame=self.stack.pop().unwrap();let c=&mut self.counters[frame.id];c.calls+=1;c.inclusive+=elapsed;c.exclusive+=elapsed as i128-frame.child as i128;
  if let Some(parent)=self.stack.last_mut(){parent.child+=elapsed}else{self.roots+=elapsed}
 }
}
thread_local!{static ENABLED:Cell<bool>=const{Cell::new(false)};static RECORDER:RefCell<Option<Recorder>>=const{RefCell::new(None)};}
#[cfg(feature="direct-method-profile")]
#[must_use]pub(crate) struct Span {start:Option<Instant>,token:u64,_same_thread:PhantomData<Rc<()>>}
#[cfg(feature="direct-method-profile")]
#[inline]pub(crate) fn span(id:usize)->Span{
 if !ENABLED.with(Cell::get){return Span{start:None,token:0,_same_thread:PhantomData}}
 let start=Instant::now();let token=RECORDER.with(|state|{let mut state=state.borrow_mut();let r=state.as_mut().expect("enabled profiling context");let token=r.next_token;r.next_token+=1;r.stack.push(Frame{id,token,child:0});r.max_depth=r.max_depth.max(r.stack.len());token});
 Span{start:Some(start),token,_same_thread:PhantomData}
}
#[cfg(feature="direct-method-profile")]
impl Drop for Span {fn drop(&mut self){if let Some(start)=self.start{let elapsed=start.elapsed().as_nanos();RECORDER.with(|state|{if let Some(r)=state.borrow_mut().as_mut(){r.exit(self.token,elapsed)}})}}}
/// Production spans have no clock, TLS, allocation, or drop work.
#[cfg(not(feature="direct-method-profile"))]
#[must_use]pub(crate) struct Span;
#[cfg(not(feature="direct-method-profile"))]
#[inline]pub(crate) fn span(_id:usize)->Span{Span}
pub(crate) const AVAILABLE:bool=cfg!(feature="direct-method-profile");
pub(crate) struct Session {previous:Option<Recorder>,previous_enabled:bool,enabled:bool,finished:bool,_same_thread:PhantomData<Rc<()>>}
impl Session {
 pub(crate) fn begin(enabled:bool)->Self{assert!(!enabled||AVAILABLE,"method profiling requires direct-method-profile build feature");let previous=RECORDER.with(|r|r.replace(if enabled{Some(Recorder::new(true))}else{None}));let previous_enabled=ENABLED.with(|e|e.replace(enabled));Self{previous,previous_enabled,enabled,finished:false,_same_thread:PhantomData}}
 fn finish(mut self)->(Recorder,Option<f64>){let r=RECORDER.with(|r|r.replace(self.previous.take())).unwrap_or_else(||Recorder::new(false));ENABLED.with(|e|e.set(self.previous_enabled));self.finished=true;let elapsed=r.started.map(|s|s.elapsed().as_secs_f64());(r,elapsed)}
 pub(crate) fn finish_frames(self)->PolarsResult<Vec<(&'static str,DataFrame)>>{
  if !self.enabled {return Ok(Vec::new())}
  let enabled=self.enabled;let(r,elapsed)=self.finish();let n=INVENTORY.len();
  let timing=|v:f64|enabled.then_some(v);
  let profile=df_from_columns(vec![
   Column::new("method".into(),INVENTORY.iter().map(|x|x.0).collect::<Vec<_>>()),Column::new("calls".into(),r.counters.iter().map(|c|c.calls).collect::<Vec<_>>()),
   Column::new("inclusive_seconds".into(),r.counters.iter().map(|c|timing(c.inclusive as f64/1e9)).collect::<Vec<_>>()),Column::new("exclusive_seconds".into(),r.counters.iter().map(|c|timing(c.exclusive as f64/1e9)).collect::<Vec<_>>()),
   Column::new("inclusive_ns_per_call".into(),r.counters.iter().map(|c|(enabled&&c.calls>0).then(||c.inclusive as f64/c.calls as f64)).collect::<Vec<_>>()),Column::new("exclusive_ns_per_call".into(),r.counters.iter().map(|c|(enabled&&c.calls>0).then(||c.exclusive as f64/c.calls as f64)).collect::<Vec<_>>()),Column::new("enabled".into(),vec![enabled;n])])?;
  let gaps=[
   ("Demo::direct_method_profile_probe / probe","direct_method_profile.rs / direct_model_columns.rs","uninstrumented","Standalone no-op calibration API; intentionally outside replay method inventory and self-profiling"),
   ("upstream.serializer/entity/decoder internals","boon / pbdems2","uninstrumented","Caller boundaries include induced upstream work; no internal method or stock-Boon attribution"),
   ("upstream.standard-library collections/string allocation","std","uninstrumented","Included in caller exclusive; Node/State/Binding/PositionAnchor/Zip clone/equality boundaries are explicit"),
   ("upstream.Polars workers and dataframe kernels","polars","uninstrumented","DataFrame-building wrappers timed; helper-thread internals are not profiled"),
   ("python.PyDict/PyDataFrame/Arrow conversion","direct_model_columns.rs","uninstrumented","Occurs after detached native profiling session"),
   ("profiling machinery and compiler-generated Default/Drop","direct_method_profile.rs / derives","uninstrumented","Avoid recursive self-instrumentation; timer/stack overhead remains in measured ancestors or session residual"),
   ("unpropagated worker-thread work","native helper threads","uninstrumented","Thread-local accounting covers only the invoking parser/projection thread; no cross-thread total claim"),
  ];let inventory:Vec<_>=INVENTORY.iter().copied().chain(gaps).collect();
  let catalog=df_from_columns(vec![Column::new("method".into(),inventory.iter().map(|r|r.0).collect::<Vec<_>>()),Column::new("source_file".into(),inventory.iter().map(|r|r.1).collect::<Vec<_>>()),Column::new("coverage".into(),inventory.iter().map(|r|r.2).collect::<Vec<_>>()),Column::new("notes".into(),inventory.iter().map(|r|r.3).collect::<Vec<_>>())])?;
  let exclusive=r.counters.iter().map(|c|c.exclusive).sum::<i128>()as f64/1e9;let roots=r.roots as f64/1e9;
  let diag=df_from_columns(vec![Column::new("enabled".into(),[enabled]),Column::new("schema".into(),["boon-direct-method-profile-v1"]),Column::new("session_seconds".into(),[elapsed]),Column::new("root_inclusive_seconds".into(),[timing(roots)]),Column::new("exclusive_sum_seconds".into(),[timing(exclusive)]),Column::new("unattributed_seconds".into(),[elapsed.map(|v|v-roots)]),Column::new("root_minus_exclusive_seconds".into(),[timing(roots-exclusive)]),Column::new("stack_errors".into(),[r.errors]),Column::new("open_spans".into(),[r.stack.len()as u64]),Column::new("max_depth".into(),[r.max_depth as u64]),Column::new("profiled_threads".into(),[if enabled{1u64}else{0}])])?;
  Ok(vec![("method_profile",profile),("method_inventory",catalog),("method_profile_diagnostics",diag)])
 }
}
impl Drop for Session {fn drop(&mut self){if !self.finished{RECORDER.with(|r|{r.replace(self.previous.take());});ENABLED.with(|e|e.set(self.previous_enabled));}}}
/// Calibration only: loop, TLS check, clock and accounting overhead. Never
/// subtract this result from feature timings; actual nesting has different costs.
pub(crate) fn probe(iterations:usize)->PolarsResult<DataFrame>{
 let mut enableds=vec![];let mut repeats=vec![];let mut elapsed=vec![];let mut calls=vec![];
 for repeat in 0..3u32{for enabled in [false,true]{let session=Session::begin(enabled);let start=Instant::now();for _ in 0..iterations{let _span=std::hint::black_box(span(0));}let duration=start.elapsed().as_secs_f64();let(r,_)=session.finish();enableds.push(enabled);repeats.push(repeat);elapsed.push(duration);calls.push(r.counters[0].calls);}}
 df_from_columns(vec![Column::new("enabled".into(),enableds),Column::new("repeat".into(),repeats),Column::new("iterations".into(),vec![iterations as u64;6]),Column::new("elapsed_seconds".into(),elapsed.clone()),Column::new("ns_per_iteration".into(),elapsed.iter().map(|s|s*1e9/iterations as f64).collect::<Vec<_>>()),Column::new("recorded_calls".into(),calls),Column::new("scope".into(),vec!["no-op span plus loop; overhead not subtracted from profiles";6])])
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn exact_nested_and_recursive_accounting(){let mut r=Recorder::new(false);r.stack.push(Frame{id:0,token:0,child:0});r.stack.push(Frame{id:0,token:1,child:0});r.exit(1,30);r.exit(0,100);assert_eq!(r.counters[0].calls,2);assert_eq!(r.counters[0].inclusive,130);assert_eq!(r.counters[0].exclusive,100);assert_eq!(r.roots,100);assert_eq!(r.errors,0);}
 #[test]fn signed_over_account_is_not_clamped(){let mut r=Recorder::new(false);r.stack.push(Frame{id:0,token:0,child:100});r.exit(0,30);assert_eq!(r.counters[0].exclusive,-70);}
 #[test]fn wrong_stack_order_is_visible(){let mut r=Recorder::new(false);r.stack.push(Frame{id:0,token:7,child:0});r.exit(8,1);assert_eq!(r.errors,1);assert_eq!(r.stack.len(),1);}
 #[test]fn disabled_mode_has_no_frames(){assert!(Session::begin(false).finish_frames().unwrap().is_empty());}
 #[cfg(feature="direct-method-profile")]
 #[test]fn early_return_and_unwind_close_spans(){let session=Session::begin(true);fn early(){let _s=span(0);return}early();let _=std::panic::catch_unwind(||{let _s=span(1);panic!("test unwind")});let(r,_)=session.finish();assert_eq!(r.counters[0].calls,1);assert_eq!(r.counters[1].calls,1);assert!(r.stack.is_empty());assert_eq!(r.errors,0);}
 #[cfg(feature="direct-method-profile")]
 #[test]fn independent_threads_and_nested_sessions_do_not_mix(){let outer=Session::begin(true);{let _s=span(0);}let thread=std::thread::spawn(||{let s=Session::begin(true);{let _span=span(1);}let(r,_)=s.finish();assert_eq!(r.counters[0].calls,0);assert_eq!(r.counters[1].calls,1);});thread.join().unwrap();{let inner=Session::begin(false);let _s=span(1);drop(inner);}let(r,_)=outer.finish();assert_eq!(r.counters[0].calls,1);assert_eq!(r.counters[1].calls,0);}
}

}
#[cfg(feature="direct-method-profile")]
pub(crate) use enabled::{Session,Span,span,probe,AVAILABLE};
#[cfg(not(feature="direct-method-profile"))]
pub(crate) const AVAILABLE:bool=false;
#[cfg(not(feature="direct-method-profile"))]
#[must_use]pub(crate) struct Span;
#[cfg(not(feature="direct-method-profile"))]
#[inline(always)]pub(crate) fn span(_id:usize)->Span{Span}
/// No TLS, clocks, counters, allocations or destructor in production.
#[cfg(not(feature="direct-method-profile"))]
pub(crate) struct Session;
#[cfg(not(feature="direct-method-profile"))]
impl Session {
 #[inline(always)]pub(crate) fn begin(enabled:bool)->Self{assert!(!enabled,"method profiling requires direct-method-profile build feature");Self}
 #[inline(always)]pub(crate) fn finish_frames(self)->PolarsResult<Vec<(&'static str,DataFrame)>>{Ok(Vec::new())}
}
#[cfg(not(feature="direct-method-profile"))]
pub(crate) fn probe(_iterations:usize)->PolarsResult<DataFrame>{Err(PolarsError::ComputeError("method profiling requires direct-method-profile build feature".into()))}
#[cfg(all(test,not(feature="direct-method-profile")))]mod production_tests {
 use super::*;
 #[test]fn session_and_span_are_zero_sized(){assert_eq!(std::mem::size_of::<Session>(),0);assert_eq!(std::mem::size_of::<Span>(),0);assert!(Session::begin(false).finish_frames().unwrap().is_empty());}
 #[test]#[should_panic(expected="requires direct-method-profile")]fn enabled_request_is_rejected(){Session::begin(true);}
}
