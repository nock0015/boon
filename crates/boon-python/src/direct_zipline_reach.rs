//! Public lane aggregates only. Raw graph observation stays available through
//! the separate audit accessor; no physical zipline node enters model tables.
//! Distances are recorded cumulative-route units from the current qualified
//! home endpoint. Ownership connectivity is not a claim of ride usability;
//! m_bDisableZippingToByPlayers is deliberately not used as a cable barrier.
use super::*;
pub(super) type Homes=BTreeMap<i64,[(Id,f64);2]>;
struct Zip {id:Id,lane:Option<i64>,team:Option<i64>,distance:Option<f64>,capturable:Option<bool>,links:Option<Vec<Id>>}
impl Clone for Zip {fn clone(&self)->Self{let _method_span=crate::direct_method_profile::span(74);Self{id:self.id.clone(),lane:self.lane.clone(),team:self.team.clone(),distance:self.distance.clone(),capturable:self.capturable.clone(),links:self.links.clone()}}}

fn observe(ctx:&Context,e:&Entity)->Zip{let _method_span=crate::direct_method_profile::span(75);
 let links=count(ctx,e,"m_vecConnections",32).and_then(|len|{let mut ids=vec![];for slot in 0..len{let raw=ai(ctx,e,"m_vecConnections",slot,"")?;let handle=u32::try_from(raw).ok()?;let target=ctx.entities().get_by_handle(handle).filter(|t|t.active&&t.class_name.as_ref()=="CCitadelZipLineNode")?;let id=(target.index,target.serial);if ids.contains(&id){return None}ids.push(id);}Some(ids)});
 Zip{id:(e.index,e.serial),lane:i(ctx,e,"m_iPrimaryLane"),team:i(ctx,e,"m_iTeamNum"),distance:n(ctx,e,"m_flCumulativeDistance").filter(|v|*v>=0.),capturable:b(ctx,e,"m_bCapturable"),links}
}
#[cfg(test)]
fn reaches(lane:i64,nodes:&[Zip],homes:&mut Homes)->[Option<f64>;2]{let _method_span=crate::direct_method_profile::span(76);
 if nodes.is_empty()||nodes.iter().any(|n|n.lane!=Some(lane)||n.distance.is_none()||n.links.is_none()||!n.team.is_some_and(|v|matches!(v,0|2|3|4))){return [None;2]}
 let by_id:HashMap<_,_>=nodes.iter().map(|n|(n.id,n)).collect();if by_id.len()!=nodes.len(){return [None;2]}
 for node in nodes{for target in node.links.as_ref().unwrap(){if *target==node.id||!by_id.get(target).is_some_and(|other|other.links.as_ref().unwrap().contains(&node.id)){return [None;2]}}}
 let min=nodes.iter().map(|n|n.distance.unwrap()).fold(f64::INFINITY,f64::min);let max=nodes.iter().map(|n|n.distance.unwrap()).fold(f64::NEG_INFINITY,f64::max);
 if min!=0.||max<=min{return [None;2]}
 let starts:Vec<_>=nodes.iter().filter(|n|n.distance==Some(min)).collect();let ends:Vec<_>=nodes.iter().filter(|n|n.distance==Some(max)).collect();
 if starts.len()!=1||ends.len()!=1{return [None;2]}let anchors=[starts[0],ends[0]];
 if anchors.iter().zip([2,3]).any(|(n,team)|n.team!=Some(team)||n.capturable!=Some(false)||n.links.as_ref().unwrap().len()!=1){return [None;2]}
 let origin=[(anchors[0].id,min),(anchors[1].id,max)];if let Some(old)=homes.get(&lane){if *old!=origin{return [None;2]}}else{homes.insert(lane,origin);}
 let mut result=[None;2];for(side,team)in [2,3].into_iter().enumerate(){
  let mut seen=HashSet::from([origin[side].0]);let mut queue=vec![origin[side].0];let mut reach=0_f64;
  while let Some(id)=queue.pop(){let node=by_id[&id];reach=reach.max((node.distance.unwrap()-origin[side].1).abs());for target in node.links.as_ref().unwrap(){let other=by_id[target];if other.team==Some(team)&&seen.insert(*target){queue.push(*target);}}}
  result[side]=Some(reach);
 }result
}
// Topology is independent of ownership. Retaining dense adjacency lets a team
// update reuse validation and allocations, including traversal scratch space.
#[derive(Default)]
struct Topology {
 ids: Vec<Id>, distances: Vec<f64>, edges: Vec<Vec<usize>>,
 anchors: Option<[usize; 2]>, stack: Vec<usize>, visited: Vec<u64>, visit: u64,
}
impl Topology {
 fn build(lane:i64,nodes:&[Zip])->Self {
  let mut graph=Self::default();
  if nodes.is_empty()||nodes.iter().any(|n|n.lane!=Some(lane)||n.distance.is_none()||n.links.is_none()){return graph}
  let by_id:HashMap<_,_>=nodes.iter().enumerate().map(|(index,node)|(node.id,index)).collect();
  if by_id.len()!=nodes.len(){return graph}
  for node in nodes {
   let mut edges=Vec::with_capacity(node.links.as_ref().unwrap().len());
   for target in node.links.as_ref().unwrap(){
    let Some(&index)=by_id.get(target) else{return Self::default()};
    if *target==node.id||!nodes[index].links.as_ref().unwrap().contains(&node.id){return Self::default()}
    edges.push(index);
   }
   graph.edges.push(edges);
  }
  let min=nodes.iter().map(|n|n.distance.unwrap()).fold(f64::INFINITY,f64::min);
  let max=nodes.iter().map(|n|n.distance.unwrap()).fold(f64::NEG_INFINITY,f64::max);
  if min!=0.||max<=min{return Self::default()}
  let mut starts=nodes.iter().enumerate().filter(|(_,n)|n.distance==Some(min));
  let start=starts.next().unwrap().0;if starts.next().is_some(){return Self::default()}
  let mut ends=nodes.iter().enumerate().filter(|(_,n)|n.distance==Some(max));
  let end=ends.next().unwrap().0;if ends.next().is_some(){return Self::default()}
  if [start,end].into_iter().any(|index|nodes[index].capturable!=Some(false)||graph.edges[index].len()!=1){return Self::default()}
  graph.ids=nodes.iter().map(|n|n.id).collect();graph.distances=nodes.iter().map(|n|n.distance.unwrap()).collect();
  graph.visited.resize(nodes.len(),0);graph.anchors=Some([start,end]);graph
 }
 fn reaches(&mut self,lane:i64,nodes:&[Zip],homes:&mut Homes)->[Option<f64>;2]{
  let _method_span=crate::direct_method_profile::span(76);
  let Some(anchors)=self.anchors else{return [None;2]};
  if nodes.iter().any(|n|!n.team.is_some_and(|v|matches!(v,0|2|3|4)))||anchors.into_iter().zip([2,3]).any(|(index,team)|nodes[index].team!=Some(team)){return [None;2]}
  let origin=anchors.map(|index|(self.ids[index],self.distances[index]));
  if let Some(old)=homes.get(&lane){if *old!=origin{return [None;2]}}else{homes.insert(lane,origin);}
  let mut result=[None;2];
  for (side,team) in [2,3].into_iter().enumerate(){
   self.visit=self.visit.wrapping_add(1);if self.visit==0{self.visited.fill(0);self.visit=1;}
   self.stack.clear();let start=anchors[side];self.stack.push(start);self.visited[start]=self.visit;let mut reach=0_f64;
   while let Some(index)=self.stack.pop(){
    reach=reach.max((self.distances[index]-origin[side].1).abs());
    for &target in &self.edges[index]{if nodes[target].team==Some(team)&&self.visited[target]!=self.visit{self.visited[target]=self.visit;self.stack.push(target);}}
   }
   result[side]=Some(reach);
  }
  result
 }
}
#[derive(Default)]
struct LaneCache{nodes:Vec<Zip>,topology:Topology,values:[Option<f64>;2],#[cfg(test)] recomputations:usize}
impl LaneCache {
 fn refresh(&mut self,lane:i64,nodes:Vec<Zip>,unknown:bool,homes:&mut Homes){
  let topology_changed=self.nodes.len()!=nodes.len()||self.nodes.iter().zip(&nodes).any(|(old,new)|!old.same_topology(new));
  if topology_changed{self.topology=Topology::build(lane,&nodes);}
  self.nodes=nodes;
  self.values=if unknown{[None;2]}else{self.topology.reaches(lane,&self.nodes,homes)};
  #[cfg(test)] {self.recomputations+=1;}
 }
}
impl Zip {
 fn same_topology(&self,other:&Self)->bool{self.id==other.id&&self.lane==other.lane&&self.distance==other.distance&&self.capturable==other.capturable&&self.links==other.links}
 fn same_observation(&self,other:&Self)->bool{self.same_topology(other)&&self.team==other.team}
}
/// Updated only at the existing projection cadence. `dirty` must include all
/// source callbacks since the previous projection, including lifecycle events.
#[derive(Default)]
pub(super) struct Cache {
 nodes:BTreeMap<Id,Zip>,lanes:BTreeMap<i64,LaneCache>,unknown:bool,rows:Vec<Node>,seeded:bool,revision:u64,
}
impl Cache {
 pub(super) fn revision(&self)->u64{self.revision}
 pub(super) fn rows(&self)->&[Node]{&self.rows}
 pub(super) fn project(&mut self,ctx:&Context,entities:&[&Entity],dirty:&HashSet<i32>,homes:&mut Homes)->&[Node]{
  let _method_span=crate::direct_method_profile::span(77);
  let membership_changed=self.nodes.len()!=entities.len()||entities.iter().any(|e|!self.nodes.contains_key(&(e.index,e.serial)));
  // Handle resolution depends on the target's active generation as well as
  // the source's fields. On membership changes retry even invalid old links.
  let observations:Vec<_>=entities.iter().filter(|e|membership_changed||dirty.contains(&e.index)).map(|e|observe(ctx,e)).collect();
  let present:HashSet<_>=if membership_changed{entities.iter().map(|e|(e.index,e.serial)).collect()}else{HashSet::new()};
  self.update(observations,membership_changed.then_some(&present),homes);
  &self.rows
 }
 fn update(&mut self,observations:Vec<Zip>,present:Option<&HashSet<Id>>,homes:&mut Homes){
  let mut affected=std::collections::BTreeSet::new();
  if let Some(present)=present{self.nodes.retain(|id,node|{let keep=present.contains(id);if !keep{if let Some(lane)=node.lane{affected.insert(lane);}}keep});}
  for node in observations {
   if self.nodes.get(&node.id).is_some_and(|old|old.same_observation(&node)){continue}
   let lane=node.lane;
   if let Some(old)=self.nodes.insert(node.id,node){if let Some(lane)=old.lane{affected.insert(lane);}}
   if let Some(lane)=lane{affected.insert(lane);}
  }
  let unknown=self.nodes.values().any(|node|node.lane.is_none());
  if self.unknown!=unknown{affected.extend(self.lanes.keys().copied());affected.extend(self.nodes.values().filter_map(|n|n.lane));}
  self.unknown=unknown;
  if affected.is_empty()&&self.seeded{return}
  let old_values:Vec<_>=self.lanes.iter().map(|(&lane,cached)|(lane,cached.values)).collect();
  for lane in affected {
   let nodes:Vec<_>=self.nodes.values().filter(|node|node.lane==Some(lane)).cloned().collect();
   if nodes.is_empty(){self.lanes.remove(&lane);}else{self.lanes.entry(lane).or_default().refresh(lane,nodes,unknown,homes);}
  }
  if !old_values.iter().copied().eq(self.lanes.iter().map(|(&lane,cached)|(lane,cached.values))){
   self.rows=self.lanes.iter().map(|(&lane,cached)|lane_node(lane,cached.values)).collect();self.revision=self.revision.wrapping_add(1);
  }
  self.seeded=true;
 }
}
fn lane_node(lane:i64,values:[Option<f64>;2])->Node {
 let mut node=Node::new(format!("zipline-reach:lane:{lane}"),"zipline",0);node.class=Some(format!("public-zipline-ownership-reach-lane:{lane}"));
 for (team,value) in [2,3].into_iter().zip(values){if let Some(value)=value{node.extras.insert(format!("zipline_ownership_connected_reach_team{team}"),value);}}node
}

#[cfg(test)]mod tests{
 use super::*;
 fn node(index:i32,team:i64,distance:f64,links:&[i32])->Zip{Zip{id:(index,1),lane:Some(1),team:Some(team),distance:Some(distance),capturable:Some(!matches!(index,1|4)),links:Some(links.iter().map(|i|(*i,1)).collect())}}
 fn graph()->Vec<Zip>{vec![node(1,2,0.,&[2]),node(2,2,10.,&[1,3]),node(3,0,20.,&[2,4]),node(4,3,30.,&[3]),node(5,2,25.,&[])]}
 #[test]fn reach_is_home_connected_and_neutral_or_team4_are_barriers(){let mut homes=Homes::new();let mut nodes=graph();assert_eq!(reaches(1,&nodes,&mut homes),[Some(10.),Some(0.)]);nodes[2].team=Some(4);assert_eq!(reaches(1,&nodes,&mut homes),[Some(10.),Some(0.)]);nodes[2].team=Some(3);assert_eq!(reaches(1,&nodes,&mut homes),[Some(10.),Some(10.)]);}
 #[test]fn ownership_reach_requires_no_unqualified_ride_usability_flags(){assert_eq!(reaches(1,&graph(),&mut Homes::new()),[Some(10.),Some(0.)]);}
 #[test]fn unqualified_topology_or_changed_origin_is_unknown(){let mut homes=Homes::new();let mut nodes=graph();assert!(reaches(1,&nodes,&mut homes)[0].is_some());nodes[0].id=(1,2);nodes[1].links.as_mut().unwrap()[0]=(1,2);assert_eq!(reaches(1,&nodes,&mut homes),[None;2]);nodes=graph();nodes[2].links=None;assert_eq!(reaches(1,&nodes,&mut Homes::new()),[None;2]);nodes=graph();nodes[1].team=None;assert_eq!(reaches(1,&nodes,&mut Homes::new()),[None;2]);}
 fn submit(cache:&mut Cache,nodes:Vec<Zip>,homes:&mut Homes){let present=nodes.iter().map(|n|n.id).collect();cache.update(nodes,Some(&present),homes);}
 fn second_lane()->Vec<Zip>{graph().into_iter().map(|mut n|{n.lane=Some(2);n.id.0+=10;for id in n.links.as_mut().unwrap(){id.0+=10;}n}).collect()}
 #[test]fn dense_reach_matches_reference_ownership_and_invalid_topology(){
  for a in [None,Some(0),Some(2),Some(3),Some(4),Some(7)]{for b in [None,Some(0),Some(2),Some(3),Some(4),Some(7)]{
   let mut nodes=graph();nodes[1].team=a;nodes[2].team=b;
   assert_eq!(Topology::build(1,&nodes).reaches(1,&nodes,&mut Homes::new()),reaches(1,&nodes,&mut Homes::new()));
  }}
  for mode in 0..8{let mut nodes=graph();match mode{
   0=>nodes[1].links=None,1=>nodes[1].links=Some(vec![(2,1)]),2=>nodes[1].links=Some(vec![(99,1)]),
   3=>nodes[0].distance=Some(1.),4=>nodes[2].distance=Some(30.),5=>nodes[0].capturable=None,
   6=>nodes[0].team=Some(3),_=>nodes[1].links=Some(vec![(3,1)]),
  };assert_eq!(Topology::build(1,&nodes).reaches(1,&nodes,&mut Homes::new()),reaches(1,&nodes,&mut Homes::new()));}
 }
 #[test]fn cache_updates_only_changed_lane_and_reuses_dense_topology(){
  let mut cache=Cache::default();let mut homes=Homes::new();let mut nodes=graph();nodes.extend(second_lane());submit(&mut cache,nodes.clone(),&mut homes);
  assert_eq!(cache.lanes[&1].recomputations,1);assert_eq!(cache.lanes[&2].recomputations,1);let revision=cache.revision();
  submit(&mut cache,nodes.clone(),&mut homes);assert_eq!(cache.revision(),revision);assert_eq!(cache.lanes[&1].recomputations,1);
  let edges=cache.lanes[&1].topology.edges.as_ptr();nodes[2].team=Some(2);submit(&mut cache,nodes,&mut homes);
  assert_eq!(cache.lanes[&1].values,[Some(20.),Some(0.)]);assert_eq!(cache.lanes[&1].recomputations,2);assert_eq!(cache.lanes[&2].recomputations,1);assert_eq!(cache.lanes[&1].topology.edges.as_ptr(),edges);assert_ne!(cache.revision(),revision);
 }
 #[test]fn unknown_lane_suppresses_all_lanes_without_qualifying_homes(){
  let mut cache=Cache::default();let mut homes=Homes::new();let mut nodes=graph();let mut unknown=node(99,0,5.,&[]);unknown.lane=None;nodes.push(unknown);submit(&mut cache,nodes,&mut homes);
  assert!(homes.is_empty());assert_eq!(cache.lanes[&1].values,[None;2]);
  submit(&mut cache,graph(),&mut homes);assert_eq!(cache.lanes[&1].values,[Some(10.),Some(0.)]);assert!(homes.contains_key(&1));
  let mut nodes=graph();nodes[2].lane=None;submit(&mut cache,nodes,&mut homes);assert_eq!(cache.lanes[&1].values,[None;2]);
  submit(&mut cache,graph(),&mut homes);assert_eq!(cache.lanes[&1].values,[Some(10.),Some(0.)]);
 }
 #[test]fn generation_reuse_retains_original_home_and_lane_removal_order(){
  let mut cache=Cache::default();let mut homes=Homes::new();let mut nodes=graph();nodes.extend(second_lane());submit(&mut cache,nodes,&mut homes);let original=homes[&1];
  let mut nodes=graph();nodes[0].id.1=2;nodes[1].links.as_mut().unwrap()[0].1=2;submit(&mut cache,nodes,&mut homes);
  assert_eq!(cache.lanes[&1].values,[None;2]);assert_eq!(homes[&1],original);assert_eq!(cache.rows.len(),1);
  submit(&mut cache,vec![],&mut homes);assert!(cache.rows.is_empty());assert_eq!(homes[&1],original);
 }
 #[test]fn unchanged_result_keeps_revision_and_visit_wrap_resets_scratch(){
  let mut cache=Cache::default();let mut homes=Homes::new();submit(&mut cache,graph(),&mut homes);let revision=cache.revision();
  let mut nodes=graph();nodes[2].team=Some(4);submit(&mut cache,nodes,&mut homes);assert_eq!(cache.revision(),revision);
  let graph=graph();let mut topology=Topology::build(1,&graph);topology.visit=u64::MAX;topology.visited.fill(1);
  assert_eq!(topology.reaches(1,&graph,&mut Homes::new()),[Some(10.),Some(0.)]);
 }

 #[test]fn lane_change_invalidates_old_and_new_and_keeps_sorted_output(){
  let mut cache=Cache::default();let mut homes=Homes::new();let mut nodes=graph();nodes.extend(second_lane());submit(&mut cache,nodes.clone(),&mut homes);
  nodes[4].lane=Some(2);submit(&mut cache,nodes,&mut homes);
  assert_eq!(cache.lanes[&1].recomputations,2);assert_eq!(cache.lanes[&2].recomputations,2);
  assert_eq!(cache.lanes[&1].values,[Some(10.),Some(0.)]);assert_eq!(cache.lanes[&2].values,[Some(10.),Some(0.)]);
  assert_eq!(cache.rows[0].class.as_deref(),Some("public-zipline-ownership-reach-lane:1"));assert_eq!(cache.rows[1].class.as_deref(),Some("public-zipline-ownership-reach-lane:2"));
 }

}
