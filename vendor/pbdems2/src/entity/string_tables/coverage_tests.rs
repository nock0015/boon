use crate::error::Error;
use crate::limits::DecodeLimits;
use crate::test_utils::BitWriter;

use super::*;

fn push_raw_entry(writer: &mut BitWriter, string: Option<&str>, user_data: Option<&[u8]>) {
    writer.push_bool(true);
    writer.push_bool(string.is_some());
    if let Some(string) = string {
        writer.push_bool(false);
        writer.push_c_string(string);
    }
    writer.push_bool(user_data.is_some());
    if let Some(user_data) = user_data {
        writer.push_bits(user_data.len() as u64, MAX_USERDATA_BITS);
        writer.push_bytes(user_data);
    }
}

#[test]
fn parses_raw_and_history_strings_with_variable_user_data() {
    let mut writer = BitWriter::default();
    push_raw_entry(&mut writer, Some("alpha"), Some(&[1, 2, 3]));

    writer.push_bool(true);
    writer.push_bool(true);
    writer.push_bool(true);
    writer.push_bits(0, 5);
    writer.push_bits(3, MAX_STRING_BITS);
    writer.push_c_string("ine");
    writer.push_bool(false);

    let mut container = StringTableContainer::new();
    assert!(
        !container
            .handle_create(CreateStringTable::new("names", 2, writer.finish()))
            .expect("valid table")
    );

    let table = container.find_table("names").expect("created table");
    assert_eq!(table.name(), "names");
    assert_eq!(table.entries().len(), 2);
    assert_eq!(table.get(0).unwrap().string.as_deref(), Some("alpha"));
    assert_eq!(
        table.get(0).unwrap().user_data.as_deref(),
        Some(&[1, 2, 3][..])
    );
    assert_eq!(table.get(1).unwrap().string.as_deref(), Some("alpine"));
    assert!(table.get(1).unwrap().user_data.is_none());
    assert_eq!(table.dirty_indices(), &[0, 1]);
    assert!(table.changes().is_empty());
    assert!(table.get(2).is_none());
}

#[test]
fn parses_fixed_width_user_data_with_partial_final_byte() {
    let mut writer = BitWriter::default();
    writer.push_bool(true);
    writer.push_bool(false);
    writer.push_bool(true);
    writer.push_bits(0x0cab, 12);

    let mut container = StringTableContainer::new();
    container
        .handle_create(
            CreateStringTable::new("fixed", 1, writer.finish()).with_fixed_user_data(2, 12),
        )
        .expect("valid fixed-width data");

    assert_eq!(
        container
            .find_table("fixed")
            .unwrap()
            .get(0)
            .unwrap()
            .user_data,
        Some(vec![0xab, 0x0c])
    );
}

#[test]
fn variable_user_data_scratch_handles_shrinking_and_growing_entries() {
    let mut writer = BitWriter::default();
    for size in [256, 0, 3, 1024, 1] {
        push_raw_entry(&mut writer, None, Some(&vec![size as u8; size]));
    }
    let mut tables = StringTableContainer::new();
    tables
        .handle_create(CreateStringTable::new("sizes", 5, writer.finish()))
        .unwrap();
    for (entry, size) in tables
        .find_table("sizes")
        .unwrap()
        .entries()
        .iter()
        .zip([256, 0, 3, 1024, 1])
    {
        assert_eq!(
            entry.user_data.as_deref(),
            Some(vec![size as u8; size].as_slice())
        );
    }
}

#[test]
fn decompresses_outer_table_data_and_inner_entry_data() {
    let expected = b"repeated payload repeated payload";
    let compressed_user_data = snap::raw::Encoder::new().compress_vec(expected).unwrap();

    let mut writer = BitWriter::default();
    writer.push_bool(true);
    writer.push_bool(false);
    writer.push_bool(true);
    writer.push_bool(true);
    writer.push_ubitvar(compressed_user_data.len() as u32);
    writer.push_bytes(&compressed_user_data);
    let compressed_table_data = snap::raw::Encoder::new()
        .compress_vec(&writer.finish())
        .unwrap();

    let message = CreateStringTable::new("compressed", 1, compressed_table_data)
        .with_flags(1)
        .with_varint_bitcounts()
        .with_compressed_data();
    let mut container = StringTableContainer::new();
    container
        .handle_create(message)
        .expect("valid compressed data");

    assert_eq!(
        container
            .find_table("compressed")
            .unwrap()
            .get(0)
            .unwrap()
            .user_data
            .as_deref(),
        Some(expected.as_slice())
    );
}

#[test]
fn updates_existing_entries_without_clearing_omitted_fields() {
    let mut create = BitWriter::default();
    push_raw_entry(&mut create, Some("persistent"), Some(&[1]));
    let mut container = StringTableContainer::new();
    container
        .handle_create(CreateStringTable::new("updates", 1, create.finish()))
        .unwrap();
    container.clear_dirty();

    let mut update = BitWriter::default();
    push_raw_entry(&mut update, None, Some(&[9, 8]));
    assert!(
        !container
            .handle_update(UpdateStringTable::new(0, 1, update.finish()))
            .expect("valid update")
    );

    let table = container.find_table("updates").unwrap();
    let entry = table.get(0).unwrap();
    assert_eq!(entry.string.as_deref(), Some("persistent"));
    assert_eq!(entry.user_data.as_deref(), Some(&[9, 8][..]));
    assert_eq!(table.dirty_indices(), &[0]);
}

#[test]
fn initial_index_jump_creates_gaps() {
    let mut writer = BitWriter::default();
    writer.push_bool(false);
    writer.push_uvarint32(1);
    writer.push_bool(true);
    writer.push_bool(false);
    writer.push_c_string("third");
    writer.push_bool(false);

    let mut container = StringTableContainer::new();
    container
        .handle_create(CreateStringTable::new("sparse", 1, writer.finish()))
        .expect("valid sparse table");

    let table = container.find_table("sparse").unwrap();
    assert_eq!(table.entries().len(), 3);
    assert!(table.get(0).unwrap().string.is_none());
    assert!(table.get(1).unwrap().string.is_none());
    assert_eq!(table.get(2).unwrap().string.as_deref(), Some("third"));
    assert_eq!(table.dirty_indices(), &[2]);
}

#[test]
fn relative_index_jumps_preserve_sparse_entries_on_create_and_update() {
    let mut container = StringTableContainer::new();
    for generation in 0..2 {
        let mut writer = BitWriter::default();
        for (value, jump) in [Some(1), None, Some(0), Some(0), Some(128)]
            .into_iter()
            .enumerate()
        {
            writer.push_bool(jump.is_none());
            if let Some(jump) = jump {
                writer.push_uvarint32(jump);
            }
            writer.push_bool(false); // No key update.
            writer.push_bool(true);
            writer.push_bits(1, MAX_USERDATA_BITS);
            writer.push_bytes(&[generation * 10 + value as u8]);
        }
        let data = writer.finish();
        if generation == 0 {
            container
                .handle_create(CreateStringTable::new("events", 5, data))
                .unwrap();
        } else {
            container.clear_dirty();
            container
                .handle_update(UpdateStringTable::new(0, 5, data))
                .unwrap();
        }

        let table = container.find_table("events").unwrap();
        // Each jump is relative to the previous index, starting at -1.
        assert_eq!(table.dirty_indices(), &[2, 3, 5, 7, 137]);
        assert_eq!(table.entries().len(), 138);
        for (value, index) in [2, 3, 5, 7, 137].into_iter().enumerate() {
            assert_eq!(
                table.get(index).unwrap().user_data.as_deref(),
                Some([generation * 10 + value as u8].as_slice())
            );
        }
        for index in [0, 1, 4, 6, 136] {
            assert!(table.get(index).unwrap().user_data.is_none());
        }
    }
}

#[test]
fn full_updates_extend_tables_and_refresh_instance_baselines() {
    let mut writer = BitWriter::default();
    push_raw_entry(&mut writer, Some("7"), Some(&[1, 2]));
    let mut container = StringTableContainer::new();
    assert!(
        container
            .handle_create(CreateStringTable::new(
                INSTANCE_BASELINE_TABLE_NAME,
                1,
                writer.finish(),
            ))
            .unwrap()
    );
    container.update_instance_baselines();
    assert_eq!(container.instance_baseline(7), Some(&[1, 2][..]));

    container
        .do_full_update([
            (
                INSTANCE_BASELINE_TABLE_NAME.to_owned(),
                vec![
                    StringTableEntry::new(Some("ignored replacement".to_owned()), Some(vec![3])),
                    StringTableEntry::new(Some("8".to_owned()), Some(vec![4, 5])),
                ],
            ),
            (
                "unknown".to_owned(),
                vec![StringTableEntry::new(None, Some(vec![99]))],
            ),
        ])
        .unwrap();
    container.update_instance_baselines();

    let table = container.find_table(INSTANCE_BASELINE_TABLE_NAME).unwrap();
    assert_eq!(table.get(0).unwrap().string.as_deref(), Some("7"));
    assert_eq!(table.get(0).unwrap().user_data.as_deref(), Some(&[3][..]));
    assert_eq!(table.get(1).unwrap().string.as_deref(), Some("8"));
    assert_eq!(container.instance_baseline(7), Some(&[3][..]));
    assert_eq!(container.instance_baseline(8), Some(&[4, 5][..]));
    assert_eq!(table.dirty_indices(), &[0, 0, 1]);

    container.clear_dirty();
    assert!(
        container
            .tables()
            .iter()
            .all(|table| table.dirty_indices().is_empty())
    );
}

#[test]
fn rejects_negative_identifiers_and_counts() {
    let mut container = StringTableContainer::new();
    let create_error = container
        .handle_create(CreateStringTable::new("bad", -1, Vec::new()))
        .expect_err("negative count is invalid");
    assert!(
        matches!(create_error, Error::Parse { context } if context.contains("negative string-table entry count"))
    );

    let update_error = container
        .handle_update(UpdateStringTable::new(-1, 0, Vec::new()))
        .expect_err("negative table ID is invalid");
    assert!(
        matches!(update_error, Error::Parse { context } if context.contains("negative string table ID"))
    );
}

#[test]
fn enforces_table_entry_data_and_snapshot_limits() {
    let entry_limits = DecodeLimits::default().with_max_string_table_entries(0);
    let mut container = StringTableContainer::new();
    let entry_error = container
        .handle_create_with_limits(
            CreateStringTable::new("limited", 1, Vec::new()),
            &entry_limits,
        )
        .expect_err("entry count exceeds limit");
    assert!(matches!(
        entry_error,
        Error::LimitExceeded {
            resource: "string table entries",
            limit: 0,
            actual: 1
        }
    ));

    let data_limits = DecodeLimits::default().with_max_string_table_bytes(0, 0);
    let data_error = container
        .handle_create_with_limits(CreateStringTable::new("limited", 0, vec![0]), &data_limits)
        .expect_err("encoded data exceeds limit");
    assert!(matches!(
        data_error,
        Error::LimitExceeded {
            resource: "string-table data",
            limit: 0,
            actual: 1
        }
    ));

    container
        .handle_create(CreateStringTable::new("snapshot", 0, Vec::new()))
        .unwrap();
    let snapshot_error = container
        .do_full_update_with_limits(
            [(
                "snapshot".to_owned(),
                vec![StringTableEntry::new(None, None)],
            )],
            &entry_limits,
        )
        .expect_err("snapshot exceeds limit");
    assert!(matches!(
        snapshot_error,
        Error::LimitExceeded {
            resource: "full string-table entries",
            limit: 0,
            actual: 1
        }
    ));
}

#[test]
fn rejects_invalid_compressed_payloads_and_oversized_fixed_data() {
    let mut container = StringTableContainer::new();
    let compressed_error = container
        .handle_create(
            CreateStringTable::new("compressed", 0, vec![1, 2, 3]).with_compressed_data(),
        )
        .expect_err("invalid Snappy data");
    assert!(matches!(compressed_error, Error::Decompress(_)));

    let mut writer = BitWriter::default();
    writer.push_bool(true);
    writer.push_bool(false);
    writer.push_bool(true);
    let fixed_error = container
        .handle_create(
            CreateStringTable::new("fixed", 1, writer.finish())
                .with_fixed_user_data((MAX_USERDATA_SIZE + 1) as i32, 8),
        )
        .expect_err("fixed data exceeds parser buffer");
    assert!(matches!(fixed_error, Error::Parse { context } if context.contains("fixed user data")));
}

#[test]
fn rejects_invalid_inner_compression_and_user_data_limit() {
    let mut invalid = BitWriter::default();
    invalid.push_bool(true);
    invalid.push_bool(false);
    invalid.push_bool(true);
    invalid.push_bool(true);
    invalid.push_bits(3, MAX_USERDATA_BITS);
    invalid.push_bytes(&[1, 2, 3]);
    let mut container = StringTableContainer::new();
    let error = container
        .handle_create(CreateStringTable::new("inner", 1, invalid.finish()).with_flags(1))
        .expect_err("invalid inner Snappy payload");
    assert!(matches!(error, Error::Decompress(_)));

    let mut oversized = BitWriter::default();
    push_raw_entry(&mut oversized, None, Some(&[1, 2]));
    let limits = DecodeLimits::default().with_max_string_table_user_data_bytes(1);
    let error = container
        .handle_create_with_limits(
            CreateStringTable::new("limited-data", 1, oversized.finish()),
            &limits,
        )
        .expect_err("entry user data exceeds limit");
    assert!(matches!(
        error,
        Error::LimitExceeded {
            resource: "string-table user data",
            limit: 1,
            actual: 2
        }
    ));
}

#[test]
fn ordered_changes_keep_each_write_and_distinguish_snapshots() {
    let mut container = StringTableContainer::new();
    container
        .handle_create(CreateStringTable::new("events", 0, vec![]).with_change_tracking())
        .unwrap();
    for (key, data) in [
        (Some("apply"), Some(&[1][..])),
        (None, Some(&[2][..])),
        (Some("remove"), None),
    ] {
        let mut writer = BitWriter::default();
        push_raw_entry(&mut writer, key, data);
        container
            .handle_update(UpdateStringTable::new(0, 1, writer.finish()))
            .unwrap();
    }
    let table = container.find_table("events").unwrap();
    assert_eq!(table.dirty_indices(), &[0, 0, 0]);
    let changes = table.changes();
    assert_eq!(changes.len(), 3);
    assert_eq!(changes[0].entry.string.as_deref(), Some("apply"));
    assert_eq!(changes[0].entry.user_data.as_deref(), Some(&[1][..]));
    assert_eq!(changes[1].entry.user_data.as_deref(), Some(&[2][..]));
    assert!(changes[1].entry.string.is_none());
    assert!(changes[2].entry.user_data.is_none());
    assert_eq!(table.get(0).unwrap().user_data.as_deref(), Some(&[2][..]));
    let saved = container.clone();
    container
        .do_full_update([(
            "events".into(),
            vec![StringTableEntry::new(None, Some(vec![3]))],
        )])
        .unwrap();
    assert_eq!(
        container.find_table("events").unwrap().changes()[3].kind,
        StringTableChangeKind::Snapshot
    );
    assert_eq!(saved.find_table("events").unwrap().changes().len(), 3);
    container.clear_dirty();
    let table = container.find_table("events").unwrap();
    assert!(table.changes().is_empty());
    assert!(table.dirty_indices().is_empty());
    assert_eq!(table.get(0).unwrap().user_data.as_deref(), Some(&[3][..]));
}
