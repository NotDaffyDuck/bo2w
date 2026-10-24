//! bo2mp: the maps' moving props. BO2 plays them from the client scripts
//! (client scripts do not run here; this is their part):
//! - `_fxanim.csc`: every map model named "fxanim" waits its `fxanim_wait`
//!   (or `randomfloatrange( fxanim_wait_min, fxanim_wait_max )`), after the
//!   level notify `fxanim_waittill_1` when it has one, then plays its scenes
//!   (`fxanim_scene_1..3`; "delete" and "hide" are not animations) in turn,
//!   each with its `fxanim_fx_1..5` bolted to `fxanim_fx_N_tag`; the next
//!   scene starts on `fxanim_waittill_2` / `_3` or when the scene ends. A
//!   model with `fxanim_parent` is held on that parent's `fxanim_tag`, and
//!   its parent gets one more start per child (fxanim_init appends it once
//!   for each). Maps whose client script skips this: mp_array, mp_cracked,
//!   mp_mountain, mp_nuked. Downhill's scene table is from its own compiled
//!   `mp_downhill_fx.csc` (in its zone; the reference scripts lack it).
//! - `mp_la_fx.csc:103`, `mp_socotra_fx.csc:83` and `mp_magma_fx.csc:118`
//!   (Magma's runs as its DLC pass ends): the "fxanim_level" wires
//!   start after `randomfloatrange( 0.1, 1.5 )` s at a speed of
//!   `randomfloatrange( 0.75, 1.4 )`, and every note of their animation
//!   sparks at the wire's end joint. `mp_bridge_fx.csc:106` does the same
//!   for its "fxanim" wires, after the fxanim pass has started them: the
//!   wire's `useanimtree` gives it a new tree (`CScr_UseAnimTree` always
//!   makes one), so that first start is dropped.
//! - `mp_drone_fx.csc:102`: the factory's robot arms ("drone_fxanim") play at
//!   once (the "_off" animations when the game type turns map scripting
//!   off); the welders spark at "tag_fx" on their weld notes; the drone's
//!   parts ("drone_linkto") ride the conveyor's joints where they stand.
//! - `mp_studio_fx.csc:124` (unless the game type turns map scripting
//!   off): at the first snapshot the pirates ride the boats' joints
//!   ("pirate_boats_link") playing their own loops, the T-rex's head and
//!   stand play one of their three moves together every
//!   `randomfloatrange( 10, 15 )` s (never the same twice running; the
//!   ribs ride the head's "body_base_jnt"), and the brontosaurus chews,
//!   looking around once every `randomfloatrange( 15, 20 )` s; each change
//!   blends over 0.5 s (`clearanim( old, 0.5 )` with
//!   `animscripted( new, 1.0, 0.5, 1.0 )`), the T-rex's moves over 0.05 s
//!   (`clearanim( old, 0.05 )` with `setanimrestart( new, 1.0, 0.0, 1.0 )`).
//! - `mp_castaway_fx.csc:99` (at the first snapshot): the three crabs
//!   ("fxanim_dlc3_crab_0N") play their two moves in turn, each held for
//!   its length and then `randomfloatrange( 15, 45 )` s.
//! - `mp_uplink_fx.csc:107`, `mp_frostbite_fx.csc:62` and
//!   `mp_takeoff_fx.csc:85` (at the first snapshot): the radar (Frost's
//!   crane) plays its five animations in turn, each followed by its own
//!   wait (`level.radar_waits`: five `randomfloatrange( 5, 10 )` s, drawn
//!   once); Uplink's shows four red beacons. Frost's runs its DLC pass
//!   twice, so each of those models starts twice. Takeoff's
//!   decontamination blasters steam at the nozzles their notes turn on
//!   (and stop on their "off" notes), its planets glow, and its blue lights
//!   (exploders 2001 and 2002) come on.
//! - `mp_vertigo_fx.csc:112` (at the first snapshot): each blind
//!   ("fxanim_level" with `fxanim_scene_1` "blinds_impact") waits for a
//!   pane to shatter within 64 units of it (`glass_smash`), then swings
//!   once and hangs in its idle.
//! - `mp_nuketown_2020.csc:58`: the end-game cars flip and the display glass
//!   breaks on the server's `nuke_car_flip`; a car's "glass_shatter" note is
//!   exploder 1005 and its "camera_goes_dark" the "blackout" vision. The
//!   sprinkler plays its loop sound (`mp_nuketown_2020_fx.csc:87`).
//! - A DLC map's `_fxanim_dlc.csc` (`_dlc3`, `_dlc4`) fxanim_init_dlc,
//!   threaded as fxanim_init ends: the same as fxanim_init for its own
//!   models ("fxanim_dlc", "fxanim_dlc3", "fxanim_dlc4") with the map's own
//!   animation tree.
//! A note "sndnt#<alias>" on any of these animations plays that sound at the
//! prop (the engine's CG_PlayClientSoundNoteTracks).
//!
//! Each map is added once its client script is checked (several DLC maps
//! have their own handlers on top).

use std::collections::HashMap;

use bevy_ecs::prelude::World;
use gsc_t6::{Key, Value};
use xmodel_runtime::DObjAnim as _;

use super::{Zm, frame, with_vm};

/// The maps whose props are played here (their client scripts checked).
pub(super) const MAPS: [&str; 31] = [
    "mp_la",
    "mp_dockside",
    "mp_carrier",
    "mp_drone",
    "mp_express",
    "mp_hijacked",
    "mp_meltdown",
    "mp_overflow",
    "mp_nightclub",
    "mp_raid",
    "mp_slums",
    "mp_village",
    "mp_turbine",
    "mp_socotra",
    "mp_nuketown_2020",
    "mp_skate",
    "mp_hydro",
    "mp_mirage",
    "mp_concert",
    "mp_dig",
    "mp_pod",
    "mp_paintball",
    "mp_magma",
    "mp_vertigo",
    "mp_bridge",
    "mp_studio",
    "mp_castaway",
    "mp_uplink",
    "mp_frostbite",
    "mp_takeoff",
    "mp_downhill",
];

/// A scene's animation: the map's `level.scr_anim["fxanim_props"]`.
fn scene_anim(map: &str, scene: &str) -> Option<&'static str> {
    Some(match (map, scene) {
        ("mp_bridge", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_bridge", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_bridge", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_bridge", "wirespark_long") => "fxanim_gp_wirespark_long_anim",
        ("mp_bridge", "wirespark_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_carrier", "towing_crane") => "fxanim_mp_carrier_towing_crane_anim",
        ("mp_carrier", "sparking_wires_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_castaway", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_castaway", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_castaway", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_castaway", "river_debris_barrel") => "fxanim_angola_river_debris_barrel_anim",
        ("mp_castaway", "shark_fins") => "fxanim_mp_stu_shark_fins_anim",
        ("mp_concert", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_concert", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_concert", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_concert", "london_eye") => "fxanim_mp_con_london_eye_anim",
        ("mp_dig", "roaches") => "fxanim_gp_roaches_anim",
        ("mp_dockside", "roofvent_rotate") => "fxanim_gp_roofvent_anim",
        ("mp_dockside", "buoy_fast") => "fxanim_gp_buoy_fast_anim",
        ("mp_dockside", "crane_wires") => "fxanim_mp_dockside_crane_wires_anim",
        ("mp_dockside", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_dockside", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_dockside", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_downhill", "wirespark_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_downhill", "teardrop_flag") => "fxanim_gp_teardrop_flag_loop_anim",
        ("mp_downhill", "windsock") => "fxanim_gp_windsock_anim",
        ("mp_downhill", "quad_chair") => "viewmodel_fxanim_mp_downhill_quad_chair_loop_anim",
        ("mp_downhill", "xmas_lights") => "fxanim_gp_xmas_lights_anim",
        ("mp_downhill", "standing_skiis") => "fxanim_mp_downhill_standing_skiis_loop_anim",
        ("mp_downhill", "standing_skiis_02") => "fxanim_mp_downhill_standing_skiis_loop_02_anim",
        ("mp_drone", "vines_strangler_fig") => "fxanim_gp_vines_strangler_fig_anim",
        ("mp_drone", "vines_aquilaria") => "fxanim_gp_vines_aquilaria_anim",
        ("mp_drone", "vines_strangler_fig_alt") => "fxanim_gp_vines_strangler_fig_alt_anim",
        ("mp_express", "banner_side_thin") => "fxanim_mp_express_banner_side_thin_anim",
        ("mp_frostbite", "laundry") => "fxanim_gp_dryer_loop_anim",
        ("mp_frostbite", "dock_chain") => "fxanim_mp_ver_stair_chain_sign_anim",
        ("mp_hijacked", "umbrella_01") => "fxanim_gp_umbrella_01_anim",
        ("mp_hijacked", "dryer") => "fxanim_gp_dryer_loop_anim",
        ("mp_hijacked", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_hijacked", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_hijacked", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_hydro", "wirespark_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_hydro", "wires_long") => "fxanim_mp_hydro_wires_long_anim",
        ("mp_hydro", "wires_med") => "fxanim_mp_hydro_wires_med_anim",
        ("mp_hydro", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_hydro", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_hydro", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_hydro", "wires_med_02") => "fxanim_mp_hydro_wires_med_02_anim",
        ("mp_hydro", "wires_long_02") => "fxanim_mp_hydro_wires_long_02_anim",
        ("mp_la", "sparking_wires_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_la", "hanging_wires") => "fxanim_mp_la_hanging_wires_anim",
        ("mp_la", "elevator_doors") => "fxanim_mp_la_elevator_doors_anim",
        ("mp_la", "clearance_pipe") => "fxanim_gp_garage_clearance_pipe_anim",
        ("mp_la", "sparking_wires_long") => "fxanim_gp_wirespark_long_anim",
        ("mp_magma", "sparking_wires_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_magma", "sparking_wires_long") => "fxanim_gp_wirespark_long_anim",
        ("mp_magma", "japan_sign_square") => "fxanim_mp_magma_japan_sign_square_anim",
        ("mp_magma", "squid_sign_eyes_01") => "fxanim_mp_magma_squid_sign_eyes_01_anim",
        ("mp_magma", "squid_sign_eyes_02") => "fxanim_mp_magma_squid_sign_eyes_02_anim",
        ("mp_magma", "japan_sign_fish") => "fxanim_mp_magma_japan_sign_fish_anim",
        ("mp_mirage", "lantern_lrg") => "fxanim_mp_mirage_lantern_lrg_anim",
        ("mp_mirage", "lantern_sm") => "fxanim_mp_mirage_lantern_sm_anim",
        ("mp_mirage", "lamp") => "fxanim_mp_mirage_lamp_anim",
        ("mp_mirage", "ruined_lanterns") => "fxanim_mp_mirage_lanterns_ruined_anim",
        ("mp_mirage", "lanterns_string") => "fxanim_mp_mirage_lanterns_string_anim",
        ("mp_mirage", "lantern_lrg_02") => "fxanim_mp_mirage_lantern_lrg_02_anim",
        ("mp_nightclub", "solar_system") => "fxanim_karma_solar_system_anim",
        ("mp_nightclub", "skylight") => "fxanim_gp_skylight_anim",
        ("mp_nightclub", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_nightclub", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_nightclub", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_nightclub", "roofvent") => "fxanim_gp_roofvent_anim",
        ("mp_nightclub", "shopping_lights_short") => "fxanim_gp_shopping_lights_short_anim",
        ("mp_nightclub", "shopping_lights_long") => "fxanim_gp_shopping_lights_long_anim",
        ("mp_nuketown_2020", "sprinkler") => "fxanim_mp_nuked2025_sprinkler_anim",
        ("mp_nuketown_2020", "dome") => "fxanim_mp_nuked2025_dome_anim",
        ("mp_nuketown_2020", "gate_sign") => "fxanim_mp_nuked2025_gate_sign_anim",
        ("mp_nuketown_2020", "carwash_hoses") => "fxanim_mp_nuked2025_carwash_hoses_anim",
        ("mp_overflow", "canopy08_lrg") => "fxanim_yemen_cloth_canopy08_lrg_anim",
        ("mp_overflow", "canopy04_lrg") => "fxanim_yemen_cloth_canopy04_lrg_anim",
        ("mp_overflow", "canopy07_lrg") => "fxanim_yemen_cloth_canopy07_lrg_anim",
        ("mp_overflow", "awning_store") => "fxanim_gp_awning_store_mideast_anim",
        ("mp_overflow", "canopy04") => "fxanim_yemen_cloth_canopy04_anim",
        ("mp_overflow", "canopy08") => "fxanim_yemen_cloth_canopy08_anim",
        ("mp_pod", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_pod", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_pod", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_pod", "sheet_med") => "fxanim_gp_cloth_sheet_med_anim",
        ("mp_raid", "umbrella") => "fxanim_gp_umbrella_01_anim",
        ("mp_raid", "dryer_loop") => "fxanim_gp_dryer_loop_anim",
        ("mp_skate", "air_dancer") => "fxanim_mp_skate_air_dancer_anim",
        ("mp_skate", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_skate", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_skate", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_skate", "ferris_wheel_spin") => "fxanim_mp_skate_ferris_wheel_spin_anim",
        ("mp_skate", "banner_01") => "fxanim_mp_skate_banner_01_anim",
        ("mp_skate", "teardrop_flag") => "fxanim_gp_teardrop_flag_loop_anim",
        ("mp_slums", "roofvent") => "fxanim_gp_roofvent_anim",
        ("mp_slums", "control_wire_sm") => "fxanim_gp_control_wire_sm_anim",
        ("mp_slums", "dryer_loop") => "fxanim_gp_dryer_loop_anim",
        ("mp_socotra", "ropes_hang") => "fxanim_gp_ropes_hang_01_anim",
        ("mp_socotra", "rope_toggle") => "fxanim_gp_rope_toggle_anim",
        ("mp_socotra", "rope_arch") => "fxanim_mp_socotra_rope_arch_anim",
        ("mp_socotra", "wire_coil") => "fxanim_gp_wire_coil_01_anim",
        ("mp_socotra", "wirespark_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_socotra", "wirespark_long") => "fxanim_gp_wirespark_long_anim",
        ("mp_socotra", "yemen_pent_long") => "fxanim_gp_flag_yemen_pent_long_anim",
        ("mp_socotra", "roaches") => "fxanim_gp_roaches_anim",
        ("mp_socotra", "cloth_sheet_med") => "fxanim_gp_cloth_sheet_med_anim",
        ("mp_studio", "roofvent") => "fxanim_gp_roofvent_anim",
        ("mp_studio", "roofvent_slow") => "fxanim_gp_roofvent_slow_anim",
        ("mp_studio", "dino_eggs") => "fxanim_mp_stu_dino_eggs_anim",
        ("mp_studio", "noose") => "fxanim_mp_stu_noose_anim",
        ("mp_studio", "t_rex_barn") => "fxanim_mp_stu_t_rex_barn_anim",
        ("mp_studio", "t_rex_fence_01") => "fxanim_mp_stu_t_rex_fence_01_anim",
        ("mp_studio", "t_rex_fence_02") => "fxanim_mp_stu_t_rex_fence_02_anim",
        ("mp_studio", "t_rex_fence_03") => "fxanim_mp_stu_t_rex_fence_03_anim",
        ("mp_studio", "robot_spider_01") => "fxanim_mp_stu_robot_01_anim",
        ("mp_studio", "robot_spider_02") => "fxanim_mp_stu_robot_02_anim",
        ("mp_studio", "robot_spider_03") => "fxanim_mp_stu_robot_03_anim",
        ("mp_studio", "shark_fins") => "fxanim_mp_stu_shark_fins_anim",
        ("mp_studio", "pirate_boats") => "fxanim_mp_stu_pirate_boats_anim",
        ("mp_studio", "brontosaurus_chew_anim") => "fxanim_mp_stu_brontosaurus_chew_anim",
        ("mp_studio", "brontosaurus_look_around") => "fxanim_mp_stu_brontosaurus_look_around_anim",
        ("mp_studio", "robot_scanning") => "fxanim_mp_stu_robot_scanning",
        ("mp_studio", "t_rex_stand_01") => "fxanim_mp_stu_t_rex_stand_01_anim",
        ("mp_studio", "t_rex_stand_02") => "fxanim_mp_stu_t_rex_stand_02_anim",
        ("mp_studio", "t_rex_stand_03") => "fxanim_mp_stu_t_rex_stand_03_anim",
        ("mp_studio", "captain_01") => "fxanim_mp_stu_pirate_captain_01_anim",
        ("mp_studio", "captain_02") => "fxanim_mp_stu_pirate_captain_02_anim",
        ("mp_studio", "oarsmen_01") => "fxanim_mp_stu_pirate_oarsmen_01_anim",
        ("mp_studio", "oarsmen_02") => "fxanim_mp_stu_pirate_oarsmen_02_anim",
        ("mp_studio", "captain_jailed") => "fxanim_mp_stu_pirate_jailed_captain_anim",
        ("mp_studio", "oarsmen_jailed") => "fxanim_mp_stu_pirate_jailed_oarsmen_anim",
        ("mp_takeoff", "seagull_circle_01") => "fxanim_gp_seagull_circle_01_anim",
        ("mp_takeoff", "seagull_circle_02") => "fxanim_gp_seagull_circle_02_anim",
        ("mp_takeoff", "seagull_circle_03") => "fxanim_gp_seagull_circle_03_anim",
        ("mp_takeoff", "windsock") => "fxanim_gp_windsock_anim",
        ("mp_turbine", "wires_02") => "fxanim_mp_turbine_wires_02_anim",
        ("mp_turbine", "wires_03") => "fxanim_mp_turbine_wires_03_anim",
        ("mp_turbine", "bridge_cables") => "fxanim_mp_turbine_bridge_cables_anim",
        ("mp_turbine", "wires_01") => "fxanim_mp_turbine_wires_01_anim",
        ("mp_uplink", "ant_rooftop") => "fxanim_gp_antenna_rooftop_anim",
        ("mp_vertigo", "wirespark_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_vertigo", "stair_chain") => "fxanim_mp_ver_stair_chain_sign_anim",
        ("mp_vertigo", "roofvent_modern") => "fxanim_gp_roofvent_slow_anim",
        ("mp_vertigo", "cell_antenna") => "fxanim_gp_cell_antenna_anim",
        ("mp_vertigo", "antenna_rooftop") => "fxanim_gp_antenna_rooftop_anim",
        ("mp_vertigo", "deck_antenna") => "fxanim_gp_deck_antenna_anim",
        ("mp_vertigo", "ant_rooftop_small") => "fxanim_gp_antenna_rooftop_small_anim",
        ("mp_vertigo", "sat_dish") => "fxanim_gp_satellite_dish_anim",
        ("mp_vertigo", "com_tower") => "fxanim_gp_communication_tower_anim",
        ("mp_village", "sparking_wires_med") => "fxanim_gp_wirespark_med_anim",
        ("mp_village", "windsock") => "fxanim_gp_windsock_anim",
        ("mp_village", "wire_coil_01") => "fxanim_gp_wire_coil_01_anim",
        ("mp_village", "sign_sway_lrg") => "fxanim_gp_sign_sway_lrg_anim",
        _ => return None,
    })
}

/// A DLC map's own pass (its `_fxanim_dlc*.csc` fxanim_init_dlc): the
/// models it takes, by targetname.
fn dlc_pass(map: &str) -> Option<&'static str> {
    Some(match map {
        "mp_concert" => "fxanim_dlc",
        "mp_magma" => "fxanim_dlc",
        "mp_vertigo" => "fxanim_dlc",
        "mp_dig" => "fxanim_dlc4",
        "mp_pod" => "fxanim_dlc4",
        "mp_paintball" => "fxanim_dlc3",
        "mp_bridge" => "fxanim_dlc3",
        "mp_studio" => "fxanim_dlc",
        "mp_castaway" => "fxanim_dlc3",
        "mp_uplink" => "fxanim_dlc3",
        "mp_frostbite" => "fxanim_dlc4",
        "mp_takeoff" => "fxanim_dlc4",
        _ => return None,
    })
}

/// A scene's animation in a DLC map's own tree
/// (`level.scr_anim["fxanim_props_dlc"]`, `_dlc3`, `_dlc4`: one a map).
fn dlc_scene_anim(map: &str, scene: &str) -> Option<&'static str> {
    Some(match (map, scene) {
        ("mp_bridge", "wires_01") => "fxanim_mp_bridge_wires_01_anim",
        ("mp_bridge", "wires_02") => "fxanim_mp_bridge_wires_02_anim",
        ("mp_bridge", "wires_03") => "fxanim_mp_bridge_wires_03_anim",
        ("mp_bridge", "truck_parts") => "viewmodel_fxanim_mp_bridge_truck_parts_anim",
        ("mp_bridge", "truck_wires") => "fxanim_mp_bridge_truck_wires_anim",
        ("mp_bridge", "control_wires") => "fxanim_mp_bridge_control_wires_anim",
        ("mp_bridge", "wires_billboard") => "fxanim_mp_bridge_wires_billboard_anim",
        ("mp_bridge", "wires_generator") => "fxanim_mp_bridge_wires_generator_anim",
        ("mp_bridge", "wires_road_hole") => "fxanim_mp_bridge_wires_road_hole_anim",
        ("mp_castaway", "speed_boat") => "viewmodel_fxanim_mp_cast_speed_boat_anim",
        ("mp_castaway", "zodiac_boats") => "viewmodel_fxanim_mp_cast_zodiac_boats_anim",
        ("mp_castaway", "palm_tree_float") => "viewmodel_fxanim_mp_cast_palm_tree_float_anim",
        ("mp_castaway", "fish_grp_01") => "fxanim_mp_cast_fish_grp_01_anim",
        ("mp_castaway", "floating_boxes") => "fxanim_mp_cast_floating_boxes_anim",
        ("mp_castaway", "raft_ropes") => "fxanim_mp_cast_raft_ropes_anim",
        ("mp_castaway", "floating_seaweed") => "fxanim_mp_cast_seaweed_anim",
        ("mp_castaway", "floating_boxes_fx") => "fxanim_mp_cast_floating_boxes_fx_anim",
        ("mp_castaway", "crab_blue_01_01") => "fxanim_mp_cast_crab_blue_01_01_anim",
        ("mp_castaway", "crab_blue_01_02") => "fxanim_mp_cast_crab_blue_01_02_anim",
        ("mp_castaway", "crab_blue_02_01") => "fxanim_mp_cast_crab_blue_02_01_anim",
        ("mp_castaway", "crab_blue_02_02") => "fxanim_mp_cast_crab_blue_02_02_anim",
        ("mp_castaway", "crab_blue_03_01") => "fxanim_mp_cast_crab_blue_03_01_anim",
        ("mp_castaway", "crab_blue_03_02") => "fxanim_mp_cast_crab_blue_03_02_anim",
        ("mp_castaway", "hammock") => "fxanim_mp_cast_hammock_anim",
        ("mp_castaway", "vine_cluster") => "fxanim_gp_vine_cluster_anim",
        ("mp_concert", "wires_stage_rear") => "fxanim_mp_con_wires_stage_rear_anim",
        ("mp_concert", "wires_stage_scaffold") => "fxanim_mp_con_wires_stage_scaffold_anim",
        ("mp_concert", "wires_seats_scaffold") => "fxanim_mp_con_wires_seats_scaffold_anim",
        ("mp_concert", "river_boats") => "fxanim_mp_con_boat_link_anim",
        ("mp_concert", "wires_stage_right") => "fxanim_mp_con_wires_stage_right_anim",
        ("mp_dig", "wires_yellow_01") => "fxanim_mp_dig_wires_yellow_01_anim",
        ("mp_dig", "wires_yellow_02") => "fxanim_mp_dig_wires_yellow_02_anim",
        ("mp_dig", "wires_yellow_03") => "fxanim_mp_dig_wires_yellow_03_anim",
        ("mp_dig", "ropes_01") => "fxanim_mp_dig_ropes_01_anim",
        ("mp_dig", "fuel_pipe") => "fxanim_mp_dig_fuel_pipe_anim",
        ("mp_dig", "wires_yellow_04") => "fxanim_mp_dig_wires_yellow_04_anim",
        ("mp_dig", "wires_yellow_05") => "fxanim_mp_dig_wires_yellow_05_anim",
        ("mp_dig", "wires_yellow_06") => "fxanim_mp_dig_wires_yellow_06_anim",
        ("mp_frostbite", "pennants_01") => "fxanim_mp_frost_pennants_01_anim",
        ("mp_frostbite", "pennants_02") => "fxanim_mp_frost_pennants_02_anim",
        ("mp_frostbite", "candy_sign") => "fxanim_mp_frost_candy_sign_anim",
        ("mp_frostbite", "crane01") => "fxanim_mp_frostbite_crane_01_anim",
        ("mp_frostbite", "crane02") => "fxanim_mp_frostbite_crane_02_anim",
        ("mp_frostbite", "crane03") => "fxanim_mp_frostbite_crane_03_anim",
        ("mp_frostbite", "crane04") => "fxanim_mp_frostbite_crane_04_anim",
        ("mp_frostbite", "crane05") => "fxanim_mp_frostbite_crane_05_anim",
        ("mp_frostbite", "river_ice") => "fxanim_mp_frost_ice_anim",
        ("mp_frostbite", "river_ice2") => "fxanim_mp_frost_ice_02_anim",
        ("mp_frostbite", "gate") => "fxanim_mp_frost_gate_anim",
        ("mp_magma", "hanging_lantern_01") => "fxanim_mp_magma_hanging_lantern_01_anim",
        ("mp_magma", "hanging_lantern_02") => "fxanim_mp_magma_hanging_lantern_02_anim",
        ("mp_magma", "train_wire_01") => "fxanim_mp_magma_train_wire_01_anim",
        ("mp_magma", "train_wire_02") => "fxanim_mp_magma_train_wire_02_anim",
        ("mp_magma", "train_wire_03") => "fxanim_mp_magma_train_wire_03_anim",
        ("mp_magma", "sushi_conveyor") => "fxanim_mp_magma_sushi_conveyor_anim",
        ("mp_paintball", "wires_01") => "fxanim_mp_paint_wires_01_anim",
        ("mp_paintball", "wires_02") => "fxanim_mp_paint_wires_02_anim",
        ("mp_paintball", "shop_banners") => "fxanim_paint_shop_banner_01_anim",
        ("mp_paintball", "shop_banners_wall") => "fxanim_paint_shop_banner_02_anim",
        ("mp_paintball", "wires_03") => "fxanim_mp_paint_wires_03_anim",
        ("mp_pod", "wire_01") => "fxanim_mp_pod_wire_01_anim",
        ("mp_pod", "wire_02") => "fxanim_mp_pod_wire_02_anim",
        ("mp_pod", "wire_03") => "fxanim_mp_pod_wire_03_anim",
        ("mp_pod", "wire_04") => "fxanim_mp_pod_wire_04_anim",
        ("mp_pod", "wire_05") => "fxanim_mp_pod_wire_05_anim",
        ("mp_pod", "wire_06") => "fxanim_mp_pod_wire_06_anim",
        ("mp_pod", "wire_07") => "fxanim_mp_pod_wire_07_anim",
        ("mp_pod", "wire_08") => "fxanim_mp_pod_wire_08_anim",
        ("mp_pod", "wire_09") => "fxanim_mp_pod_wire_09_anim",
        ("mp_pod", "shirt03") => "fxanim_gp_shirts03_anim",
        ("mp_pod", "vine_clump_01") => "fxanim_mp_pod_vine_clump_01_anim",
        ("mp_pod", "vine_clump_02") => "fxanim_mp_pod_vine_clump_02_anim",
        ("mp_pod", "vine_clump_long") => "fxanim_mp_pod_vine_clump_long_anim",
        ("mp_pod", "vine_med_leafy") => "fxanim_mp_pod_vine_med_leafy_anim",
        ("mp_pod", "vine_med_bare") => "fxanim_mp_pod_vine_med_bare_anim",
        ("mp_pod", "vine_long_leafy") => "fxanim_mp_pod_vine_long_leafy_anim",
        ("mp_pod", "vine_long_bare") => "fxanim_mp_pod_vine_long_bare_anim",
        ("mp_pod", "bushy_clump_01") => "fxanim_mp_pod_bushy_clump_01_anim",
        ("mp_pod", "bushy_clump_02") => "fxanim_mp_pod_bushy_clump_02_anim",
        ("mp_pod", "vine_loops_pod") => "fxanim_mp_pod_vine_loops_pod_anim",
        ("mp_pod", "pool_wires") => "fxanim_mp_pod_pool_wires_anim",
        ("mp_takeoff", "decont_blasters") => "fxanim_mp_takeoff_decont_blasters_anim",
        ("mp_takeoff", "scaffold_wires_01") => "fxanim_mp_takeoff_scaffold_wires_01_anim",
        ("mp_takeoff", "crane_hooks") => "fxanim_mp_takeoff_crane_hooks_anim",
        ("mp_takeoff", "rattling_sign") => "fxanim_mp_takeoff_rattling_sign_anim",
        ("mp_takeoff", "radar01") => "fxanim_mp_takeoff_satellite_dish_01_anim",
        ("mp_takeoff", "radar02") => "fxanim_mp_takeoff_satellite_dish_02_anim",
        ("mp_takeoff", "radar03") => "fxanim_mp_takeoff_satellite_dish_03_anim",
        ("mp_takeoff", "radar04") => "fxanim_mp_takeoff_satellite_dish_04_anim",
        ("mp_takeoff", "radar05") => "fxanim_mp_takeoff_satellite_dish_05_anim",
        ("mp_takeoff", "banners") => "fxanim_mp_takeoff_banner_01_anim",
        ("mp_takeoff", "planets") => "fxanim_mp_takeoff_planets_anim",
        ("mp_takeoff", "banners_lrg") => "fxanim_mp_takeoff_banner_lrg_anim",
        ("mp_uplink", "uplink_gate") => "fxanim_mp_uplink_gate_anim",
        ("mp_uplink", "uplink_gate_b") => "fxanim_mp_uplink_gate_b_anim",
        ("mp_uplink", "cliff_sign") => "fxanim_mp_uplink_cliff_sign_anim",
        ("mp_uplink", "radar01") => "fxanim_mp_uplink_vista_radar01_anim",
        ("mp_uplink", "radar02") => "fxanim_mp_uplink_vista_radar02_anim",
        ("mp_uplink", "radar03") => "fxanim_mp_uplink_vista_radar03_anim",
        ("mp_uplink", "radar04") => "fxanim_mp_uplink_vista_radar04_anim",
        ("mp_uplink", "radar05") => "fxanim_mp_uplink_vista_radar05_anim",
        ("mp_uplink", "sat_dish2") => "fxanim_gp_satellite_dish2_anim",
        ("mp_uplink", "ant_rooftop2_small") => "fxanim_gp_antenna_rooftop2_small_anim",
        ("mp_uplink", "fence01") => "fxanim_mp_uplink_fence01_anim",
        ("mp_uplink", "fence02") => "fxanim_mp_uplink_fence02_anim",
        ("mp_uplink", "vines_01") => "fxanim_mp_uplink_vines_01_anim",
        ("mp_uplink", "vines_02") => "fxanim_mp_uplink_vines_02_anim",
        ("mp_vertigo", "hook") => "fxanim_mp_ver_hook_anim",
        ("mp_vertigo", "blinds_impact") => "fxanim_gp_blinds_long_impact_anim",
        ("mp_vertigo", "blinds_idle") => "fxanim_gp_blinds_long_idle_anim",
        _ => return None,
    })
}

/// An `fxanim_fx_N` effect name's file (the map's `level._effect`).
fn effect(name: &str) -> Option<&'static str> {
    Some(match name {
        "fx_light_buoy_red_blink" => "light/fx_light_buoy_red_blink",
        "fx_mp_castaway_crate_fire" => "maps/mp_maps/fx_mp_castaway_crate_fire",
        "fx_mp_nightclub_spotlight" => "maps/mp_maps/fx_mp_nightclub_spotlight",
        "fx_mp_studio_robot_laser" => "maps/mp_maps/fx_mp_studio_robot_laser",
        "fx_water_fire_sprinkler_thin" => "water/fx_water_fire_sprinkler_thin",
        _ => return None,
    })
}

const WIRE_FX: &str = "maps/mp_maps/fx_mp_elec_spark_burst_xsm_thin";
const WELD_FX: &str = "maps/mp_maps/fx_mp_drone_robot_sparks";
/// drone_animate_fx's notes that spark.
const WELD_NOTES: [&str; 18] = [
    "seam_weld_01_start",
    "seam_weld_01_stop",
    "seam_weld_02_start",
    "seam_weld_02_stop",
    "seam_weld_03_start",
    "seam_weld_03_stop",
    "seam_weld_04_start",
    "seam_weld_04_stop",
    "tack_weld_01",
    "tack_weld_02",
    "tack_weld_03",
    "tack_weld_04",
    "tack_weld_05",
    "tack_weld_06",
    "tack_weld_07",
    "tack_weld_08",
    "tack_weld_09",
    "tack_weld_10",
];
/// Nuketown's end-game cars and glass (mp_nuketown_2020_fx.csc:65).
pub(super) const NUKED_CAR_ANIMS: [&str; 2] =
    ["fxanim_mp_nuked2025_car01_anim", "fxanim_mp_nuked2025_car02_anim"];
pub(super) const NUKED_GLASS_ANIM: &str = "fxanim_mp_nuked2025_display_glass_anim";

#[derive(Clone, Debug)]
enum Kind {
    Scene,
    /// The joint its spark comes from (every note sparks).
    Wire(&'static str),
    Welder,
    /// Nuketown's end-game car: its notes break the glass and black the
    /// screen out (`dark`: done, it stops listening).
    Car { dark: bool },
    /// Takeoff's decontamination blasters: their notes steam at the nozzles.
    Nozzles,
}

#[derive(Clone, Debug)]
struct Anim {
    clip: String,
    len: i64,
    looping: bool,
    notes: Vec<(String, f32)>,
}

#[derive(Clone, Debug)]
enum Scene {
    Delete,
    Hide,
    /// Its animation (none when the map has no such scene or the animation
    /// is not loaded: BO2 prints an error and plays nothing).
    Play(Option<Anim>),
}

/// A script thread's start: its wait (ms) and when it is due (`None` while
/// it waits for `fxanim_waittill_1`).
#[derive(Clone, Debug)]
struct Start {
    wait: i64,
    due: Option<i64>,
    done: bool,
}

/// The animation a prop is leaving while the next one blends in (a map
/// script's `clearanim( old, out )` then the new animation with blend time
/// `in`): its start, when the change came, and the two blend times (ms).
#[derive(Clone, Debug)]
struct Fade {
    anim: Anim,
    start: i64,
    at: i64,
    out_ms: i64,
    in_ms: i64,
}

#[derive(Clone, Debug)]
struct Prop {
    n: u32,
    kind: Kind,
    scenes: Vec<Scene>,
    /// The level notifies `fxanim_waittill_1..3`.
    until: [Option<String>; 3],
    /// Its effects: (file, tag).
    fx: Vec<(&'static str, String)>,
    speed: f32,
    starts: Vec<Start>,
    cur: usize,
    start: Option<i64>,
    /// How far its notes are fired, in passes of its animation.
    fired: f64,
    /// The level notify the next scene waits for.
    waiting: Option<String>,
    /// What shows when it starts (`fxanim_hide`: itself, its children).
    reveal: Vec<u32>,
    /// The animation it is blending out of.
    fade: Option<Fade>,
}

impl Prop {
    fn new(n: u32, kind: Kind, scenes: Vec<Scene>, starts: Vec<Start>) -> Prop {
        Prop {
            n,
            kind,
            scenes,
            until: Default::default(),
            fx: Vec::new(),
            speed: 1.0,
            starts,
            cur: 0,
            start: None,
            fired: 0.0,
            waiting: None,
            reveal: Vec::new(),
            fade: None,
        }
    }
}

/// A model held on its parent's joint.
#[derive(Clone, Debug)]
struct Child {
    n: u32,
    parent: u32,
    bone: String,
    /// The drone's parts: where they stand (their `linkto` keeps it).
    own: Option<glam::Mat4>,
    /// From the joint to the model (`fxanim_parent`: none, it is moved onto
    /// the joint).
    offset: glam::Mat4,
}

enum Out {
    Fx(u32, &'static str, String),
    Sound(u32, String),
    Show(u32),
    Hide(u32),
    Exploder(i32),
    /// A client script's `exploder( num )`.
    ExploderNum(i32),
    StopFx(u32, &'static str, String),
    Dark,
}

/// A map script's own loop on one model (Cove's crabs, the radars and the
/// crane of Uplink, Frost and Takeoff): its animations in turn, each held
/// for its length and then a wait.
#[derive(Clone, Debug)]
struct Turns {
    n: u32,
    anims: Vec<Option<Anim>>,
    wait: Wait,
    cur: usize,
    due: i64,
}

#[derive(Clone, Debug)]
enum Wait {
    /// `randomfloatrange( lo, hi )` s, drawn as each animation starts.
    Drawn(f32, f32),
    /// Each animation's own (ms), drawn once (`level.radar_waits`).
    Fixed(Vec<i64>),
}

/// Studio's dinosaurs (mp_studio_fx.csc `t_rex_animate_fx`,
/// `brontosaurus_animate_fx`): what plays next, and when.
#[derive(Default)]
struct Dinos {
    /// The T-rex's head and stand, and their three moves
    /// (`t_rex_fence_0N`, `t_rex_stand_0N`).
    trex: Option<([u32; 2], [Vec<Option<Anim>>; 2])>,
    trex_cur: Option<usize>,
    trex_due: i64,
    /// The brontosaurus, its chew and its look around.
    bronto: Option<(u32, Option<Anim>, Option<Anim>)>,
    looking: bool,
    bronto_due: i64,
}

#[derive(Default)]
pub(crate) struct MpFxProps {
    found: bool,
    props: Vec<Prop>,
    children: Vec<Child>,
    dinos: Dinos,
    turns: Vec<Turns>,
    /// Vertigo's blinds still waiting for glass to break near them.
    blinds: Vec<(u32, [f32; 3])>,
    /// The parents' skeletons (built once their models are up).
    dobjs: HashMap<u32, xmodel_runtime::DObj>,
    /// What a level notify started, played on the next tick.
    pending: Vec<Out>,
    seed: u64,
    /// Where the props that move by root motion started (origin, angles).
    homes: HashMap<u32, ([f32; 3], [f32; 3])>,
}

impl MpFxProps {
    fn rand(&mut self) -> u64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.seed >> 33
    }
    /// `randomfloatrange( lo, hi )`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (self.rand() as f32 / (1u64 << 31) as f32)
    }
}

fn anim(zm: &Zm, clip: &str) -> Option<Anim> {
    let a = zm.anims.get(clip)?;
    let len = ((f32::from(a.numframes) / a.framerate.max(1.0)) * 1000.0).round() as i64;
    Some(Anim {
        clip: clip.to_owned(),
        len: len.max(1),
        looping: a.looping,
        notes: a.notifies.clone(),
    })
}

/// The KVPs fxanim_init and the map scripts read.
const KEYS: [&str; 25] = [
    "targetname",
    "fxanim_scene_1",
    "fxanim_scene_2",
    "fxanim_scene_3",
    "fxanim_wait",
    "fxanim_wait_min",
    "fxanim_wait_max",
    "fxanim_waittill_1",
    "fxanim_waittill_2",
    "fxanim_waittill_3",
    "fxanim_fx_1",
    "fxanim_fx_1_tag",
    "fxanim_fx_2",
    "fxanim_fx_2_tag",
    "fxanim_fx_3",
    "fxanim_fx_3_tag",
    "fxanim_fx_4",
    "fxanim_fx_4_tag",
    "fxanim_fx_5",
    "fxanim_fx_5_tag",
    "fxanim_hide",
    "fxanim_parent",
    "fxanim_tag",
    "script_animation",
    "script_noteworthy",
];
// Each key's place above.
const TN: usize = 0;
const SCENE: usize = 1;
const WAIT: usize = 4;
const UNTIL: usize = 7;
const FX: usize = 10;
const HIDE: usize = 20;
const PARENT: usize = 21;
const TAG: usize = 22;
const ANIM: usize = 23;
const NOTEWORTHY: usize = 24;

struct Row {
    n: u32,
    model: String,
    origin: [f32; 3],
    angles: [f32; 3],
    /// Each key's value (`None` when not defined).
    kv: Vec<Option<String>>,
}

impl Row {
    fn get(&self, i: usize) -> Option<&str> {
        self.kv[i].as_deref()
    }
    fn num(&self, i: usize) -> Option<f32> {
        self.get(i).and_then(|v| v.trim().parse().ok())
    }
}

/// What the props found missing: animations not loaded, names the map's
/// script does not have.
#[derive(Default)]
struct Missing {
    anims: Vec<String>,
    names: Vec<String>,
}

/// A scene's animation from the map's own tree, for a map script's own
/// `setanimrestart` / `animscripted`.
/// `dlc`: from its DLC tree.
fn anim_of(zm: &Zm, map: &str, name: &str, dlc: bool, miss: &mut Missing) -> Option<Anim> {
    match scene_of(zm, map, name, dlc, miss) {
        Scene::Play(a) => a,
        Scene::Delete | Scene::Hide => None,
    }
}

/// `dlc`: from the map's DLC tree (its fxanim_init_dlc pass).
fn scene_of(zm: &Zm, map: &str, name: &str, dlc: bool, miss: &mut Missing) -> Scene {
    let clip = if dlc { dlc_scene_anim(map, name) } else { scene_anim(map, name) };
    match name {
        "delete" => Scene::Delete,
        "hide" => Scene::Hide,
        _ => Scene::Play(match clip {
            Some(clip) => {
                let a = anim(zm, clip);
                if a.is_none() {
                    miss.anims.push(clip.to_owned());
                }
                a
            }
            None => {
                miss.names.push(name.to_owned());
                None
            }
        }),
    }
}

fn find(world: &mut World) {
    let map = world.resource::<Zm>().map.clone();
    let mut fx = std::mem::take(&mut world.resource_mut::<Zm>().mp_fxprops);
    fx.found = true;
    fx.seed = 0x5eed_f00d;
    if !MAPS.contains(&map.as_str()) {
        world.resource_mut::<Zm>().mp_fxprops = fx;
        return;
    }
    let rows: Vec<Row> = with_vm(world, |vm, world| {
        let keys: Vec<_> = KEYS.iter().map(|k| vm.intern(k)).collect();
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(_, e)| e.map && e.classname == "script_model")
            .filter_map(|(n, e)| {
                let o = e.obj?;
                let kv = keys
                    .iter()
                    .map(|&k| {
                        let v = vm.raw_field(o, k);
                        (!v.is_undefined()).then(|| vm.to_text(&v))
                    })
                    .collect();
                Some(Row { n: *n, model: e.model.clone(), origin: e.origin, angles: e.angles, kv })
            })
            .collect()
    })
    .unwrap_or_default();
    let by_name = |name: &str| rows.iter().find(|r| r.get(TN) == Some(name)).map(|r| r.n);
    let now = world.resource::<Zm>().now_ms;
    let mut props: Vec<Prop> = Vec::new();
    let mut children: Vec<Child> = Vec::new();
    let mut hidden: Vec<u32> = Vec::new();
    let mut miss = Missing::default();
    let mut sprinklers: Vec<u32> = Vec::new();
    {
        let zm = world.resource::<Zm>();
        // fxanim_init: each "fxanim" model; a child adds a start to its
        // parent (appended after the others, in order). Then a DLC map's
        // fxanim_init_dlc (`level.fx_anim_level_init`, threaded as
        // fxanim_init ends) does the same for its own models, with its own
        // tree (a parent it adds plays from that tree too).
        // Frost's fxanim_init runs its DLC pass twice (at once, and again at
        // the first snapshot): each of its models gets a second start.
        let twice = if map == "mp_frostbite" { 2 } else { 1 };
        let passes = std::iter::once(("fxanim", false))
            .chain(dlc_pass(&map).into_iter().flat_map(|tn| std::iter::repeat_n((tn, true), twice)));
        for (tn, dlc) in passes {
            let mut order: Vec<u32> = Vec::new();
            let mut extra: Vec<u32> = Vec::new();
            for r in rows.iter().filter(|r| r.get(TN) == Some(tn)) {
                match r.get(PARENT) {
                    Some(p) => match by_name(p) {
                        Some(parent) => {
                            extra.push(parent);
                            if !children.iter().any(|c| c.n == r.n) {
                                children.push(Child {
                                    n: r.n,
                                    parent,
                                    bone: r.get(TAG).unwrap_or("").to_owned(),
                                    own: None,
                                    offset: glam::Mat4::IDENTITY,
                                });
                            }
                        }
                        None => miss.names.push(format!("parent {p}")),
                    },
                    None => order.push(r.n),
                }
            }
            for n in order.into_iter().chain(extra) {
                let Some(r) = rows.iter().find(|r| r.n == n) else { continue };
                // `_fxanim_wait`: each start draws its own wait.
                let wait = match (r.num(WAIT), r.num(WAIT + 1), r.num(WAIT + 2)) {
                    (Some(w), _, _) => w,
                    (None, Some(lo), Some(hi)) => fx.range(lo, hi),
                    _ => 0.0,
                };
                let wait = (wait * 1000.0) as i64;
                let due = r.get(UNTIL).is_none().then_some(now + wait);
                let start = Start { wait, due, done: false };
                if let Some(p) = props.iter_mut().find(|p| p.n == n) {
                    p.starts.push(start);
                    continue;
                }
                // `_fxanim_get_anim_count`: 1 without scene_2, 2 without scene_3.
                let count = if r.get(SCENE + 1).is_none() {
                    1
                } else if r.get(SCENE + 2).is_none() {
                    2
                } else {
                    3
                };
                let scenes = (0..count)
                    .map(|i| scene_of(zm, &map, r.get(SCENE + i).unwrap_or(""), dlc, &mut miss))
                    .collect();
                let mut p = Prop::new(n, Kind::Scene, scenes, vec![start]);
                p.until = std::array::from_fn(|i| r.get(UNTIL + i).map(str::to_owned));
                for i in 0..5 {
                    let Some(name) = r.get(FX + 2 * i) else { continue };
                    match effect(name) {
                        Some(file) => p.fx.push((file, r.get(FX + 2 * i + 1).unwrap_or("").to_owned())),
                        None => miss.names.push(format!("effect {name}")),
                    }
                }
                // `_fxanim_hide`: hidden until it starts while the key is true.
                if r.num(HIDE).is_some_and(|v| v != 0.0) {
                    hidden.push(n);
                    p.reveal.push(n);
                }
                props.push(p);
            }
        }
        // `_fxanim_model_link`: a child with `fxanim_hide` defined shows
        // when its parent starts.
        for c in &children {
            let Some(r) = rows.iter().find(|r| r.n == c.n) else { continue };
            if r.get(HIDE).is_some()
                && let Some(p) = props.iter_mut().find(|p| p.n == c.parent)
            {
                hidden.push(c.n);
                p.reveal.push(c.n);
            }
        }
        // mp_la_fx.csc / mp_socotra_fx.csc fxanim_init, mp_magma_fx.csc /
        // mp_bridge_fx.csc fxanim_level_init (`level.fx_anim_level_dlc_init`):
        // the wires (every wire's wait and speed are drawn, then the known
        // ones play). Detour's takes the "fxanim" models.
        if map == "mp_la" || map == "mp_socotra" || map == "mp_magma" || map == "mp_bridge" {
            let tn = if map == "mp_bridge" { "fxanim" } else { "fxanim_level" };
            let wires: Vec<&Row> = rows.iter().filter(|r| r.get(TN) == Some(tn)).collect();
            let draws: Vec<(f32, f32)> =
                wires.iter().map(|_| (fx.range(0.1, 1.5), fx.range(0.75, 1.4))).collect();
            for (r, (wait, speed)) in wires.into_iter().zip(draws) {
                let scene = r.get(SCENE).unwrap_or("");
                let bone = match (map.as_str(), scene) {
                    (_, "sparking_wires_med") | ("mp_socotra" | "mp_bridge", "wirespark_med") => "med_spark_06_jnt",
                    (_, "sparking_wires_long") | ("mp_socotra" | "mp_bridge", "wirespark_long") => "long_spark_06_jnt",
                    // Its switch has no case: the wire is not animated.
                    _ => continue,
                };
                let wait = (wait * 1000.0) as i64;
                let scenes = vec![scene_of(zm, &map, scene, false, &mut miss)];
                let start = Start { wait, due: Some(now + wait), done: false };
                let mut p = Prop::new(r.n, Kind::Wire(bone), scenes, vec![start]);
                p.speed = speed;
                // Detour: its new tree drops the fxanim pass's start.
                props.retain(|q| q.n != r.n);
                props.push(p);
            }
        }
        // mp_drone_fx.csc fxanim_init: the factory.
        if map == "mp_drone" {
            let allow =
                super::natives_game::mp_setting(&zm.gamesettings, &zm.gamesettings_set, "allowMapScripting");
            let on = match &allow {
                Value::Int(i) => *i != 0,
                Value::Float(f) => *f != 0.0,
                _ => false,
            };
            diag::info!(
                Sim,
                "bo2mp mp_drone: allowMapScripting {allow:?} (robot arms play their {} animations)",
                if on { "working" } else { "_off" }
            );
            for r in rows.iter().filter(|r| r.get(TN) == Some("drone_fxanim")) {
                if r.model == "fxanim_mp_drone_factory_link_mod" {
                    // drone_link: each part keeps where it stands on its joint.
                    for c in rows.iter().filter(|c| c.get(TN) == Some("drone_linkto")) {
                        children.push(Child {
                            n: c.n,
                            parent: r.n,
                            bone: c.get(NOTEWORTHY).unwrap_or("").to_owned(),
                            own: Some(placement(c.origin, c.angles)),
                            offset: glam::Mat4::IDENTITY,
                        });
                    }
                }
                let Some(sa) = r.get(ANIM) else { continue };
                // `level.drone_anims[sa + "_off"]` is the "_off_anim" clip.
                let clip = if on {
                    sa.to_owned()
                } else {
                    format!("{}_off_anim", sa.strip_suffix("_anim").unwrap_or(sa))
                };
                let a = anim(zm, &clip);
                if a.is_none() {
                    miss.anims.push(clip);
                }
                let kind = if sa.contains("drone_factory_welder") { Kind::Welder } else { Kind::Scene };
                let start = Start { wait: 0, due: Some(now), done: false };
                props.push(Prop::new(r.n, kind, vec![Scene::Play(a)], vec![start]));
            }
        }
        // mp_studio_fx.csc fxanim_init: the pirates and dinosaurs, at the
        // first snapshot.
        if map == "mp_studio" {
            let allow =
                super::natives_game::mp_setting(&zm.gamesettings, &zm.gamesettings_set, "allowMapScripting");
            let on = match &allow {
                Value::Int(i) => *i != 0,
                Value::Float(f) => *f != 0.0,
                _ => false,
            };
            diag::info!(
                Sim,
                "bo2mp mp_studio: allowMapScripting {allow:?} (pirates and dinosaurs {})",
                if on { "play" } else { "stay still" }
            );
            if on {
                // pirate_animate_fx: moved onto the boat's joint, linked,
                // then its own loop (`setanimrestart`).
                let boats = by_name("pirate_boats_link");
                for (tn, bone, scene) in [
                    ("pirate_captain_1", "boat_01_jnt", "captain_01"),
                    ("pirate_captain_2", "boat_02_jnt", "captain_02"),
                    ("pirate_oarsman_1", "boat_01_jnt", "oarsmen_01"),
                    ("pirate_oarsman_2", "boat_02_jnt", "oarsmen_02"),
                ] {
                    let (Some(n), Some(parent)) = (by_name(tn), boats) else {
                        miss.names.push(tn.to_owned());
                        continue;
                    };
                    children.push(Child {
                        n,
                        parent,
                        bone: bone.to_owned(),
                        own: None,
                        offset: glam::Mat4::IDENTITY,
                    });
                    let a = anim_of(zm, &map, scene, false, &mut miss);
                    let start = Start { wait: 0, due: Some(now), done: false };
                    props.push(Prop::new(n, Kind::Scene, vec![Scene::Play(a)], vec![start]));
                }
                // t_rex_animate_fx: still until its first move; the ribs
                // ride the head.
                match (by_name("t_rex_fence"), by_name("t_rex_stand")) {
                    (Some(head), Some(stand)) => {
                        let mut moves: [Vec<Option<Anim>>; 2] = Default::default();
                        for i in 1..=3 {
                            moves[0].push(anim_of(zm, &map, &format!("t_rex_fence_0{i}"), false, &mut miss));
                            moves[1].push(anim_of(zm, &map, &format!("t_rex_stand_0{i}"), false, &mut miss));
                        }
                        for n in [head, stand] {
                            props.push(Prop::new(n, Kind::Scene, vec![Scene::Play(None)], Vec::new()));
                        }
                        fx.dinos.trex = Some(([head, stand], moves));
                        fx.dinos.trex_due = now + (fx.range(10.0, 15.0) * 1000.0) as i64;
                        match by_name("trex_fence_ribs") {
                            Some(ribs) => children.push(Child {
                                n: ribs,
                                parent: head,
                                bone: "body_base_jnt".to_owned(),
                                own: None,
                                offset: glam::Mat4::IDENTITY,
                            }),
                            None => miss.names.push("trex_fence_ribs".to_owned()),
                        }
                    }
                    _ => miss.names.push("t_rex_fence / t_rex_stand".to_owned()),
                }
                // brontosaurus_animate_fx: chewing from the start.
                match by_name("brontosaurus_head") {
                    Some(n) => {
                        let chew = anim_of(zm, &map, "brontosaurus_chew_anim", false, &mut miss);
                        let look = anim_of(zm, &map, "brontosaurus_look_around", false, &mut miss);
                        let start = Start { wait: 0, due: Some(now), done: false };
                        props.push(Prop::new(n, Kind::Scene, vec![Scene::Play(chew.clone())], vec![start]));
                        fx.dinos.bronto = Some((n, chew, look));
                        fx.dinos.bronto_due = now + (fx.range(15.0, 20.0) * 1000.0) as i64;
                    }
                    None => miss.names.push("brontosaurus_head".to_owned()),
                }
            }
        }
        // mp_castaway_fx.csc fxanim_init: the crabs.
        if map == "mp_castaway" {
            for i in 1..=3 {
                let Some(n) = by_name(&format!("fxanim_dlc3_crab_0{i}")) else { continue };
                let anims = (1..=2)
                    .map(|k| anim_of(zm, &map, &format!("crab_blue_0{i}_0{k}"), true, &mut miss))
                    .collect();
                add_turns(&mut fx, &mut props, n, anims, Wait::Drawn(15.0, 45.0), now);
            }
        }
        // mp_uplink_fx.csc / mp_frostbite_fx.csc / mp_takeoff_fx.csc
        // fxanim_init: the radar (Frost's crane).
        let radar = match map.as_str() {
            "mp_uplink" => Some(("fxanim_dlc3_radar", "radar0")),
            "mp_frostbite" => Some(("fxanim_dlc4_crane", "crane0")),
            "mp_takeoff" => Some(("fxanim_dlc4_radar", "radar0")),
            _ => None,
        };
        if let Some((tn, scene)) = radar {
            match by_name(tn) {
                Some(n) => {
                    let waits = (0..5).map(|_| (fx.range(5.0, 10.0) * 1000.0) as i64).collect();
                    let anims =
                        (1..=5).map(|i| anim_of(zm, &map, &format!("{scene}{i}"), true, &mut miss)).collect();
                    if map == "mp_uplink" {
                        for i in 1..=4 {
                            fx.pending.push(Out::Fx(n, BEACON_FX, format!("fx_link_0{i}_jnt")));
                        }
                    }
                    add_turns(&mut fx, &mut props, n, anims, Wait::Fixed(waits), now);
                }
                None => miss.names.push(tn.to_owned()),
            }
        }
        if map == "mp_takeoff" {
            // fxanim_decontamination_think: the blasters play
            // (`animflaggedscripted`), steaming as their notes say.
            match by_name("fxanim_dlc4_blasters") {
                Some(n) => {
                    let a = anim_of(zm, &map, "decont_blasters", true, &mut miss);
                    let start = Start { wait: 0, due: Some(now), done: false };
                    props.push(Prop::new(n, Kind::Nozzles, vec![Scene::Play(a)], vec![start]));
                }
                None => miss.names.push("fxanim_dlc4_blasters".to_owned()),
            }
            // fxanim_planets_think: each planet's glow.
            for r in rows
                .iter()
                .filter(|r| r.get(TN) == Some("fxanim_dlc4") && r.model == "fxanim_mp_takeoff_planets_mod")
            {
                for (file, tag) in PLANETS {
                    fx.pending.push(Out::Fx(r.n, file, tag.to_owned()));
                }
            }
            // playexploderonstart: the blue lights.
            fx.pending.push(Out::ExploderNum(2001));
            fx.pending.push(Out::ExploderNum(2002));
        }
        // mp_vertigo_fx.csc fxanim_init: the blinds, still until glass
        // breaks near them.
        if map == "mp_vertigo" {
            for r in rows
                .iter()
                .filter(|r| r.get(TN) == Some("fxanim_level") && r.get(SCENE) == Some("blinds_impact"))
            {
                let impact = anim_of(zm, &map, "blinds_impact", true, &mut miss);
                let idle = anim_of(zm, &map, "blinds_idle", true, &mut miss);
                props.push(Prop::new(r.n, Kind::Scene, vec![Scene::Play(impact), Scene::Play(idle)], Vec::new()));
                fx.blinds.push((r.n, r.origin));
            }
        }
        // mp_nuketown_2020_fx.csc fxanim_init: the sprinkler's sound.
        if map == "mp_nuketown_2020" {
            sprinklers = rows
                .iter()
                .filter(|r| r.get(TN) == Some("fxanim") && r.model == "fxanim_mp_nuked2025_sprinkler_mod")
                .map(|r| r.n)
                .collect();
        }
    }
    miss.anims.sort_unstable();
    miss.anims.dedup();
    miss.names.sort_unstable();
    miss.names.dedup();
    diag::info!(
        Sim,
        "bo2mp moving props on {map}: {} ({} wires, {} welders, {} held on a parent, {} loop sounds){}{}",
        props.len(),
        props.iter().filter(|p| matches!(p.kind, Kind::Wire(_))).count(),
        props.iter().filter(|p| matches!(p.kind, Kind::Welder)).count(),
        children.len(),
        sprinklers.len(),
        if miss.anims.is_empty() { String::new() } else { format!("; animations not loaded: {:?}", miss.anims) },
        if miss.names.is_empty() { String::new() } else { format!("; not in the map's script: {:?}", miss.names) },
    );
    fx.props = props;
    fx.children = children;
    let mut zm = world.resource_mut::<Zm>();
    for n in hidden {
        if let Some(e) = zm.ents.get_mut(&n) {
            e.hidden = true;
        }
    }
    for n in sprinklers {
        if let Some(e) = zm.ents.get_mut(&n) {
            e.loop_sound = Some("amb_track_sprinkler".to_owned());
        }
    }
    zm.mp_fxprops = fx;
}

/// `animscripted` (`_fxanim.csc:157`) carries a prop along its animation's
/// root motion: Black Ops II's CG_CalcEntityScriptedAnimTransform sets it at
/// its start place moved by XAnimCalcAbsDelta's translation, turned with
/// the start angles (Nuketown's greenhouse sprinkler rides its ceiling track
/// this way; it stood still without it). Not shown: the root motion's turn
/// (Black Ops II's delta quaternions are not read yet).
fn root_motion(world: &mut World, now: i64) {
    let moves: Vec<(u32, [f32; 3])> = {
        let mut zm = world.resource_mut::<Zm>();
        let zm = &mut *zm;
        let fx = &mut zm.mp_fxprops;
        let mut moves = Vec::new();
        for p in &fx.props {
            let Some(start) = p.start else { continue };
            let Scene::Play(Some(a)) = &p.scenes[p.cur] else { continue };
            let Some(t6) = zm.anims.get(&a.clip) else { continue };
            if t6.delta_trans.len() < 2 {
                continue;
            }
            let (home, angles) = match fx.homes.get(&p.n) {
                Some(&home) => home,
                None => {
                    let Some(e) = zm.ents.get(&p.n) else { continue };
                    fx.homes.insert(p.n, (e.origin, e.angles));
                    (e.origin, e.angles)
                }
            };
            let mut pass = (now - start).max(0) as f64 * f64::from(p.speed) / a.len as f64;
            pass = if a.looping { pass.fract() } else { pass.min(1.0) };
            let delta = super::actors::anim_root(t6, pass as f32);
            let at = placement(home, angles).transform_point3(glam::Vec3::from(delta));
            moves.push((p.n, at.to_array()));
        }
        moves
    };
    let mut zm = world.resource_mut::<Zm>();
    for (n, at) in moves {
        if let Some(e) = zm.ents.get_mut(&n) {
            e.origin = at;
        }
    }
}

/// IW angles (pitch, yaw, roll in degrees) at `origin` as a matrix.
fn placement(origin: [f32; 3], angles: [f32; 3]) -> glam::Mat4 {
    let [pitch, yaw, roll] = angles.map(f32::to_radians);
    glam::Mat4::from_rotation_translation(
        glam::Quat::from_euler(glam::EulerRot::ZYX, yaw, pitch, roll),
        glam::Vec3::from(origin),
    )
}

/// A matrix's origin and IW angles.
fn pose_of(m: glam::Mat4) -> ([f32; 3], [f32; 3]) {
    let f = m.x_axis.truncate().normalize_or_zero();
    let l = m.y_axis.truncate().normalize_or_zero();
    let u = m.z_axis.truncate().normalize_or_zero();
    let flat = (f.x * f.x + f.y * f.y).sqrt();
    let yaw = f.y.atan2(f.x).to_degrees();
    let pitch = (-f.z).atan2(flat).to_degrees();
    let roll = if flat > 1e-4 { l.z.atan2(u.z).to_degrees() } else { 0.0 };
    (m.w_axis.truncate().to_array(), [pitch, yaw, roll])
}

/// The notes whose time lies in (`from`, `to`], counted in passes of the
/// animation: a looping one fires each note once a pass, a single one once.
/// (Which of a note at the end and one at the start fires first on a wrap
/// is not measured.)
fn crossed(a: &Anim, from: f64, to: f64, out: &mut Vec<String>) {
    let from = from.max(to - 2.0);
    let last = if a.looping { to.floor() } else { 0.0 };
    let mut k = from.floor().max(0.0);
    while k <= last {
        for (name, t) in &a.notes {
            let at = k + f64::from(*t);
            if at > from && at <= to {
                out.push(name.clone());
            }
        }
        k += 1.0;
    }
}

/// (Re)starts scene `i`: its animation, then its effects.
fn start_scene(p: &mut Prop, i: usize, now: i64, out: &mut Vec<Out>) {
    p.cur = i;
    p.start = None;
    p.fired = -1e-9;
    p.waiting = None;
    match &p.scenes[i] {
        // A deleted client model is gone from view: hidden here.
        Scene::Delete | Scene::Hide => out.push(Out::Hide(p.n)),
        Scene::Play(_) => {
            p.start = Some(now);
            for (file, tag) in &p.fx {
                out.push(Out::Fx(p.n, file, tag.clone()));
            }
        }
    }
    // `_fxanim_change_anim`: the next scene waits for its level notify.
    if i + 1 < p.scenes.len() {
        p.waiting = p.until[i + 1].clone();
    }
}

/// Starts, sparks and moves the props (after the models are shown, so
/// effects bolt to them).
pub(super) fn advance(world: &mut World, now: i64) {
    if !world.resource::<Zm>().mp_fxprops.found {
        if now < 1000 {
            return;
        }
        find(world);
    }
    let out = {
        let mut zm = world.resource_mut::<Zm>();
        let mut fx = std::mem::take(&mut zm.mp_fxprops);
        let mut out = std::mem::take(&mut fx.pending);
        dinos(&mut fx, now);
        turns(&mut fx, now);
        for p in &mut fx.props {
            // A start that is due: `fxanim_start`, then the first scene.
            let mut started = false;
            for s in &mut p.starts {
                if !s.done && s.due.is_some_and(|due| now >= due) {
                    s.done = true;
                    started = true;
                }
            }
            if started {
                for &n in &p.reveal {
                    out.push(Out::Show(n));
                }
                start_scene(p, 0, now, &mut out);
            }
            let Some(start) = p.start else { continue };
            let Scene::Play(Some(a)) = &p.scenes[p.cur] else { continue };
            let mut pass = (now - start).max(0) as f64 * f64::from(p.speed) / a.len as f64;
            if !a.looping {
                pass = pass.min(1.0);
            }
            if pass <= p.fired {
                continue;
            }
            let mut notes = Vec::new();
            crossed(a, p.fired, pass, &mut notes);
            let ended = p.fired < 1.0 && pass >= 1.0;
            p.fired = pass;
            for note in notes {
                if let Some(alias) = note.strip_prefix("sndnt#") {
                    out.push(Out::Sound(p.n, alias.to_owned()));
                }
                match &mut p.kind {
                    Kind::Wire(bone) => out.push(Out::Fx(p.n, WIRE_FX, (*bone).to_owned())),
                    Kind::Welder if WELD_NOTES.contains(&note.as_str()) => {
                        out.push(Out::Fx(p.n, WELD_FX, "tag_fx".to_owned()))
                    }
                    Kind::Car { dark } if !*dark => match note.as_str() {
                        "glass_shatter" => out.push(Out::Exploder(1005)),
                        "camera_goes_dark" => {
                            *dark = true;
                            out.push(Out::Dark);
                        }
                        _ => {}
                    },
                    Kind::Nozzles => nozzles(p.n, &note, &mut out),
                    _ => {}
                }
            }
            // `_fxanim_wait_for_anim_to_end`: without a notify the next
            // scene starts when this one first ends.
            if ended && p.waiting.is_none() && p.cur + 1 < p.scenes.len() {
                let next = p.cur + 1;
                start_scene(p, next, now, &mut out);
            }
        }
        zm.mp_fxprops = fx;
        out
    };
    root_motion(world, now);
    move_children(world, now);
    for o in out {
        match o {
            Out::Fx(n, file, tag) => {
                super::natives_fx::bolt_effect(world, n, file, &tag, false);
            }
            Out::Sound(n, alias) => {
                let origin = world.resource::<Zm>().ents.get(&n).map(|e| e.origin);
                if let Some(origin) = origin {
                    super::natives_fx::sound(world, crate::EventAudience::All, &alias, origin);
                }
            }
            Out::Show(n) => set_hidden(world, n, false),
            Out::Hide(n) => set_hidden(world, n, true),
            Out::Exploder(id) => {
                with_vm(world, |vm, world| {
                    super::natives_fx::client_exploder(vm, world, &Value::Int(id))
                });
                diag::info!(Sim, "bo2mp moving props: exploder {id}");
            }
            Out::Dark => {
                let found = asset_world::set_t6_vision("blackout");
                diag::info!(Sim, "bo2mp moving props: visionsetnaked blackout (vision found: {found})");
            }
            Out::ExploderNum(num) => {
                let found = with_vm(world, |vm, world| {
                    let found = super::natives_fx::exploder_origin(vm, num).is_some();
                    super::natives_fx::fire_exploder(vm, world, &Key::Int(num));
                    found
                });
                diag::info!(Sim, "bo2mp moving props: exploder num {num} (found: {found:?})");
            }
            Out::StopFx(n, file, tag) => {
                super::natives_fx::bolt_effect(world, n, file, &tag, true);
            }
        }
    }
}

/// Uplink's radar's beacons (`fx_light_beacon_red_blink_fst`).
const BEACON_FX: &str = "light/fx_light_beacon_red_blink_fst";

/// Takeoff's planets' glows (`fxanim_planets_think`).
const PLANETS: [(&str, &str); 8] = [
    ("maps/mp_maps/fx_mp_tak_glow_blue", "earth_jnt"),
    ("maps/mp_maps/fx_mp_tak_glow_red", "jupiter_jnt"),
    ("maps/mp_maps/fx_mp_tak_glow_red", "mars_jnt"),
    ("maps/mp_maps/fx_mp_tak_glow_yellow", "mercury_jnt"),
    ("maps/mp_maps/fx_mp_tak_glow_blue", "neptune_jnt"),
    ("maps/mp_maps/fx_mp_tak_glow_yellow", "saturn_jnt"),
    ("maps/mp_maps/fx_mp_tak_glow_blue", "uranus_jnt"),
    ("maps/mp_maps/fx_mp_tak_glow_yellow", "venus_jnt"),
];

/// Takeoff's blasters' nozzles (`self.nozzletags`) and their steam.
const NOZZLES: [(&str, &str); 4] = [
    ("one", "nozzle_01_tag_jnt"),
    ("two", "nozzle_02_tag_jnt"),
    ("three", "nozzle_03_tag_jnt"),
    ("four", "nozzle_04_tag_jnt"),
];
const NOZZLE_FX: &str = "maps/mp_maps/fx_mp_tak_steam_nozzle";

/// A blasters note: `strtok( note, "_" )` must give four tokens, "nozzles"
/// first and "on" or "off" last; each token naming a nozzle turns its
/// steam on (`playfxontag`) or off (`stopfx`).
fn nozzles(n: u32, note: &str, out: &mut Vec<Out>) {
    let tokens: Vec<&str> = note.split('_').filter(|t| !t.is_empty()).collect();
    if tokens.len() != 4 || tokens[0] != "nozzles" || !matches!(tokens[3], "on" | "off") {
        return;
    }
    let on = tokens[3] == "on";
    for t in &tokens {
        if let Some((_, tag)) = NOZZLES.iter().find(|(name, _)| name == t) {
            let tag = (*tag).to_owned();
            out.push(if on { Out::Fx(n, NOZZLE_FX, tag) } else { Out::StopFx(n, NOZZLE_FX, tag) });
        }
    }
}

fn turn_wait(fx: &mut MpFxProps, wait: &Wait, i: usize) -> i64 {
    match wait {
        Wait::Drawn(lo, hi) => (fx.range(*lo, *hi) * 1000.0) as i64,
        Wait::Fixed(w) => w.get(i).copied().unwrap_or(0),
    }
}

/// Starts a map script's own loop on `n`: its first animation now.
fn add_turns(fx: &mut MpFxProps, props: &mut Vec<Prop>, n: u32, anims: Vec<Option<Anim>>, wait: Wait, now: i64) {
    let Some(first) = anims.first().cloned() else { return };
    let len = first.as_ref().map_or(0, |a| a.len);
    let w = turn_wait(fx, &wait, 0);
    let start = Start { wait: 0, due: Some(now), done: false };
    props.push(Prop::new(n, Kind::Scene, vec![Scene::Play(first)], vec![start]));
    fx.turns.push(Turns { n, anims, wait, cur: 0, due: now + len + w });
}

/// The map scripts' own loops: the next animation, when due.
fn turns(fx: &mut MpFxProps, now: i64) {
    for i in 0..fx.turns.len() {
        if now < fx.turns[i].due {
            continue;
        }
        let t = fx.turns[i].clone();
        let cur = (t.cur + 1) % t.anims.len();
        let a = t.anims[cur].clone();
        let len = a.as_ref().map_or(0, |a| a.len);
        let wait = turn_wait(fx, &t.wait, cur);
        restart(fx, t.n, a, now);
        let t = &mut fx.turns[i];
        t.cur = cur;
        t.due = now + len + wait;
    }
}

/// A pane that shattered at `at` (the client's `glass_smash`): Vertigo's
/// blinds within 64 units (`distancesquared( org, self.origin ) < 4096`)
/// swing (`blinds_impact`), then hang in their idle; each only once.
pub(super) fn glass_smash(world: &mut World, at: [f32; 3]) {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    let fx = &mut zm.mp_fxprops;
    let mut started = Vec::new();
    fx.blinds.retain(|&(n, o)| {
        let near = (0..3).map(|i| (o[i] - at[i]).powi(2)).sum::<f32>() < 4096.0;
        if near {
            started.push(n);
        }
        !near
    });
    for n in started {
        if let Some(p) = fx.props.iter_mut().find(|p| p.n == n) {
            p.starts.push(Start { wait: 0, due: Some(now), done: false });
        }
        diag::info!(Sim, "bo2mp mp_vertigo: glass broke at {at:?}, blinds {n} swing");
    }
}

/// Restarts prop `n` on `a` from now (a map script's `setanimrestart` /
/// `animscripted`).
fn restart(fx: &mut MpFxProps, n: u32, a: Option<Anim>, now: i64) {
    if let Some(p) = fx.props.iter_mut().find(|p| p.n == n) {
        p.scenes = vec![Scene::Play(a)];
        p.cur = 0;
        p.start = Some(now);
        p.fired = -1e-9;
        p.waiting = None;
        p.fade = None;
    }
}

/// Restarts prop `n` on `a` from now while the animation it leaves fades
/// out over `out_ms` and `a` blends in over `in_ms` (a map script's
/// `clearanim( old, out )` then `animscripted( new, 1.0, in, 1.0 )`).
fn restart_blended(fx: &mut MpFxProps, n: u32, a: Option<Anim>, now: i64, out_ms: i64, in_ms: i64) {
    let leaving = fx.props.iter().find(|p| p.n == n).and_then(|p| {
        let Scene::Play(Some(old)) = p.scenes.get(p.cur)? else { return None };
        Some(Fade { anim: old.clone(), start: p.start?, at: now, out_ms, in_ms })
    });
    diag::info!(
        Sim,
        "bo2mp fxanim blend: {n} {} -> {} at {now} ms (out {out_ms} ms, in {in_ms} ms)",
        leaving.as_ref().map_or("-", |f| f.anim.clip.as_str()),
        a.as_ref().map_or("-", |a| a.clip.as_str())
    );
    restart(fx, n, a, now);
    if let Some(p) = fx.props.iter_mut().find(|p| p.n == n) {
        p.fade = leaving;
    }
}

/// Studio's dinosaurs' next moves, when due.
fn dinos(fx: &mut MpFxProps, now: i64) {
    // The T-rex: `randomint( 3 )` again until it is not the current move;
    // head and stand play it together, then the next wait is drawn.
    if now >= fx.dinos.trex_due
        && let Some((parts, moves)) = fx.dinos.trex.clone()
    {
        let next = loop {
            let i = (fx.rand() % 3) as usize;
            if fx.dinos.trex_cur != Some(i) {
                break i;
            }
        };
        fx.dinos.trex_cur = Some(next);
        for (n, m) in parts.into_iter().zip(moves) {
            restart_blended(fx, n, m[next].clone(), now, 50, 0);
        }
        fx.dinos.trex_due = now + (fx.range(10.0, 15.0) * 1000.0) as i64;
    }
    // The brontosaurus: looks around for the animation's length
    // (`getanimlength`), then chews until the next wait is up.
    if now >= fx.dinos.bronto_due
        && let Some((n, chew, look)) = fx.dinos.bronto.clone()
    {
        if fx.dinos.looking {
            restart_blended(fx, n, chew, now, 500, 500);
            fx.dinos.bronto_due = now + (fx.range(15.0, 20.0) * 1000.0) as i64;
        } else {
            let len = look.as_ref().map_or(0, |a| a.len);
            restart_blended(fx, n, look, now, 500, 500);
            fx.dinos.bronto_due = now + len;
        }
        fx.dinos.looking = !fx.dinos.looking;
    }
}

fn set_hidden(world: &mut World, n: u32, hidden: bool) {
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
        e.hidden = hidden;
    }
}

/// The models held on a parent's joint follow its animation.
fn move_children(world: &mut World, now: i64) {
    if world.resource::<Zm>().mp_fxprops.children.is_empty() {
        return;
    }
    let parents: Vec<u32> = {
        let fx = &world.resource::<Zm>().mp_fxprops;
        let mut v: Vec<u32> = fx.children.iter().map(|c| c.parent).collect();
        v.sort_unstable();
        v.dedup();
        v.retain(|p| !fx.dobjs.contains_key(p));
        v
    };
    // A parent's skeleton, once its model is up; the drone's parts take
    // their offset from where its joint stands then (its bind pose; the
    // conveyor's animation starts after drone_link).
    for parent in parents {
        let Some((model, origin, angles)) =
            world.resource::<Zm>().ents.get(&parent).map(|e| (e.model.clone(), e.origin, e.angles))
        else {
            continue;
        };
        let Some(cap) = frame(world).model_capability(&model).flatten() else { continue };
        let Ok(dobj) = xmodel_runtime::DObj::build(&[(&cap.pose, None)]) else { continue };
        let bind = dobj.pose(&[], &dobj.all_parts(), placement(origin, angles));
        let mut zm = world.resource_mut::<Zm>();
        let mut fx = std::mem::take(&mut zm.mp_fxprops);
        for c in fx.children.iter_mut().filter(|c| c.parent == parent) {
            let Some(b) = dobj.find(&c.bone).and_then(|b| bind.get(b)) else {
                diag::info!(Sim, "bo2mp moving props: ent{} has no joint {} for ent{}", parent, c.bone, c.n);
                continue;
            };
            if let Some(own) = c.own.take() {
                c.offset = b.inverse() * own;
            }
        }
        fx.dobjs.insert(parent, dobj);
        zm.mp_fxprops = fx;
    }
    let moves: Vec<(u32, [f32; 3], [f32; 3])> = {
        let zm = world.resource::<Zm>();
        let fx = &zm.mp_fxprops;
        let mut moves = Vec::new();
        for (&parent, dobj) in &fx.dobjs {
            let Some(e) = zm.ents.get(&parent) else { continue };
            let place = placement(e.origin, e.angles);
            // One instance per playing animation (two while it blends).
            let playing: Vec<_> = present_of(fx, parent, now)
                .into_iter()
                .filter_map(|(clip, frac, _, w)| Some((zm.clips.get(&clip.to_ascii_lowercase())?.clone(), frac, w)))
                .collect();
            let tracks: Vec<Vec<Option<usize>>> = playing
                .iter()
                .map(|(clip, _, _)| clip.tracks.iter().map(|t| dobj.find(&t.name)).collect())
                .collect();
            let insts: Vec<_> = playing
                .iter()
                .zip(&tracks)
                .map(|((clip, frac, w), tracks)| xmodel_runtime::AnimInstance {
                    clip,
                    tracks,
                    time: frac * clip.duration().max(1e-3),
                    weight: *w,
                    parts: None,
                })
                .collect();
            let posed = dobj.pose(&insts, &dobj.all_parts(), place);
            for c in fx.children.iter().filter(|c| c.parent == parent) {
                let Some(m) = dobj.find(&c.bone).and_then(|b| posed.get(b)) else { continue };
                let (origin, angles) = pose_of(*m * c.offset);
                moves.push((c.n, origin, angles));
            }
        }
        moves
    };
    let mut zm = world.resource_mut::<Zm>();
    for (n, origin, angles) in moves {
        if let Some(e) = zm.ents.get_mut(&n) {
            e.origin = origin;
            e.angles = angles;
        }
    }
}

/// A level notify on the client (a server `clientnotify`, or the map's own
/// client script): the props waiting on it start (after their
/// `fxanim_wait`), or move to their next scene.
pub(super) fn level_notify(world: &mut World, name: &str) {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    let mut fx = std::mem::take(&mut zm.mp_fxprops);
    let mut out = std::mem::take(&mut fx.pending);
    for p in &mut fx.props {
        if p.until[0].as_deref() == Some(name) {
            for s in p.starts.iter_mut().filter(|s| !s.done && s.due.is_none()) {
                s.due = Some(now + s.wait);
                diag::info!(Sim, "bo2mp moving prop ent{} starts on {name}", p.n);
            }
        }
        if p.waiting.as_deref() == Some(name) {
            let next = p.cur + 1;
            start_scene(p, next, now, &mut out);
        }
    }
    fx.pending = out;
    zm.mp_fxprops = fx;
}

/// Plays `clip` on entity `n` from now (a map script's `animscripted`:
/// Nuketown's end-game cars, `car`, and glass).
pub(super) fn play(world: &mut World, n: u32, clip: &str, car: bool) {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    let a = anim(&zm, clip);
    diag::info!(Sim, "bo2mp moving props: ent{n} plays {clip} (loaded: {})", a.is_some());
    let kind = if car { Kind::Car { dark: false } } else { Kind::Scene };
    let mut p = Prop::new(n, kind, vec![Scene::Play(a)], Vec::new());
    p.start = Some(now);
    p.fired = -1e-9;
    let props = &mut zm.mp_fxprops.props;
    props.retain(|p| p.n != n);
    props.push(p);
}

/// Plays `clip` on entity `n` from its start at `rate` times its speed
/// (`setanimrestart( clip, 1, 0, rate )`: the AGR's rocket pods).
pub(super) fn restart_at(world: &mut World, n: u32, clip: &str, rate: f32) {
    play(world, n, clip, false);
    if let Some(p) = world.resource_mut::<Zm>().mp_fxprops.props.iter_mut().find(|p| p.n == n) {
        p.speed = rate;
    }
}

/// Entity `n` is gone: its animation with it.
pub(super) fn forget(world: &mut World, n: u32) {
    world.resource_mut::<Zm>().mp_fxprops.props.retain(|p| p.n != n);
}

/// Where animation `a`, started at `start`, is at `now`: (clip, fraction,
/// rate per second).
fn position(a: &Anim, start: i64, speed: f32, now: i64) -> (String, f32, f32) {
    let pass = (now - start).max(0) as f32 * speed / a.len as f32;
    let rate = speed * 1000.0 / a.len as f32;
    if a.looping {
        (a.clip.clone(), pass.fract(), rate)
    } else if pass >= 1.0 {
        (a.clip.clone(), 1.0, 0.0)
    } else {
        (a.clip.clone(), pass, rate)
    }
}

fn present_of(fx: &MpFxProps, n: u32, now: i64) -> Vec<(String, f32, f32, f32)> {
    let Some(p) = fx.props.iter().find(|p| p.n == n) else { return Vec::new() };
    let Some(start) = p.start else { return Vec::new() };
    let Scene::Play(Some(a)) = &p.scenes[p.cur] else { return Vec::new() };
    let (clip, frac, rate) = position(a, start, p.speed, now);
    // While it blends, the old animation's weight falls to 0 over its blend
    // time and the new one's rises to 1 over its own (at once for 0), both
    // still playing; the pair is weighed against its sum.
    let Some(f) = &p.fade else { return vec![(clip, frac, rate, 1.0)] };
    let ramp = |ms: i64| if ms <= 0 { 1.0 } else { ((now - f.at) as f32 / ms as f32).clamp(0.0, 1.0) };
    let (out, inn) = (1.0 - ramp(f.out_ms), ramp(f.in_ms));
    if out <= 0.0 {
        return vec![(clip, frac, rate, 1.0)];
    }
    let (old, old_frac, old_rate) = position(&f.anim, f.start, p.speed, now);
    let total = out + inn;
    vec![(old, old_frac, old_rate, out / total), (clip, frac, rate, inn / total)]
}

/// A prop's playing animations: (clip, fraction, rate per second, weight);
/// two while it blends from one into the next.
pub(super) fn present(world: &World, n: u32, now: i64) -> Vec<(String, f32, f32, f32)> {
    present_of(&world.resource::<Zm>().mp_fxprops, n, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim(clip: &str, len: i64, looping: bool) -> Option<Anim> {
        Some(Anim { clip: clip.into(), len, looping, notes: Vec::new() })
    }

    fn weights(fx: &MpFxProps, now: i64) -> Vec<(String, f32)> {
        present_of(fx, 7, now).into_iter().map(|(c, _, _, w)| (c, (w * 1000.0).round() / 1000.0)).collect()
    }

    #[test]
    fn brontosaurus_blends_half_a_second_and_trex_fades_out_fast() {
        let mut fx = MpFxProps::default();
        fx.props.push(Prop::new(7, Kind::Scene, Vec::new(), Vec::new()));
        restart(&mut fx, 7, anim("chew", 6000, true), 0);
        assert_eq!(weights(&fx, 9000), vec![("chew".into(), 1.0)]);
        // The brontosaurus: `clearanim( chew, 0.5 )`, `animscripted( look, 1.0, 0.5, 1.0 )`.
        restart_blended(&mut fx, 7, anim("look", 4000, false), 10_000, 500, 500);
        assert_eq!(weights(&fx, 10_000), vec![("chew".into(), 1.0), ("look".into(), 0.0)]);
        assert_eq!(weights(&fx, 10_250), vec![("chew".into(), 0.5), ("look".into(), 0.5)]);
        assert_eq!(weights(&fx, 10_500), vec![("look".into(), 1.0)]);
        // The old one keeps playing from where it was while it fades.
        let p = present_of(&fx, 7, 10_250);
        assert!((p[0].1 - (10_250.0 / 6000.0f32).fract()).abs() < 1e-4 && p[1].1 > 0.0);
        // The T-rex: `clearanim( prev, 0.05 )`, `setanimrestart( next, 1.0, 0.0, 1.0 )`.
        restart_blended(&mut fx, 7, anim("roar", 3000, false), 20_000, 50, 0);
        assert_eq!(weights(&fx, 20_000), vec![("look".into(), 0.5), ("roar".into(), 0.5)]);
        assert_eq!(weights(&fx, 20_025), vec![("look".into(), 0.333), ("roar".into(), 0.667)]);
        assert_eq!(weights(&fx, 20_050), vec![("roar".into(), 1.0)]);
        // A plain restart drops any blend.
        restart_blended(&mut fx, 7, anim("chew", 6000, true), 30_000, 500, 500);
        restart(&mut fx, 7, anim("chew", 6000, true), 30_100);
        assert_eq!(weights(&fx, 30_100), vec![("chew".into(), 1.0)]);
    }
}
