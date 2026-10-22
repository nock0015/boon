//! Additive causal elapsed-play coordinates. Engine-authored clock values stay raw.
//! Every subtraction is checked integer arithmetic before one f64 conversion.
#[derive(Clone,Copy,Debug,Default,PartialEq)]
pub(super) struct CaptureStamp {
 pub capture_tick:Option<i64>,pub timing_segment_id:Option<u64>,pub elapsed_play_seconds:Option<f64>,
}
#[derive(Clone,Copy,Debug)]
pub(super) struct Input {
 pub tick:i32,pub raw_match_seconds:Option<f64>,pub controller_tick_base:Option<i64>,pub total_paused_ticks:Option<i64>,
 pub tick_interval_seconds:f64,pub raw_match_clock_at_last_update:Option<f64>,pub raw_match_clock_update_tick:Option<i64>,
 pub game_paused:Option<bool>,pub server_paused:Option<bool>,
}
#[derive(Clone,Copy,Debug)]
pub(super) struct Evidence {
 pub input:Input,pub elapsed_play_seconds:Option<f64>,pub timing_segment_id:Option<u64>,pub e1_time_available:bool,
}
impl Evidence {pub fn capture(self)->CaptureStamp{CaptureStamp{capture_tick:Some(i64::from(self.input.tick)),timing_segment_id:self.timing_segment_id,elapsed_play_seconds:self.elapsed_play_seconds}}}
#[derive(Clone,Copy,Debug)]struct Anchor{raw:f64,base:i64,paused:i64}
#[derive(Default)]pub(super) struct Clock {
 interval:Option<f64>,last_tick:Option<i32>,last_base:Option<i64>,last_paused:Option<i64>,baseline:Option<Anchor>,
 raw_anchor:Option<Anchor>,segment:u64,last_unpaused:Option<(i64,f64)>,
}
fn delta(base:i64,base0:i64,paused:i64,paused0:i64)->Result<i64,String>{
 let active=base.checked_sub(base0).and_then(|x|paused.checked_sub(paused0).and_then(|p|x.checked_sub(p))).ok_or("elapsed clock integer overflow")?;
 if active<0{return Err("negative elapsed-play active tick delta".into());}Ok(active)
}
impl Clock {
 pub fn observe(&mut self,input:Input)->Result<Evidence,String>{
  if input.tick<0{return Err("negative causal source tick".into());}
  let dt=input.tick_interval_seconds;if !dt.is_finite()||dt<=0.{return Err("invalid elapsed-play tick interval".into());}
  if self.interval.is_some_and(|old|old.to_bits()!=dt.to_bits()){return Err("elapsed-play tick interval changed".into());}self.interval=Some(dt);
  if self.last_tick.is_some_and(|old|input.tick<=old){return Err("non-increasing causal source tick".into());}self.last_tick=Some(input.tick);
  for (value,last,label) in [(input.controller_tick_base,&mut self.last_base,"controller tick base"),(input.total_paused_ticks,&mut self.last_paused,"paused tick count")]{
   if let Some(value)=value{if value<0||last.is_some_and(|old|value<old){return Err(format!("elapsed-play {label} reset or negative"));}*last=Some(value);}}
  if let (Some(raw),Some(base),Some(paused))=(input.raw_match_clock_at_last_update,input.raw_match_clock_update_tick,input.total_paused_ticks){
   if !raw.is_finite()||base<0{return Err("invalid raw clock anchor".into());}
   let next=Anchor{raw,base,paused};
   if let Some(old)=self.raw_anchor {
    if old.raw!=raw||old.base!=base {
     let predicted=old.raw+delta(base,old.base,paused,old.paused)?as f64*dt;
     // Replicated anchors and dt originate in f32. Predict in f64, then
     // round ONCE to f32 for anchor compatibility only (numeric +/-zero equal).
     // The elapsed axis is never rounded or reanchored.
     if !predicted.is_finite()||!(predicted as f32).is_finite(){return Err("nonfinite raw-anchor prediction".into());}
     if predicted as f32!=raw as f32{self.segment=self.segment.checked_add(1).ok_or("timing segment overflow")?;}
     self.raw_anchor=Some(next);
    }
   }else{self.raw_anchor=Some(next);}
  }
  let mut elapsed=None;
  // Absence of a true pause flag is not affirmative unpaused evidence.
  let unpaused=input.game_paused==Some(false)&&input.server_paused==Some(false);
  if unpaused {if let (Some(raw),Some(base),Some(count))=(input.raw_match_seconds,input.controller_tick_base,input.total_paused_ticks){
   if raw.is_finite(){let baseline=*self.baseline.get_or_insert(Anchor{raw,base,paused:count});let active=delta(base,baseline.base,count,baseline.paused)?;
    let value=baseline.raw+active as f64*dt;if !value.is_finite(){return Err("nonfinite elapsed-play clock".into());}
    if self.last_unpaused.is_some_and(|(ticks,old)|active<ticks||value<old){return Err("elapsed-play clock moved backward across qualified unpaused observations".into());}
    self.last_unpaused=Some((active,value));elapsed=Some(value);
   }
  }}
  Ok(Evidence{input,elapsed_play_seconds:elapsed,timing_segment_id:Some(self.segment),e1_time_available:elapsed.is_some()&&unpaused&&input.raw_match_seconds.is_some_and(f64::is_finite)})
 }
}
#[cfg(test)]mod tests {
 use super::*;
 fn input(tick:i32,base:i64,paused:i64,raw:f64)->Input{Input{tick,controller_tick_base:Some(base),total_paused_ticks:Some(paused),raw_match_seconds:Some(raw),tick_interval_seconds:0.015625,raw_match_clock_at_last_update:Some(20.),raw_match_clock_update_tick:Some(100),game_paused:Some(false),server_paused:Some(false)}}
 #[test]fn fixed_reference_delta_preserves_initial_raw_offset(){let mut c=Clock::default();assert_eq!(c.observe(input(1,100,0,20.)).unwrap().elapsed_play_seconds,Some(20.));assert_eq!(c.observe(input(2,164,0,-500.)).unwrap().elapsed_play_seconds,Some(21.));}
 #[test]fn pause_counter_lag_is_checked_only_at_unpaused_points(){let mut c=Clock::default();c.observe(input(1,100,0,20.)).unwrap();let mut p=input(2,164,0,20.);p.game_paused=Some(true);assert!(!c.observe(p).unwrap().e1_time_available);let r=c.observe(input(3,164,64,20.)).unwrap();assert_eq!(r.elapsed_play_seconds,Some(20.));}
 #[test]fn unknown_capture_is_not_backfilled(){let mut c=Clock::default();let mut a=input(1,100,0,20.);a.raw_match_seconds=None;let before=c.observe(a).unwrap().capture();c.observe(input(2,101,0,20.)).unwrap();assert_eq!(before.capture_tick,Some(1));assert_eq!(before.elapsed_play_seconds,None);}
 #[test]fn interval_counter_and_active_regressions_fail(){let mut c=Clock::default();c.observe(input(1,100,0,20.)).unwrap();let mut a=input(2,101,0,20.);a.tick_interval_seconds=0.02;assert!(c.observe(a).is_err());let mut c=Clock::default();c.observe(input(1,100,0,20.)).unwrap();assert!(c.observe(input(2,99,0,20.)).is_err());let mut c=Clock::default();c.observe(input(1,100,0,20.)).unwrap();assert!(c.observe(input(2,101,2,20.)).is_err());}
 #[test]fn incompatible_raw_anchor_increments_segment_not_elapsed_baseline(){let mut c=Clock::default();c.observe(input(1,100,0,20.)).unwrap();let mut a=input(2,164,0,0.);a.raw_match_clock_at_last_update=Some(0.);a.raw_match_clock_update_tick=Some(164);let got=c.observe(a).unwrap();assert_eq!(got.timing_segment_id,Some(1));assert_eq!(got.elapsed_play_seconds,Some(21.));}
}

#[cfg(test)]mod additional_clock_tests {
 use super::*;
 fn i(tick:i32,base:i64,paused:i64,raw:Option<f64>)->Input{Input{tick,controller_tick_base:Some(base),total_paused_ticks:Some(paused),raw_match_seconds:raw,tick_interval_seconds:0.015625,raw_match_clock_at_last_update:None,raw_match_clock_update_tick:None,game_paused:Some(false),server_paused:Some(false)}}
 #[test]fn large_integer_baseline_subtracts_before_float_conversion(){let mut c=Clock::default();let base=9_007_199_254_740_992;c.observe(i(1,base,7,Some(-30.))).unwrap();let v=c.observe(i(2,base+3,8,Some(-30.))).unwrap();assert_eq!(v.elapsed_play_seconds,Some(-29.96875));}
 #[test]fn controller_or_pause_reset_and_duplicate_source_tick_fail(){for mode in 0..3{let mut c=Clock::default();c.observe(i(1,100,2,Some(20.))).unwrap();let next=match mode{0=>i(2,99,2,Some(20.)),1=>i(2,101,1,Some(20.)),_=>i(1,101,2,Some(20.))};assert!(c.observe(next).is_err());}}
 #[test]fn raw_unknown_rows_preserve_order_and_have_distinct_unknown_stamp(){let mut c=Clock::default();let a=c.observe(i(1,100,0,None)).unwrap();let b=c.observe(i(2,101,0,Some(50.))).unwrap();let d=c.observe(i(3,102,0,None)).unwrap();assert_eq!(a.capture().capture_tick,Some(1));assert!(!a.e1_time_available);assert_eq!(b.elapsed_play_seconds,Some(50.));assert_eq!(d.capture().capture_tick,Some(3));assert_eq!(d.elapsed_play_seconds,None);assert_eq!(d.timing_segment_id,b.timing_segment_id);}
}

#[cfg(test)]mod anchor_representation_tests {
 use super::*;
 fn a(tick:i32,base:i64,anchor:f32)->Input{Input{tick,raw_match_seconds:Some(f64::from(anchor)),controller_tick_base:Some(base),total_paused_ticks:Some(0),tick_interval_seconds:f64::from(0.1f32),raw_match_clock_at_last_update:Some(f64::from(anchor)),raw_match_clock_update_tick:Some(base),game_paused:Some(false),server_paused:Some(false)}}
 #[test]fn fractional_dt_rounds_only_anchor_compatibility_once(){let mut c=Clock::default();c.observe(a(1,100,0.2)).unwrap();let got=c.observe(a(2,101,0.3)).unwrap();assert_eq!(got.timing_segment_id,Some(0));let expected=f64::from(0.2f32)+f64::from(0.1f32);assert_ne!(expected,f64::from(0.3f32));assert_eq!(got.elapsed_play_seconds,Some(expected));}
 #[test]fn one_f32_step_correction_is_a_real_segment_revision(){let mut c=Clock::default();c.observe(a(1,100,0.2)).unwrap();let corrected=f32::from_bits(0.3f32.to_bits()+1);let got=c.observe(a(2,101,corrected)).unwrap();assert_eq!(got.timing_segment_id,Some(1));assert_eq!(got.elapsed_play_seconds,Some(f64::from(0.2f32)+f64::from(0.1f32)));}
 #[test]fn signed_zero_anchor_comparison_is_numeric(){let mut c=Clock::default();c.observe(a(1,100,0.)).unwrap();let got=c.observe(a(2,100,-0.)).unwrap();assert_eq!(got.timing_segment_id,Some(0));}
}

#[cfg(test)]mod qualification_proof_tests {
 use super::*;
 fn sample(tick:i32,game:Option<bool>,server:Option<bool>)->Input{Input{tick,raw_match_seconds:Some(20.),controller_tick_base:Some(100+i64::from(tick)),total_paused_ticks:Some(0),tick_interval_seconds:0.015625,raw_match_clock_at_last_update:Some(20.),raw_match_clock_update_tick:Some(100),game_paused:game,server_paused:server}}
 #[test]fn one_or_both_unknown_pause_flags_never_qualify_or_backfill(){
  for(game,server)in [(None,Some(false)),(Some(false),None),(None,None)]{
   let mut c=Clock::default();let unknown=c.observe(sample(1,game,server)).unwrap();assert!(!unknown.e1_time_available);assert_eq!(unknown.elapsed_play_seconds,None);assert_eq!(unknown.capture().capture_tick,Some(1));
   let valid=c.observe(sample(2,Some(false),Some(false))).unwrap();assert!(valid.e1_time_available);assert_eq!(valid.elapsed_play_seconds,Some(20.));assert_eq!(unknown.capture().elapsed_play_seconds,None);
   let later_unknown=c.observe(sample(3,game,server)).unwrap();assert!(!later_unknown.e1_time_available);assert_eq!(later_unknown.elapsed_play_seconds,None);assert_eq!(later_unknown.capture().capture_tick,Some(3));assert_eq!(later_unknown.timing_segment_id,valid.timing_segment_id);
  }
 }
 #[test]fn prebaseline_raw_finite_unknown_or_paused_capture_keeps_segment_identity(){
  for(game,server)in [(None,Some(false)),(Some(false),None),(None,None),(Some(true),Some(false))]{
   let mut c=Clock::default();let unknown=c.observe(sample(1,game,server)).unwrap();let stamp=unknown.capture();assert_eq!(stamp.capture_tick,Some(1));assert_eq!(stamp.timing_segment_id,Some(0));assert_eq!(stamp.elapsed_play_seconds,None);assert!(!unknown.e1_time_available);
   let valid=c.observe(sample(2,Some(false),Some(false))).unwrap();assert_eq!(valid.timing_segment_id,Some(0));assert!(valid.e1_time_available);assert_eq!(stamp.capture_tick,Some(1));assert_eq!(stamp.timing_segment_id,Some(0));assert_eq!(stamp.elapsed_play_seconds,None);
   let mut revised=sample(3,Some(false),Some(false));revised.raw_match_clock_at_last_update=Some(99.);revised.raw_match_clock_update_tick=Some(103);let changed=c.observe(revised).unwrap();assert_eq!(changed.timing_segment_id,Some(1));assert_eq!(stamp.timing_segment_id,Some(0));assert_eq!(stamp.elapsed_play_seconds,None);
  }
 }
 #[test]fn negative_first_source_tick_is_rejected(){let mut c=Clock::default();let error=c.observe(sample(-1,Some(false),Some(false))).unwrap_err();assert!(error.contains("negative causal source tick"));}
}
