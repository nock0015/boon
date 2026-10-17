#[derive(Debug, Default)]
pub(crate) struct BitWriter {
    bytes: Vec<u8>,
    bit_position: usize,
}

impl BitWriter {
    pub(crate) fn push_bool(&mut self, value: bool) {
        self.push_bits(u64::from(value), 1);
    }

    pub(crate) fn push_bits(&mut self, value: u64, count: usize) {
        assert!(count <= u64::BITS as usize);
        for bit in 0..count {
            let byte_index = self.bit_position / 8;
            let bit_index = self.bit_position % 8;
            if byte_index == self.bytes.len() {
                self.bytes.push(0);
            }
            if value & (1_u64 << bit) != 0 {
                self.bytes[byte_index] |= 1 << bit_index;
            }
            self.bit_position += 1;
        }
    }

    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.push_bits(u64::from(byte), 8);
        }
    }

    pub(crate) fn push_c_string(&mut self, value: &str) {
        self.push_bytes(value.as_bytes());
        self.push_bits(0, 8);
    }

    pub(crate) fn push_uvarint32(&mut self, mut value: u32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            self.push_bits(u64::from(byte), 8);
            if value == 0 {
                return;
            }
        }
    }

    pub(crate) fn push_varint32(&mut self, value: i32) {
        let encoded = ((value as u32) << 1) ^ ((value >> 31) as u32);
        self.push_uvarint32(encoded);
    }

    pub(crate) fn push_ubitvar(&mut self, value: u32) {
        match value {
            0..=15 => self.push_bits(u64::from(value), 6),
            16..=255 => {
                self.push_bits(u64::from((value & 15) | 16), 6);
                self.push_bits(u64::from(value >> 4), 4);
            }
            256..=4095 => {
                self.push_bits(u64::from((value & 15) | 32), 6);
                self.push_bits(u64::from(value >> 4), 8);
            }
            _ => {
                self.push_bits(u64::from((value & 15) | 48), 6);
                self.push_bits(u64::from(value >> 4), 28);
            }
        }
    }

    pub(crate) fn push_ubitvarfp(&mut self, value: u32) {
        match value {
            0..=3 => {
                self.push_bool(true);
                self.push_bits(u64::from(value), 2);
            }
            4..=15 => {
                self.push_bool(false);
                self.push_bool(true);
                self.push_bits(u64::from(value), 4);
            }
            16..=1023 => {
                self.push_bits(0b100, 3);
                self.push_bits(u64::from(value), 10);
            }
            1024..=131_071 => {
                self.push_bits(0b1000, 4);
                self.push_bits(u64::from(value), 17);
            }
            _ => {
                self.push_bits(0, 4);
                self.push_bits(u64::from(value), 31);
            }
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use crate::io::BitReader;

    use super::*;

    #[test]
    fn round_trips_source_varint_encodings() {
        for value in [0, 3, 15, 16, 255, 256, 4095, 65_535] {
            let mut writer = BitWriter::default();
            writer.push_ubitvar(value);
            assert_eq!(
                BitReader::new(&writer.finish()).read_ubitvar().unwrap(),
                value
            );
        }

        for value in [0, 3, 15, 16, 1023, 1024, 131_071, 1_000_000] {
            let mut writer = BitWriter::default();
            writer.push_ubitvarfp(value);
            assert_eq!(
                BitReader::new(&writer.finish()).read_ubitvarfp().unwrap(),
                value
            );
        }
    }

    #[test]
    fn round_trips_signed_and_unsigned_varints() {
        let mut writer = BitWriter::default();
        writer.push_uvarint32(300);
        writer.push_varint32(-123);
        let bytes = writer.finish();
        let mut reader = BitReader::new(&bytes);
        assert_eq!(reader.read_uvarint32().unwrap(), 300);
        assert_eq!(reader.read_varint32().unwrap(), -123);
    }
}
