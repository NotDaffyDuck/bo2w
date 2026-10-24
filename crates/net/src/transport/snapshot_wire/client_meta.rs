//! Everything the host tells one client about itself in a snapshot: controls, loadout, ammo,
//! score, HUD, shock and view effects, the weapon lock and the weapon-view inputs.

use sim::{
    ClientSnapshotMeta, KillcamHud, LoadoutSpec, MENU_COMMAND_TAIL, MissileTarget, RadarMode,
    RemoteMissile, ScriptControls, WeaponLock,
};

use super::codec::{
    Decoded, Wire, pack_bits, put_flagged, put_list, put_short_list, refuse, take_flagged,
    take_list, take_short_list, unpack_bits,
};
use super::screen::{put_hud_bank, put_shellshock, take_hud_bank, take_shellshock};
use crate::transport::wire::{WireReader, WireWriter};

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

/// Eight script switches as bytes, the walk-by-walk lock-out switches as one packed byte. The
/// "fire disabled" switch has never been sent; a client always sees it off.
impl Wire for ScriptControls {
    fn put(&self, out: &mut WireWriter) {
        [
            self.frozen,
            self.weapons_disabled,
            self.offhands_disabled,
            self.switch_disabled,
            self.jump_disabled,
            self.usability_disabled,
            self.linked,
            self.stunned,
        ]
        .put(out);
        self.switch_to.put(out);
        pack_bits([
            self.prone_disabled,
            self.crouch_disabled,
            self.stand_disabled,
            self.sprint_disabled,
            self.melee_disabled,
            self.ads_disabled,
        ])
        .put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let [frozen, weapons_disabled, offhands_disabled, switch_disabled, jump_disabled, usability_disabled, linked, stunned] =
            <[bool; 8]>::take(input)?;
        let switch_to = u32::take(input)?;
        let [prone_disabled, crouch_disabled, stand_disabled, sprint_disabled, melee_disabled, ads_disabled] =
            unpack_bits(input.get_u8()?);
        Ok(Self {
            frozen,
            weapons_disabled,
            offhands_disabled,
            switch_disabled,
            jump_disabled,
            usability_disabled,
            linked,
            stunned,
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
fn put_killcam(out: &mut WireWriter, banner: &Option<KillcamHud>) {
    out.put_u8(match banner {
        None => 0,
        Some(KillcamHud { final_kill: false }) => 1,
        Some(KillcamHud { final_kill: true }) => 2,
    });
}

fn take_killcam(input: &mut WireReader<'_>) -> Decoded<Option<KillcamHud>> {
    match input.get_u8()? {
        0 => Ok(None),
        1 => Ok(Some(KillcamHud { final_kill: false })),
        2 => Ok(Some(KillcamHud { final_kill: true })),
        _ => refuse("bad killcam HUD tag"),
    }
}

impl Wire for RadarMode {
    fn put(&self, out: &mut WireWriter) {
        out.put_u8(self.wire_tag());
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match Self::from_wire_tag(input.get_u8()?) {
            Some(mode) => Ok(mode),
            None => refuse("bad radar mode"),
        }
    }
}

impl Wire for RemoteMissile {
    fn put(&self, out: &mut WireWriter) {
        self.projectile.put(out);
        self.entnum.put(out);
        self.angles.put(out);
        pack_bits([self.armed, self.boosted, self.attack, self.coasting, self.t6, self.held])
            .put(out);
        self.unlink_at_ms.unwrap_or(i32::MIN).put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let projectile = Wire::take(input)?;
        let entnum = i32::take(input)?;
        let angles = <[f32; 3]>::take(input)?;
        let [armed, boosted, attack, coasting, t6, held] = unpack_bits(input.get_u8()?);
        let unlink_at_ms = Some(i32::take(input)?).filter(|&ms| ms != i32::MIN);
        Ok(Self { projectile, entnum, angles, armed, boosted, attack, coasting, unlink_at_ms, t6, held })
    }
}

/// What a lock-on is aimed at: nothing, a point, an entity, or a player (then its offset).
fn put_aim(out: &mut WireWriter, aim: &Option<MissileTarget>) {
    let offset = match aim {
        None => {
            out.put_u8(0);
            return;
        }
        Some(MissileTarget::Point(point)) => {
            out.put_u8(1);
            point
        }
        Some(MissileTarget::Entity { entity, offset }) => {
            out.put_u8(2);
            entity.put(out);
            offset
        }
        Some(MissileTarget::Player { client, life, offset }) => {
            out.put_u8(3);
            client.put(out);
            life.put(out);
            offset
        }
    };
    offset.put(out);
}

fn take_aim(input: &mut WireReader<'_>) -> Decoded<Option<MissileTarget>> {
    let tag = input.get_u8()?;
    if tag == 0 {
        return Ok(None);
    }
    if !(1..=3).contains(&tag) {
        return refuse("bad missile target tag");
    }
    let first = if tag >= 2 { Some(u32::take(input)?) } else { None };
    let second = if tag >= 2 { Some(u32::take(input)?) } else { None };
    let offset = <[f32; 3]>::take(input)?;
    Ok(Some(match (tag, first, second) {
        (2, Some(number), Some(generation)) => {
            match sim::EntityRef::from_parts(number as i32, generation) {
                Ok(entity) => MissileTarget::Entity { entity, offset },
                Err(_) => return refuse("bad missile target entity"),
            }
        }
        (3, Some(client), Some(life)) => MissileTarget::Player {
            client: sim::ClientId(client),
            life: sim::LifeSequence(life),
            offset,
        },
        _ => MissileTarget::Point(offset),
    }))
}

impl Wire for WeaponLock {
    fn put(&self, out: &mut WireWriter) {
        self.weapon.put(out);
        self.life.put(out);
        self.flags.put(out);
        self.target.put(out);
        put_aim(out, &self.aim);
        self.acquire_started_at.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let weapon = u32::take(input)?;
        let life = u32::take(input)?;
        let flags = input.get_u8()?;
        let target = <[f32; 3]>::take(input)?;
        let aim = take_aim(input)?;
        let acquire_started_at = i32::take(input)?;
        Ok(Self { weapon, life, flags, target, aim, acquire_started_at })
    }
}

impl Wire for ClientSnapshotMeta {
    fn put(&self, out: &mut WireWriter) {
        self.controls.put(out);
        put_killcam(out, &self.killcam_hud);
        self.lifecycle.put(out);
        self.life_sequence.put(out);
        self.item_use_spawn_ms.put(out);
        put_flagged(out, &self.item_use_entity);
        [self.ammo_clip, self.ammo_stock, self.score, self.kills, self.deaths, self.kill_streak]
            .put(out);
        self.radar.put(out);
        put_flagged(out, &self.remote_missile);
        put_flagged(out, &self.loadout);
        put_list(out, &self.ammo_by_weapon);
        put_short_list(out, &self.taped_mag_spent);
        self.weapon_shot_count.put(out);
        pack_bits([self.burst_latch, self.burst_latch_secondary]).put(out);
        pack_bits([self.rechamber_pending, self.rechamber_pending_secondary]).put(out);
        put_flagged(out, &self.dead_since_tick);
        self.look_at_killer_yaw.put(out);
        self.name.put(out);
        put_hud_bank(out, &self.hud_archival);
        put_hud_bank(out, &self.hud_current);
        put_flagged(out, &self.ffa_team);
        [self.client_state_team, self.rank, self.prestige].put(out);
        [self.player_card_icon, self.player_card_title, self.player_card_nameplate].put(out);
        put_list(out, &self.client_dvars);
        put_shellshock(out, &self.shellshock);
        self.view_effects.put(out);
        debug_assert!(self.menu_commands.len() <= MENU_COMMAND_TAIL);
        put_short_list(out, &self.menu_commands);
        put_flagged(out, &self.location_selection);
        self.weapon_lock.put(out);
        put_flagged(out, &self.t6_view);
        self.t6_perks.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let controls = ScriptControls::take(input)?;
        let killcam_hud = take_killcam(input)?;
        let lifecycle = Wire::take(input)?;
        let life_sequence = Wire::take(input)?;
        let item_use_spawn_ms = i32::take(input)?;
        let item_use_entity = take_flagged(input, "bad EntityKernel relation tag")?;
        let [ammo_clip, ammo_stock, score, kills, deaths, kill_streak] = <[i32; 6]>::take(input)?;
        let radar = RadarMode::take(input)?;
        let remote_missile = take_flagged(input, "bad remote missile tag")?;
        let loadout = take_flagged(input, "bad loadout tag")?;
        let ammo_by_weapon = take_list(input, 32)?;
        let taped_mag_spent = take_short_list(input, 255)?;
        let weapon_shot_count = input.get_u8()?;
        let [burst_latch, burst_latch_secondary] = unpack_bits(input.get_u8()?);
        let [rechamber_pending, rechamber_pending_secondary] = unpack_bits(input.get_u8()?);
        let dead_since_tick = take_flagged(input, "bad dead_since_tick tag")?;
        let look_at_killer_yaw = i32::take(input)?;
        let name = <[u8; 16]>::take(input)?;
        let hud_archival = take_hud_bank(input)?;
        let hud_current = take_hud_bank(input)?;
        let ffa_team = take_flagged(input, "bad ffa_team tag")?;
        let [client_state_team, rank, prestige] = <[i32; 3]>::take(input)?;
        let [player_card_icon, player_card_title, player_card_nameplate] =
            <[u32; 3]>::take(input)?;
        let client_dvars = take_list(input, 16)?;
        let shellshock = take_shellshock(input)?;
        let view_effects = Wire::take(input)?;
        let menu_commands = take_short_list(input, 255)?;
        let location_selection = take_flagged(input, "bad location selection tag")?;
        let weapon_lock = WeaponLock::take(input)?;
        let t6_view = take_flagged(input, "bad t6 view tag")?;
        let t6_perks = u64::take(input)?;
        Ok(Self {
            controls,
            weapon_lock,
            killcam_hud,
            lifecycle,
            loadout,
            life_sequence,
            item_use_spawn_ms,
            item_use_entity,
            ammo_clip,
            ammo_stock,
            score,
            kills,
            deaths,
            kill_streak,
            radar,
            remote_missile,
            ammo_by_weapon,
            taped_mag_spent,
            weapon_shot_count,
            burst_latch,
            burst_latch_secondary,
            rechamber_pending,
            rechamber_pending_secondary,
            dead_since_tick,
            look_at_killer_yaw,
            name,
            hud_archival,
            hud_current,
            ffa_team,
            client_state_team,
            rank,
            prestige,
            player_card_icon,
            player_card_title,
            player_card_nameplate,
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
