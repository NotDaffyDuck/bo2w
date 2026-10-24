//! Moving a client's state into the snapshot meta and back, plus the T6 view reads that go with it.

use super::{ClientMatchState, ClientSnapshotMeta, MENU_COMMAND_TAIL, MenuCommand, MenuCommandKind, T6ViewInputs};
use crate::t6_gun::T6Gun;

impl ClientMatchState {
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
        self.menu_commands.push(MenuCommand { serial, kind });
        if let Some(excess) = self.menu_commands.len().checked_sub(MENU_COMMAND_TAIL) {
            self.menu_commands.drain(..excess);
        }
    }

    /// What goes out in a snapshot about this client. The HUD rows and the killcam are filled in by the caller.
    pub(crate) fn to_snapshot_meta(&self) -> ClientSnapshotMeta {
        ClientSnapshotMeta {
            controls: self.controls,
            weapon_lock: self.weapon_lock,
            killcam_hud: None,
            lifecycle: self.lifecycle,
            loadout: self.loadout.clone(),
            life_sequence: self.life_sequence,
            item_use_spawn_ms: self.item_use_spawn_ms,
            item_use_entity: self.item_use_entity,
            ammo_clip: self.ammo_clip,
            ammo_stock: self.ammo_stock,
            ammo_by_weapon: self.ammo_by_weapon.clone(),
            taped_mag_spent: self.taped_mag_spent.clone(),
            weapon_shot_count: self.weapon_shot_count,
            burst_latch: self.burst_latch,
            burst_latch_secondary: self.burst_latch_secondary,
            rechamber_pending: self.rechamber_pending,
            rechamber_pending_secondary: self.rechamber_pending_secondary,
            score: self.score,
            kills: self.kills,
            deaths: self.deaths,
            kill_streak: self.kill_streak,
            radar: self.radar,
            remote_missile: self.remote_missile,
            dead_since_tick: self.dead_since_tick,
            look_at_killer_yaw: self.look_at_killer_yaw,
            name: self.name,
            ffa_team: self.ffa_team,
            client_state_team: self.client_state_team,
            rank: self.rank,
            prestige: self.prestige,
            player_card_icon: self.player_card_icon,
            player_card_title: self.player_card_title,
            player_card_nameplate: self.player_card_nameplate,
            hud_archival: Vec::new(),
            hud_current: Vec::new(),
            client_dvars: self.client_dvars.clone(),
            shellshock: self.shellshock.clone(),
            view_effects: self.view_effects.clone(),
            menu_commands: self.menu_commands.clone(),
            location_selection: self.location_selection.clone(),
            t6_view: self.t6_predicted_view(),
            t6_perks: self.t6_perks,
        }
    }

    /// Takes the server's word for everything a snapshot carries about this client.
    pub(crate) fn adopt_snapshot_meta(&mut self, meta: &ClientSnapshotMeta) {
        let ClientSnapshotMeta {
            controls,
            weapon_lock,
            killcam_hud: _,
            lifecycle,
            loadout,
            life_sequence,
            item_use_spawn_ms,
            item_use_entity,
            ammo_clip,
            ammo_stock,
            ammo_by_weapon,
            taped_mag_spent,
            weapon_shot_count,
            burst_latch,
            burst_latch_secondary,
            rechamber_pending,
            rechamber_pending_secondary,
            score,
            kills,
            deaths,
            kill_streak,
            radar,
            remote_missile,
            dead_since_tick,
            look_at_killer_yaw,
            name,
            ffa_team,
            client_state_team,
            rank,
            prestige,
            player_card_icon,
            player_card_title,
            player_card_nameplate,
            hud_archival: _,
            hud_current: _,
            client_dvars,
            shellshock,
            view_effects,
            menu_commands,
            location_selection,
            t6_view,
            t6_perks,
        } = meta.clone();
        self.controls = controls;
        self.weapon_lock = weapon_lock;
        self.lifecycle = lifecycle;
        self.loadout = loadout;
        self.life_sequence = life_sequence;
        self.item_use_spawn_ms = item_use_spawn_ms;
        self.item_use_entity = item_use_entity;
        self.ammo_clip = ammo_clip;
        self.ammo_stock = ammo_stock;
        self.ammo_by_weapon = ammo_by_weapon;
        self.taped_mag_spent = taped_mag_spent;
        self.weapon_shot_count = weapon_shot_count;
        self.burst_latch = burst_latch;
        self.burst_latch_secondary = burst_latch_secondary;
        self.rechamber_pending = rechamber_pending;
        self.rechamber_pending_secondary = rechamber_pending_secondary;
        self.score = score;
        self.kills = kills;
        self.deaths = deaths;
        self.kill_streak = kill_streak;
        self.radar = radar;
        self.remote_missile = remote_missile;
        self.dead_since_tick = dead_since_tick;
        self.look_at_killer_yaw = look_at_killer_yaw;
        self.name = name;
        self.ffa_team = ffa_team;
        self.client_state_team = client_state_team;
        self.rank = rank;
        self.prestige = prestige;
        self.player_card_icon = player_card_icon;
        self.player_card_title = player_card_title;
        self.player_card_nameplate = player_card_nameplate;
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
    use crate::match_state::{ClientLifecycle, RadarMode};

    fn kind(n: u8) -> MenuCommandKind {
        MenuCommandKind::Open(format!("m{n}"))
    }

    #[test]
    fn menu_commands_are_numbered_and_only_the_newest_tail_is_kept() {
        let mut state = ClientMatchState::default();
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
        let mut server = ClientMatchState {
            lifecycle: ClientLifecycle::Alive,
            score: 450,
            kills: 5,
            deaths: 2,
            kill_streak: 3,
            radar: RadarMode::default(),
            name: *b"Tester\0\0\0\0\0\0\0\0\0\0",
            rank: 12,
            t6_perks: 0b101,
            ammo_clip: 20,
            ammo_stock: 80,
            ..Default::default()
        };
        server.set_ammo(4, 20, 80);
        server.push_menu_command(kind(1));
        let meta = server.to_snapshot_meta();

        let mut client = ClientMatchState::default();
        client.adopt_snapshot_meta(&meta);
        assert_eq!(client.to_snapshot_meta(), meta);
        assert_eq!((client.score, client.kills, client.rank, client.t6_perks), (450, 5, 12, 0b101));
        assert_eq!(client.ammo_for(4), (20, 80));
    }
}
