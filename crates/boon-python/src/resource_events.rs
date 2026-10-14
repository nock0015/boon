use crate::*;

pub(super) struct Row {
    pub tick: i32,
    pub sequence: u64,
    pub steam_id: Option<u64>,
    pub controller_index: Option<i32>,
    pub source: Option<(i32, u32, String)>,
    pub pawn_handle: Option<u32>,
    pub bag: Option<boon_proto::proto::CCitadelUserMsgSoulBagPickup>,
    pub bag_status: &'static str,
    pub message: boon_proto::proto::CCitadelUserMsgCurrencyChanged,
}

pub(super) fn frame(rows: &[Row]) -> PolarsResult<DataFrame> {
    macro_rules! column {
        ($name:expr, $value:expr) => {
            Column::new($name.into(), rows.iter().map($value).collect::<Vec<_>>())
        };
    }
    df_from_columns(vec![
        column!("tick", |r| r.tick),
        column!("sequence", |r| r.sequence),
        column!("steam_id", |r| r.steam_id),
        column!("controller_index", |r| r.controller_index),
        column!("userid", |r| r.message.userid),
        column!("recipient_pawn_handle", |r| r.pawn_handle),
        column!("soul_bag_status", |r| r.bag_status),
        column!("bag_pickup_player", |r| r
            .bag
            .as_ref()
            .and_then(|b| b.pickup_player)),
        column!("bag_victim_player", |r| r
            .bag
            .as_ref()
            .and_then(|b| b.victim_player)),
        column!("bag_gold", |r| r.bag.as_ref().and_then(|b| b.killfeed_gold)),
        column!("currency_type", |r| r.message.currency_type),
        column!("currency_source", |r| r.message.currency_source),
        column!("amount", |r| r.message.delta),
        column!("new_value", |r| r.message.new_value),
        column!("victim_handle", |r| r.message.victim),
        column!("ability_id", |r| r.message.ability_id),
        column!("notification", |r| r.message.notification),
        column!("source_entity_index", |r| r.source.as_ref().map(|e| e.0)),
        column!("source_entity_serial", |r| r.source.as_ref().map(|e| e.1)),
        column!("source_entity_class", |r| r
            .source
            .as_ref()
            .map(|e| e.2.as_str())),
        column!("source_x", |r| r
            .message
            .victim_pos
            .as_ref()
            .and_then(|p| p.x)),
        column!("source_y", |r| r
            .message
            .victim_pos
            .as_ref()
            .and_then(|p| p.y)),
        column!("source_z", |r| r
            .message
            .victim_pos
            .as_ref()
            .and_then(|p| p.z)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_currency_facts_stay_null_and_raw_codes_survive() {
        let row = Row {
            tick: 40,
            sequence: 0,
            steam_id: None,
            controller_index: None,
            source: None,
            pawn_handle: None,
            bag: None,
            bag_status: "unresolved",
            message: boon_proto::proto::CCitadelUserMsgCurrencyChanged {
                currency_source: Some(987),
                delta: Some(-20),
                ..Default::default()
            },
        };
        let df = frame(&[row]).unwrap();
        assert_eq!(df.column("steam_id").unwrap().null_count(), 1);
        assert_eq!(df.column("source_entity_serial").unwrap().null_count(), 1);
        assert_eq!(df.column("currency_type").unwrap().null_count(), 1);
        assert_eq!(
            df.column("currency_source").unwrap().i32().unwrap().get(0),
            Some(987)
        );
        assert_eq!(
            df.column("amount").unwrap().i32().unwrap().get(0),
            Some(-20)
        );
        assert_eq!(frame(&[]).unwrap().height(), 0);
    }
}

// Join only original same-tick messages; never borrow a future bag event.
// A missing/ambiguous recipient, amount, or duplicate notification is unknown.
pub(super) struct BagRow {
    pub tick: i32,
    pub message: boon_proto::proto::CCitadelUserMsgSoulBagPickup,
}
pub(super) fn link_bags(rows: &mut [Row], bags: &[BagRow]) {
    let mut by_tick = std::collections::HashMap::<i32, Vec<&BagRow>>::new();
    for bag in bags {
        by_tick.entry(bag.tick).or_default().push(bag);
    }
    let mut multiplicity = std::collections::HashMap::new();
    for row in rows.iter() {
        *multiplicity
            .entry((
                row.tick,
                row.pawn_handle,
                row.message.delta,
                row.message.currency_type,
            ))
            .or_insert(0usize) += 1;
    }
    for row in rows {
        row.bag = None;
        row.bag_status = "no-matching-notification";
        let Some(candidates) = by_tick.get(&row.tick) else {
            continue;
        };
        let Some(pawn) = row.pawn_handle else {
            row.bag_status = "unresolved";
            continue;
        };
        let possible: Vec<_> = candidates
            .iter()
            .filter(|b| b.message.pickup_player.is_none() || b.message.pickup_player == Some(pawn))
            .collect();
        if possible.is_empty() {
            continue;
        }
        row.bag_status = "unresolved";
        if possible.len() == 1
            && multiplicity[&(
                row.tick,
                row.pawn_handle,
                row.message.delta,
                row.message.currency_type,
            )] == 1
        {
            let bag = &possible[0].message;
            if bag.pickup_player == Some(pawn)
                && row.message.delta.is_some_and(|n| n > 0)
                && bag.killfeed_gold == row.message.delta
            {
                row.bag = Some(bag.clone());
                row.bag_status = "matched";
            }
        }
    }
}
pub(super) fn bag_frame(rows: &[BagRow]) -> PolarsResult<DataFrame> {
    df_from_columns(vec![
        Column::new(
            "tick".into(),
            rows.iter().map(|r| r.tick).collect::<Vec<_>>(),
        ),
        Column::new(
            "pickup_player".into(),
            rows.iter()
                .map(|r| r.message.pickup_player)
                .collect::<Vec<_>>(),
        ),
        Column::new(
            "victim_player".into(),
            rows.iter()
                .map(|r| r.message.victim_player)
                .collect::<Vec<_>>(),
        ),
        Column::new(
            "killfeed_gold".into(),
            rows.iter()
                .map(|r| r.message.killfeed_gold)
                .collect::<Vec<_>>(),
        ),
    ])
}

#[cfg(test)]
mod bag_tests {
    use super::*;
    fn receipt(tick: i32, pawn: Option<u32>, amount: i32, ledger: i32) -> Row {
        Row {
            tick,
            sequence: 0,
            steam_id: None,
            controller_index: None,
            source: None,
            pawn_handle: pawn,
            bag: None,
            bag_status: "unresolved",
            message: boon_proto::proto::CCitadelUserMsgCurrencyChanged {
                delta: Some(amount),
                currency_type: Some(ledger),
                ..Default::default()
            },
        }
    }
    fn bag(tick: i32, pawn: u32, amount: i32) -> BagRow {
        BagRow {
            tick,
            message: boon_proto::proto::CCitadelUserMsgSoulBagPickup {
                pickup_player: Some(pawn),
                victim_player: Some(99),
                killfeed_gold: Some(amount),
            },
        }
    }
    #[test]
    fn joins_both_ledgers_without_creating_extra_receipts() {
        let mut rows = vec![
            receipt(10, Some(42), 150, 0),
            receipt(10, Some(42), 150, 3),
            receipt(11, Some(42), 150, 0),
        ];
        link_bags(&mut rows, &[bag(10, 42, 150)]);
        assert_eq!(rows[0].bag_status, "matched");
        assert_eq!(rows[1].bag_status, "matched");
        assert_eq!(rows[2].bag_status, "no-matching-notification");
        assert_eq!(frame(&rows).unwrap().height(), 3);
    }
    #[test]
    fn missing_identity_wrong_amount_and_duplicate_messages_do_not_match() {
        for (pawn, amount) in [(None, 150), (Some(42), 149), (Some(42), -150)] {
            let mut rows = vec![receipt(10, pawn, amount, 0)];
            link_bags(&mut rows, &[bag(10, 42, 150)]);
            assert!(rows[0].bag.is_none());
        }
        let mut rows = vec![receipt(10, Some(42), 150, 0)];
        link_bags(&mut rows, &[bag(10, 42, 150), bag(10, 42, 150)]);
        assert!(rows[0].bag.is_none());
        let mut rows = vec![receipt(10, Some(42), 150, 0), receipt(10, Some(42), 150, 0)];
        link_bags(&mut rows, &[bag(10, 42, 150)]);
        assert!(rows.iter().all(|r| r.bag.is_none()));
    }
    #[test]
    fn complete_generation_handle_is_required_and_bag_notifications_are_independent() {
        let mut rows = vec![receipt(10, Some(42 + (2 << 14)), 150, 0)];
        let bags = [bag(10, 42 + (1 << 14), 150)];
        link_bags(&mut rows, &bags);
        assert!(rows[0].bag.is_none());
        assert_eq!(bag_frame(&bags).unwrap().height(), 1);
        assert_eq!(bag_frame(&[]).unwrap().height(), 0);
    }
}

// Serializer integer widths/signs vary. The real sample uses U64 handles.
// Bound the value to the original handle range and preserve the generation.
pub(super) fn recorded_handle(value: Option<&boon_parser::FieldValue>) -> Option<u32> {
    match value {
        Some(boon_parser::FieldValue::I32(h)) if (0..16777215).contains(h) => Some(*h as u32),
        Some(boon_parser::FieldValue::U32(h)) if *h < 16777215 => Some(*h),
        Some(boon_parser::FieldValue::U64(h)) if *h < 16777215 => Some(*h as u32),
        Some(boon_parser::FieldValue::I64(h)) if (0..16777215).contains(h) => Some(*h as u32),
        _ => None,
    }
}
#[cfg(test)]
mod handle_tests {
    use super::*;
    use boon_parser::FieldValue;
    #[test]
    fn real_signed_and_unsigned_handles_keep_their_generation() {
        assert_eq!(
            recorded_handle(Some(&FieldValue::I32(7897163))),
            Some(7897163)
        );
        assert_eq!(
            recorded_handle(Some(&FieldValue::U32(7897163))),
            Some(7897163)
        );
        assert_eq!(
            recorded_handle(Some(&FieldValue::U64(7897163))),
            Some(7897163)
        );
        assert_eq!(
            recorded_handle(Some(&FieldValue::I64(7897163))),
            Some(7897163)
        );
        for value in [
            FieldValue::U64(u64::MAX),
            FieldValue::I64(-1),
            FieldValue::I32(-1),
            FieldValue::U32(16777215),
            FieldValue::F32(1.),
        ] {
            assert_eq!(recorded_handle(Some(&value)), None);
        }
        assert_eq!(recorded_handle(None), None);
    }
}
