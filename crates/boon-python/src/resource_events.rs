use crate::*;

pub(super) struct Row {
    pub tick: i32,
    pub sequence: u64,
    pub steam_id: Option<u64>,
    pub controller_index: Option<i32>,
    pub source: Option<(i32, u32, String)>,
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
