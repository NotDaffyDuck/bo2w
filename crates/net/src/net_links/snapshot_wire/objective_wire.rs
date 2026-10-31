//! Match objectives and the scripted look of the world: compass markers, server info, scripted
//! effects, camera visions, fog, earthquakes and the vehicles drawn on the compass.

use gamemode_t6::Team;
use sim::{
    CompassMark, CompassRide, MAX_SCRIPT_EARTHQUAKES, MatchBoard, ObjectiveState,
    ScriptEarthquake, PlacedEffect, ScriptFog, ScriptFogParams, ScriptSunFog,
};

use super::codec::{
    Wire, put_flagged, put_list, put_short_list, refuse, take_counted, take_flagged,
    take_list, take_short_list,
};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Rows reserved for before the bytes behind a count prove they exist.
const ROW_RESERVE: usize = 16;

impl Wire for CompassMark {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.index.put(wr);
        (self.state as u8).put(wr);
        self.origin.put(wr);
        (self.team as u8).put(wr);
        self.icon.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let index = u8::take(rd)?;
        let Some(state) = ObjectiveState::from_u8(rd.get_u8()?) else {
            return refuse("objective state");
        };
        let origin = <[f32; 3]>::take(rd)?;
        let Some(team) = Team::from_packed_u8(rd.get_u8()?) else {
            return refuse("objective team");
        };
        let icon = String::take(rd)?;
        Ok(Self { index, state, origin, team, icon })
    }
}

/// The start time is always on the wire, as a zero when the effect has not been triggered.
impl Wire for PlacedEffect {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.id.put(wr);
        self.effect.put(wr);
        self.origin.put(wr);
        self.forward.put(wr);
        self.up_dir.put(wr);
        self.start_ms.is_some().put(wr);
        self.start_ms.unwrap_or(0).put(wr);
        self.repeat_ms.put(wr);
        self.cull_distance.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let id = u32::take(rd)?;
        let effect = u8::take(rd)?;
        let origin = <[f32; 3]>::take(rd)?;
        let forward = <[f32; 3]>::take(rd)?;
        let up = <[f32; 3]>::take(rd)?;
        let triggered = bool::take(rd)?;
        let start = i32::take(rd)?;
        Ok(Self {
            id,
            effect,
            origin,
            forward,
            up_dir: up,
            start_ms: triggered.then_some(start),
            repeat_ms: i32::take(rd)?,
            cull_distance: f32::take(rd)?,
        })
    }
}

wire_fields!(ScriptSunFog { tint_rgb, sun_way, from_deg, to_deg, scale });

impl Wire for ScriptFogParams {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.from_dist.put(wr);
        self.half_dist.put(wr);
        self.tint_rgb.put(wr);
        self.thickest.put(wr);
        put_flagged(wr, &self.sun);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let fog = Self {
            from_dist: f32::take(rd)?,
            half_dist: f32::take(rd)?,
            tint_rgb: <[f32; 3]>::take(rd)?,
            thickest: f32::take(rd)?,
            sun: take_flagged(rd, "invalid fog sun tag")?,
        };
        if fog.valid() { Ok(fog) } else { refuse("invalid fog parameters") }
    }
}

impl Wire for ScriptFog {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.from.put(wr);
        self.to.put(wr);
        self.began_ms.put(wr);
        self.lasts_ms.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let fog = Self {
            from: ScriptFogParams::take(rd)?,
            to: ScriptFogParams::take(rd)?,
            began_ms: i32::take(rd)?,
            lasts_ms: i32::take(rd)?,
        };
        if fog.lasts_ms < 0 { refuse("negative fog duration") } else { Ok(fog) }
    }
}

impl Wire for ScriptEarthquake {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.id.put(wr);
        self.origin.put(wr);
        self.scale.put(wr);
        self.reach.put(wr);
        self.began_ms.put(wr);
        self.lasts_ms.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let quake = Self {
            id: u32::take(rd)?,
            origin: <[f32; 3]>::take(rd)?,
            scale: f32::take(rd)?,
            reach: f32::take(rd)?,
            began_ms: i32::take(rd)?,
            lasts_ms: i32::take(rd)?,
        };
        if quake.valid() { Ok(quake) } else { refuse("invalid earthquake") }
    }
}

/// The two icons are written one after the other, each behind its own length.
impl Wire for CompassRide {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.origin.put(wr);
        self.turn_deg.put(wr);
        self.owner.put(wr);
        self.team.put(wr);
        for icon in &self.icons {
            icon.put(wr);
        }
        self.size.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let origin = <[f32; 3]>::take(rd)?;
        let yaw = f32::take(rd)?;
        let owner = u32::take(rd)?;
        let team = i32::take(rd)?;
        let first_icon = String::take(rd)?;
        let second_icon = String::take(rd)?;
        let size = <[i32; 2]>::take(rd)?;
        Ok(Self { origin, turn_deg: yaw, owner, team, icons: [first_icon, second_icon], size })
    }
}

impl Wire for MatchBoard {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.scores.put(wr);
        put_short_list(wr, &self.compass);
        put_list(wr, &self.server_info);
        self.game_end_time.put(wr);
        self.scripted_effects.put(wr);
        put_list(wr, &self.effects);
        put_flagged(wr, &self.naked_vision);
        put_flagged(wr, &self.thermal_vision);
        put_flagged(wr, &self.missile_vision);
        put_flagged(wr, &self.night_vision);
        put_flagged(wr, &self.pain_vision);
        put_flagged(wr, &self.fog);
        debug_assert!(self.earthquakes.len() <= MAX_SCRIPT_EARTHQUAKES);
        put_short_list(wr, &self.earthquakes);
        put_list(wr, &self.vehicles);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let scores = <[i32; 3]>::take(rd)?;
        let compass = take_short_list(rd, ROW_RESERVE)?;
        let server_info = take_list(rd, ROW_RESERVE)?;
        let game_end_time = i32::take(rd)?;
        let scripted_effects = bool::take(rd)?;
        let effects = take_list(rd, ROW_RESERVE)?;
        let naked_vision = take_flagged(rd, "bad vision tag")?;
        let thermal_vision_val = take_flagged(rd, "bad vision tag")?;
        let missile_vision = take_flagged(rd, "bad vision tag")?;
        let night_vision = take_flagged(rd, "bad vision tag")?;
        let pain_vision = take_flagged(rd, "bad vision tag")?;
        let fog = take_flagged(rd, "invalid fog tag")?;
        let quake_count = usize::from(rd.get_u8()?);
        if quake_count > MAX_SCRIPT_EARTHQUAKES {
            return refuse("too many earthquakes");
        }
        let earthquakes = take_counted(rd, quake_count, MAX_SCRIPT_EARTHQUAKES)?;
        let vehicles = take_list(rd, ROW_RESERVE)?;
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
            thermal_vision: thermal_vision_val,
            missile_vision,
            night_vision,
            pain_vision,
        })
    }
}
