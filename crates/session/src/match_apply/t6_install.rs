//! A Black Ops II map's own scripts and data, as the simulation takes them.
//!
//! The asset side hands over what the zones held (`assets::T6ScriptSet`);
//! this turns it into `sim::T6Install`, adding the two things only the loaded
//! match knows: the skeletons of the models a script can put on an entity,
//! and the animation clips actors are posed with.

use std::collections::BTreeMap;
use std::sync::Arc;

use asset_core::AssetNamespace;
use assets::{PreparedFpvMeshes, PreparedXAnims, T6ScriptSet};
use sim::t6_playeranim::T6PlayerAnimsRes;

/// A destination struct built from the named fields of a source value: the
/// `copy` ones as they are, the `clone` ones cloned, and any `set` ones from
/// the expression given.
macro_rules! carry {
    ($src:expr => $($dest:ident)::+ {
        copy: [$($c:ident),* $(,)?],
        clone: [$($k:ident),* $(,)?]
        $(, set: { $($f:ident : $v:expr),* $(,)? })?
    }) => {
        $($dest)::+ {
            $($c: $src.$c,)*
            $($k: $src.$k.clone(),)*
            $($($f: $v,)*)?
        }
    };
}

/// What the match adds to the zones' data.
pub(super) struct T6Inputs<'a> {
    pub zone: &'a str,
    pub gametype: &'a str,
    /// The bots the front end asked for: friends, enemies, difficulty.
    pub bots: Option<(u32, u32, u32)>,
    pub fpv_meshes: &'a PreparedFpvMeshes,
    pub xanims: &'a PreparedXAnims,
}

pub(super) fn build(set: &T6ScriptSet, with: &T6Inputs) -> sim::T6Install {
    let playeranim = T6PlayerAnimsRes::from_text(set.playeranim.as_ref(), &[]).0;
    let player_clips = playeranim
        .as_deref()
        .map_or_else(Vec::new, |anims| anims.anims.clone());
    sim::T6Install {
        map: with.zone.to_owned(),
        gametype: with.gametype.to_owned(),
        objects: set.objects.clone(),
        tables: tables(set),
        entities: set.entities.clone(),
        path_nodes: path_nodes(set),
        path_tables: carry!(set.path_tables => sim::T6PathTables {
            copy: [node_count, original_count],
            clone: [vis, smooth]
        }),
        anims: anims(set),
        animstatedefs: set.animstatedefs.clone(),
        models: models(with.fpv_meshes),
        clips: clips(&clip_names(set, &player_clips), with.xanims),
        strings: set.strings.iter().cloned().collect(),
        sound_aliases: set.sound_aliases.iter().cloned().collect(),
        gamesettings: super::config::gamesettings(&set.gamesettings, with.gametype),
        bots: with.bots,
        sound_lengths: set.sound_lengths.iter().cloned().collect(),
        retrievable_weapons: set.retrievable_weapons.clone(),
        vehicles: vehicles(set),
        playeranim,
        destructibles: set.destructibles.clone(),
    }
}

/// The data tables, under their lower-case names.
fn tables(set: &T6ScriptSet) -> BTreeMap<String, sim::T6Table> {
    let mut tables = BTreeMap::new();
    for (name, columns, rows, cells) in &set.tables {
        tables.insert(
            name.to_ascii_lowercase(),
            sim::T6Table {
                columns: *columns,
                rows: *rows,
                cells: cells.clone(),
            },
        );
    }
    tables
}

fn path_nodes(set: &T6ScriptSet) -> Vec<sim::T6PathNode> {
    set.path_nodes
        .iter()
        .map(|node| {
            carry!(node => sim::T6PathNode {
                copy: [ty, spawnflags, origin, angle, radius],
                clone: [
                    targetname, target, script_noteworthy, script_linkname,
                    animscript, links
                ]
            })
        })
        .collect()
}

fn anims(set: &T6ScriptSet) -> Vec<sim::T6Anim> {
    set.anims
        .iter()
        .map(|anim| {
            carry!(anim => sim::T6Anim {
                copy: [numframes, framerate, looping],
                clone: [name, delta_trans, notifies]
            })
        })
        .collect()
}

/// How a helicopter flies. The body's pitch and roll limits are the
/// vehicle's own, not the helicopter section's.
fn heli(drive: &asset_t6::VehicleDrive) -> sim::T6HeliDrive {
    carry!(drive.heli => sim::T6HeliDrive {
        copy: [
            rot_rate, rot_accel, tilt_from_accel, tilt_from_decel, tilt_from_vel,
            tilt_speed, use_collmap, decel, tilt_from_axes, tilt_from_fwd_yaw,
            vel_at_max_tilt, tilt_momentum, accel_toward_view, maintain_height,
            height_limit, height_min, height_max, ceiling_min
        ],
        clone: [],
        set: {
            max_body_pitch: drive.max_body_pitch,
            max_body_roll: drive.max_body_roll,
        }
    })
}

fn drive(drive: &asset_t6::VehicleDrive) -> sim::T6VehicleDrive {
    carry!(drive => sim::T6VehicleDrive {
        copy: [
            camera_mode, max_speed, max_speed_vertical, accel, accel_vertical,
            reverse_scale, camera_range, camera_height, camera_pitch, camera_fov,
            camera_spring, turret_pitch, third_person_driver, buttons,
            boost_button, hand_brake_button, boost_accel_mult, boost_duration,
            boost_speed_increase, steer_angle_max, steer_speed, tire_fric,
            auto_hand_brake_min_speed, peel_out_max_speed, skid_speed_min,
            remote_control, bullet_damage, armor_piercing_damage, grenade_damage,
            projectile_damage, projectile_splash_damage, heavy_explosive_damage,
            no_directional_damage, vehicle_type, collision_damage, collision_speed
        ],
        clone: [],
        set: { heli: heli(drive) }
    })
}

fn vehicles(set: &T6ScriptSet) -> Vec<sim::T6Vehicle> {
    set.vehicles
        .iter()
        .map(|vehicle| sim::T6Vehicle {
            name: vehicle.name.clone(),
            turret: vehicle.turret_weapon.clone(),
            gunners: vehicle.gunner_weapons.clone(),
            drive: drive(&vehicle.drive),
        })
        .collect()
}

/// The skeleton and hit boxes of every BO2 model a script can put on an
/// entity.
fn models(fpv: &PreparedFpvMeshes) -> Vec<(String, Arc<xmodel_runtime::RetainedModelCapability>)> {
    let mut models = Vec::new();
    for name in fpv.0.names_in(AssetNamespace::T6) {
        let Some(entry) = fpv.0.get(AssetNamespace::T6, name) else {
            continue;
        };
        if let Some(capability) = entry.skel.retained_capability() {
            models.push((name.to_owned(), Arc::new(capability)));
        }
    }
    models
}

/// The animations that need a clip: the ones an animstatedef names (actors are
/// hit-tested in them), the players' own, and the maps' moving props.
/// Each once, in that order.
fn clip_names(set: &T6ScriptSet, player: &[String]) -> Vec<String> {
    let in_a_statedef = |name: &str| {
        set.animstatedefs
            .iter()
            .any(|(_, text)| text.contains(name))
    };
    let named = set
        .anims
        .iter()
        .map(|anim| anim.name.as_str())
        .filter(|name| in_a_statedef(name));
    let props = set
        .anims
        .iter()
        .map(|anim| anim.name.as_str())
        .filter(|name| name.starts_with("fxanim_"));
    let mut names: Vec<String> = Vec::new();
    let candidates = named.chain(player.iter().map(String::as_str)).chain(props);
    for name in candidates {
        if !names.iter().any(|seen| seen == name) {
            names.push(name.to_owned());
        }
    }
    names
}

fn clips(
    names: &[String],
    xanims: &PreparedXAnims,
) -> Vec<(String, Arc<xmodel_runtime::AnimClip>)> {
    names
        .iter()
        .filter_map(|name| {
            let clip = xanims.0.clip(AssetNamespace::T6, name)?;
            Some((name.to_ascii_lowercase(), clip))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim(name: &str) -> assets::T6AnimFacts {
        assets::T6AnimFacts {
            name: name.to_owned(),
            ..Default::default()
        }
    }

    fn set_with_anims(names: &[&str], statedefs: &[&str]) -> T6ScriptSet {
        T6ScriptSet {
            anims: names.iter().map(|name| anim(name)).collect(),
            animstatedefs: statedefs
                .iter()
                .map(|text| ("def".to_owned(), (*text).to_owned()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn tables_are_found_under_lower_case_names() {
        let set = T6ScriptSet {
            tables: vec![(
                "Maps/Mp/Gametypes.CSV".into(),
                2,
                1,
                vec!["a".into(), "b".into()],
            )],
            ..Default::default()
        };
        let tables = tables(&set);
        let table = &tables["maps/mp/gametypes.csv"];
        assert_eq!((table.columns, table.rows), (2, 1));
        assert_eq!(table.cells, vec!["a", "b"]);
    }

    #[test]
    fn an_animation_in_a_statedef_gets_a_clip() {
        let set = set_with_anims(&["run", "idle"], &["state run loop"]);
        assert_eq!(clip_names(&set, &[]), vec!["run"]);
    }

    #[test]
    fn the_players_animations_follow_and_props_come_last() {
        let set = set_with_anims(&["fxanim_door", "run"], &["run"]);
        let player = vec!["stand_idle".to_owned()];
        assert_eq!(
            clip_names(&set, &player),
            vec!["run", "stand_idle", "fxanim_door"]
        );
    }

    #[test]
    fn a_clip_wanted_twice_is_made_once() {
        let set = set_with_anims(&["fxanim_a", "run"], &["fxanim_a run"]);
        let player = vec!["run".to_owned()];
        assert_eq!(clip_names(&set, &player), vec!["fxanim_a", "run"]);
    }

    #[test]
    fn nothing_named_means_no_clips() {
        assert!(clip_names(&T6ScriptSet::default(), &[]).is_empty());
    }

    #[test]
    fn a_path_node_keeps_everything_it_was_given() {
        let node = asset_t6::PathNodeRef {
            ty: 3,
            spawnflags: 4,
            targetname: "a".into(),
            target: "b".into(),
            origin: [1.0, 2.0, 3.0],
            angle: 90.0,
            radius: 16.0,
            links: vec![(5, 1.5, true)],
            ..Default::default()
        };
        let set = T6ScriptSet {
            path_nodes: vec![node],
            ..Default::default()
        };
        let nodes = path_nodes(&set);
        assert_eq!(nodes.len(), 1);
        assert_eq!((nodes[0].ty, nodes[0].spawnflags), (3, 4));
        assert_eq!(nodes[0].origin, [1.0, 2.0, 3.0]);
        assert_eq!(nodes[0].links, vec![(5, 1.5, true)]);
        assert_eq!(nodes[0].targetname, "a");
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn a_helicopter_takes_its_body_limits_from_the_vehicle() {
        let mut def = asset_t6::VehicleDrive::default();
        def.max_body_pitch = 12.0;
        def.max_body_roll = 7.0;
        def.heli.rot_rate = 90.0;
        let mapped = drive(&def);
        assert_eq!(mapped.heli.max_body_pitch, 12.0);
        assert_eq!(mapped.heli.max_body_roll, 7.0);
        assert_eq!(mapped.heli.rot_rate, 90.0);
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn a_vehicle_keeps_its_weapons_and_speed() {
        let mut def = asset_t6::VehicleDrive::default();
        def.max_speed = 33.0;
        let set = T6ScriptSet {
            vehicles: vec![asset_t6::VehicleRef {
                name: "tank".into(),
                turret_weapon: "gun".into(),
                gunner_weapons: vec!["mg".into()],
                drive: def,
                ..Default::default()
            }],
            ..Default::default()
        };
        let vehicles = vehicles(&set);
        assert_eq!(vehicles[0].name, "tank");
        assert_eq!(vehicles[0].turret, "gun");
        assert_eq!(vehicles[0].gunners, vec!["mg"]);
        assert_eq!(vehicles[0].drive.max_speed, 33.0);
    }
}
