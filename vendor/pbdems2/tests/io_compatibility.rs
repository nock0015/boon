use pbdems2::io::{BitReader, ByteReader};

#[test]
fn bit_reader_exposes_the_combined_api() {
    let bytes = [
        0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 0xef, 0xcd, 0xab, 0x90, 0x78, 0x56, 0x34, 0x12,
    ];
    let mut reader = BitReader::new(&bytes);

    assert_eq!(reader.peek_bits(16).unwrap(), 0x1234);
    assert_eq!(reader.position(), 0);
    assert_eq!(reader.read_u16().unwrap(), 0x1234);
    assert_eq!(reader.read_u32().unwrap(), 0x1234_5678);
    assert_eq!(reader.read_u64().unwrap(), 0x1234_5678_90ab_cdef);
}

#[test]
fn byte_reader_exposes_signed_and_unsigned_fixed_width_reads() {
    let bytes = [0x34, 0x12, 0xfe, 0xff, 0xff, 0xff];
    let mut reader = ByteReader::new(&bytes);
    assert_eq!(reader.read_u16().unwrap(), 0x1234);
    assert_eq!(reader.read_i32().unwrap(), -2);
}

#[test]
fn bit_reader_reads_a_null_terminated_string() {
    let mut reader = BitReader::new(b"source2\0remaining");
    assert_eq!(reader.read_string().unwrap(), "source2");
}
