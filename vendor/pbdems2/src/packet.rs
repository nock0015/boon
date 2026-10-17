//! Game-neutral framing for messages carried inside Source 2 packets.

use crate::error::{Error, Result};
use crate::io::BitReader;
use crate::limits::DecodeLimits;

/// Borrowed framing metadata for one inner packet message.
///
/// Source 2 packet payloads are not necessarily byte-aligned. Use
/// [`PacketMessageFrame::payload`] when a borrowed byte slice is available, or
/// [`PacketMessageFrame::copy_payload`] with a reusable buffer otherwise.
#[derive(Debug, Clone, Copy)]
pub struct PacketMessageFrame<'a> {
    index: usize,
    bit_offset: usize,
    payload_bit_offset: usize,
    end_bit_offset: usize,
    message_type: u32,
    encoded_size: usize,
    data: &'a [u8],
}

impl<'a> PacketMessageFrame<'a> {
    /// Zero-based message index in this packet.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Bit offset at which the message-type header begins.
    pub const fn bit_offset(&self) -> usize {
        self.bit_offset
    }

    /// Bit offset at which the encoded protobuf payload begins.
    pub const fn payload_bit_offset(&self) -> usize {
        self.payload_bit_offset
    }

    /// Bit offset immediately after the payload.
    pub const fn end_bit_offset(&self) -> usize {
        self.end_bit_offset
    }

    /// Source 2 network-message identifier.
    pub const fn message_type(&self) -> u32 {
        self.message_type
    }

    /// Encoded protobuf payload size in bytes.
    pub const fn encoded_size(&self) -> usize {
        self.encoded_size
    }

    /// Borrow the payload directly when it happens to be byte-aligned.
    ///
    /// Most packet messages are bit-aligned rather than byte-aligned. In that
    /// case this returns `None`; use [`PacketMessageFrame::copy_payload`].
    pub fn payload(&self) -> Option<&'a [u8]> {
        if !self.payload_bit_offset.is_multiple_of(8) {
            return None;
        }
        let start = self.payload_bit_offset / 8;
        let end = start.checked_add(self.encoded_size)?;
        self.data.get(start..end)
    }

    /// Copy an unaligned payload into a reusable, contiguous byte buffer.
    ///
    /// The buffer is cleared first. Byte-aligned payloads use a direct slice
    /// copy; unaligned payloads are reconstructed without temporary allocation.
    pub fn copy_payload(&self, output: &mut Vec<u8>) -> Result<()> {
        let mut copy = || -> Result<()> {
            output.clear();
            output
                .try_reserve(self.encoded_size)
                .map_err(|_| Error::Allocation {
                    resource: "inner packet message",
                    requested: self.encoded_size,
                })?;
            if let Some(payload) = self.payload() {
                output.extend_from_slice(payload);
                return Ok(());
            }

            output.resize(self.encoded_size, 0);
            let mut reader = BitReader::new(self.data);
            reader.skip_bits(self.payload_bit_offset)?;
            reader.read_bytes(output)
        };

        copy().map_err(|source| Error::PacketMessage {
            bit_offset: self.bit_offset,
            message_type: Some(self.message_type),
            source: Box::new(source),
        })
    }

    /// Borrow an aligned payload or reconstruct an unaligned one in `scratch`.
    ///
    /// This is the convenient counterpart to [`Self::payload`] and
    /// [`Self::copy_payload`] for adapters that only need a contiguous byte
    /// slice. The scratch buffer is reused and is left untouched when the
    /// payload can be borrowed directly.
    pub fn payload_or_copy<'scratch>(
        &'scratch self,
        scratch: &'scratch mut Vec<u8>,
    ) -> Result<&'scratch [u8]>
    where
        'a: 'scratch,
    {
        if let Some(payload) = self.payload() {
            return Ok(payload);
        }
        self.copy_payload(scratch)?;
        Ok(scratch)
    }
}

/// Strict, allocation-free iterator over messages inside a packet payload.
///
/// Framing reads Valve's `ubitvar` message type followed by a protobuf varint
/// byte length. Up to one final byte of packet padding is ignored, matching the
/// Source 2 packet-stream convention. Message payloads are validated against
/// [`DecodeLimits::max_packet_message_bytes`].
pub struct PacketMessageIter<'a> {
    reader: BitReader<'a>,
    index: usize,
    limits: DecodeLimits,
    failed: bool,
}

impl<'a> PacketMessageIter<'a> {
    /// Iterate a packet using default decode limits.
    pub fn new(data: &'a [u8]) -> Self {
        Self::with_limits(data, DecodeLimits::default())
    }

    /// Iterate a packet using explicit decode limits.
    pub fn with_limits(data: &'a [u8], limits: DecodeLimits) -> Self {
        Self {
            reader: BitReader::new(data),
            index: 0,
            limits,
            failed: false,
        }
    }

    /// Limits applied to framed payload sizes.
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    fn fail(&mut self, bit_offset: usize, message_type: Option<u32>, source: Error) -> Error {
        self.failed = true;
        Error::PacketMessage {
            bit_offset,
            message_type,
            source: Box::new(source),
        }
    }
}

impl<'a> Iterator for PacketMessageIter<'a> {
    type Item = Result<PacketMessageFrame<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.bits_remaining() <= 8 {
            return None;
        }

        let bit_offset = self.reader.position();
        let message_type = match self.reader.read_ubitvar() {
            Ok(value) => value,
            Err(error) => return Some(Err(self.fail(bit_offset, None, error))),
        };
        let encoded_size = match self.reader.read_uvarint32() {
            Ok(value) => value as usize,
            Err(error) => return Some(Err(self.fail(bit_offset, Some(message_type), error))),
        };
        if let Err(error) = self.limits.ensure(
            "inner packet message",
            encoded_size,
            self.limits.max_packet_message_bytes(),
        ) {
            return Some(Err(self.fail(bit_offset, Some(message_type), error)));
        }
        let Some(payload_bits) = encoded_size.checked_mul(8) else {
            return Some(Err(self.fail(
                bit_offset,
                Some(message_type),
                Error::Parse {
                    context: format!(
                        "inner packet message size {encoded_size} overflows bit length"
                    ),
                },
            )));
        };
        let payload_bit_offset = self.reader.position();
        if let Err(error) = self.reader.skip_bits(payload_bits) {
            return Some(Err(self.fail(bit_offset, Some(message_type), error)));
        }
        let frame = PacketMessageFrame {
            index: self.index,
            bit_offset,
            payload_bit_offset,
            end_bit_offset: self.reader.position(),
            message_type,
            encoded_size,
            data: self.reader.data(),
        };
        self.index += 1;
        Some(Ok(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct BitWriter {
        data: Vec<u8>,
        position: usize,
    }

    impl BitWriter {
        fn push_bits(&mut self, value: u32, count: usize) {
            for index in 0..count {
                if self.position.is_multiple_of(8) {
                    self.data.push(0);
                }
                if value & (1 << index) != 0 {
                    self.data[self.position / 8] |= 1 << (self.position % 8);
                }
                self.position += 1;
            }
        }

        fn push_ubitvar(&mut self, value: u32) {
            if value < 16 {
                self.push_bits(value, 6);
            } else if value < 256 {
                self.push_bits((value & 15) | 16, 6);
                self.push_bits(value >> 4, 4);
            } else if value < 4096 {
                self.push_bits((value & 15) | 32, 6);
                self.push_bits(value >> 4, 8);
            } else {
                self.push_bits((value & 15) | 48, 6);
                self.push_bits(value >> 4, 28);
            }
        }

        fn push_uvarint32(&mut self, mut value: u32) {
            loop {
                let mut byte = (value & 0x7f) as u8;
                value >>= 7;
                if value != 0 {
                    byte |= 0x80;
                }
                self.push_bits(u32::from(byte), 8);
                if value == 0 {
                    break;
                }
            }
        }

        fn push_message(&mut self, message_type: u32, payload: &[u8]) {
            self.push_ubitvar(message_type);
            self.push_uvarint32(payload.len() as u32);
            for &byte in payload {
                self.push_bits(u32::from(byte), 8);
            }
        }
    }

    #[test]
    fn iterates_unaligned_and_aligned_payloads_without_allocating() {
        let mut writer = BitWriter::default();
        writer.push_message(7, b"abc");
        writer.push_message(72, b"de");

        let frames = PacketMessageIter::new(&writer.data)
            .collect::<Result<Vec<_>>>()
            .expect("valid packet");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].index(), 0);
        assert_eq!(frames[0].message_type(), 7);
        assert_eq!(frames[0].bit_offset(), 0);
        assert_eq!(frames[0].payload_bit_offset(), 14);
        assert_eq!(frames[0].encoded_size(), 3);
        assert_eq!(frames[0].payload(), None);

        let mut payload = Vec::new();
        assert_eq!(
            frames[0]
                .payload_or_copy(&mut payload)
                .expect("copy unaligned payload"),
            b"abc"
        );
        assert_eq!(payload, b"abc");

        assert_eq!(frames[1].message_type(), 72);
        assert_eq!(frames[1].payload(), Some(&b"de"[..]));
        assert_eq!(
            frames[1]
                .payload_or_copy(&mut payload)
                .expect("borrow aligned payload"),
            b"de"
        );
        // Borrowing the aligned payload does not overwrite reusable scratch.
        assert_eq!(payload, b"abc");
    }

    #[test]
    fn reports_truncated_payload_with_message_context_and_stops() {
        let mut writer = BitWriter::default();
        writer.push_ubitvar(7);
        writer.push_uvarint32(4);
        writer.push_bits(u32::from(b'x'), 8);
        let mut messages = PacketMessageIter::new(&writer.data);

        assert!(matches!(
            messages.next(),
            Some(Err(Error::PacketMessage {
                bit_offset: 0,
                message_type: Some(7),
                source,
            })) if matches!(*source, Error::Overflow { .. })
        ));
        assert!(messages.next().is_none());
    }

    #[test]
    fn applies_packet_message_size_limits_before_payload_reads() {
        let mut writer = BitWriter::default();
        writer.push_message(3, b"abc");
        let limits = DecodeLimits::default().with_max_packet_message_bytes(2);

        assert!(matches!(
            PacketMessageIter::with_limits(&writer.data, limits).next(),
            Some(Err(Error::PacketMessage { source, .. }))
                if matches!(*source, Error::LimitExceeded {
                    resource: "inner packet message",
                    limit: 2,
                    actual: 3,
                })
        ));
    }

    #[test]
    fn ignores_one_final_byte_of_packet_padding() {
        assert!(PacketMessageIter::new(&[0xff]).next().is_none());
    }
}
