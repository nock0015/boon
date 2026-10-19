// Exact invocation-local immutable payload revisions. This is a custom
// copy-on-write design, not hash equality or a library performance claim.
use std::sync::{Arc,OnceLock};
#[derive(Default)]
struct ExtraRevision {values:BTreeMap<String,f64>,reflexive:OnceLock<bool>,position_zero:OnceLock<Arc<ExtraRevision>>,position_absent:OnceLock<Arc<ExtraRevision>>}
impl Clone for ExtraRevision{fn clone(&self)->Self{Self{values:self.values.clone(),reflexive:self.reflexive.clone(),position_zero:OnceLock::new(),position_absent:OnceLock::new()}}}
#[derive(Clone,Default)]
struct SharedExtras(Arc<ExtraRevision>);
impl SharedExtras {
 fn insert(&mut self,key:String,value:f64)->Option<f64>{
  // Preserve the latest signed-zero and NaN bits. An exactly identical write
  // leaves the immutable revision intact; equality itself remains IEEE.
  if let Some(old)=self.0.values.get(&key){if old.to_bits()==value.to_bits(){return Some(*old)}}
  if key=="position-observation-age"&&value.to_bits()==0_f64.to_bits(){
   let old=self.0.values.get(&key).copied();let next=self.0.position_zero.get_or_init(||{let mut next=ExtraRevision::default();next.values=self.0.values.clone();next.values.insert(key,value);Arc::new(next)}).clone();self.0=next;return old
  }
  let revision=Arc::make_mut(&mut self.0);revision.reflexive.take();revision.position_zero.take();revision.position_absent.take();revision.values.insert(key,value)
 }
 fn remove(&mut self,key:&str)->Option<f64>{let old=*self.0.values.get(key)?;
  if key=="position-observation-age"{let next=self.0.position_absent.get_or_init(||{let mut next=ExtraRevision::default();next.values=self.0.values.clone();next.values.remove(key);Arc::new(next)}).clone();self.0=next;return Some(old)}
  let revision=Arc::make_mut(&mut self.0);revision.reflexive.take();revision.position_zero.take();revision.position_absent.take();revision.values.remove(key)
 }
 fn clear(&mut self){if !self.0.values.is_empty(){self.0=Arc::default();}}
}
impl std::ops::Deref for SharedExtras{type Target=BTreeMap<String,f64>;fn deref(&self)->&Self::Target{&self.0.values}}
impl std::ops::DerefMut for SharedExtras{fn deref_mut(&mut self)->&mut Self::Target{let revision=Arc::make_mut(&mut self.0);revision.reflexive.take();revision.position_zero.take();revision.position_absent.take();&mut revision.values}}
impl PartialEq for SharedExtras{fn eq(&self,other:&Self)->bool{
 if Arc::ptr_eq(&self.0,&other.0){return *self.0.reflexive.get_or_init(||self.0.values.values().all(|v|!v.is_nan()))}
 self.0.values==other.0.values
}}
impl IntoIterator for SharedExtras{type Item=(String,f64);type IntoIter=std::collections::btree_map::IntoIter<String,f64>;fn into_iter(self)->Self::IntoIter{Arc::try_unwrap(self.0).unwrap_or_else(|p|(*p).clone()).values.into_iter()}}
impl<'a> IntoIterator for &'a SharedExtras{type Item=(&'a String,&'a f64);type IntoIter=std::collections::btree_map::Iter<'a,String,f64>;fn into_iter(self)->Self::IntoIter{self.0.values.iter()}}
#[derive(Clone,Default)]
struct SharedRelations(Arc<Vec<(String,i32)>>);
impl SharedRelations{fn clear(&mut self){if !self.0.is_empty(){self.0=Arc::default();}}}
impl std::ops::Deref for SharedRelations{type Target=Vec<(String,i32)>;fn deref(&self)->&Self::Target{&self.0}}
impl std::ops::DerefMut for SharedRelations{fn deref_mut(&mut self)->&mut Self::Target{Arc::make_mut(&mut self.0)}}
impl PartialEq for SharedRelations{fn eq(&self,other:&Self)->bool{Arc::ptr_eq(&self.0,&other.0)||self.0==other.0}}
impl From<Vec<(String,i32)>> for SharedRelations{fn from(values:Vec<(String,i32)>)->Self{Self(Arc::new(values))}}
impl IntoIterator for SharedRelations{type Item=(String,i32);type IntoIter=std::vec::IntoIter<(String,i32)>;fn into_iter(self)->Self::IntoIter{Arc::try_unwrap(self.0).unwrap_or_else(|p|(*p).clone()).into_iter()}}
impl<'a> IntoIterator for &'a SharedRelations{type Item=&'a (String,i32);type IntoIter=std::slice::Iter<'a,(String,i32)>;fn into_iter(self)->Self::IntoIter{self.0.iter()}}
#[cfg(test)]mod shared_payload_tests{use super::*;
 #[test]fn extra_revisions_keep_ieee_and_latest_bits(){let mut a=SharedExtras::default();a.insert("v".into(),0.);let b=a.clone();assert!(a==b);assert!(Arc::ptr_eq(&a.0,&b.0));a.insert("v".into(),-0.);assert!(a==b);assert!(!Arc::ptr_eq(&a.0,&b.0));assert_eq!(a["v"].to_bits(),(-0_f64).to_bits());a.insert("v".into(),f64::NAN);let b=a.clone();assert!(a!=b);assert!(a!=a);a.insert("v".into(),1.);let b=a.clone();assert!(a==b);a.values_mut().for_each(|v|*v=f64::NAN);assert!(a!=a);assert!(b==b);}
 #[test]fn normalized_position_revisions_reuse_exact_raw_payload(){let mut raw=SharedExtras::default();raw.insert("x".into(),1.);let mut a=raw.clone();a.insert("position-observation-age".into(),0.);let mut b=raw.clone();b.insert("position-observation-age".into(),0.);assert!(Arc::ptr_eq(&a.0,&b.0));let mut absent_a=a.clone();absent_a.remove("position-observation-age");let mut absent_b=a.clone();absent_b.remove("position-observation-age");assert!(Arc::ptr_eq(&absent_a.0,&absent_b.0));a.insert("x".into(),2.);a.remove("position-observation-age");assert_eq!(a["x"],2.);}
 #[test]fn redundant_writes_keep_exact_revision(){let mut a=SharedExtras::default();a.insert("v".into(),2.);let b=a.clone();a.remove("absent");a.insert("v".into(),2.);assert!(Arc::ptr_eq(&a.0,&b.0));}
 #[test]fn relations_preserve_duplicate_order_and_detach_on_mutation(){let mut a=SharedRelations::from(vec![("z".into(),2),("a".into(),1),("z".into(),2)]);let b=a.clone();assert!(a==b);a.push(("other".into(),1));assert!(a!=b);assert_eq!(b.into_iter().collect::<Vec<_>>(),vec![("z".into(),2),("a".into(),1),("z".into(),2)]);}
}
