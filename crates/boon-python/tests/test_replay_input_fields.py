"""Replay-backed current upgrades and team minimap facts."""

import polars as pl
import pytest
from polars.testing import assert_frame_equal

from boon import Demo


def test_recorded_upgrade_vector_is_distinct_from_missing(demo: Demo) -> None:
    frame = demo.snapshots("player_ticks", every=640)
    assert frame.schema["upgrade_ids"] == pl.List(pl.UInt32)
    assert frame.schema["boon_count"] == pl.Int64
    known = frame.filter(pl.col("upgrade_ids").is_not_null())
    if known.is_empty():
        pytest.skip("fixture has no decodable controller upgrade vector")
    assert known["upgrade_ids"].list.len().max() <= 64


def test_enemy_minimap_uses_recorded_team_icons(demo: Demo) -> None:
    frame = demo.enemy_minimap_ticks(every=640)
    assert set(frame.columns) == {
        "tick", "observer_team", "target_team", "target_hero_id", "target_steam_id",
        "target_entity_index", "target_entity_serial", "fow_entity_index_raw",
        "visible_on_map", "map_x", "map_y", "tick_hidden_raw",
        "last_seen_tick", "last_seen_x", "last_seen_y",
    }
    if frame.is_empty():
        pytest.skip("fixture has no decoded enemy FOW rows")
    assert frame.filter(pl.col("observer_team") == pl.col("target_team")).is_empty()
    assert frame.filter(
        pl.col("target_entity_index").is_null() & pl.col("target_steam_id").is_not_null()
    ).is_empty()
    assert not frame.filter(~pl.col("visible_on_map").fill_null(False)).select(
        pl.col("map_x").is_not_null().any() | pl.col("map_y").is_not_null().any()
    ).item()
    shown = frame.filter(pl.col("visible_on_map").fill_null(False))
    assert shown.filter(
        (pl.col("map_x") < 0) | (pl.col("map_x") > 255)
        | (pl.col("map_y") < 0) | (pl.col("map_y") > 255)
    ).is_empty()
    assert frame.filter(pl.col("last_seen_tick") > pl.col("tick")).is_empty()


def test_enemy_minimap_steam_id_follows_unique_pawn_controller(demo: Demo) -> None:
    frame = demo.enemy_minimap_ticks(every=640)
    assert frame.schema["target_steam_id"] == pl.UInt64
    known = frame.filter(pl.col("target_steam_id").is_not_null())
    if known.is_empty():
        pytest.skip("fixture has no uniquely bound controller and enemy FOW row")
    roster = {
        row["steam_id"]: row["hero_id"]
        for row in demo.players.select("steam_id", "hero_id").to_dicts()
    }
    assert set(known["target_steam_id"].to_list()) <= set(roster)
    assert all(
        row["target_hero_id"] is None
        or roster[row["target_steam_id"]] == row["target_hero_id"]
        for row in known.select("target_steam_id", "target_hero_id").to_dicts()
    )
    by_pawn = known.group_by("target_entity_index", "target_entity_serial").agg(
        pl.col("target_steam_id").n_unique().alias("steam_ids")
    )
    assert by_pawn["steam_ids"].max() == 1


def test_selected_minimap_ticks_preserve_last_seen_history(demo: Demo) -> None:
    sampled = demo.enemy_minimap_ticks(every=640)
    ticks = sampled["tick"].unique().sort().to_list()[:3]
    if not ticks:
        pytest.skip("fixture has no decoded enemy FOW rows")
    selected = demo.enemy_minimap_ticks(ticks=ticks)
    assert set(selected["tick"].unique().to_list()) == set(ticks)
    # A selected query must still read prior visible rows when forming memory.
    expected = sampled.filter(pl.col("tick").is_in(ticks))
    assert_frame_equal(
        selected.sort(["tick", "observer_team", "fow_entity_index_raw"]),
        expected.sort(["tick", "observer_team", "fow_entity_index_raw"]),
    )


def test_existing_objective_rows_expose_mid_boss_health(demo: Demo) -> None:
    boss = demo.objectives.filter(pl.col("objective_type") == "mid_boss")
    assert demo.objectives.schema["entity_serial"] == pl.UInt32
    if boss.is_empty():
        pytest.skip("fixture has no observed mid-boss entity")
    assert boss.select(pl.col("max_health").min()).item() > 0
    assert boss.filter(pl.col("health") > pl.col("max_health")).is_empty()


def test_target_observations_keep_identity_metadata_and_null_hidden_payload(demo: Demo) -> None:
    frame = demo.target_observations(ticks=[1])
    assert set(frame.columns) == {
        "tick", "observer_team", "target_class", "fow_class_raw", "target_team",
        "target_entity_index", "target_entity_serial", "identity_known",
        "fow_entry_present", "shown", "map_x", "map_y", "raw_health_byte",
        "position_available", "health_byte_available", "observation_reason",
        "rift_disabled", "rift_enable_state", "capture_active_known",
        "capture_active", "rift_position_available", "rift_x", "rift_y", "rift_z",
    }
    assert frame.schema["target_entity_index"] == pl.Int32
    assert frame.schema["target_entity_serial"] == pl.UInt32
    assert frame.filter(~pl.col("identity_known")).is_empty()
    assert frame.filter(
        ~pl.col("shown").fill_null(False) & (
            pl.col("map_x").is_not_null()
            | pl.col("map_y").is_not_null()
            | pl.col("raw_health_byte").is_not_null()
        )
    ).is_empty()
    assert frame.filter(
        ~pl.col("position_available")
        & (pl.col("map_x").is_not_null() | pl.col("map_y").is_not_null())
    ).is_empty()
    assert frame.filter(
        ~pl.col("health_byte_available") & pl.col("raw_health_byte").is_not_null()
    ).is_empty()
    if not frame.is_empty():
        assert set(frame["target_class"].unique().to_list()) <= {
            "CNPC_TrooperBoss", "CNPC_Boss_Tier2", "CCitadel_KothCashIn"
        }


def test_target_observations_reject_ticks_outside_demo(demo: Demo) -> None:
    with pytest.raises(ValueError, match="within the demo tick range"):
        demo.target_observations(ticks=[demo.total_ticks + 1])
