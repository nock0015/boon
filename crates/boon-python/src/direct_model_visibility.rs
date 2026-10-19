//! The model sees exactly its initial source class filter during a shared scan.
//! The view owns no entities and is local to one invocation. Schema changes
//! clear numeric decisions while preserving the initial class-name set.
use super::*;

#[derive(Default)]
pub(crate) struct ModelEntityView {
    initial_classes:Option<HashSet<String>>,
    classes:HashMap<i32,(std::sync::Arc<str>,bool)>,
}
impl ModelEntityView {
    pub(crate) fn restricted<'a>(names:impl IntoIterator<Item=&'a str>)->Self {
        Self {initial_classes:Some(names.into_iter().map(str::to_owned).collect()),classes:HashMap::new()}
    }
    pub(crate) fn invalidate_schema(&mut self) {self.classes.clear();}
    pub(crate) fn eligible(&mut self,entity:&Entity)->bool {
        let Some(initial)=self.initial_classes.as_ref()else{return true};
        if let Some((name,eligible))=self.classes.get(&entity.class_id) {
            if std::sync::Arc::ptr_eq(name,&entity.class_name)||name==&entity.class_name {return *eligible}
        }
        let eligible=initial.contains(entity.class_name.as_ref());
        self.classes.insert(entity.class_id,(entity.class_name.clone(),eligible));eligible
    }
    pub(crate) fn get<'a>(&mut self,entities:&'a boon_parser::EntityContainer,index:i32)->Option<&'a Entity> {
        entities.get(index).filter(|entity|self.eligible(entity))
    }
    pub(crate) fn get_by_handle<'a>(&mut self,entities:&'a boon_parser::EntityContainer,handle:u32)->Option<&'a Entity> {
        entities.get_by_handle(handle).filter(|entity|self.eligible(entity))
    }
}
#[cfg(test)]mod tests {
    use super::*;
    fn entity(class:i32,name:&str)->Entity {Entity::from_fields(7,9,class,name,true,Default::default()).unwrap()}
    #[test]fn initial_filter_caches_true_and_false_and_survives_schema_replacement() {
        let mut view=ModelEntityView::restricted(["CCitadelPlayerPawn"]);
        for (id,name,expected)in [(1,"Other",false),(1,"CCitadelPlayerPawn",true),(1,"Other",false),(2,"CCitadelPlayerPawn",true)] {
            let entity=entity(id,name);assert_eq!(view.eligible(&entity),expected);assert_eq!(view.eligible(&entity),expected);
        }
        view.invalidate_schema();assert!(!view.eligible(&entity(1,"Other")));assert!(view.eligible(&entity(1,"CCitadelPlayerPawn")));
    }
    #[test]fn raw_slot_and_full_handle_resolution_hide_companion_classes() {
        let mut entities=boon_parser::EntityContainer::new();entities.insert(entity(1,"Other")).unwrap();
        let mut outer=ModelEntityView::restricted(["CCitadelPlayerPawn"]);assert!(outer.get(&entities,7).is_none());assert!(outer.get_by_handle(&entities,7+(9<<14)).is_none());
        // A separate/nested view cannot broaden its caller's invocation.
        let mut nested=ModelEntityView::default();assert!(nested.get(&entities,7).is_some());assert!(outer.get(&entities,7).is_none());
        entities.insert(entity(1,"CCitadelPlayerPawn")).unwrap();assert!(outer.get(&entities,7).is_some());assert!(outer.get_by_handle(&entities,7+(8<<14)).is_none());
    }
}
