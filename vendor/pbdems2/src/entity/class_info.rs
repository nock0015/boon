use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::error::{Error, Result};
use crate::limits::DecodeLimits;

/// A single network entity class entry.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ClassEntry {
    /// Numeric class ID as it appears in packet-entities updates.
    pub class_id: i32,
    /// Networked class name (e.g. `"CCSPlayerPawn"`).
    ///
    /// Shared rather than owned: every entity create copies this name, so an
    /// `Arc<str>` makes that a refcount bump instead of a heap allocation.
    pub network_name: Arc<str>,
    /// Name of the serializer that decodes this class's fields.
    pub table_name: String,
}

impl ClassEntry {
    /// Construct a neutral class entry from a game protobuf adapter.
    pub fn new(
        class_id: i32,
        network_name: impl Into<Arc<str>>,
        table_name: impl Into<String>,
    ) -> Self {
        Self {
            class_id,
            network_name: network_name.into(),
            table_name: table_name.into(),
        }
    }
}

/// Maps Source 2 class IDs to their network serializer names.
#[derive(Debug, Clone)]
pub struct ClassInfo {
    // Clones describe the same immutable schema. Holding this token prevents
    // address reuse from making a replacement table look like the old one.
    schema_id: Arc<()>,
    classes: Vec<ClassEntry>,
    bits: usize,
    lookup: Vec<Option<usize>>,
    name_lookup: FxHashMap<Arc<str>, i32>,
}

impl ClassInfo {
    /// Create an empty class map.
    pub fn empty() -> Self {
        Self {
            schema_id: Arc::new(()),
            classes: Vec::new(),
            bits: 1,
            lookup: Vec::new(),
            name_lookup: FxHashMap::default(),
        }
    }

    /// Validate and build class information with default limits.
    pub fn try_from_entries(classes: impl IntoIterator<Item = ClassEntry>) -> Result<Self> {
        Self::try_from_entries_with_limits(classes, &DecodeLimits::default())
    }

    /// Validate and build class information with explicit limits.
    pub fn try_from_entries_with_limits(
        classes: impl IntoIterator<Item = ClassEntry>,
        limits: &DecodeLimits,
    ) -> Result<Self> {
        let classes: Vec<_> = classes.into_iter().collect();
        limits.ensure("network classes", classes.len(), limits.max_classes())?;

        let mut max_id = 0_usize;
        let mut name_lookup = FxHashMap::default();
        let mut seen_ids = FxHashMap::default();
        for (index, class) in classes.iter().enumerate() {
            let class_id = usize::try_from(class.class_id).map_err(|_| Error::Parse {
                context: format!("negative network class ID {}", class.class_id),
            })?;
            limits.ensure("network class ID", class_id, limits.max_class_id())?;
            if let Some(previous) = seen_ids.insert(class_id, index) {
                return Err(Error::Parse {
                    context: format!(
                        "duplicate network class ID {class_id} at indices {previous} and {index}"
                    ),
                });
            }
            if let Some(previous) = name_lookup.insert(class.network_name.clone(), class.class_id) {
                return Err(Error::Parse {
                    context: format!(
                        "duplicate network class name '{}' for IDs {previous} and {}",
                        class.network_name, class.class_id
                    ),
                });
            }
            max_id = max_id.max(class_id);
        }

        let bits = if max_id == 0 {
            1
        } else {
            (usize::BITS - max_id.leading_zeros()) as usize
        };
        let mut lookup = if classes.is_empty() {
            Vec::new()
        } else {
            vec![None; max_id + 1]
        };
        for (index, class) in classes.iter().enumerate() {
            lookup[class.class_id as usize] = Some(index);
        }

        Ok(Self {
            schema_id: Arc::new(()),
            classes,
            bits,
            lookup,
            name_lookup,
        })
    }

    pub(crate) fn schema_id(&self) -> &Arc<()> {
        &self.schema_id
    }

    /// All class entries in adapter-provided order.
    pub fn classes(&self) -> &[ClassEntry] {
        &self.classes
    }

    /// Number of bits needed to decode a numeric class ID.
    pub const fn bits(&self) -> usize {
        self.bits
    }

    /// Number of registered classes.
    pub fn len(&self) -> usize {
        self.classes.len()
    }

    /// Whether no classes are registered.
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// Look up a class entry by numeric ID in constant time.
    pub fn by_id(&self, class_id: i32) -> Option<&ClassEntry> {
        let class_id = usize::try_from(class_id).ok()?;
        let index = self.lookup.get(class_id)?.as_ref()?;
        self.classes.get(*index)
    }

    /// Get the network name for a numeric class ID.
    pub fn name_by_id(&self, class_id: i32) -> Option<&str> {
        self.by_id(class_id)
            .map(|class| class.network_name.as_ref())
    }

    /// Find a numeric class ID by network name in constant time.
    pub fn id_of(&self, network_name: &str) -> Option<i32> {
        self.name_lookup.get(network_name).copied()
    }
}

impl Default for ClassInfo {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_info(ids: &[(i32, &str)]) -> Result<ClassInfo> {
        ClassInfo::try_from_entries(
            ids.iter()
                .map(|(class_id, network_name)| ClassEntry::new(*class_id, *network_name, "")),
        )
    }

    #[test]
    fn computes_class_id_width() {
        assert_eq!(class_info(&[]).expect("valid").bits(), 1);
        assert_eq!(class_info(&[(0, "A")]).expect("valid").bits(), 1);
        assert_eq!(class_info(&[(10, "B")]).expect("valid").bits(), 4);
        assert_eq!(class_info(&[(255, "C")]).expect("valid").bits(), 8);
    }

    #[test]
    fn looks_up_sparse_ids_and_names() {
        let info = class_info(&[(5, "Hero"), (10, "Creep")]).expect("valid");
        assert_eq!(info.name_by_id(10), Some("Creep"));
        assert_eq!(info.name_by_id(6), None);
        assert_eq!(info.id_of("Hero"), Some(5));
    }

    #[test]
    fn rejects_negative_ids() {
        assert!(class_info(&[(-1, "Invalid"), (2, "Valid")]).is_err());
    }

    #[test]
    fn rejects_duplicate_ids_and_names() {
        assert!(class_info(&[(1, "A"), (1, "B")]).is_err());
        assert!(class_info(&[(1, "A"), (2, "A")]).is_err());
    }

    #[test]
    fn applies_configured_class_id_limit() {
        let limits = DecodeLimits::default().with_class_limits(8, 3);
        let result =
            ClassInfo::try_from_entries_with_limits([ClassEntry::new(4, "TooLarge", "")], &limits);
        assert!(matches!(result, Err(Error::LimitExceeded { .. })));
    }
}
