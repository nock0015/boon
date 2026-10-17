//! Resource limits used while decoding untrusted demo data.

use crate::error::{Error, Result};

/// Conservative upper bounds for allocations and repeated decode work.
///
/// The defaults are intentionally much larger than ordinary Source 2 data,
/// while still preventing a corrupt length field from requesting effectively
/// unbounded memory. Callers parsing known-large captures can raise individual
/// limits with the builder methods.
///
/// # Example
///
/// ```
/// use pbdems2::DecodeLimits;
///
/// let limits = DecodeLimits::default().with_max_command_body_bytes(4 * 1024 * 1024);
/// assert_eq!(limits.max_command_body_bytes(), 4 * 1024 * 1024);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeLimits {
    max_command_body_bytes: usize,
    max_decompressed_command_bytes: usize,
    max_packet_message_bytes: usize,
    max_string_table_data_bytes: usize,
    max_decompressed_string_table_bytes: usize,
    max_string_table_entries: usize,
    max_string_table_user_data_bytes: usize,
    max_field_string_bytes: usize,
    max_classes: usize,
    max_class_id: usize,
    max_serializers: usize,
    max_serializer_fields: usize,
    max_symbols: usize,
    max_fixed_array_length: usize,
    max_packet_entity_updates: usize,
    max_field_paths: usize,
}

impl DecodeLimits {
    /// Maximum encoded bytes in one outer demo command.
    pub const fn max_command_body_bytes(&self) -> usize {
        self.max_command_body_bytes
    }

    /// Maximum bytes after decompressing one outer demo command.
    pub const fn max_decompressed_command_bytes(&self) -> usize {
        self.max_decompressed_command_bytes
    }

    /// Maximum bytes in one inner packet message.
    pub const fn max_packet_message_bytes(&self) -> usize {
        self.max_packet_message_bytes
    }

    /// Maximum encoded bytes carried by one string-table message.
    pub const fn max_string_table_data_bytes(&self) -> usize {
        self.max_string_table_data_bytes
    }

    /// Maximum bytes after decompressing string-table data.
    pub const fn max_decompressed_string_table_bytes(&self) -> usize {
        self.max_decompressed_string_table_bytes
    }

    /// Maximum number of entries in one string table.
    pub const fn max_string_table_entries(&self) -> usize {
        self.max_string_table_entries
    }

    /// Maximum decompressed user-data bytes in one string-table entry.
    pub const fn max_string_table_user_data_bytes(&self) -> usize {
        self.max_string_table_user_data_bytes
    }

    /// Maximum bytes in a decoded entity string or binary block.
    pub const fn max_field_string_bytes(&self) -> usize {
        self.max_field_string_bytes
    }

    /// Maximum number of network classes.
    pub const fn max_classes(&self) -> usize {
        self.max_classes
    }

    /// Maximum accepted numeric class identifier.
    pub const fn max_class_id(&self) -> usize {
        self.max_class_id
    }

    /// Maximum number of flattened serializers.
    pub const fn max_serializers(&self) -> usize {
        self.max_serializers
    }

    /// Maximum total number of flattened fields.
    pub const fn max_serializer_fields(&self) -> usize {
        self.max_serializer_fields
    }

    /// Maximum number of flattened-serializer symbols.
    pub const fn max_symbols(&self) -> usize {
        self.max_symbols
    }

    /// Maximum element count accepted from a fixed-array type declaration.
    pub const fn max_fixed_array_length(&self) -> usize {
        self.max_fixed_array_length
    }

    /// Maximum entity deltas in one packet-entities message.
    pub const fn max_packet_entity_updates(&self) -> usize {
        self.max_packet_entity_updates
    }

    /// Maximum field paths in one entity update.
    pub const fn max_field_paths(&self) -> usize {
        self.max_field_paths
    }

    /// Set the encoded outer-command body limit.
    #[must_use]
    pub const fn with_max_command_body_bytes(mut self, value: usize) -> Self {
        self.max_command_body_bytes = value;
        self
    }

    /// Set the decompressed outer-command body limit.
    #[must_use]
    pub const fn with_max_decompressed_command_bytes(mut self, value: usize) -> Self {
        self.max_decompressed_command_bytes = value;
        self
    }

    /// Set the inner packet-message size limit.
    #[must_use]
    pub const fn with_max_packet_message_bytes(mut self, value: usize) -> Self {
        self.max_packet_message_bytes = value;
        self
    }

    /// Set both encoded and decompressed string-table data limits.
    #[must_use]
    pub const fn with_max_string_table_bytes(
        mut self,
        encoded: usize,
        decompressed: usize,
    ) -> Self {
        self.max_string_table_data_bytes = encoded;
        self.max_decompressed_string_table_bytes = decompressed;
        self
    }

    /// Set the maximum entry count for one string table.
    #[must_use]
    pub const fn with_max_string_table_entries(mut self, value: usize) -> Self {
        self.max_string_table_entries = value;
        self
    }

    /// Set the maximum decompressed user-data size for one table entry.
    #[must_use]
    pub const fn with_max_string_table_user_data_bytes(mut self, value: usize) -> Self {
        self.max_string_table_user_data_bytes = value;
        self
    }

    /// Set the decoded entity string and binary-block limit.
    #[must_use]
    pub const fn with_max_field_string_bytes(mut self, value: usize) -> Self {
        self.max_field_string_bytes = value;
        self
    }

    /// Set network-class count and numeric-ID limits.
    #[must_use]
    pub const fn with_class_limits(mut self, count: usize, max_id: usize) -> Self {
        self.max_classes = count;
        self.max_class_id = max_id;
        self
    }

    /// Set flattened serializer, field, and symbol count limits.
    #[must_use]
    pub const fn with_serializer_limits(
        mut self,
        serializers: usize,
        fields: usize,
        symbols: usize,
    ) -> Self {
        self.max_serializers = serializers;
        self.max_serializer_fields = fields;
        self.max_symbols = symbols;
        self
    }

    /// Set the maximum fixed-array element count.
    #[must_use]
    pub const fn with_max_fixed_array_length(mut self, value: usize) -> Self {
        self.max_fixed_array_length = value;
        self
    }

    /// Set the packet-entity update limit.
    #[must_use]
    pub const fn with_max_packet_entity_updates(mut self, value: usize) -> Self {
        self.max_packet_entity_updates = value;
        self
    }

    /// Set the field-path count limit for one entity update.
    #[must_use]
    pub const fn with_max_field_paths(mut self, value: usize) -> Self {
        self.max_field_paths = value;
        self
    }

    pub(crate) fn ensure(&self, resource: &'static str, actual: usize, limit: usize) -> Result<()> {
        if actual > limit {
            return Err(Error::LimitExceeded {
                resource,
                limit,
                actual,
            });
        }
        Ok(())
    }
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_command_body_bytes: 64 * 1024 * 1024,
            max_decompressed_command_bytes: 256 * 1024 * 1024,
            max_packet_message_bytes: 64 * 1024 * 1024,
            max_string_table_data_bytes: 64 * 1024 * 1024,
            max_decompressed_string_table_bytes: 256 * 1024 * 1024,
            max_string_table_entries: 1_048_576,
            max_string_table_user_data_bytes: 8 * 1024 * 1024,
            max_field_string_bytes: 1024 * 1024,
            max_classes: 65_536,
            max_class_id: 65_535,
            max_serializers: 65_536,
            max_serializer_fields: 1_048_576,
            max_symbols: 1_048_576,
            max_fixed_array_length: 65_536,
            max_packet_entity_updates: 16_384,
            max_field_paths: 65_536,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_changes_only_the_selected_limit() {
        let default = DecodeLimits::default();
        let limits = default.with_max_command_body_bytes(1234);
        assert_eq!(limits.max_command_body_bytes(), 1234);
        assert_eq!(
            limits.max_decompressed_command_bytes(),
            default.max_decompressed_command_bytes()
        );
    }

    #[test]
    fn ensure_reports_the_resource_and_sizes() {
        let limits = DecodeLimits::default();
        assert!(limits.ensure("fixture", 4, 4).is_ok());
        assert!(matches!(
            limits.ensure("fixture", 5, 4),
            Err(Error::LimitExceeded {
                resource: "fixture",
                limit: 4,
                actual: 5,
            })
        ));
    }
}
