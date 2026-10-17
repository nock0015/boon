use std::sync::Arc;

use super::{ClassInfo, Serializer, SerializerContainer};

#[derive(Clone)]
struct Binding {
    name: Arc<str>,
    serializer: Arc<Serializer>,
}

/// Per-container bindings for one immutable class/serializer schema pair.
/// Owns at most one schema pair, without raw pointers or a global cache.
#[derive(Clone, Default)]
pub(super) struct SerializerBindings {
    classes: Option<Arc<()>>,
    serializers: Option<Arc<()>>,
    entries: Vec<Option<Binding>>,
}

impl SerializerBindings {
    pub(super) fn refresh(&mut self, classes: &ClassInfo, serializers: &SerializerContainer) {
        if self
            .classes
            .as_ref()
            .is_some_and(|id| Arc::ptr_eq(id, classes.schema_id()))
            && self
                .serializers
                .as_ref()
                .is_some_and(|id| Arc::ptr_eq(id, serializers.schema_id()))
        {
            return;
        }
        // IDs and their maximum were already validated when ClassInfo was built.
        let slots = classes
            .classes()
            .iter()
            .map(|class| class.class_id as usize + 1)
            .max()
            .unwrap_or(0);
        self.entries.clear();
        self.entries.resize_with(slots, || None);
        for class in classes.classes() {
            self.entries[class.class_id as usize] =
                serializers
                    .shared(&class.network_name)
                    .map(|serializer| Binding {
                        name: Arc::clone(&class.network_name),
                        serializer,
                    });
        }
        self.classes = Some(Arc::clone(classes.schema_id()));
        self.serializers = Some(Arc::clone(serializers.schema_id()));
    }

    pub(super) fn get<'a>(
        &'a self,
        class_id: i32,
        name: &Arc<str>,
        serializers: &'a SerializerContainer,
    ) -> Option<&'a Serializer> {
        let binding = usize::try_from(class_id)
            .ok()
            .and_then(|id| self.entries.get(id))
            .and_then(Option::as_ref);
        if let Some(binding) = binding
            && (Arc::ptr_eq(&binding.name, name) || binding.name == *name)
        {
            return Some(&binding.serializer);
        }
        // Manually inserted entities can have names or IDs outside ClassInfo.
        // Preserve the public API's name-based lookup and delayed missing-name error.
        serializers.get(name)
    }
}

#[cfg(test)]
mod tests;
