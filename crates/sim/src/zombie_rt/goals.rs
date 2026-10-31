//! bo2mp: BO2's objectives (`objective_add` and the rest): the flags, bomb
//! sites, capture zones and carried things its HUD marks in the world and
//! on the compass. The engine keeps 32 (`level.objectives`, reset at each
//! level start); each client is sent the ones it may see
//! (`G_UpdateObjectivesForClient`), and BO2's own HUD scripts draw them.

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, arg, entnum, int, num, text, vec3};

pub(crate) const MAX: usize = 32;

/// One objective (BO2's `objective_t`).
#[derive(Clone, Default)]
pub(crate) struct MissionMarker {
    /// 0 empty, 1 active, 2 invisible.
    pub state: u8,
    pub spot: [f32; 3],
    /// The entity it follows (`objective_onentity`).
    pub entity: Option<u32>,
    /// Its size, and its icon's material (`objective_icon`).
    pub size: [f32; 2], pub icon: String,
    pub owner: Option<u32>,
    /// Its name (`&"dom_a"`; empty = none): a named objective goes to
    /// every client and the HUD picks who sees it.
    pub name: String,
    /// The teams whose HUD shows it (`objective_visibleteams`): bit
    /// `1 << team`.
    pub team_mask: i32,
    /// 0-255 (`objective_setprogress` x 255).
    pub progress: u8,
    /// The clients using it (`objective_setplayerusing`), a bit each.
    pub using: u64,
    /// The clients it is hidden from (`objective_setinvisibletoplayer`).
    pub hidden: u64,
    pub gamemode_flags: u8,
    /// Its team (`objective_team`): 0 none, 1 allies, 2 axis, 3-8, 9
    /// spectator.
    pub team: u8,
}

/// BO2's team number for a team name (`Scr_GetTeam`).
pub(crate) fn team_number(name: &str) -> Result<u8, String> {
    Ok(match name {
        "allies" => 1,
        "axis" => 2,
        "team3" => 3,
        "team4" => 4,
        "team5" => 5,
        "team6" => 6,
        "team7" => 7,
        "team8" => 8,
        "spectator" => 9,
        "neutral" | "none" | "free" => 0,
        _ => {
            return Err(format!(
                "'{name}' is an illegal team string. Must be allies, axis, team3, team4, team5, team6, team7, team8, none, or spectator."
            ));
        }
    })
}

/// bo2mp: a BO2 team number (`team_number`) as a client's team
/// (`client_state_team`). Both are BO2's `team_t`.
pub(crate) fn client_team(team: u8) -> i32 {
    i32::from(team)
}

fn index(a: &[ScriptValue]) -> Result<usize, String> {
    let i = int(a, 0)?;
    usize::try_from(i)
        .ok()
        .filter(|i| *i < MAX)
        .ok_or_else(|| format!("index {i} is an illegal objective index. Valid indexes are 0 to {}", MAX - 1))
}

fn state(vm: &Vm<World>, a: &[ScriptValue], i: usize) -> Result<u8, String> {
    match text(vm, a, i).as_str() {
        "empty" => Ok(0),
        "active" => Ok(1),
        "invisible" => Ok(2),
        s => Err(format!(
            "Illegal objective state \"{s}\". Valid states are \"empty\", \"invisible\", \"current\", \"active\""
        )),
    }
}

/// A client's bit in a use or hidden mask.
fn bit(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Result<u64, String> {
    let c = super::client(vm, world, v).map_err(|_| "SetVisibleToPlayer called on non player entity".to_owned())?;
    Ok(1u64.checked_shl(c.0).unwrap_or(0))
}

fn obj(world: &mut World, i: usize) -> impl std::ops::DerefMut<Target = MissionMarker> + '_ {
    bevy_ecs::prelude::Mut::map_unchanged(world.resource_mut::<Zm>(), move |zm| &mut zm.goal_list[i])
}

/// Its origin rounded as the engine rounds it (toward zero).
fn rounded(v: [f32; 3]) -> [f32; 3] {
    v.map(f32::trunc)
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    f!("objective_add", |vm, world, _, a| {
        let i = index(a)?;
        let st = state(vm, a, 1)?;
        let origin = if a.len() > 2 { Some(rounded(vec3(a, 2)?)) } else { None };
        let name = (a.len() > 3).then(|| text(vm, a, 3));
        let holder = (a.len() > 4).then(|| entnum(vm, arg(a, 4))).flatten();
        let mut o = obj(world, i);
        o.hidden = 0;
        o.entity = None;
        o.state = st;
        if let Some(place) = origin {
            o.spot = place;
            if let Some(label) = name {
                o.name = label;
            }
            if a.len() > 4 {
                o.owner = holder;
            }
        }
        o.team = 0;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_state", |vm, world, _, a| {
        let i = index(a)?;
        let st = state(vm, a, 1)?;
        let mut o = obj(world, i);
        o.state = st;
        if st != 1 {
            o.entity = None;
        }
        Ok(ScriptValue::Undefined)
    });
    f!("objective_delete", |_, world, _, a| {
        let i = index(a)?;
        let mut o = obj(world, i);
        o.entity = None;
        o.state = 0;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_icon", |vm, world, _, a| {
        let i = index(a)?;
        let icon = text(vm, a, 1);
        if icon.len() > 63 {
            return Err(format!("Objective icon name is too long (> 63): {icon}"));
        }
        obj(world, i).icon = icon;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_position", |_, world, _, a| {
        let i = index(a)?;
        let v = rounded(vec3(a, 1)?);
        obj(world, i).spot = v;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_onentity", |vm, world, _, a| {
        let i = index(a)?;
        let n = entnum(vm, arg(a, 1)).ok_or("argument 2 is not an entity")?;
        obj(world, i).entity = Some(n);
        Ok(ScriptValue::Undefined)
    });
    f!("objective_clearentity", |_, world, _, a| {
        let i = index(a)?;
        obj(world, i).entity = None;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_team", |vm, world, _, a| {
        let i = index(a)?;
        let t = team_number(&text(vm, a, 1))?;
        obj(world, i).team = t;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_visibleteams", |_, world, _, a| {
        let i = index(a)?;
        let m = int(a, 1)?;
        obj(world, i).team_mask = i32::from(m as i16);
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setvisibletoplayer", |vm, world, _, a| {
        let i = index(a)?;
        let b = bit(vm, world, arg(a, 1))?;
        obj(world, i).hidden &= !b;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setinvisibletoplayer", |vm, world, _, a| {
        let i = index(a)?;
        let b = bit(vm, world, arg(a, 1))?;
        obj(world, i).hidden |= b;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setvisibletoall", |_, world, _, a| {
        let i = index(a)?;
        obj(world, i).hidden = 0;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setinvisibletoall", |_, world, _, a| {
        let i = index(a)?;
        obj(world, i).hidden = u64::MAX;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setplayerusing", |vm, world, _, a| {
        let i = index(a)?;
        let b = bit(vm, world, arg(a, 1))?;
        obj(world, i).using |= b;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_clearplayerusing", |vm, world, _, a| {
        let i = index(a)?;
        let b = bit(vm, world, arg(a, 1))?;
        obj(world, i).using &= !b;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_clearallusing", |_, world, _, a| {
        let i = index(a)?;
        obj(world, i).using = 0;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setprogress", |_, world, _, a| {
        let i = index(a)?;
        let f = num(a, 1)? * 255.0;
        obj(world, i).progress = if f > 0.0 { f.min(255.0) as u8 } else { 0 };
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setsize", |vm, world, _, a| {
        let i = index(a)?;
        let size = if a.len() == 2 {
            let n = entnum(vm, arg(a, 1)).ok_or("Illegal entity parameter for Objective_SetSize.")?;
            // Its bounds across: a box trigger's width and length, a
            // radius trigger's diameter.
            let zm = world.resource::<Zm>();
            let e = zm.ents.get(&n).ok_or("Illegal entity parameter for Objective_SetSize.")?;
            match e.box_dims {
                Some(d) => [d[0], d[1]],
                None => [e.reach * 2.0, e.reach * 2.0],
            }
        } else {
            [num(a, 1)?, num(a, 2)?]
        };
        obj(world, i).size = size;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_setgamemodeflags", |_, world, _, a| {
        let i = index(a)?;
        let v = int(a, 1)?;
        obj(world, i).gamemode_flags = v as u8;
        Ok(ScriptValue::Undefined)
    });
    f!("objective_getgamemodeflags", |_, world, _, a| {
        let i = index(a)?;
        Ok(ScriptValue::Int(i32::from(world.resource::<Zm>().goal_list[i].gamemode_flags)))
    });
}

/// Each client's objectives (`bo2mp_objectives`), as the engine sends
/// them: not empty, and named, or of no team or his team and not hidden
/// from him. One row each, `;` between:
/// `index,state,x,y,z,entity,icon,name,teammask,progress,flags,team,using`
/// (entity -1 = none; its position is the entity's own, a player's his
/// HUD takes from its snapshot; `using` the clients using it, a hex bit
/// mask).
pub(super) fn post_goals(world: &mut World, clients: &[(u32, String)]) {
    let rows: Vec<(usize, MissionMarker, [f32; 3])> = {
        let objs: Vec<(usize, MissionMarker)> = world
            .resource::<Zm>()
            .goal_list
            .iter()
            .enumerate()
            .where_value(|o| o.state != 0)
            .map(|(i, o)| (i, o.clone()))
            .collect();
        objs
            .into_iter()
            .map(|(i, o)| {
                let at = o.entity.and_then(|n| entity_origin(world, n)).unwrap_or(o.spot);
                (i, o, at)
            })
            .collect()
    };
    // bo2mp: the client scripts' own objs (a separate 32, after the
    // server's): Express's train cars and the Hellstorm, on every client,
    // each at its entity's origin and yaw. Their last three columns are
    // BO2's objectiveMapInfo bits in hex (2 rotateWithEntity, 8 clipToMap,
    // 0x10 colorMaterialInCode), the icon size and the yaw, then for one
    // coloured in code its colour as this client sees it.
    let train = super::clientfields::client_objectives(world);
    let clean = |s: &str| s.replace([',', ';'], "_");
    for (c, team) in clients {
        let team = team_number(team).unwrap_or(0);
        let me = 1u64.checked_shl(*c).unwrap_or(0);
        let shown = |o: &MissionMarker| !o.name.is_empty() || ((o.team == 0 || o.team == team) && o.hidden & me == 0);
        let train = train.iter().enumerate().map(|(k, o)| {
            let (n, icon, at, yaw) = (o.n, o.icon, o.at, o.heading);
            let tint = match o.owner {
                Some(owner) if o.map_info & 0x10 != 0 => relative_team_colour(owner, *c, clients),
                _ => "",
            };
            format!("{},1,{},{},{},{n},{icon},,0,0,0,0,0,{:x},{},{yaw},{tint}", 32 + k, at[0], at[1], at[2], o.map_info, o.size)
        });
        let line = rows
            .iter()
            .filter(|row| shown(&row.1))
            .map(|(i, o, at)| {
                format!(
                    "{i},{},{},{},{},{},{},{},{},{},{},{},{:x}",
                    o.state,
                    at[0],
                    at[1],
                    at[2],
                    o.entity.map_or(-1, i64::from),
                    clean(&o.icon),
                    clean(&o.name),
                    o.team_mask,
                    o.progress,
                    o.gamemode_flags,
                    o.team,
                    o.using
                )
            })
            .chain(train)
            .collect::<Vec<_>>()
            .join(";");
        super::set_client_dvar(world, *c, "bo2mp_objectives", &line);
    }
}

/// bo2mp: BO2's `CG_RelativeTeamColor` (its `g_TeamColor_` colours, as
/// rrggbb), for a mark coloured in code (the Hellstorm's), as client `me`
/// sees one fired by `owner`: his own or his team's in MyTeam green, a
/// spectator's in Spectator grey, anyone else's (every other player's in
/// free-for-all) in EnemyTeam red.
///
/// Not ported: the party colour and the colour-blind Alt colours.
fn relative_team_colour(owner: u32, me: u32, clients: &[(u32, String)]) -> &'static str {
    let team = |c: u32| clients.iter().find(|(n, _)| *n == c).and_then(|(_, t)| team_number(t).ok()).unwrap_or(0);
    let theirs = team(owner);
    if theirs == 9 {
        "404040"
    } else if owner == me || (theirs != 0 && theirs == team(me)) {
        "66b366"
    } else {
        "ff5059"
    }
}

#[cfg(test)]
#[test]
fn a_hellstorms_mark_is_green_to_its_team_and_red_to_the_rest() {
    let clients = [(0, "allies".to_owned()), (1, "allies".to_owned()), (2, "axis".to_owned())];
    assert_eq!(relative_team_colour(0, 0, &clients), "66b366");
    assert_eq!(relative_team_colour(0, 1, &clients), "66b366");
    assert_eq!(relative_team_colour(0, 2, &clients), "ff5059");
    // Free-for-all: nobody's team is set, so only his own is green.
    let ffa = [(0, "free".to_owned()), (1, "free".to_owned())];
    assert_eq!(relative_team_colour(0, 0, &ffa), "66b366");
    assert_eq!(relative_team_colour(0, 1, &ffa), "ff5059");
}

/// Where an objective on an entity is: a player's is his own (the HUD
/// follows him in its snapshot, so the row does not change as he runs);
/// another entity's, its origin.
fn entity_origin(world: &mut World, n: u32) -> Option<[f32; 3]> {
    if world.resource::<Zm>().players.contains_key(&n) {
        return None;
    }
    world.resource::<Zm>().ents.get(&n).map(|e| e.origin)
}
