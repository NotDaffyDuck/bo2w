//! bo2mp: what a swing that struck something tells the players, as BO2's
//! `Weapon_Melee_internal` decides it: the hit sound or the miss sound, and
//! whether the attacker sees blood.
//!
//! A player struck by a player: the scripts decide (`callback_playermelee`).
//! A teammate in a team game with friendly fire off is a miss; a riot shield
//! is always a hit. No blood on a shield, and none on a teammate unless the
//! game is hardcore. Anything else struck is a hit only when it is an AI (a
//! zombie, a dog) or a planted riot shield, and only an AI bleeds.

use bevy_ecs::prelude::World;
use entity_t6::team;
use gsc_t6::{ObjRef, Value, Vm};

use super::{T6Runtime, Zm, with_vm};
use crate::ScriptModelId;

/// What the swing struck.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Struck {
    /// A player; `shield`: his riot shield took it.
    Player { client: u32, shield: bool },
    /// A script model: a zombie, a dog, a planted riot shield, a car.
    Model(ScriptModelId),
}

/// What the players are told about one swing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Verdict {
    /// The hit sound (else the miss sound).
    pub hit: bool,
    /// A riot shield (carried or planted) took it: the knife-on-shield sound.
    pub shield: bool,
    /// The attacker sees blood.
    pub blood: bool,
}

/// The planted riot shield's name (`level.riotshield_name`, _riotshield.gsc).
const PLANTED_SHIELD: &str = "riotshield_mp";

/// Whether a BO2 map runs (its rules apply).
pub(crate) fn runs(world: &World) -> bool {
    world.contains_resource::<T6Runtime>()
}

/// BO2's verdict for `attacker`'s swing, or None when no BO2 map runs.
pub(crate) fn verdict(world: &mut World, attacker: u32, struck: Struck) -> Option<Verdict> {
    with_vm(world, |vm, world| match struck {
        Struck::Player { client, shield } => {
            let level = vm.level;
            let game = Game {
                teambased: gsc_t6::truthy(&field(vm, level, "teambased")),
                // `level.friendlyfire == 0` (undefined is not 0).
                friendly_fire: !matches!(field(vm, level, "friendlyfire"), Value::Int(0))
                    && !matches!(field(vm, level, "friendlyfire"), Value::Float(f) if f == 0.0),
                // The engine reads `scr_hardcore`; our scripts keep it as `level.hardcoremode`.
                hardcore: gsc_t6::truthy(&field(vm, level, "hardcoremode")),
            };
            let zm = world.resource::<Zm>();
            let (a, v) = (
                zm.players.get(&attacker).map(|p| p.obj),
                zm.players.get(&client).map(|p| p.obj),
            );
            let mut teams =
                |name: &str| [a, v].map(|o| o.map_or_else(String::new, |o| text(vm, o, name)));
            let [att_team, vic_team] = teams("team");
            let [att_session, vic_session] = teams("sessionteam");
            player_rules(
                game,
                att_team == vic_team,
                engine_team(&att_session),
                engine_team(&vic_session),
                shield,
            )
        }
        Struck::Model(id) => {
            let zm = world.resource::<Zm>();
            let Some(n) = zm
                .presences
                .by_ent
                .iter()
                .find(|(_, s)| s.id == id)
                .map(|(n, _)| *n)
            else {
                return Verdict::default();
            };
            let actor = zm.actors.get(&n).is_some_and(|a| a.alive);
            // G_EntityIsDeployedRiotshield: the shield `deployriotshield` made damageable.
            let planted = !actor
                && zm
                    .ents
                    .get(&n)
                    .filter(|e| e.can_damage)
                    .and_then(|e| e.obj)
                    .is_some_and(|o| text(vm, o, "targetname") == PLANTED_SHIELD);
            Verdict {
                hit: actor || planted,
                shield: planted,
                blood: actor,
            }
        }
    })
}

fn field(vm: &mut Vm<World>, obj: ObjRef, name: &str) -> Value {
    let f = vm.intern(name);
    vm.raw_field(obj, f)
}

fn text(vm: &mut Vm<World>, obj: ObjRef, name: &str) -> String {
    let v = field(vm, obj, name);
    vm.to_text(&v)
}

/// The engine's team for a `sessionteam` (as the field sets it).
fn engine_team(session: &str) -> i32 {
    match session {
        "axis" => team::AXIS,
        "allies" => team::ALLIES,
        "spectator" => team::SPECTATOR,
        _ => team::FREE,
    }
}

/// The match settings the rules read.
#[derive(Clone, Copy)]
struct Game {
    teambased: bool,
    friendly_fire: bool,
    hardcore: bool,
}

/// A player struck by a player: `same_team` compares the scripts' `team`s,
/// `attacker` and `victim` are the engine's teams (`sessionteam`).
fn player_rules(game: Game, same_team: bool, attacker: i32, victim: i32, shield: bool) -> Verdict {
    // callback_playermelee, then finishmeleehit: a hit unless a teammate with
    // friendly fire off; a shield is always a hit.
    let scripted_hit = !(game.teambased && same_team && !game.friendly_fire);
    // Weapon_Melee_internal: no blood on a teammate (outside hardcore) or on a shield.
    let teammate = attacker != team::FREE && !game.hardcore && attacker == victim;
    Verdict {
        hit: scripted_hit || shield,
        shield,
        blood: !shield && !teammate,
    }
}

#[cfg(test)]
mod tests {
    use super::{Game, Verdict, player_rules};
    use entity_t6::team;

    const TDM: Game = Game {
        teambased: true,
        friendly_fire: false,
        hardcore: false,
    };

    #[test]
    fn an_enemy_knifed_is_a_hit_that_bleeds() {
        let v = player_rules(TDM, false, team::ALLIES, team::AXIS, false);
        assert_eq!(
            v,
            Verdict {
                hit: true,
                shield: false,
                blood: true
            }
        );
    }

    #[test]
    fn a_teammate_knifed_with_friendly_fire_off_is_a_miss_without_blood() {
        let v = player_rules(TDM, true, team::AXIS, team::AXIS, false);
        assert_eq!(
            v,
            Verdict {
                hit: false,
                shield: false,
                blood: false
            }
        );
        // Friendly fire on: the hit sound, still no blood.
        let ff = Game {
            friendly_fire: true,
            ..TDM
        };
        assert!(player_rules(ff, true, team::AXIS, team::AXIS, false).hit);
        assert!(!player_rules(ff, true, team::AXIS, team::AXIS, false).blood);
        // Hardcore: a teammate bleeds.
        let hc = Game {
            friendly_fire: true,
            hardcore: true,
            ..TDM
        };
        assert!(player_rules(hc, true, team::AXIS, team::AXIS, false).blood);
    }

    #[test]
    fn a_shield_is_a_hit_without_blood() {
        let v = player_rules(TDM, true, team::AXIS, team::AXIS, true);
        assert_eq!(
            v,
            Verdict {
                hit: true,
                shield: true,
                blood: false
            }
        );
        assert!(!player_rules(TDM, false, team::ALLIES, team::AXIS, true).blood);
    }

    #[test]
    fn free_for_all_players_always_bleed() {
        // Free-for-all: one script team name, no engine team.
        let ffa = Game {
            teambased: false,
            ..TDM
        };
        let v = player_rules(ffa, true, team::FREE, team::FREE, false);
        assert_eq!(
            v,
            Verdict {
                hit: true,
                shield: false,
                blood: true
            }
        );
    }
}
