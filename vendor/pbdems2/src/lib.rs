//! Game-neutral parsing infrastructure for Valve Source 2 demo files.
//!
//! The core crate owns the game-independent PBDEMS2 container, bit encodings,
//! flattened serializers, entity state, string tables, and coordinate helpers.
//! Game crates provide their generated protobuf types and select an entity
//! decoding dialect.
//!
//! The [`guide`] module explains the shared file structure, packet framing,
//! serializers, string tables, entities, adapter boundary, and playback flow.
//!
//! # Example
//!
//! Walk the outer command stream, decompressing each body in turn:
//!
//! ```no_run
//! use pbdems2::demo::Demo;
//!
//! let bytes = std::fs::read("match.dem")?;
//! let demo = Demo::new(&bytes)?;
//!
//! let mut body = Vec::new();
//! for frame in demo.commands() {
//!     let frame = frame?;
//!     frame.decode_body(&mut body)?;
//!     println!("tick {}, command {}", frame.header().tick, frame.header().cmd);
//! }
//! # Ok::<(), pbdems2::Error>(())
//! ```
//!
//! Reading entity state needs a game crate's generated protobuf types:
//! implement [`DemoAdapter`] to convert them into the neutral types in
//! [`entity`], then drive it with [`DemoParser`].
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

pub mod demo;
pub mod entity;
pub mod error;
pub mod guide;
pub mod io;
pub mod limits;
#[cfg(feature = "mmap")]
#[cfg_attr(docsrs, doc(cfg(feature = "mmap")))]
pub mod mmap;
pub mod packet;
pub mod playback;
pub mod position;

#[cfg(test)]
mod test_utils;

pub use entity::Entity;
pub use error::{Error, Result};
pub use limits::DecodeLimits;
#[cfg(feature = "mmap")]
#[cfg_attr(docsrs, doc(cfg(feature = "mmap")))]
pub use mmap::MappedDemo;
pub use packet::{PacketMessageFrame, PacketMessageIter};
pub use playback::{
    CheckpointAdapter, CommandContext, DemoAdapter, DemoParser, ParserState, PlaybackCheckpoint,
    PlaybackSegment, PlaybackSession, PreparedPlayback,
};
