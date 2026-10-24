//! What the console's `bot` command leaves for the host to act on: a count of bots to ask the game's bot script for,
//! a hold switch, and teleports. The console only fills these; [`crate::BotsPlugin`] empties them.

use bevy::prelude::Resource;
use sim::ClientId;

/// `bot add N`: bots asked for but not handed to the game yet.
#[derive(Resource, Debug, Default)]
pub struct BotAddQueue {
    asked: u32,
}

impl BotAddQueue {
    /// Ask for `count` more bots; a count of 0 still asks for one.
    pub fn push(&mut self, count: u32) {
        self.asked = self.asked.saturating_add(count.max(1));
    }

    pub(crate) fn take(&mut self) -> u32 {
        std::mem::take(&mut self.asked)
    }
}

/// `bot hold on|off`: while on, bots keep their view but neither move nor press anything.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct BotHold(pub bool);

/// Which bots a teleport moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotTpTarget {
    /// Every bot the host runs.
    All,
    /// One client, skipped with a warning unless it is a bot.
    Id(ClientId),
}

/// Where a teleport puts its bots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotTpWhere {
    /// A fixed point; a missing yaw or pitch keeps the bot's own.
    Absolute { origin: [f32; 3], yaw: Option<f32>, pitch: Option<f32> },
    /// `height` units straight over the local player, looking down at them.
    Above { height: f32 },
}

/// One `bot tp`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotTpRequest {
    pub target: BotTpTarget,
    pub where_: BotTpWhere,
}

/// `bot tp` requests in the order typed.
#[derive(Resource, Debug, Default)]
pub struct BotTpQueue {
    waiting: Vec<BotTpRequest>,
}

impl BotTpQueue {
    pub fn push(&mut self, request: BotTpRequest) {
        self.waiting.push(request);
    }

    pub(crate) fn take(&mut self) -> Vec<BotTpRequest> {
        std::mem::take(&mut self.waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_add_of_zero_still_asks_for_one_and_asks_add_up() {
        let mut adds = BotAddQueue::default();
        adds.push(0);
        adds.push(3);
        assert_eq!(adds.take(), 4);
        assert_eq!(adds.take(), 0);
    }

    #[test]
    fn teleports_come_out_in_the_order_typed() {
        let mut tps = BotTpQueue::default();
        let first = BotTpRequest { target: BotTpTarget::All, where_: BotTpWhere::Above { height: 64.0 } };
        let second = BotTpRequest {
            target: BotTpTarget::Id(ClientId(3)),
            where_: BotTpWhere::Absolute { origin: [1.0, 2.0, 3.0], yaw: Some(90.0), pitch: None },
        };
        tps.push(first);
        tps.push(second);
        assert_eq!(tps.take(), vec![first, second]);
        assert!(tps.take().is_empty());
    }
}
