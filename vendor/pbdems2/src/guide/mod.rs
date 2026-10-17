//! PBDEMS2 format and parser architecture guide.
//!
//! These chapters document the Source 2 mechanisms shared by game-specific
//! parsers. Generated protobuf types, message identifiers, events, and domain
//! models remain the responsibility of each game's [`DemoAdapter`](crate::DemoAdapter).
//!
//! Start with [`file_structure`], then follow the wire data through
//! [`packet_messages`], [`serializers`], [`string_tables`], [`entities`], and
//! [`playback`].

#[doc = include_str!("file-structure.md")]
pub mod file_structure {}

#[doc = include_str!("packet-messages.md")]
pub mod packet_messages {}

#[doc = include_str!("serializers.md")]
pub mod serializers {}

#[doc = include_str!("string-tables.md")]
pub mod string_tables {}

#[doc = include_str!("entities.md")]
pub mod entities {}

#[doc = include_str!("playback.md")]
pub mod playback {}
