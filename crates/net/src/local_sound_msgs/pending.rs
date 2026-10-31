//! The server's queue of per-client sounds, from script request to frame delivery.

use bevy::prelude::Resource;
use sim::ClientId;

use super::HostSoundCue;

/// A sound a script asked for by name; it has no wire index until it is looked up.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NamedSound {
    listener: ClientId,
    stop: bool,
    sound_name: String,
}

#[derive(Resource, Debug, Default)]
pub struct StagedHostSounds {
    named: Vec<NamedSound>,
    ready: Vec<(ClientId, HostSoundCue)>,
    /// Sounds dropped because the alias table has no entry (index 0) for them.
    pub discarded_zero_slot: u64,
    /// Indexed sounds that were never taken by a client and were drained away.
    pub unclaimed: u64,
    pub newest_enqueued_sound: Option<String>,
    pub final_slot_position: u8,
    pub latest_discarded_sound: Option<String>,
}

impl StagedHostSounds {
    /// Queue a sound by alias name, to be given an index by [`Self::claim_critical_section`].
    pub fn enqueue_sound_name(&mut self, listener: ClientId, stop: bool, sound_name: impl Into<String>) {
        let sound_name = sound_name.into();
        self.newest_enqueued_sound = Some(sound_name.clone());
        self.named.push(NamedSound { listener, stop, sound_name });
    }

    pub fn enqueue_sound_name_u32(&mut self, slot_number: u32, stop: bool, sound_name: impl Into<String>) {
        self.enqueue_sound_name(ClientId(slot_number), stop, sound_name);
    }

    /// Look up the queued names in the world's sound-alias table and move them to the indexed
    /// queue. A name with no table entry is dropped.
    pub fn claim_critical_section(&mut self, game: &mut sim::SimWorld) {
        for sound in std::mem::take(&mut self.named) {
            let index = game.sound_alias_index(&sound.sound_name);
            if index == 0 {
                self.discarded_zero_slot = self.discarded_zero_slot.saturating_add(1);
                self.latest_discarded_sound = Some(sound.sound_name);
                continue;
            }
            self.final_slot_position = index;
            self.ready.push((sound.listener, command(sound.stop, index)));
        }
    }

    /// Take the sounds the world queued itself, which already carry an index.
    pub fn absorb_sim_queue(&mut self, game: &mut sim::SimWorld) {
        for sound in game.take_pending_local_sounds() {
            if sound.alias_index == 0 {
                self.discarded_zero_slot = self.discarded_zero_slot.saturating_add(1);
                continue;
            }
            self.ready
                .push((sound.recipient, command(sound.halt, sound.alias_index)));
        }
    }

    /// Remove and return the indexed sounds for `listener`, in the order they were queued.
    pub fn hand_to(&mut self, listener: ClientId) -> Vec<HostSoundCue> {
        let (mine, rest) = std::mem::take(&mut self.ready)
            .into_iter()
            .partition::<Vec<_>, _>(|(owner, _)| *owner == listener);
        self.ready = rest;
        mine.into_iter().map(|(_, sound)| sound).collect()
    }

    /// Remove and return every indexed sound nobody took, counting them as stranded.
    pub fn flush_orphaned(&mut self) -> Vec<(ClientId, HostSoundCue)> {
        let unclaimed_rows = std::mem::take(&mut self.ready);
        self.unclaimed = self.unclaimed.saturating_add(unclaimed_rows.len() as u64);
        unclaimed_rows
    }

    pub fn is_empty(&self) -> bool {
        self.named.is_empty() && self.ready.is_empty()
    }
}

fn command(stop: bool, index: u8) -> HostSoundCue {
    if stop { HostSoundCue::silence(index) } else { HostSoundCue::start(index) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_indexed(entries: &[(u32, HostSoundCue)]) -> StagedHostSounds {
        let mut pending = StagedHostSounds::default();
        pending.ready = entries.iter().map(|&(id, sound)| (ClientId(id), sound)).collect();
        pending
    }

    #[test]
    fn a_new_queue_is_empty() {
        assert!(StagedHostSounds::default().is_empty());
    }

    #[test]
    fn pushing_an_alias_remembers_it_and_makes_the_queue_busy() {
        let mut pending = StagedHostSounds::default();
        pending.enqueue_sound_name_u32(2, false, "mpl_flag_pickup");
        assert!(!pending.is_empty());
        assert_eq!(pending.newest_enqueued_sound.as_deref(), Some("mpl_flag_pickup"));
        assert_eq!(pending.named[0].listener, ClientId(2));
        assert!(!pending.named[0].stop);
    }

    #[test]
    fn a_named_sound_alone_is_not_yet_takeable() {
        let mut pending = StagedHostSounds::default();
        pending.enqueue_sound_name(ClientId(1), false, "x");
        assert!(pending.hand_to(ClientId(1)).is_empty());
    }

    #[test]
    fn taking_for_a_client_leaves_the_others_in_order() {
        let mut pending = with_indexed(&[
            (1, HostSoundCue::start(10)),
            (2, HostSoundCue::start(20)),
            (1, HostSoundCue::silence(11)),
            (3, HostSoundCue::start(30)),
        ]);
        assert_eq!(pending.hand_to(ClientId(1)), [HostSoundCue::start(10), HostSoundCue::silence(11)]);
        assert_eq!(
            pending.ready,
            [(ClientId(2), HostSoundCue::start(20)), (ClientId(3), HostSoundCue::start(30))]
        );
        assert!(pending.hand_to(ClientId(1)).is_empty());
    }

    #[test]
    fn draining_the_stranded_empties_the_queue_and_counts_them() {
        let mut pending = with_indexed(&[(1, HostSoundCue::start(1)), (2, HostSoundCue::start(2))]);
        let left = pending.flush_orphaned();
        assert_eq!(left.len(), 2);
        assert_eq!(pending.unclaimed, 2);
        assert!(pending.is_empty());
        pending.ready.push((ClientId(1), HostSoundCue::start(3)));
        pending.flush_orphaned();
        assert_eq!(pending.unclaimed, 3, "the count keeps adding");
    }

    #[test]
    fn a_command_is_a_play_or_a_stop_of_its_index() {
        assert_eq!(command(false, 4), HostSoundCue::start(4));
        assert_eq!(command(true, 4), HostSoundCue::silence(4));
    }
}
