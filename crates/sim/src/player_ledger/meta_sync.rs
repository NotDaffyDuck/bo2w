//! Moving a client's state into the snapshot meta and back, plus the T6 view reads that go with it.

use super::{PlayerMatchRecord, SeatTickNotes, MENU_COMMAND_TAIL, MenuOrder, MenuCommandKind, T6ViewInputs};
use crate::gun_brain::T6Gun;

impl PlayerMatchRecord {
    /// The T6 values the BO2 viewmodel player reads: the server's from the last snapshot when there is one, else this
    /// side's own brain (a local game). `None` means the brain is off.
    pub fn t6_view(&self) -> Option<T6ViewInputs> {
        self.t6_view.or_else(|| self.t6_predicted_view())
    }

    /// Client prediction: the view values of this side's own brain after the newest predicted move. BO2's viewmodel
    /// reads its predicted player state, so the gun and its state come from the same move (the snapshot's lag the
    /// predicted gun, and a swap would show the new gun for a frame while the old one's drop still runs).
    pub fn t6_predicted_view(&self) -> Option<T6ViewInputs> {
        self.t6_gun.as_deref().map(T6Gun::view_inputs)
    }

    /// This side's own brain's raw weapon state and the offhand item in hand, for BO2's cancel check on a switch press;
    /// `None` when this side runs no T6 brain for them. On a client the newest predicted move wins: a snapshot rewinds
    /// `t6_gun` to the move the server last ran until the next prediction replays, and BO2 asks its latest predicted
    /// state, so a press in a slow frame still sees the grenade the screen shows.
    pub fn t6_offhand(&self) -> Option<(i32, u32)> {
        let newest = self.t6_gun_history.last().map(|(_, gun)| &**gun);
        newest.or(self.t6_gun.as_deref()).map(T6Gun::offhand_in_hand)
    }

    /// Queues a menu command for the client, numbering it after the last and keeping only the newest few.
    pub(crate) fn push_menu_command(&mut self, kind: MenuCommandKind) {
        let serial = self.menu_commands.last().map_or(1, |last| last.serial.wrapping_add(1));
        self.menu_commands.push(MenuOrder { serial, kind });
        if let Some(excess) = self.menu_commands.len().checked_sub(MENU_COMMAND_TAIL) {
            self.menu_commands.drain(..excess);
        }
    }

    /// What goes out in a snapshot about this client. The HUD rows and the killcam are filled in by the caller.
    pub(crate) fn as_wire_meta(&self) -> super::SeatTickNotes {
        let me = self;
        crate::SeatTickNotes {
            controls: me.controls,
            weapon_lock: me.weapon_lock,
            killcam_hud: Option::default(),
            lifecycle: me.lifecycle,
            loadout: me.kit.clone(),
            life_sequence: me.life_sequence,
            item_ready_ms: me.item_ready_ms,
            item_use_entity: me.use_target,
            mag_rounds: me.mag_rounds,
            ammo_stock: me.spare_rounds,
            rounds_per_gun: me.rounds_per_gun.clone(),
            taped_mag_spent: me.taped_mag_spent.clone(),
            weapon_shot_count: me.shots_per_gun,
            burst_latch: me.burst_held,
            burst_latch_secondary: me.burst_latch_secondary,
            rechamber_pending: me.bolt_due,
            rechamber_pending_secondary: me.rechamber_pending_secondary,
            score: me.score,
            kills: me.frags,
            deaths: me.died,
            kill_streak: me.kill_streak,
            radar: me.radar,
            remote_missile: me.remote_missile,
            died_at_step: me.died_at_step,
            look_at_killer_yaw: me.killer_turn,
            name: me.name,
            ffa_team: me.solo_side,
            client_state_team: me.seat_side,
            rank: me.grade,
            prestige: me.star,
            card_icon: me.card_icon,
            card_title: me.card_title,
            card_plate: me.card_plate,
            hud_archival: Vec::default(),
            hud_current: Vec::default(),
            client_dvars: me.client_dvars.clone(),
            shellshock: me.shellshock.clone(),
            view_effects: me.view_effects.clone(),
            menu_commands: me.menu_commands.clone(),
            location_selection: me.location_selection.clone(),
            t6_view: self.t6_predicted_view(),
            t6_perks: me.t6_perks,
        }
    }

    /// Takes the server's word for everything a snapshot carries about this client.
    pub(crate) fn take_over_snapshot_info(&mut self, row: &SeatTickNotes) {
        let SeatTickNotes {
            controls,
            weapon_lock,
            killcam_hud: _,
            lifecycle,
            loadout,
            life_sequence: life_no,
            item_ready_ms: got_item_use_spawn_ms,
            item_use_entity: got_item_use_entity,
            mag_rounds: ammo_clip,
            ammo_stock: spare,
            rounds_per_gun: got_ammo_by_weapon,
            taped_mag_spent,
            weapon_shot_count: got_weapon_shot_count,
            burst_latch: got_burst_latch,
            burst_latch_secondary,
            rechamber_pending: got_rechamber_pending,
            rechamber_pending_secondary,
            score,
            kills,
            deaths,
            kill_streak,
            radar,
            remote_missile,
            died_at_step: got_dead_since_tick,
            look_at_killer_yaw: got_look_at_killer_yaw,
            name,
            ffa_team,
            client_state_team: got_client_state_team,
            rank,
            prestige,
            card_icon: got_player_card_icon,
            card_title: got_player_card_title,
            card_plate: got_player_card_nameplate,
            hud_archival: _,
            hud_current: _,
            client_dvars,
            shellshock,
            view_effects,
            menu_commands,
            location_selection,
            t6_view,
            t6_perks,
        } = row.clone();
        self.controls = controls;
        self.weapon_lock = weapon_lock;
        self.lifecycle = lifecycle;
        self.kit = loadout;
        self.life_sequence = life_no;
        self.item_ready_ms = got_item_use_spawn_ms;
        self.use_target = got_item_use_entity;
        self.mag_rounds = ammo_clip;
        self.spare_rounds = spare;
        self.rounds_per_gun = got_ammo_by_weapon;
        self.taped_mag_spent = taped_mag_spent;
        self.shots_per_gun = got_weapon_shot_count;
        self.burst_held = got_burst_latch;
        self.burst_latch_secondary = burst_latch_secondary;
        self.bolt_due = got_rechamber_pending;
        self.rechamber_pending_secondary = rechamber_pending_secondary;
        self.score = score;
        self.frags = kills;
        self.died = deaths;
        self.kill_streak = kill_streak;
        self.radar = radar;
        self.remote_missile = remote_missile;
        self.died_at_step = got_dead_since_tick;
        self.killer_turn = got_look_at_killer_yaw;
        self.name = name;
        self.solo_side = ffa_team;
        self.seat_side = got_client_state_team;
        self.grade = rank;
        self.star = prestige;
        self.card_icon = got_player_card_icon;
        self.card_title = got_player_card_title;
        self.card_plate = got_player_card_nameplate;
        self.client_dvars = client_dvars;
        self.shellshock = shellshock;
        self.view_effects = view_effects;
        self.menu_commands = menu_commands;
        self.location_selection = location_selection;
        self.t6_view = t6_view;
        self.t6_perks = t6_perks;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_ledger::{PlayerSessionStage, RadarMode};

    fn kind(n: u8) -> MenuCommandKind {
        MenuCommandKind::Show(format!("m{n}"))
    }

    #[test]
    fn menu_commands_are_numbered_and_only_the_newest_tail_is_kept() {
        let mut state = PlayerMatchRecord::default();
        for n in 0..(MENU_COMMAND_TAIL as u8 + 3) {
            state.push_menu_command(kind(n));
        }
        assert_eq!(state.menu_commands.len(), MENU_COMMAND_TAIL);
        let serials: Vec<u32> = state.menu_commands.iter().map(|c| c.serial).collect();
        let first = serials[0];
        assert_eq!(first, 4);
        assert!(serials.windows(2).all(|pair| pair[1] == pair[0] + 1));
    }

    #[test]
    fn a_snapshot_row_round_trips_what_the_server_owns() {
        let mut server = PlayerMatchRecord {
            lifecycle: PlayerSessionStage::Alive,
            score: 450,
            frags: 5,
            died: 2,
            kill_streak: 3,
            radar: RadarMode::default(),
            name: *b"Tester\0\0\0\0\0\0\0\0\0\0",
            grade: 12,
            t6_perks: 0b101,
            mag_rounds: 20,
            spare_rounds: 80,
            ..Default::default()
        };
        server.store_ammo_counts(4, 20, 80);
        server.push_menu_command(kind(1));
        let meta = server.as_wire_meta();

        let mut client = PlayerMatchRecord::default();
        client.take_over_snapshot_info(&meta);
        assert_eq!(client.as_wire_meta(), meta);
        assert_eq!((client.score, client.frags, client.grade, client.t6_perks), (450, 5, 12, 0b101));
        assert_eq!(client.ammo_counts_of(4), (20, 80));
    }
}
