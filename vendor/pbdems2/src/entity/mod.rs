//! Entity system for parsing and representing game state.
//!
//! This module handles everything related to entities in the demo:
//! - Entity lifecycle (create, update, delete)
//! - Serializers and field definitions
//! - Class information
//! - Field value decoding
//! - String tables

mod class_info;
mod entities;
mod field_decoder;
pub mod field_path;
mod field_value;
mod quantized_float;
mod serializer_bindings;
mod serializers;
mod string_tables;

pub use class_info::{ClassEntry, ClassInfo};
pub use entities::{
    ENTITY_HANDLE_INDEX_MASK, Entity, EntityChange, EntityChangeKind, EntityContainer, EntityId,
    INVALID_ENTITY_HANDLE, PacketEntities, protobuf_handle_index,
};
pub use field_decoder::{
    BareCharEncoding, DecodeProfile, Decoder, FieldDecodeContext, FieldMetadata,
    FieldSpecialDescriptor, PreciseQAngleMode, get_field_metadata,
};
pub use field_value::FieldValue;
pub use quantized_float::QuantizedFloat;
pub use serializers::{
    FieldType, FlattenedField, FlattenedSerializer, FlattenedSerializerDefinition, Serializer,
    SerializerContainer, SerializerField, parse_type,
};
pub use string_tables::{
    CreateStringTable, INSTANCE_BASELINE_TABLE_NAME, StringTable, StringTableChange,
    StringTableChangeKind, StringTableContainer, StringTableEntry, UpdateStringTable,
};
