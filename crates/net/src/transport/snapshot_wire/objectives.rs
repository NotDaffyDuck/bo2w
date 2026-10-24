//! Match objectives and the scripted look of the world: compass markers, server info, scripted
//! effects, camera visions, fog, earthquakes and the vehicles drawn on the compass.

use gamemode_t6::Team;
use sim::{
    CompassObjective, CompassVehicle, MAX_SCRIPT_EARTHQUAKES, ObjectiveMatch, ObjectiveState,
    ScriptEarthquake, ScriptEffect, ScriptFog, ScriptFogParams, ScriptSunFog,
};

use super::codec::{
    Decoded, Wire, put_flagged, put_list, put_short_list, refuse, take_counted, take_flagged,
    take_list, take_short_list,
};
use crate::transport::wire::{WireReader, WireWriter};

/// Rows reserved for before the bytes behind a count prove they exist.
const ROW_RESERVE: usize = 16;

impl Wire for CompassObjective {
    fn put(&self, out: &mut WireWriter) {
        self.index.put(out);
        (self.state as u8).put(out);
        self.origin.put(out);
        (self.team as u8).put(out);
        self.icon.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let index = u8::take(input)?;
        let Some(state) = ObjectiveState::from_u8(input.get_u8()?) else {
            return refuse("objective state");
        };
        let origin = <[f32; 3]>::take(input)?;
        let Some(team) = Team::from_packed_u8(input.get_u8()?) else {
            return refuse("objective team");
        };
        let icon = String::take(input)?;
        Ok(Self { index, state, origin, team, icon })
    }
}

/// The start time is always on the wire, as a zero when the effect has not been triggered.
impl Wire for ScriptEffect {
    fn put(&self, out: &mut WireWriter) {
        self.id.put(out);
        self.effect.put(out);
        self.origin.put(out);
        self.forward.put(out);
        self.up.put(out);
        self.start_ms.is_some().put(out);
        self.start_ms.unwrap_or(0).put(out);
        self.repeat_ms.put(out);
        self.cull_distance.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let id = u32::take(input)?;
        let effect = u8::take(input)?;
        let origin = <[f32; 3]>::take(input)?;
        let forward = <[f32; 3]>::take(input)?;
        let up = <[f32; 3]>::take(input)?;
        let triggered = bool::take(input)?;
        let start = i32::take(input)?;
        Ok(Self {
            id,
            effect,
            origin,
            forward,
            up,
            start_ms: triggered.then_some(start),
            repeat_ms: i32::take(input)?,
            cull_distance: f32::take(input)?,
        })
    }
}

wire_fields!(ScriptSunFog { color_rgb, sun_dir, begin_angle_deg, end_angle_deg, scale });

impl Wire for ScriptFogParams {
    fn put(&self, out: &mut WireWriter) {
        self.start_dist.put(out);
        self.halfway_dist.put(out);
        self.color_rgb.put(out);
        self.max_opacity.put(out);
        put_flagged(out, &self.sun);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let fog = Self {
            start_dist: f32::take(input)?,
            halfway_dist: f32::take(input)?,
            color_rgb: <[f32; 3]>::take(input)?,
            max_opacity: f32::take(input)?,
            sun: take_flagged(input, "invalid fog sun tag")?,
        };
        if fog.valid() { Ok(fog) } else { refuse("invalid fog parameters") }
    }
}

impl Wire for ScriptFog {
    fn put(&self, out: &mut WireWriter) {
        self.from.put(out);
        self.to.put(out);
        self.start_ms.put(out);
        self.duration_ms.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let fog = Self {
            from: ScriptFogParams::take(input)?,
            to: ScriptFogParams::take(input)?,
            start_ms: i32::take(input)?,
            duration_ms: i32::take(input)?,
        };
        if fog.duration_ms < 0 { refuse("negative fog duration") } else { Ok(fog) }
    }
}

impl Wire for ScriptEarthquake {
    fn put(&self, out: &mut WireWriter) {
        self.id.put(out);
        self.origin.put(out);
        self.scale.put(out);
        self.radius.put(out);
        self.start_ms.put(out);
        self.duration_ms.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let quake = Self {
            id: u32::take(input)?,
            origin: <[f32; 3]>::take(input)?,
            scale: f32::take(input)?,
            radius: f32::take(input)?,
            start_ms: i32::take(input)?,
            duration_ms: i32::take(input)?,
        };
        if quake.valid() { Ok(quake) } else { refuse("invalid earthquake") }
    }
}

/// The two icons are written one after the other, each behind its own length.
impl Wire for CompassVehicle {
    fn put(&self, out: &mut WireWriter) {
        self.origin.put(out);
        self.yaw.put(out);
        self.owner.put(out);
        self.team.put(out);
        for icon in &self.icons {
            icon.put(out);
        }
        self.size.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let origin = <[f32; 3]>::take(input)?;
        let yaw = f32::take(input)?;
        let owner = u32::take(input)?;
        let team = i32::take(input)?;
        let first_icon = String::take(input)?;
        let second_icon = String::take(input)?;
        let size = <[i32; 2]>::take(input)?;
        Ok(Self { origin, yaw, owner, team, icons: [first_icon, second_icon], size })
    }
}

impl Wire for ObjectiveMatch {
    fn put(&self, out: &mut WireWriter) {
        self.scores.put(out);
        put_short_list(out, &self.compass);
        put_list(out, &self.server_info);
        self.game_end_time.put(out);
        self.scripted_effects.put(out);
        put_list(out, &self.effects);
        put_flagged(out, &self.naked_vision);
        put_flagged(out, &self.thermal_vision);
        put_flagged(out, &self.missile_vision);
        put_flagged(out, &self.night_vision);
        put_flagged(out, &self.pain_vision);
        put_flagged(out, &self.fog);
        debug_assert!(self.earthquakes.len() <= MAX_SCRIPT_EARTHQUAKES);
        put_short_list(out, &self.earthquakes);
        put_list(out, &self.vehicles);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let scores = <[i32; 3]>::take(input)?;
        let compass = take_short_list(input, ROW_RESERVE)?;
        let server_info = take_list(input, ROW_RESERVE)?;
        let game_end_time = i32::take(input)?;
        let scripted_effects = bool::take(input)?;
        let effects = take_list(input, ROW_RESERVE)?;
        let naked_vision = take_flagged(input, "bad vision tag")?;
        let thermal_vision = take_flagged(input, "bad vision tag")?;
        let missile_vision = take_flagged(input, "bad vision tag")?;
        let night_vision = take_flagged(input, "bad vision tag")?;
        let pain_vision = take_flagged(input, "bad vision tag")?;
        let fog = take_flagged(input, "invalid fog tag")?;
        let quake_count = usize::from(input.get_u8()?);
        if quake_count > MAX_SCRIPT_EARTHQUAKES {
            return refuse("too many earthquakes");
        }
        let earthquakes = take_counted(input, quake_count, MAX_SCRIPT_EARTHQUAKES)?;
        let vehicles = take_list(input, ROW_RESERVE)?;
        Ok(Self {
            scores,
            compass,
            vehicles,
            server_info,
            game_end_time,
            scripted_effects,
            effects,
            fog,
            earthquakes,
            naked_vision,
            thermal_vision,
            missile_vision,
            night_vision,
            pain_vision,
        })
    }
}
