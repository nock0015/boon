use super::*;
use crate::entity::{
    BareCharEncoding, ClassEntry, DecodeProfile, FlattenedField, FlattenedSerializer,
    FlattenedSerializerDefinition, PreciseQAngleMode,
};

fn schemas(kind: &str) -> SerializerContainer {
    SerializerContainer::parse(
        FlattenedSerializer::new(
            vec![
                FlattenedSerializerDefinition::new(Some(0), vec![0]),
                FlattenedSerializerDefinition::new(Some(1), vec![0]),
            ],
            vec!["A".into(), "B".into(), kind.into(), "value".into()],
            vec![FlattenedField::new(Some(2), Some(3))],
        ),
        DecodeProfile::new(
            BareCharEncoding::NullTerminatedString,
            PreciseQAngleMode::Centered,
        ),
    )
    .unwrap()
}

fn classes(names: &[&str]) -> ClassInfo {
    ClassInfo::try_from_entries(
        names
            .iter()
            .enumerate()
            .map(|(i, name)| ClassEntry::new(i as i32, *name, "")),
    )
    .unwrap()
}

#[test]
fn replacement_serializers_invalidate_even_with_identical_names_and_ids() {
    let classes = classes(&["A", "B"]);
    let initial = schemas("bool");
    let replacement = schemas("uint32");
    let name = &classes.by_id(0).unwrap().network_name;
    let mut cache = SerializerBindings::default();
    cache.refresh(&classes, &initial);
    assert_eq!(
        cache.get(0, name, &initial).unwrap().fields[0].var_type,
        "bool"
    );
    cache.refresh(&classes, &replacement);
    assert_eq!(
        cache.get(0, name, &replacement).unwrap().fields[0].var_type,
        "uint32"
    );
}

#[test]
fn class_remapping_rebinds_ids_and_preserves_name_fallback() {
    let first = classes(&["A", "B"]);
    let next = classes(&["B", "A"]);
    let serializers = schemas("bool");
    let mut cache = SerializerBindings::default();
    cache.refresh(&first, &serializers);
    cache.refresh(&next, &serializers);
    assert_eq!(cache.entries[0].as_ref().unwrap().name.as_ref(), "B");
    assert_eq!(
        cache
            .get(0, &next.by_id(0).unwrap().network_name, &serializers)
            .unwrap()
            .name,
        "B"
    );
    // Existing/manual entities may still carry the old class name or no valid ID.
    for id in [-1, 0, i32::MAX] {
        assert_eq!(
            cache.get(id, &Arc::from("A"), &serializers).unwrap().name,
            "A"
        );
    }
}

#[test]
fn cloned_schemas_keep_identity_but_new_tables_do_not() {
    let classes = classes(&["A"]);
    let serializers = schemas("bool");
    let mut cache = SerializerBindings::default();
    cache.refresh(&classes, &serializers);
    let slots = cache.entries.as_ptr();
    cache.refresh(&classes.clone(), &serializers.clone());
    assert_eq!(cache.entries.as_ptr(), slots);
    assert!(Arc::ptr_eq(
        cache.classes.as_ref().unwrap(),
        classes.schema_id()
    ));
    let mut cloned = cache.clone();
    cloned.refresh(&classes, &schemas("uint32"));
    assert_eq!(
        cache.get(0, &Arc::from("A"), &serializers).unwrap().fields[0].var_type,
        "bool"
    );
}

#[test]
fn unused_missing_serializers_and_empty_replacements_are_allowed() {
    let serializers = schemas("bool");
    let mut cache = SerializerBindings::default();
    cache.refresh(&classes(&["A", "Missing"]), &serializers);
    assert!(cache.get(1, &Arc::from("Missing"), &serializers).is_none());
    cache.refresh(&ClassInfo::empty(), &SerializerContainer::default());
    assert!(cache.entries.is_empty());
    assert!(SerializerBindings::default().entries.is_empty());
}
