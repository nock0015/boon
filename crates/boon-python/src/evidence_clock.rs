//! Exact on-screen match-clock declaration and source metadata admission.
//! The resolved conversion core is shared with Demo.tick_to_match_seconds.
use serde_json::{Value, json};

pub(crate) fn header() -> Value {
    json!({"schema":"boon-ability-usage-match-clock-v1","field":"source_game_time",
        "domain":"on-screen-match-seconds","conversion":"Demo.tick_to_match_seconds",
        "classPredicate":"contains-Ability-or-Item","unknown":"null"})
}
pub(crate) fn eligible(class: &str) -> bool {
    class.contains("Ability") || class.contains("Item")
}
pub(crate) fn resolved_match_seconds(
    active_ticks: i32,
    tick_rate: i32,
    pregame: Option<f64>,
) -> Option<f64> {
    let pregame = pregame.filter(|value| value.is_finite())?;
    if tick_rate == 0 {
        return None;
    }
    let value = active_ticks as f64 / tick_rate as f64 - pregame;
    value.is_finite().then_some(value)
}
pub(crate) fn validate_header(row: &Value) -> Result<bool, String> {
    match row.get("abilityUsageClock") {
        None => Ok(false),
        Some(value) if value == &header() => Ok(true),
        _ => Err("Unknown entity-evidence ability-usage clock".into()),
    }
}
pub(crate) fn validate_record(row: &Value, declared: bool) -> Result<(), String> {
    let has = row.get("source_game_time");
    let applies = row
        .get("entity_class")
        .and_then(Value::as_str)
        .is_some_and(eligible);
    if !declared || !applies {
        if has.is_some() {
            return Err("Undeclared or ineligible entity source game time".into());
        }
        return Ok(());
    }
    match has {
        Some(Value::Null) => Ok(()),
        Some(value) if value.as_f64().is_some_and(f64::is_finite) => Ok(()),
        _ => Err("Missing or invalid entity source game time".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_declaration_and_case_sensitive_eligibility_are_exact() {
        assert_eq!(header()["conversion"], "Demo.tick_to_match_seconds");
        assert!(validate_header(&json!({"abilityUsageClock":header()})).unwrap());
        assert!(!validate_header(&json!({})).unwrap());
        let mut h = header();
        h["domain"] = json!("simulation-seconds");
        assert!(validate_header(&json!({"abilityUsageClock":h})).is_err());
        for class in [
            "CCitadel_Ability_ZipLine",
            "CItemXP",
            "CCitadelItemPunchableNeutralGold",
        ] {
            assert!(eligible(class));
        }
        for class in ["CCitadelPlayerPawn", "CNPC_Trooper", "cabilityextra"] {
            assert!(!eligible(class));
        }
    }
    #[test]
    fn resolved_core_preserves_negative_pregame_and_pause_adjusted_time() {
        assert_eq!(resolved_match_seconds(0, 64, Some(30.)), Some(-30.));
        assert_eq!(resolved_match_seconds(1920, 64, Some(30.)), Some(0.));
        // count_active_ticks already subtracts exactly the recorded pause ticks.
        let pauses = [2, 3, 6];
        let tick = 10;
        let active = tick - pauses.iter().filter(|&&p| p < tick).count() as i32;
        assert_eq!(resolved_match_seconds(active, 2, Some(4.)), Some(-0.5));
        assert_eq!(resolved_match_seconds(100, 64, None), None);
        assert_eq!(resolved_match_seconds(100, 0, Some(4.)), None);
        assert_eq!(resolved_match_seconds(100, 64, Some(f64::NAN)), None);
    }
    #[test]
    fn explicit_null_finite_and_absent_records_share_the_same_contract() {
        for kind in ["state", "delta", "absent"] {
            let mut row =
                json!({"kind":kind,"entity_class":"CCitadel_Ability_Test","source_game_time":null});
            assert!(validate_record(&row, true).is_ok());
            assert!(validate_record(&row, false).is_err());
            row["source_game_time"] = json!(-0.5);
            assert!(validate_record(&row, true).is_ok());
            row["source_game_time"] = json!(true);
            assert!(validate_record(&row, true).is_err());
            row.as_object_mut().unwrap().remove("source_game_time");
            assert!(validate_record(&row, true).is_err());
        }
        assert!(validate_record(&json!({"entity_class":"CCitadelPlayerPawn"}), true).is_ok());
        assert!(
            validate_record(
                &json!({"entity_class":"CCitadelPlayerPawn","source_game_time":null}),
                true
            )
            .is_err()
        );
    }
}
