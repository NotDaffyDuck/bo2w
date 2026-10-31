//! First-person cues read from a presented snapshot: what happened to the viewer's own gun.

use playerstate_t6::game::AvatarState;
use sim::{ClientId, PlayerSessionStage, EntityEventRecord, EventRecord, MatchOccurrence};

use super::ShownWorldFrame;

/// Things the viewer's own events say happened this snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FirstPersonEventHints {
    pub fire_confirmed: bool,
    pub trigger_released: bool,
    pub spawned: bool,
}

/// One hand's weapon timers and animation, as the player state holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewHandEntry {
    pub anim: i32,
    pub fire_timer: i32,
    pub ready_timer: i32,
    pub kick_timer: i32,
    pub hand_state: i32,
    pub hand_flags: i32,
    pub shots: i32,
}

/// Both hands plus the values the gun's animation is blended from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewHandRawReading {
    pub hands: [ViewHandEntry; 2],
    pub weapon_raise_fraction: f32,
    pub scope_in_wait: i32,
    pub previous_gun_hand: i32,
    pub held_weapon: u32,
    pub secondary_hand_slot: i32,
    pub weapon_state_bits: u32,
}

impl ViewHandRawReading {
    fn of(ps: &AvatarState) -> Self {
        // One hand from its animation, state, three timers (fire, ready, kick), flags and shots.
        let hand = |anim: i32, state: i32, timers: [i32; 3], flags: i32, shots: i32| {
            let mut entry = ViewHandEntry { anim, hand_state: state, hand_flags: flags, ..Default::default() };
            [entry.fire_timer, entry.ready_timer, entry.kick_timer] = timers;
            entry.shots = shots;
            entry
        };
        let main_timers = [ps.weapon_timer, ps.weapon_ready_wait, ps.gun_lockout_kick_stamp];
        let main_hand = hand(ps.weap_anim_t6, ps.weapon_state_t6, main_timers, ps.weap_hand_flags, ps.shots_fired);
        // The off hand keeps its own copy of every timer under a `_secondary` / `_left` name.
        let off_timers = [ps.weapon_time_secondary, ps.weapon_delay_secondary, ps.weapon_restrict_kick_time_secondary];
        let off_hand = hand(
            ps.weap_anim_left_t6,
            ps.weapon_state_left_t6,
            off_timers,
            ps.weap_hand_flags_secondary,
            ps.weapon_shot_count_secondary,
        );
        let mut sample = Self {
            hands: [main_hand, off_hand],
            ..Self::default()
        };
        sample.held_weapon = ps.weapon;
        sample.weapon_state_bits = ps.weapon_state_bits;
        sample.secondary_hand_slot = ps.secondary_hand_slot;
        sample.previous_gun_hand = ps.previous_gun_hand;
        sample.scope_in_wait = ps.scope_in_wait;
        sample.weapon_raise_fraction = ps.weapon_raise_fraction;
        sample
    }
}

impl ShownWorldFrame {
    pub fn hand_reading(&self, peer: ClientId) -> Option<ViewHandRawReading> {
        self.player(peer).map(ViewHandRawReading::of)
    }

    pub fn first_person_hints(&self, peer: ClientId) -> FirstPersonEventHints {
        let Some(frame) = self.snapshot() else {
            return FirstPersonEventHints::default();
        };
        cues_from_events(peer, &frame.meta.journal, &frame.meta.entity_events)
    }

    pub fn shellshock(&self, id: ClientId) -> Option<&hud_t6::flashbang::ShellshockParms> {
        let per_client = self.snapshot()?.meta.for_client(id)?;
        per_client.shellshock.as_ref()
    }

    /// The player's state, but only while the player is alive.
    pub fn living_participant(&self, id: ClientId) -> Option<&AvatarState> {
        let per_client = self.snapshot()?.meta.for_client(id)?;
        (per_client.lifecycle == PlayerSessionStage::Alive)
            .then(|| self.player(id))
            .flatten()
    }

    pub fn held_weapon(&self, peer: ClientId) -> Option<u32> {
        self.living_participant(peer).map(|ps| ps.weapon)
    }

    /// The state whose gun is drawn in first person; `None` while the gun is disabled or the
    /// view rides a scorestreak vehicle (the gun is still aimed there, just not drawn).
    pub fn first_person_weapon_holder(&self, id: ClientId) -> Option<&AvatarState> {
        if let Some(meta) = self.snapshot().map(|frame| &frame.meta) {
            let disabled = meta.for_client(id).is_some_and(|mine| mine.controls.weapons_disabled);
            let dvars = meta.script_dvars(id);
            let on_vehicle = dvars.string("bo2mp_vehicle_view").is_some_and(|view| !view.is_empty());
            if disabled || on_vehicle {
                return None;
            }
        }
        self.player(id).filter(|state| playerstate_t6::game::viewweapon_frame_runs(state.movement_mode))
    }
}

fn cues_from_events(who: ClientId, log: &[EventRecord], entity_events: &[EntityEventRecord]) -> FirstPersonEventHints {
    let mine = |record: &&EventRecord| record.audience.is_visible_to(who);
    let seen = |wanted: fn(&MatchOccurrence) -> bool| {
        log
            .iter()
            .filter(mine)
            .any(|record| wanted(&record.event))
    };
    FirstPersonEventHints {
        fire_confirmed: counts_shots(entity_events, who) > 0,
        trigger_released: seen(|event| matches!(event, MatchOccurrence::TriggerLet)),
        spawned: seen(|event| matches!(event, MatchOccurrence::Spawned { .. })),
    }
}

/// How many fire records count as the subject's shots. The three plain fire kinds count from
/// anyone, for any audience; only the last-shot-left kind is held to the subject's number and
/// audience. The wire clients rely on this, so it stays as it is.
fn counts_shots(entity_events: &[EntityEventRecord], subject: ClientId) -> usize {
    use entity_t6::EventCode;

    let plain = |kind: EventCode| {
        kind == EventCode::FIRE_WEAPON
            || kind == EventCode::GUN_LAST_ROUND
            || kind == EventCode::GUN_LEFT_HAND
    };
    let own_last_left = |record: &EntityEventRecord| {
        record.event == EventCode::GUN_LAST_ROUND_LEFT
            && subject.0 as i32 == record.payload.number
            && record.audience.is_visible_to(subject)
    };
    entity_events.iter().filter(|record| plain(record.event) || own_last_left(record)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use entity_t6::EventCode;
    use sim::{EntityEventPayload, EventRecipients, EventSequence, Tick};

    const ME: ClientId = ClientId(2);

    fn event_of(kind: EventCode, number: i32, audience: EventRecipients) -> EntityEventRecord {
        EntityEventRecord {
            sequence: EventSequence(1),
            tick: Tick(1),
            audience,
            event: kind,
            payload: EntityEventPayload { number, ..Default::default() },
        }
    }

    #[test]
    fn plain_fire_kinds_count_whoever_fired() {
        let others = [event_of(
            EventCode::FIRE_WEAPON,
            9,
            EventRecipients::Client(ClientId(9)),
        )];
        assert_eq!(counts_shots(&others, ME), 1);
    }

    #[test]
    fn the_last_shot_left_kind_must_be_the_subjects_own() {
        let mine = event_of(EventCode::GUN_LAST_ROUND_LEFT, 2, EventRecipients::All);
        let theirs = event_of(EventCode::GUN_LAST_ROUND_LEFT, 5, EventRecipients::All);
        let hidden = event_of(
            EventCode::GUN_LAST_ROUND_LEFT,
            2,
            EventRecipients::Client(ClientId(5)),
        );
        assert_eq!(counts_shots(&[mine, theirs, hidden], ME), 1);
    }

    #[test]
    fn other_kinds_are_not_shots() {
        assert_eq!(
            counts_shots(&[event_of(EventCode::NOAMMO, 2, EventRecipients::All)], ME),
            0
        );
        assert!(!cues_from_events(ME, &[], &[]).fire_confirmed);
    }

    #[test]
    fn no_snapshot_gives_no_cues_and_no_hand_sample() {
        let presented = ShownWorldFrame::default();
        assert_eq!(presented.first_person_hints(ME), FirstPersonEventHints::default());
        assert!(presented.hand_reading(ME).is_none());
        assert!(presented.living_participant(ME).is_none());
    }
}
