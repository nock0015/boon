use std::collections::{HashMap, hash_map::Entry};
use std::fmt::Write as _;

use rustc_hash::FxHashMap;
// Arc (not Rc) so that SerializerContainer is Send + Sync and can be
// shared across threads.  The serializer graph is immutable after
// construction, so atomic refcounting adds negligible overhead.
use std::sync::Arc;

use super::field_decoder::{self, DecodeProfile, FieldMetadata};
use super::field_path::FieldPath;
use crate::error::{Error, Result};
use crate::limits::DecodeLimits;

/// A flattened field mapped from a game protobuf.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FlattenedField {
    /// Index into [`FlattenedSerializer::symbols`] for the type string.
    pub var_type_sym: Option<i32>,
    /// Index into [`FlattenedSerializer::symbols`] for the field name.
    pub var_name_sym: Option<i32>,
    /// Bit width for quantized floats and QAngle.
    pub bit_count: Option<i32>,
    /// Low end of a quantized float range.
    pub low_value: Option<f32>,
    /// High end of a quantized float range.
    pub high_value: Option<f32>,
    /// Quantized float encoding flags.
    pub encode_flags: Option<i32>,
    /// Symbol index of the nested serializer, for composite fields.
    pub field_serializer_name_sym: Option<i32>,
    /// Symbol index of the dotted path prefix used for field-name lookup.
    pub send_node_sym: Option<i32>,
    /// Symbol index of an encoder hint such as `"coord"` or `"normal"`.
    pub var_encoder_sym: Option<i32>,
    /// Whether a pointer field may hold any of several concrete types.
    pub polymorphic: bool,
}

impl FlattenedField {
    /// Construct a flattened field with no quantization or nested serializer.
    pub const fn new(var_type_sym: Option<i32>, var_name_sym: Option<i32>) -> Self {
        Self {
            var_type_sym,
            var_name_sym,
            bit_count: None,
            low_value: None,
            high_value: None,
            encode_flags: None,
            field_serializer_name_sym: None,
            send_node_sym: None,
            var_encoder_sym: None,
            polymorphic: false,
        }
    }

    /// Set an optional encoded bit count.
    #[must_use]
    pub const fn with_bit_count(mut self, bit_count: Option<i32>) -> Self {
        self.bit_count = bit_count;
        self
    }

    /// Set optional quantized-float bounds.
    #[must_use]
    pub const fn with_range(mut self, low: Option<f32>, high: Option<f32>) -> Self {
        self.low_value = low;
        self.high_value = high;
        self
    }

    /// Set optional quantized-float encoding flags.
    #[must_use]
    pub const fn with_encode_flags(mut self, flags: Option<i32>) -> Self {
        self.encode_flags = flags;
        self
    }

    /// Set the optional nested serializer-name symbol.
    #[must_use]
    pub const fn with_serializer_name_sym(mut self, symbol: Option<i32>) -> Self {
        self.field_serializer_name_sym = symbol;
        self
    }

    /// Set the optional send-node symbol.
    #[must_use]
    pub const fn with_send_node_sym(mut self, symbol: Option<i32>) -> Self {
        self.send_node_sym = symbol;
        self
    }

    /// Set the optional field encoder symbol.
    #[must_use]
    pub const fn with_encoder_sym(mut self, symbol: Option<i32>) -> Self {
        self.var_encoder_sym = symbol;
        self
    }

    /// Mark a pointer field as polymorphic.
    #[must_use]
    pub const fn with_polymorphic(mut self, polymorphic: bool) -> Self {
        self.polymorphic = polymorphic;
        self
    }
}

/// A named serializer and its flattened-field indices.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FlattenedSerializerDefinition {
    /// Index into [`FlattenedSerializer::symbols`] for this serializer's name.
    pub serializer_name_sym: Option<i32>,
    /// Indices into [`FlattenedSerializer::fields`], in wire order.
    pub fields_index: Vec<i32>,
}

impl FlattenedSerializerDefinition {
    /// Construct a flattened serializer definition.
    pub const fn new(serializer_name_sym: Option<i32>, fields_index: Vec<i32>) -> Self {
        Self {
            serializer_name_sym,
            fields_index,
        }
    }
}

/// Game-independent form of CSVCMsg_FlattenedSerializer.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FlattenedSerializer {
    /// One definition per serializer, each naming its fields by index.
    pub serializers: Vec<FlattenedSerializerDefinition>,
    /// Shared string pool that every `*_sym` index refers to.
    pub symbols: Vec<String>,
    /// Field pool shared across all serializers in the message.
    pub fields: Vec<FlattenedField>,
}

impl FlattenedSerializer {
    /// Construct a neutral flattened serializer message.
    pub const fn new(
        serializers: Vec<FlattenedSerializerDefinition>,
        symbols: Vec<String>,
        fields: Vec<FlattenedField>,
    ) -> Self {
        Self {
            serializers,
            symbols,
            fields,
        }
    }
}

/// Parsed type information from a `var_type` string (e.g. `"CNetworkUtlVectorBase< int32 >"`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldType {
    /// The core type name (e.g. `"int32"`, `"CNetworkUtlVectorBase"`).
    pub base_type: String,
    /// `true` if the type ends with `*` (pointer / handle).
    pub pointer: bool,
    /// Inner type parameter for generics (e.g. `int32` inside `CNetworkUtlVectorBase<int32>`).
    pub generic_type: Option<Box<FieldType>>,
    /// Numeric array length for `Type[N]` syntax.
    pub array_length: Option<usize>,
    /// Symbolic array length for `Type[SYMBOL]` syntax (e.g. `MAX_ABILITY_DRAFT_ABILITIES`).
    pub count: Option<String>,
}

/// A single field within a serializer, describing one network property.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SerializerField {
    /// Source 2 type string (e.g. `"float32"`, `"CNetworkUtlVectorBase< int32 >"`).
    pub var_type: String,
    /// Field name (e.g. `"m_iHealth"`).
    pub var_name: String,
    /// Bit width hint for quantized floats and QAngle.
    pub bit_count: Option<i32>,
    /// Low end of quantized float range.
    pub low_value: Option<f32>,
    /// High end of quantized float range.
    pub high_value: Option<f32>,
    /// Quantized float encoding flags (see `QFE_*` constants).
    pub encode_flags: Option<i32>,
    /// Name of a nested serializer (for composite / array fields).
    pub field_serializer_name: Option<String>,
    /// Dotted path prefix used by the entity field name resolution.
    pub send_node: Option<String>,
    /// Optional encoder hint (e.g. `"coord"`, `"normal"`, `"fixed64"`).
    pub var_encoder: Option<String>,
    /// Resolved nested serializer (populated during [`SerializerContainer::parse`]).
    pub field_serializer: Option<Arc<Serializer>>,
    /// Parsed type information.
    pub field_type: FieldType,
    /// Resolved decoder and special descriptor.
    pub metadata: FieldMetadata,
}

impl SerializerField {
    /// Get a field from the nested serializer at the requested index.
    pub fn get_child(&self, index: usize) -> Option<&SerializerField> {
        self.field_serializer
            .as_ref()
            .and_then(|serializer| serializer.fields.get(index))
            .map(AsRef::as_ref)
    }

    /// Returns `true` if this field represents a variable-length array.
    pub fn is_dynamic_array(&self) -> bool {
        self.metadata.is_dynamic_array()
    }
}

/// A serializer: a named collection of fields describing an entity class.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Serializer {
    /// Serializer name, matching [`ClassEntry::table_name`](super::ClassEntry::table_name).
    pub name: String,
    /// Fields in wire order; a field path's first component indexes this list.
    pub fields: Vec<Arc<SerializerField>>,
}

impl Serializer {
    /// Resolve a dotted field name (e.g. "m_pGameRules.m_bGamePaused") to a packed u64 key.
    /// Walks the serializer hierarchy matching send_node + var_name against path components.
    /// Returns `None` when the name cannot fit in the seven-level packed path.
    pub fn resolve_field_key(&self, path: &str) -> Option<u64> {
        // A single component needs no temporary heap allocation.
        if !path.contains('.') {
            return self.resolve_parts(&[path], 0);
        }
        let parts: Vec<&str> = path.split('.').collect();
        self.resolve_parts(&parts, 0)
    }

    fn resolve_parts(&self, parts: &[&str], depth: usize) -> Option<u64> {
        // Text prefixes can span many components without adding a wire level.
        // Count serializer/array steps independently from those text components.
        if depth >= 7 || parts.is_empty() {
            return None;
        }

        for (field_idx, field) in self.fields.iter().enumerate() {
            // Compare borrowed components without allocating for each candidate.
            let mut field_parts = field
                .send_node
                .as_deref()
                .filter(|node| !node.is_empty())
                .into_iter()
                .flat_map(|node| node.split('.'))
                .chain((!field.var_name.is_empty()).then_some(field.var_name.as_str()));
            let mut remaining = parts.iter();
            if !field_parts.all(|part| remaining.next().is_some_and(|&next| next == part)) {
                continue;
            }

            let consumed = parts.len() - remaining.len();

            // If we've consumed all parts, this is the field
            if consumed == parts.len() {
                let mut fp = FieldPath::default();
                fp.data[0] = field_idx as u8;
                // last stays 0
                return Some(fp.pack());
            }

            // More parts remain — we need to recurse into a sub-serializer
            let next_part = parts[consumed];

            // Dynamic array: next part is a numeric index
            if field.is_dynamic_array() {
                if let Ok(array_idx) = next_part.parse::<usize>()
                    && let Some(ref fs) = field.field_serializer
                {
                    let inner_field = fs.fields.first()?;
                    let after_idx = consumed + 1;

                    if after_idx == parts.len() {
                        // The array element itself is the value
                        let mut fp = FieldPath::default();
                        fp.data[0] = field_idx as u8;
                        fp.data[1] = array_idx as u8;
                        fp.last = 1;
                        return Some(fp.pack());
                    }

                    // Recurse into the inner field's serializer
                    if let Some(ref inner_fs) = inner_field.field_serializer
                        && let Some(key) = inner_fs.resolve_parts(&parts[after_idx..], depth + 2)
                    {
                        let inner_fp = FieldPath::unpack(key);
                        let mut fp = FieldPath::default();
                        fp.data[0] = field_idx as u8;
                        fp.data[1] = array_idx as u8;
                        fp.last = 2 + inner_fp.last;
                        fp.data
                            .get_mut(2..=fp.last)?
                            .copy_from_slice(&inner_fp.data[..=inner_fp.last]);
                        return Some(fp.pack());
                    }
                }
                continue;
            }

            // Non-dynamic: recurse into field_serializer
            if let Some(ref fs) = field.field_serializer
                && let Some(key) = fs.resolve_parts(&parts[consumed..], depth + 1)
            {
                let inner_fp = FieldPath::unpack(key);
                let mut fp = FieldPath::default();
                fp.data[0] = field_idx as u8;
                fp.last = 1 + inner_fp.last;
                fp.data
                    .get_mut(1..=fp.last)?
                    .copy_from_slice(&inner_fp.data[..=inner_fp.last]);
                return Some(fp.pack());
            }
        }

        None
    }

    /// Convert a packed u64 key back to a dotted field name string.
    /// Walks the serializer hierarchy using the unpacked FieldPath.
    pub fn field_name_for_key(&self, key: u64) -> Option<String> {
        let fp = FieldPath::unpack(key);
        let indices = fp.data.get(..=fp.last)?;
        let mut name = String::new();
        let mut field = self.fields.get(usize::from(indices[0]))?;
        append_field_name(&mut name, field);

        for &index in &indices[1..] {
            let idx = usize::from(index);
            if field.is_dynamic_array() {
                write!(name, ".{idx}").expect("formatting an integer into a String cannot fail");
                if let Some(ref fs) = field.field_serializer {
                    field = fs.fields.first()?;
                } else {
                    break;
                }
            } else if let Some(ref fs) = field.field_serializer {
                field = fs.fields.get(idx)?;
                name.push('.');
                append_field_name(&mut name, field);
            } else {
                break;
            }
        }

        Some(name)
    }
}

fn append_field_name(output: &mut String, field: &SerializerField) {
    if let Some(node) = field.send_node.as_deref().filter(|node| !node.is_empty()) {
        output.push_str(node);
        output.push('.');
    }
    output.push_str(&field.var_name);
}

/// Container holding all parsed serializers, indexed by name.
#[derive(Clone, Default)]
pub struct SerializerContainer {
    schema_id: Arc<()>,
    // Name lookups serve the public API and cold class-to-serializer binding.
    // Entity updates normally reuse per-container class-ID bindings.
    serializers: FxHashMap<String, Arc<Serializer>>,
}

impl SerializerContainer {
    /// Build serializers from a game adapter's neutral data and decode profile.
    pub fn parse(msg: FlattenedSerializer, profile: DecodeProfile) -> Result<Self> {
        Self::parse_with_limits(msg, profile, &DecodeLimits::default())
    }

    /// Build serializers while enforcing explicit resource limits.
    pub fn parse_with_limits(
        msg: FlattenedSerializer,
        profile: DecodeProfile,
        limits: &DecodeLimits,
    ) -> Result<Self> {
        limits.ensure(
            "flattened serializers",
            msg.serializers.len(),
            limits.max_serializers(),
        )?;
        limits.ensure(
            "flattened serializer fields",
            msg.fields.len(),
            limits.max_serializer_fields(),
        )?;
        limits.ensure(
            "flattened serializer symbols",
            msg.symbols.len(),
            limits.max_symbols(),
        )?;
        for symbol in &msg.symbols {
            limits.ensure(
                "flattened serializer symbol bytes",
                symbol.len(),
                limits.max_field_string_bytes(),
            )?;
        }

        let symbols = &msg.symbols;
        let validate_symbol = |index: i32, context: &'static str| -> Result<()> {
            let index = usize::try_from(index).map_err(|_| Error::Parse {
                context: format!("negative {context} symbol index {index}"),
            })?;
            if index >= symbols.len() {
                return Err(Error::Parse {
                    context: format!(
                        "{context} symbol index {index} out of range ({} symbols)",
                        symbols.len()
                    ),
                });
            }
            Ok(())
        };
        for serializer in &msg.serializers {
            limits.ensure(
                "fields in one serializer",
                serializer.fields_index.len(),
                limits.max_serializer_fields(),
            )?;
            validate_symbol(
                serializer.serializer_name_sym.unwrap_or(0),
                "serializer name",
            )?;
        }
        for field in &msg.fields {
            for (index, context) in [
                (field.var_type_sym, "field type"),
                (field.var_name_sym, "field name"),
                (field.field_serializer_name_sym, "nested serializer"),
                (field.send_node_sym, "send node"),
                (field.var_encoder_sym, "field encoder"),
            ] {
                if let Some(index) = index {
                    validate_symbol(index, context)?;
                }
            }
        }

        let resolve_sym = |index: i32| -> &str {
            usize::try_from(index)
                .ok()
                .and_then(|index| symbols.get(index))
                .map(String::as_str)
                .unwrap_or("")
        };

        // Build fields and serializers
        let mut field_cache: HashMap<i32, Arc<SerializerField>> = HashMap::new();
        let mut serializer_map: FxHashMap<String, Arc<Serializer>> = FxHashMap::default();

        for serializer_proto in &msg.serializers {
            let ser_name = resolve_sym(serializer_proto.serializer_name_sym.unwrap_or(0));
            let mut serializer = Serializer {
                name: ser_name.to_string(),
                fields: Vec::with_capacity(serializer_proto.fields_index.len()),
            };

            for &field_index in &serializer_proto.fields_index {
                let entry = match field_cache.entry(field_index) {
                    Entry::Occupied(cached) => {
                        serializer.fields.push(Arc::clone(cached.get()));
                        continue;
                    }
                    Entry::Vacant(entry) => entry,
                };

                let field_index_usize = usize::try_from(field_index).map_err(|_| Error::Parse {
                    context: format!("negative serializer field index {field_index}"),
                })?;
                let field_proto =
                    msg.fields
                        .get(field_index_usize)
                        .ok_or_else(|| Error::Parse {
                            context: format!(
                                "serializer field index {field_index} out of range ({} fields)",
                                msg.fields.len()
                            ),
                        })?;

                let var_type = field_proto
                    .var_type_sym
                    .map(resolve_sym)
                    .unwrap_or("")
                    .to_string();
                let var_name = field_proto
                    .var_name_sym
                    .map(resolve_sym)
                    .unwrap_or("")
                    .to_string();
                let send_node = field_proto.send_node_sym.map(resolve_sym).map(String::from);
                let var_encoder = field_proto
                    .var_encoder_sym
                    .map(resolve_sym)
                    .map(String::from);
                let field_serializer_name = field_proto
                    .field_serializer_name_sym
                    .map(resolve_sym)
                    .map(String::from);

                let field_type = parse_type(&var_type);
                let mut metadata = field_decoder::get_field_metadata(
                    profile,
                    &var_type,
                    &var_name,
                    field_proto.bit_count,
                    field_proto.low_value,
                    field_proto.high_value,
                    field_proto.encode_flags,
                    var_encoder.as_deref(),
                    field_serializer_name.is_some(),
                );

                // Polymorphic fields (e.g. `m_pGameModeRules`) encode a presence
                // bool followed by a ubitvar selecting the concrete sub-type,
                // rather than the plain presence bool of a normal pointer.
                if field_proto.polymorphic {
                    metadata.decoder = field_decoder::Decoder::Poly;
                }

                // Resolve field serializer
                let field_serializer = match &metadata {
                    fm if fm.is_fixed_array() => {
                        let length = fm.fixed_array_length().unwrap_or(0);
                        limits.ensure(
                            "fixed serializer array elements",
                            length,
                            limits.max_fixed_array_length(),
                        )?;
                        // Build a pseudo-serializer containing `length` copies of the inner field
                        let inner_ser = field_serializer_name
                            .as_deref()
                            .and_then(|n| serializer_map.get(n).cloned());

                        let inner_field = SerializerField {
                            var_type: var_type.clone(),
                            var_name: var_name.clone(),
                            bit_count: field_proto.bit_count,
                            low_value: field_proto.low_value,
                            high_value: field_proto.high_value,
                            encode_flags: field_proto.encode_flags,
                            field_serializer_name: field_serializer_name.clone(),
                            send_node: send_node.clone(),
                            var_encoder: var_encoder.clone(),
                            field_serializer: inner_ser,
                            field_type: field_type.clone(),
                            metadata: metadata.clone(),
                        };
                        let inner_rc = Arc::new(inner_field);
                        let mut fields = Vec::with_capacity(length);
                        fields.resize(length, inner_rc);
                        Some(Arc::new(Serializer {
                            name: String::new(),
                            fields,
                        }))
                    }
                    fm if fm.is_dynamic_array() => {
                        let field_serializer = if fm.is_dynamic_serializer_array() {
                            field_serializer_name
                                .as_deref()
                                .and_then(|name| serializer_map.get(name).cloned())
                        } else {
                            None
                        };
                        let inner = Arc::new(SerializerField {
                            var_type: String::new(),
                            var_name: String::new(),
                            bit_count: None,
                            low_value: None,
                            high_value: None,
                            encode_flags: None,
                            field_serializer_name: None,
                            send_node: None,
                            var_encoder: None,
                            field_serializer,
                            field_type: parse_type(""),
                            metadata: fm.dynamic_array_inner_metadata(),
                        });
                        Some(Arc::new(Serializer {
                            name: String::new(),
                            fields: vec![inner],
                        }))
                    }
                    _ => field_serializer_name
                        .as_deref()
                        .and_then(|n| serializer_map.get(n).cloned()),
                };

                let field = Arc::new(SerializerField {
                    var_type,
                    var_name,
                    bit_count: field_proto.bit_count,
                    low_value: field_proto.low_value,
                    high_value: field_proto.high_value,
                    encode_flags: field_proto.encode_flags,
                    field_serializer_name,
                    send_node,
                    var_encoder,
                    field_serializer,
                    field_type,
                    metadata,
                });

                entry.insert(Arc::clone(&field));
                serializer.fields.push(field);
            }

            serializer_map.insert(serializer.name.clone(), Arc::new(serializer));
        }

        Ok(Self {
            schema_id: Arc::new(()),
            serializers: serializer_map,
        })
    }

    pub(crate) fn schema_id(&self) -> &Arc<()> {
        &self.schema_id
    }

    pub(crate) fn shared(&self, name: &str) -> Option<Arc<Serializer>> {
        self.serializers.get(name).cloned()
    }

    /// Look up a serializer by class network name.
    pub fn get(&self, name: &str) -> Option<&Serializer> {
        self.serializers.get(name).map(|arc| arc.as_ref())
    }

    /// Iterate serializer names and definitions in unspecified map order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Serializer)> {
        self.serializers
            .iter()
            .map(|(name, serializer)| (name.as_str(), serializer.as_ref()))
    }

    /// Number of parsed serializers.
    pub fn len(&self) -> usize {
        self.serializers.len()
    }

    /// Whether the container has no serializers.
    pub fn is_empty(&self) -> bool {
        self.serializers.is_empty()
    }
}

/// Parse a var_type string into a FieldType.
pub fn parse_type(s: &str) -> FieldType {
    let s = s.trim();

    // Check for pointer
    if let Some(stripped) = s.strip_suffix('*') {
        return FieldType {
            base_type: stripped.trim().to_string(),
            pointer: true,
            generic_type: None,
            array_length: None,
            count: None,
        };
    }

    // Check for array: type[length]
    if let Some(bracket_pos) = s.find('[')
        && s.ends_with(']')
    {
        let base = s[..bracket_pos].trim();
        let len_str = s[bracket_pos + 1..s.len() - 1].trim();
        let array_length = len_str.parse::<usize>().ok();
        let count = if array_length.is_none() {
            Some(len_str.to_string())
        } else {
            None
        };
        return FieldType {
            base_type: base.to_string(),
            pointer: false,
            generic_type: None,
            array_length,
            count,
        };
    }

    // Check for generic: Type< InnerType >
    if let Some(angle_pos) = s.find('<')
        && let Some(close_pos) = s.rfind('>')
    {
        let base = s[..angle_pos].trim();
        let inner = s[angle_pos + 1..close_pos].trim();
        return FieldType {
            base_type: base.to_string(),
            pointer: false,
            generic_type: Some(Box::new(parse_type(inner))),
            array_length: None,
            count: None,
        };
    }

    // Simple type
    FieldType {
        base_type: s.to_string(),
        pointer: false,
        generic_type: None,
        array_length: None,
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::field_decoder::{Decoder, FieldMetadata};
    use super::*;

    // ── parse_type ──

    #[test]
    fn parse_type_simple() {
        let ft = parse_type("int32");
        assert_eq!(ft.base_type, "int32");
        assert!(!ft.pointer);
        assert!(ft.generic_type.is_none());
        assert!(ft.array_length.is_none());
        assert!(ft.count.is_none());
    }

    #[test]
    fn parse_type_pointer() {
        let ft = parse_type("CBaseEntity*");
        assert_eq!(ft.base_type, "CBaseEntity");
        assert!(ft.pointer);
    }

    #[test]
    fn parse_type_array_numeric() {
        let ft = parse_type("int32[4]");
        assert_eq!(ft.base_type, "int32");
        assert_eq!(ft.array_length, Some(4));
        assert!(ft.count.is_none());
    }

    #[test]
    fn parse_type_array_symbolic() {
        let ft = parse_type("int32[MAX_ABILITIES]");
        assert_eq!(ft.base_type, "int32");
        assert!(ft.array_length.is_none());
        assert_eq!(ft.count.as_deref(), Some("MAX_ABILITIES"));
    }

    #[test]
    fn parse_type_generic() {
        let ft = parse_type("CNetworkUtlVectorBase< int32 >");
        assert_eq!(ft.base_type, "CNetworkUtlVectorBase");
        let inner = ft.generic_type.as_ref().unwrap();
        assert_eq!(inner.base_type, "int32");
    }

    #[test]
    fn parse_type_generic_nested() {
        let ft = parse_type("CHandle< CBaseEntity >");
        assert_eq!(ft.base_type, "CHandle");
        assert_eq!(ft.generic_type.as_ref().unwrap().base_type, "CBaseEntity");
    }

    #[test]
    fn parse_type_whitespace_trimming() {
        let ft = parse_type("  float32  ");
        assert_eq!(ft.base_type, "float32");
    }

    #[test]
    fn parse_type_empty_string() {
        let ft = parse_type("");
        assert_eq!(ft.base_type, "");
        assert!(!ft.pointer);
    }

    #[test]
    fn parse_type_complex_all_fields() {
        let ft = parse_type("uint32[16]");
        assert_eq!(ft.base_type, "uint32");
        assert!(!ft.pointer);
        assert!(ft.generic_type.is_none());
        assert_eq!(ft.array_length, Some(16));
        assert!(ft.count.is_none());
    }

    // ── Serializer key resolution ──

    fn make_field(name: &str, send_node: Option<&str>) -> Arc<SerializerField> {
        Arc::new(SerializerField {
            var_type: String::new(),
            var_name: name.to_string(),
            bit_count: None,
            low_value: None,
            high_value: None,
            encode_flags: None,
            field_serializer_name: None,
            send_node: send_node.map(String::from),
            var_encoder: None,
            field_serializer: None,
            field_type: parse_type(""),
            metadata: FieldMetadata {
                decoder: Decoder::U64,
                special: None,
            },
        })
    }

    #[test]
    fn resolve_field_key_found() {
        let ser = Serializer {
            name: "test".to_string(),
            fields: vec![make_field("m_iHealth", None)],
        };
        assert!(ser.resolve_field_key("m_iHealth").is_some());
    }

    #[test]
    fn resolve_field_key_not_found() {
        let ser = Serializer {
            name: "test".to_string(),
            fields: vec![make_field("m_iHealth", None)],
        };
        assert!(ser.resolve_field_key("m_iMana").is_none());
    }

    #[test]
    fn field_name_roundtrip() {
        let ser = Serializer {
            name: "test".to_string(),
            fields: vec![make_field("m_iHealth", None), make_field("m_iMana", None)],
        };
        let key = ser.resolve_field_key("m_iMana").unwrap();
        let name = ser.field_name_for_key(key).unwrap();
        assert_eq!(name, "m_iMana");
    }

    #[test]
    fn resolve_with_send_node() {
        let ser = Serializer {
            name: "test".to_string(),
            fields: vec![make_field("m_bPaused", Some("m_pGameRules"))],
        };
        let key = ser.resolve_field_key("m_pGameRules.m_bPaused");
        assert!(key.is_some());
        let name = ser.field_name_for_key(key.unwrap()).unwrap();
        assert_eq!(name, "m_pGameRules.m_bPaused");
    }

    #[test]
    fn nested_names_match_complete_components_and_preserve_empty_names() {
        let mut parent = make_field("parent", Some("root.node"));
        Arc::make_mut(&mut parent).field_serializer = Some(Arc::new(Serializer {
            name: "nested".into(),
            fields: vec![
                make_field("value", Some("child.node")),
                make_field("", Some("empty")),
            ],
        }));
        let serializer = Serializer {
            name: "test".into(),
            fields: vec![make_field("parent_extra", Some("root.node")), parent],
        };
        for name in [
            "root.node.parent.child.node.value",
            "root.node.parent.empty",
        ] {
            let key = serializer.resolve_field_key(name).unwrap();
            let expected = if name.ends_with("empty") {
                format!("{name}.")
            } else {
                name.into()
            };
            assert_eq!(serializer.field_name_for_key(key).unwrap(), expected);
        }
        assert!(
            serializer
                .resolve_field_key("root.node.par.child.node.value")
                .is_none()
        );
        assert!(
            serializer
                .resolve_field_key("root.node.parent.child")
                .is_none()
        );
    }

    #[test]
    fn dynamic_array_names_include_index_and_nested_send_node() {
        let mut element = make_field("", None);
        Arc::make_mut(&mut element).field_serializer = Some(Arc::new(Serializer {
            name: "element".into(),
            fields: vec![make_field("value", Some("node"))],
        }));
        let mut array = make_field("items", None);
        let field = Arc::make_mut(&mut array);
        field.metadata.special =
            Some(field_decoder::FieldSpecialDescriptor::DynamicSerializerArray);
        field.field_serializer = Some(Arc::new(Serializer {
            name: String::new(),
            fields: vec![element],
        }));
        let serializer = Serializer {
            name: "test".into(),
            fields: vec![array],
        };
        for name in ["items.17", "items.17.node.value"] {
            let key = serializer.resolve_field_key(name).unwrap();
            assert_eq!(serializer.field_name_for_key(key).unwrap(), name);
        }
        assert!(
            serializer
                .resolve_field_key("items.invalid.node.value")
                .is_none()
        );
    }

    #[test]
    fn lookup_preserves_empty_components_and_observes_schema_edits() {
        let mut serializer = Serializer {
            name: "test".into(),
            fields: vec![
                make_field("value", Some("root..branch.")),
                make_field("literal.dot", None),
                make_field("plain", Some("")),
            ],
        };
        let key = serializer.resolve_field_key("root..branch..value").unwrap();
        assert_eq!(FieldPath::unpack(key).get(0), 0);
        assert!(serializer.resolve_field_key("root.branch.value").is_none());
        assert!(serializer.resolve_field_key("literal.dot").is_none());
        let plain = serializer.resolve_field_key("plain").unwrap();
        Arc::make_mut(&mut serializer.fields[2]).var_name = "renamed".into();
        assert!(serializer.resolve_field_key("plain").is_none());
        assert_eq!(serializer.resolve_field_key("renamed"), Some(plain));
        assert!(serializer.resolve_field_key("renamed.").is_none());
    }

    proptest::proptest! {
        #[test]
        fn lookup_matches_a_component_model(
            descriptions in proptest::collection::vec(
                ("[abc.]{0,6}", proptest::option::of("[abc.]{0,6}")), 0..32
            ),
            query in "[abc.]{0,16}",
            pick in proptest::prelude::any::<usize>(),
            use_field in proptest::prelude::any::<bool>(),
        ) {
            let components = |name: &str, node: Option<&str>| {
                let mut result = Vec::new();
                if let Some(node) = node.filter(|node| !node.is_empty()) {
                    result.extend(node.split('.').map(str::to_owned));
                }
                if !name.is_empty() { result.push(name.to_owned()); }
                result
            };
            let query = if use_field && !descriptions.is_empty() {
                let (name, node) = &descriptions[pick % descriptions.len()];
                components(name, node.as_deref()).join(".")
            } else { query };
            let query_parts: Vec<_> = query.split('.').collect();
            let expected = descriptions.iter().position(|(name, node)| {
                components(name, node.as_deref()) == query_parts
            }).map(|index| {
                let mut path = FieldPath::default();
                path.data[0] = index as u8;
                path.pack()
            });
            let serializer = Serializer {
                name: "generated".into(),
                fields: descriptions.iter().map(|(name, node)| {
                    make_field(name, node.as_deref())
                }).collect(),
            };
            proptest::prop_assert_eq!(serializer.resolve_field_key(&query), expected);
        }
    }

    #[test]
    fn duplicate_serializer_names_keep_the_last_definition() {
        let serializers = SerializerContainer::parse(
            FlattenedSerializer::new(
                vec![
                    FlattenedSerializerDefinition::new(Some(0), vec![0]),
                    FlattenedSerializerDefinition::new(Some(0), vec![1]),
                ],
                vec![
                    "CTest".into(),
                    "uint32".into(),
                    "m_first".into(),
                    "m_second".into(),
                ],
                vec![
                    FlattenedField::new(Some(1), Some(2)),
                    FlattenedField::new(Some(1), Some(3)),
                ],
            ),
            DecodeProfile::new(
                super::super::field_decoder::BareCharEncoding::UnsignedVarint,
                super::super::field_decoder::PreciseQAngleMode::Raw,
            ),
        )
        .expect("valid duplicate serializer definitions");

        let serializer = serializers.get("CTest").expect("serializer exists");
        assert_eq!(serializer.fields.len(), 1);
        assert_eq!(serializer.fields[0].var_name, "m_second");
    }

    #[test]
    fn nested_names_preserve_prefixes_arrays_and_empty_components() {
        use super::super::field_decoder::FieldSpecialDescriptor;

        let leaf = Arc::new(Serializer {
            name: "leaf".into(),
            fields: vec![make_field("value", Some("stats.current"))],
        });
        let mut nested = make_field("child", Some("outer.node"));
        Arc::make_mut(&mut nested).field_serializer = Some(Arc::clone(&leaf));
        let mut array = make_field("items", Some("outer.node"));
        let mut inner = make_field("", None);
        Arc::make_mut(&mut inner).field_serializer = Some(leaf);
        let array_field = Arc::make_mut(&mut array);
        array_field.metadata.special = Some(FieldSpecialDescriptor::DynamicSerializerArray);
        array_field.field_serializer = Some(Arc::new(Serializer {
            name: String::new(),
            fields: vec![inner],
        }));
        let serializer = Serializer {
            name: "root".into(),
            fields: vec![
                nested,
                array,
                make_field("value", Some(".prefix.")),
                make_field("empty", Some("")),
            ],
        };

        for (name, expected) in [
            ("outer.node.child", vec![0]),
            ("outer.node.child.stats.current.value", vec![0, 0]),
            ("outer.node.items", vec![1]),
            ("outer.node.items.12", vec![1, 12]),
            ("outer.node.items.12.stats.current.value", vec![1, 12, 0]),
            (".prefix..value", vec![2]),
            ("empty", vec![3]),
        ] {
            let key = serializer.resolve_field_key(name).expect(name);
            let path = FieldPath::unpack(key);
            assert_eq!(&path.data[..=path.last], expected, "{name}");
            assert_eq!(serializer.field_name_for_key(key).as_deref(), Some(name));
        }
        for name in [
            "",
            "outer",
            "outer.node",
            "outer.node.child.stats",
            "outer.node.child.stats.current.value.extra",
            "outer.node.items.no_index",
            "outer.node.items.12.",
            "outer.node.items.12.missing",
        ] {
            assert_eq!(serializer.resolve_field_key(name), None, "{name}");
        }
    }

    #[test]
    fn reused_flattened_fields_share_the_cached_definition() {
        let serializers = SerializerContainer::parse(
            FlattenedSerializer::new(
                vec![
                    FlattenedSerializerDefinition::new(Some(0), vec![0, 0]),
                    FlattenedSerializerDefinition::new(Some(1), vec![0]),
                ],
                ["CFirst", "CSecond", "int32", "value"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                vec![FlattenedField::new(Some(2), Some(3))],
            ),
            DecodeProfile::new(
                super::super::field_decoder::BareCharEncoding::UnsignedVarint,
                super::super::field_decoder::PreciseQAngleMode::Raw,
            ),
        )
        .unwrap();
        let first = serializers.get("CFirst").unwrap();
        let second = serializers.get("CSecond").unwrap();
        assert!(Arc::ptr_eq(&first.fields[0], &first.fields[1]));
        assert!(Arc::ptr_eq(&first.fields[0], &second.fields[0]));
    }

    #[test]
    fn empty_dynamic_arrays_and_invalid_packed_depths_return_none() {
        use super::super::field_decoder::FieldSpecialDescriptor;

        let mut array = make_field("items", None);
        let field = Arc::make_mut(&mut array);
        field.metadata.special = Some(FieldSpecialDescriptor::DynamicSerializerArray);
        field.field_serializer = Some(Arc::new(Serializer {
            name: String::new(),
            fields: Vec::new(),
        }));
        let serializer = Serializer {
            name: "root".into(),
            fields: vec![array],
        };
        assert!(serializer.resolve_field_key("items").is_some());
        for name in ["items.0", "items.0.value"] {
            assert_eq!(serializer.resolve_field_key(name), None);
        }
        for last in [1, 7, 8, 255] {
            let key = FieldPath {
                data: [0; 7],
                last,
                finished: false,
            }
            .pack();
            assert_eq!(serializer.field_name_for_key(key), None);
        }
    }

    #[test]
    fn array_steps_not_name_components_determine_the_depth_limit() {
        use super::super::field_decoder::FieldSpecialDescriptor;

        let mut serializer = Serializer {
            name: "leaf".into(),
            fields: vec![make_field("value", Some("a.b.c.d.e.f.g.h"))],
        };
        let mut name = String::from("a.b.c.d.e.f.g.h.value");
        for depth in 0..=4 {
            let key = serializer.resolve_field_key(&name);
            if depth <= 3 {
                let key = key.expect("up to seven wire components fit");
                assert_eq!(FieldPath::unpack(key).last, depth * 2);
                assert_eq!(
                    serializer.field_name_for_key(key).as_deref(),
                    Some(name.as_str())
                );
            } else {
                assert_eq!(key, None);
            }
            let mut inner = make_field("", None);
            Arc::make_mut(&mut inner).field_serializer = Some(Arc::new(serializer));
            let mut array = make_field("items", None);
            let field = Arc::make_mut(&mut array);
            field.metadata.special = Some(FieldSpecialDescriptor::DynamicSerializerArray);
            field.field_serializer = Some(Arc::new(Serializer {
                name: String::new(),
                fields: vec![inner],
            }));
            serializer = Serializer {
                name: "parent".into(),
                fields: vec![array],
            };
            name.insert_str(0, "items.3.");
        }
    }

    #[test]
    fn name_resolution_checks_the_packed_path_depth() {
        let mut serializer = Serializer {
            name: "leaf".into(),
            fields: vec![make_field("value", None)],
        };
        let mut path = String::from("value");
        for depth in 1..=32 {
            if depth <= 7 {
                let key = serializer
                    .resolve_field_key(&path)
                    .expect("representable path");
                assert_eq!(FieldPath::unpack(key).last, depth - 1);
                assert_eq!(
                    serializer.field_name_for_key(key).as_deref(),
                    Some(path.as_str())
                );
            } else {
                assert_eq!(serializer.resolve_field_key(&path), None);
            }
            let mut parent = make_field("child", None);
            Arc::make_mut(&mut parent).field_serializer = Some(Arc::new(serializer));
            serializer = Serializer {
                name: "parent".into(),
                fields: vec![parent],
            };
            path.insert_str(0, "child.");
        }
        let invalid_key = FieldPath {
            data: [0; 7],
            last: 7,
            finished: false,
        }
        .pack();
        assert_eq!(serializer.field_name_for_key(invalid_key), None);
    }
}
