//! Everything the host tells one client about itself in a snapshot: controls, loadout, ammo,
//! score, HUD, shock and view effects, the weapon lock and the weapon-view inputs.

use sim::{
    ClientSnapshotMeta, KillcamHud, LoadoutSpec, MENU_COMMAND_TAIL, MissileTarget, RadarMode,
    SteeredMissile, ScriptControls, WeaponLock,
};

use super::codec::{
    Wire, pack_bits, put_flagged, put_list, put_short_list, refuse, take_flagged,
    take_list, take_short_list, unpack_bits,
};
use super::screen::{put_hud_bank, put_shellshock, take_hud_bank, take_shellshock};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

wire_fields!(LoadoutSpec {
    class_id,
    revision,
    primary,
    secondary,
    primary_attachments,
    secondary_attachments,
    lethal,
    tactical,
    perks,
});

/// Nine script switches as bytes, the walk-by-walk lock-out switches as one packed byte. The
/// "fire disabled" switch has never been sent; a client always sees it off.
impl Wire for ScriptControls {
    fn put(&self, wr: &mut ByteSinkWriter) {
        [
            self.stuck,
            self.weapons_disabled,
            self.offhands_disabled,
            self.switch_disabled,
            self.jump_disabled,
            self.usability_disabled,
            self.paired,
            self.stunned,
            self.slick,
        ]
        .put(wr);
        self.switch_to.put(wr);
        pack_bits([
            self.prone_disabled,
            self.crouch_disabled,
            self.stand_disabled,
            self.sprint_disabled,
            self.melee_disabled,
            self.ads_disabled,
        ])
        .put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let [frozen, weapons_disabled, offhands_disabled, switch_disabled, jump_disabled, usability_disabled, linked, stunned, slick] =
            <[bool; 9]>::take(rd)?;
        let switch_to = u32::take(rd)?;
        let [prone_disabled, crouch_disabled, stand_disabled, sprint_disabled, melee_disabled, ads_disabled] =
            unpack_bits(rd.get_u8()?);
        Ok(Self {
            stuck: frozen,
            weapons_disabled,
            offhands_disabled,
            switch_disabled,
            jump_disabled,
            usability_disabled,
            paired: linked,
            stunned,
            slick,
            prone_disabled,
            crouch_disabled,
            stand_disabled,
            sprint_disabled,
            melee_disabled,
            ads_disabled,
            switch_to,
            ..Self::default()
        })
    }
}

/// The kill-cam banner: absent, shown, or shown for the final kill.
fn put_killcam(wr: &mut ByteSinkWriter, banner: &Option<KillcamHud>) {
    wr.put_u8(match banner {
        None => 0,
        Some(KillcamHud { final_kill: false }) => 1,
        Some(KillcamHud { final_kill: true }) => 2,
    });
}

fn take_killcam(rd: &mut ByteCursorReader<'_>) -> Result<Option<KillcamHud>, PacketDecodeFault> {
    match rd.get_u8()? {
        0 => Ok(None),
        1 => Ok(Some(KillcamHud { final_kill: false })),
        2 => Ok(Some(KillcamHud { final_kill: true })),
        _ => refuse("bad killcam HUD tag"),
    }
}

impl Wire for RadarMode {
    fn put(&self, wr: &mut ByteSinkWriter) {
        wr.put_u8(self.net_code());
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match Self::from_net_code(rd.get_u8()?) {
            Some(mode) => Ok(mode),
            None => refuse("bad radar mode"),
        }
    }
}

impl Wire for SteeredMissile {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.projectile.put(wr);
        self.ent_no.put(wr);
        self.turn.put(wr);
        pack_bits([self.armed, self.boosted, self.attack, self.coasting, self.t6, self.holding])
            .put(wr);
        self.unlink_at_ms.unwrap_or(i32::MIN).put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let missile_ent = Wire::take(rd)?;
        let entnum = i32::take(rd)?;
        let angles = <[f32; 3]>::take(rd)?;
        let [armed, boosted, attack, coasting, t6, held] = unpack_bits(rd.get_u8()?);
        let unlink_at_ms = Some(i32::take(rd)?).filter(|&ms| ms != i32::MIN);
        Ok(Self { projectile: missile_ent, ent_no: entnum, turn: angles, armed, boosted, attack, coasting, unlink_at_ms, t6, holding: held })
    }
}

/// What a lock-on is aimed at: nothing, a point, an entity, or a player (then its offset).
fn put_aim(wr: &mut ByteSinkWriter, aim: &Option<MissileTarget>) {
    let offset = match aim {
        None => {
            wr.put_u8(0);
            return;
        }
        Some(MissileTarget::Point(point)) => {
            wr.put_u8(1);
            point
        }
        Some(MissileTarget::Entity { entity, offset }) => {
            wr.put_u8(2);
            entity.put(wr);
            offset
        }
        Some(MissileTarget::Player { client, life, offset }) => {
            wr.put_u8(3);
            client.put(wr);
            life.put(wr);
            offset
        }
    };
    offset.put(wr);
}

fn take_aim(rd: &mut ByteCursorReader<'_>) -> Result<Option<MissileTarget>, PacketDecodeFault> {
    let kind = u8::take(rd)?;
    if kind == 0 {
        return Ok(None);
    }
    if !(1..=3).contains(&kind) {
        return refuse("bad missile target tag");
    }
    let first = if kind >= 2 { Some(u32::take(rd)?) } else { None };
    let second = if kind >= 2 { Some(u32::take(rd)?) } else { None };
    let offset = <[f32; 3]>::take(rd)?;
    Ok(Some(match (kind, first, second) {
        (2, Some(raw_num), Some(generation)) => {
            match sim::EntHandle::from_parts(raw_num as i32, generation) {
                Ok(entity) => MissileTarget::Entity { entity, offset },
                Err(_) => return refuse("bad missile target entity"),
            }
        }
        (3, Some(raw_client), Some(life)) => MissileTarget::Player {
            client: sim::ClientId(raw_client),
            life: sim::LifeSequence(life),
            offset,
        },
        _ => MissileTarget::Point(offset),
    }))
}

impl Wire for WeaponLock {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.gun.put(wr);
        self.life_no.put(wr);
        self.bits.put(wr);
        self.target.put(wr);
        put_aim(wr, &self.aim);
        self.acquire_started_at.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let weapon = u32::take(rd)?;
        let life = u32::take(rd)?;
        let flags = rd.get_u8()?;
        let target = <[f32; 3]>::take(rd)?;
        let aim = take_aim(rd)?;
        let acquire_started_at = i32::take(rd)?;
        Ok(Self { gun: weapon, life_no: life, bits: flags, target, aim, acquire_started_at })
    }
}

impl Wire for ClientSnapshotMeta {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.controls.put(wr);
        put_killcam(wr, &self.killcam_hud);
        self.lifecycle.put(wr);
        self.life_sequence.put(wr);
        self.item_ready_ms.put(wr);
        put_flagged(wr, &self.item_use_entity);
        [self.mag_rounds, self.ammo_stock, self.score, self.kills, self.deaths, self.kill_streak]
            .put(wr);
        self.radar.put(wr);
        put_flagged(wr, &self.remote_missile);
        put_flagged(wr, &self.loadout);
        put_list(wr, &self.rounds_per_gun);
        put_short_list(wr, &self.taped_mag_spent);
        self.weapon_shot_count.put(wr);
        pack_bits([self.burst_latch, self.burst_latch_secondary]).put(wr);
        pack_bits([self.rechamber_pending, self.rechamber_pending_secondary]).put(wr);
        put_flagged(wr, &self.died_at_step);
        self.look_at_killer_yaw.put(wr);
        self.name.put(wr);
        put_hud_bank(wr, &self.hud_archival);
        put_hud_bank(wr, &self.hud_current);
        put_flagged(wr, &self.ffa_team);
        [self.client_state_team, self.rank, self.prestige].put(wr);
        [self.card_icon, self.card_title, self.card_plate].put(wr);
        put_list(wr, &self.client_dvars);
        put_shellshock(wr, &self.shellshock);
        self.view_effects.put(wr);
        debug_assert!(self.menu_commands.len() <= MENU_COMMAND_TAIL);
        put_short_list(wr, &self.menu_commands);
        put_flagged(wr, &self.location_selection);
        self.weapon_lock.put(wr);
        put_flagged(wr, &self.t6_view);
        self.t6_perks.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let controls = ScriptControls::take(rd)?;
        let killcam_hud_val = take_killcam(rd)?;
        let lifecycle = Wire::take(rd)?;
        let life_sequence_val = Wire::take(rd)?;
        let item_use_spawn_ms_val = i32::take(rd)?;
        let item_use_entity_val = take_flagged(rd, "bad EntityKernel relation tag")?;
        let [ammo_clip, ammo_stock_val, score, kills, deaths, kill_streak] = <[i32; 6]>::take(rd)?;
        let radar = RadarMode::take(rd)?;
        let remote_missile = take_flagged(rd, "bad remote missile tag")?;
        let loadout = take_flagged(rd, "bad loadout tag")?;
        let ammo_by_weapon_val = take_list(rd, 32)?;
        let taped_mag_spent = take_short_list(rd, 255)?;
        let weapon_shot_count_val = u8::take(rd)?;
        let [burst_latch_val, burst_latch_secondary] = unpack_bits(rd.get_u8()?);
        let [rechamber_pending_val, rechamber_pending_secondary] = unpack_bits(rd.get_u8()?);
        let dead_since_tick_val = take_flagged(rd, "bad dead_since_tick tag")?;
        let look_at_killer_yaw_val = i32::take(rd)?;
        let name = <[u8; 16]>::take(rd)?;
        let hud_archival_val = take_hud_bank(rd)?;
        let hud_current_val = take_hud_bank(rd)?;
        let ffa_team = take_flagged(rd, "bad ffa_team tag")?;
        let [client_state_team_val, rank, prestige] = <[i32; 3]>::take(rd)?;
        let [player_card_icon_val, player_card_title_val, player_card_nameplate_val] =
            <[u32; 3]>::take(rd)?;
        let client_dvars = take_list(rd, 16)?;
        let shellshock = take_shellshock(rd)?;
        let view_effects = Wire::take(rd)?;
        let menu_commands = take_short_list(rd, 255)?;
        let location_selection = take_flagged(rd, "bad location selection tag")?;
        let weapon_lock = WeaponLock::take(rd)?;
        let t6_view = take_flagged(rd, "bad t6 view tag")?;
        let t6_perks = u64::take(rd)?;
        Ok(Self {
            controls,
            weapon_lock,
            killcam_hud: killcam_hud_val,
            lifecycle,
            loadout,
            life_sequence: life_sequence_val,
            item_ready_ms: item_use_spawn_ms_val,
            item_use_entity: item_use_entity_val,
            mag_rounds: ammo_clip,
            ammo_stock: ammo_stock_val,
            score,
            kills,
            deaths,
            kill_streak,
            radar,
            remote_missile,
            rounds_per_gun: ammo_by_weapon_val,
            taped_mag_spent,
            weapon_shot_count: weapon_shot_count_val,
            burst_latch: burst_latch_val,
            burst_latch_secondary,
            rechamber_pending: rechamber_pending_val,
            rechamber_pending_secondary,
            died_at_step: dead_since_tick_val,
            look_at_killer_yaw: look_at_killer_yaw_val,
            name,
            hud_archival: hud_archival_val,
            hud_current: hud_current_val,
            ffa_team,
            client_state_team: client_state_team_val,
            rank,
            prestige,
            card_icon: player_card_icon_val,
            card_title: player_card_title_val,
            card_plate: player_card_nameplate_val,
            client_dvars,
            shellshock,
            view_effects,
            menu_commands,
            location_selection,
            t6_view,
            t6_perks,
        })
    }
}
