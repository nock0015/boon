use crate::error::Error;
use crate::io::BitReader;
use crate::limits::DecodeLimits;
use crate::test_utils::BitWriter;

use super::{FIELDOP_DESCRIPTORS, FIELDOP_HIERARCHY, Node, read_field_paths_with_limits};

pub(crate) const PLUS_ONE: usize = 0;
pub(crate) const FINISH: usize = FIELDOP_DESCRIPTORS.len() - 1;

fn find_code(node: &Node, target: usize, path: &mut Vec<bool>) -> bool {
    match node {
        Node::Leaf { num, .. } => *num == target,
        Node::Branch { left, right, .. } => {
            path.push(false);
            if find_code(left, target, path) {
                return true;
            }
            path.pop();

            path.push(true);
            if find_code(right, target, path) {
                return true;
            }
            path.pop();
            false
        }
    }
}

pub(crate) fn emit_op(writer: &mut BitWriter, index: usize) {
    let mut code = Vec::new();
    assert!(find_code(&FIELDOP_HIERARCHY, index, &mut code));
    for bit in code {
        writer.push_bool(bit);
    }
}

pub(crate) fn emit_single_bool_update(writer: &mut BitWriter, value: bool) {
    emit_op(writer, PLUS_ONE);
    emit_op(writer, FINISH);
    writer.push_bool(value);
}

fn emit_operands(writer: &mut BitWriter, index: usize) {
    match index {
        0..=3 | 5 | 7 | 27 | 29 | 37 | 39 => {}
        4 | 6 | 8 | 9 | 28 | 30 | 33 => writer.push_ubitvarfp(1),
        10 => {
            writer.push_ubitvarfp(1);
            writer.push_ubitvarfp(1);
        }
        11 => {
            writer.push_bits(1, 3);
            writer.push_bits(2, 3);
        }
        12 => {
            writer.push_bits(1, 4);
            writer.push_bits(2, 4);
        }
        13 | 17 => {
            writer.push_ubitvarfp(1);
            writer.push_ubitvarfp(2);
        }
        14 | 18 => {
            writer.push_bits(1, 5);
            writer.push_bits(2, 5);
        }
        15 | 19 => {
            writer.push_ubitvarfp(1);
            writer.push_ubitvarfp(2);
            writer.push_ubitvarfp(3);
        }
        16 | 20 => {
            writer.push_bits(1, 5);
            writer.push_bits(2, 5);
            writer.push_bits(3, 5);
        }
        21 => {
            writer.push_ubitvar(0);
            writer.push_ubitvarfp(1);
            writer.push_ubitvarfp(2);
        }
        23 => {
            writer.push_ubitvar(0);
            writer.push_ubitvarfp(1);
            writer.push_ubitvarfp(2);
            writer.push_ubitvarfp(3);
        }
        22 => {
            writer.push_ubitvar(0);
            writer.push_bits(1, 5);
            writer.push_bits(2, 5);
        }
        24 => {
            writer.push_ubitvar(0);
            writer.push_bits(1, 5);
            writer.push_bits(2, 5);
            writer.push_bits(3, 5);
        }
        25 => {
            writer.push_ubitvar(2);
            writer.push_ubitvar(1);
            writer.push_ubitvarfp(2);
            writer.push_ubitvarfp(3);
        }
        26 => {
            writer.push_bool(false);
            writer.push_ubitvar(2);
            writer.push_ubitvarfp(2);
            writer.push_ubitvarfp(3);
        }
        31 => writer.push_bits(1, 3),
        32 => writer.push_bits(1, 6),
        34 => {
            writer.push_ubitvarfp(1);
            writer.push_varint32(2);
        }
        35 => {
            writer.push_ubitvarfp(1);
            writer.push_bool(true);
            writer.push_varint32(2);
            writer.push_bool(false);
            writer.push_bool(true);
            writer.push_varint32(-1);
        }
        36 => {
            writer.push_bool(true);
            writer.push_varint32(2);
        }
        38 => {
            writer.push_bool(true);
            writer.push_bits(8, 4);
        }
        _ => panic!("missing operands for field operation {index}"),
    }
}

fn emit_initialized_path(writer: &mut BitWriter) {
    emit_op(writer, PLUS_ONE);
}

fn emit_deep_path(writer: &mut BitWriter) {
    emit_initialized_path(writer);
    emit_op(writer, 15);
    emit_operands(writer, 15);
}

#[test]
fn every_field_operation_decodes_through_the_huffman_tree() {
    for index in 0..FIELDOP_DESCRIPTORS.len() {
        let mut writer = BitWriter::default();
        if index != FINISH {
            if matches!(index, 27..=35 | 37) {
                emit_deep_path(&mut writer);
            } else if index != PLUS_ONE {
                emit_initialized_path(&mut writer);
            }
            emit_op(&mut writer, index);
            emit_operands(&mut writer, index);
            emit_op(&mut writer, FINISH);
        } else {
            emit_op(&mut writer, FINISH);
        }

        let bytes = writer.finish();
        let mut paths = Vec::new();
        read_field_paths_with_limits(
            &mut BitReader::new(&bytes),
            &mut paths,
            &DecodeLimits::default(),
        )
        .unwrap_or_else(|error| {
            panic!("field operation {index} failed: {error}; bytes={bytes:02x?}")
        });

        if index == FINISH {
            assert!(paths.is_empty());
        } else {
            let decoded = paths.last().expect("target operation emits a path");
            assert!(decoded.last < decoded.data.len(), "operation {index}");
            assert!(!decoded.finished);
        }
    }
}

#[test]
fn field_path_limit_stops_before_appending_the_excess_path() {
    let mut writer = BitWriter::default();
    emit_op(&mut writer, PLUS_ONE);
    emit_op(&mut writer, PLUS_ONE);
    emit_op(&mut writer, FINISH);
    let bytes = writer.finish();
    let limits = DecodeLimits::default().with_max_field_paths(1);
    let mut paths = Vec::new();

    let error = read_field_paths_with_limits(&mut BitReader::new(&bytes), &mut paths, &limits)
        .expect_err("second path exceeds the limit");

    assert!(matches!(
        error,
        Error::LimitExceeded {
            resource: "entity field paths",
            limit: 1,
            actual: 2
        }
    ));
    assert_eq!(paths.len(), 1);
}

#[test]
fn empty_field_path_stream_is_reported_as_truncated() {
    let error = read_field_paths_with_limits(
        &mut BitReader::new(&[]),
        &mut Vec::new(),
        &DecodeLimits::default(),
    )
    .expect_err("empty stream cannot contain a Huffman operation");

    assert!(matches!(error, Error::Overflow { available: 0, .. }));
}

#[test]
fn representative_operations_produce_expected_paths() {
    let mut writer = BitWriter::default();
    emit_initialized_path(&mut writer);
    emit_op(&mut writer, 11);
    emit_operands(&mut writer, 11);
    emit_op(&mut writer, 27);
    emit_operands(&mut writer, 27);
    emit_op(&mut writer, FINISH);
    let bytes = writer.finish();
    let mut paths = Vec::new();

    read_field_paths_with_limits(
        &mut BitReader::new(&bytes),
        &mut paths,
        &DecodeLimits::default(),
    )
    .expect("valid field path stream");

    assert_eq!(paths.len(), 3);
    assert_eq!(&paths[0].data[..=paths[0].last], &[0]);
    assert_eq!(&paths[1].data[..=paths[1].last], &[3, 3]);
    assert_eq!(&paths[2].data[..=paths[2].last], &[4]);
}

#[test]
fn maximum_depth_paths_can_return_to_the_root() {
    let mut writer = BitWriter::default();
    emit_initialized_path(&mut writer);
    for _ in 0..6 {
        emit_op(&mut writer, 5); // push one level
    }
    for _ in 0..6 {
        emit_op(&mut writer, 27); // pop one level and increment
    }
    emit_op(&mut writer, FINISH);
    let bytes = writer.finish();
    let mut paths = Vec::new();
    super::read_field_paths(&mut BitReader::new(&bytes), &mut paths).unwrap();

    assert_eq!(paths.len(), 13);
    assert_eq!(paths[6].last, 6);
    assert_eq!(paths[6].data, [0; 7]);
    assert_eq!(paths[12].last, 0);
    assert_eq!(paths[12].data, [1, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn every_push_operation_rejects_excessive_depth() {
    for index in 5..=26 {
        let mut writer = BitWriter::default();
        emit_initialized_path(&mut writer);
        for _ in 0..6 {
            emit_op(&mut writer, 5);
        }
        emit_op(&mut writer, index);
        if index == 26 {
            // This operation first visits every existing path component.
            for _ in 0..7 {
                writer.push_bool(false);
            }
            writer.push_ubitvar(1);
            writer.push_ubitvarfp(0);
        } else {
            emit_operands(&mut writer, index);
        }
        emit_op(&mut writer, FINISH);
        let bytes = writer.finish();
        let mut paths = Vec::new();
        let error = super::read_field_paths(&mut BitReader::new(&bytes), &mut paths)
            .expect_err("push beyond seven levels must fail");

        assert!(
            matches!(error, Error::Parse { .. }),
            "operation {index}: {error}"
        );
        assert_eq!(paths.len(), 7, "invalid path must not be emitted");
    }
}

#[test]
fn operations_requiring_a_parent_reject_the_root() {
    for index in [27, 28, 33, 34, 35, 37] {
        let mut writer = BitWriter::default();
        emit_initialized_path(&mut writer);
        emit_op(&mut writer, index);
        emit_operands(&mut writer, index);
        emit_op(&mut writer, FINISH);
        let bytes = writer.finish();
        let mut paths = Vec::new();
        let error = super::read_field_paths(&mut BitReader::new(&bytes), &mut paths)
            .expect_err("operation requires a parent component");

        assert!(
            matches!(error, Error::Parse { .. }),
            "operation {index}: {error}"
        );
        assert_eq!(paths.len(), 1, "invalid path must not be emitted");
    }
}

fn read_reference(
    br: &mut BitReader,
    paths: &mut Vec<super::FieldPath>,
    limits: &DecodeLimits,
) -> crate::error::Result<()> {
    paths.clear();
    let mut path = super::FieldPath::default();
    let mut node = &*FIELDOP_HIERARCHY;
    loop {
        node = match node {
            Node::Branch { left, right, .. } => {
                if br.read_bool()? {
                    right
                } else {
                    left
                }
            }
            Node::Leaf { .. } => unreachable!(),
        };
        if let Node::Leaf { op, .. } = node {
            op(&mut path, br)?;
            if path.finished {
                return Ok(());
            }
            limits.ensure(
                "entity field paths",
                paths.len() + 1,
                limits.max_field_paths(),
            )?;
            paths.push(path);
            node = &FIELDOP_HIERARCHY;
        }
    }
}

fn compare_reference(bytes: &[u8], offset: usize, limits: &DecodeLimits) {
    let mut actual_reader = BitReader::new(bytes);
    let mut reference_reader = BitReader::new(bytes);
    actual_reader.skip_bits(offset).unwrap();
    reference_reader.skip_bits(offset).unwrap();
    let mut actual = vec![super::FieldPath::default()];
    let mut reference = actual.clone();
    let actual_result = read_field_paths_with_limits(&mut actual_reader, &mut actual, limits);
    let reference_result = read_reference(&mut reference_reader, &mut reference, limits);
    assert_eq!(
        format!("{actual_result:?}"),
        format!("{reference_result:?}")
    );
    assert_eq!(actual_reader.position(), reference_reader.position());
    assert_eq!(format!("{actual:?}"), format!("{reference:?}"));
}

#[test]
fn prefix_decoder_matches_tree_for_every_op_alignment_truncation_and_limit() {
    for index in 0..FIELDOP_DESCRIPTORS.len() {
        for offset in 0..8 {
            let mut writer = BitWriter::default();
            writer.push_bits(0, offset);
            if index != FINISH {
                if matches!(index, 27..=35 | 37) {
                    emit_deep_path(&mut writer);
                } else if index != PLUS_ONE {
                    emit_initialized_path(&mut writer);
                }
                emit_op(&mut writer, index);
                emit_operands(&mut writer, index);
            }
            emit_op(&mut writer, FINISH);
            let bytes = writer.finish();
            for end in 0..=bytes.len() {
                if end * 8 < offset {
                    continue;
                }
                for max_fields in [0, 1, 3, usize::MAX] {
                    compare_reference(
                        &bytes[..end],
                        offset,
                        &DecodeLimits::default().with_max_field_paths(max_fields),
                    );
                }
            }
        }
    }
}

#[test]
fn prefix_decoder_matches_tree_at_maximum_path_depth_and_before_following_data() {
    let mut writer = BitWriter::default();
    emit_initialized_path(&mut writer);
    for _ in 0..2 {
        emit_op(&mut writer, 15);
        emit_operands(&mut writer, 15);
    }
    emit_op(&mut writer, PLUS_ONE);
    emit_op(&mut writer, FINISH);
    writer.push_bits(0xabcdef1234567890, 64);
    let bytes = writer.finish();
    compare_reference(&bytes, 0, &DecodeLimits::default());
}

#[test]
fn every_prefix_entry_agrees_with_the_tree_code_and_length() {
    for (value, entry) in super::FIELDOP_PREFIX.iter().enumerate() {
        let mut node = &*FIELDOP_HIERARCHY;
        let mut bits = 0;
        while let Node::Branch { left, right, .. } = node {
            if bits == super::PREFIX_BITS {
                break;
            }
            node = if value & (1 << bits) == 0 {
                left
            } else {
                right
            };
            bits += 1;
        }
        match node {
            Node::Leaf { num, .. } => {
                assert_eq!(usize::from(entry.bits), bits);
                assert!(std::ptr::fn_addr_eq(entry.op, FIELDOP_DESCRIPTORS[*num].op));
            }
            Node::Branch { .. } => assert_eq!(entry.bits, 0),
        }
    }
}
