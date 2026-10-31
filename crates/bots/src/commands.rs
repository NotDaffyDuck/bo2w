//! What the console's `bot` command leaves for the host to act on: a count of bots to ask the game's bot script for,
//! a hold switch, and teleports. The console only fills these; [`crate::AutomatedPlayersPlugin`] empties them.

use bevy::prelude::Resource;
use sim::ClientId;

/// `bot add N`: bots asked for but not handed to the game yet.
#[derive(Resource, Debug, Default)]
pub struct BotJoinQueue {
    asked: u32,
}

impl BotJoinQueue {
    /// Ask for `count` more bots; a count of 0 still asks for one.
    pub fn ask_for(&mut self, extra: u32) {
        self.asked = self.asked.saturating_add(extra.max(1));
    }

    pub(crate) fn take(&mut self) -> u32 {
        std::mem::take(&mut self.asked)
    }
}

/// `bot hold on|off`: while on, bots keep their view but neither move nor press anything.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct BotFreezeFlag(pub bool);

/// Which bots a teleport moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotWarpSubject {
    /// Every bot the host runs.
    All,
    /// One client, skipped with a warning unless it is a bot.
    Id(ClientId),
}

/// Where a teleport puts its bots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotWarpDestination {
    /// A fixed point; a missing yaw or pitch keeps the bot's own.
    Absolute { at_point: [f32; 3], turn_yaw: Option<f32>, pitch: Option<f32> },
    /// `height` units straight over the local player, looking down at them.
    Above { height: f32 },
}

/// One `bot tp`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotWarpOrder {
    pub target: BotWarpSubject,
    pub where_: BotWarpDestination,
}

/// `bot tp` requests in the order typed.
#[derive(Resource, Debug, Default)]
pub struct BotWarpQueue {
    waiting: Vec<BotWarpOrder>,
}

impl BotWarpQueue {
    pub fn enqueue(&mut self, tp: BotWarpOrder) {
        self.waiting.push(tp);
    }

    pub(crate) fn take(&mut self) -> Vec<BotWarpOrder> {
        std::mem::take(&mut self.waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::ClientId;

    #[test]
    fn an_add_of_zero_still_asks_for_one_and_asks_add_up() {
        let mut adds = BotJoinQueue::default();
        adds.ask_for(0);
        adds.ask_for(3);
        assert_eq!(adds.take(), 4);
        assert_eq!(adds.take(), 0);
    }

    #[test]
    fn teleports_come_out_in_the_order_typed() {
        let mut tps = BotWarpQueue::default();
        let first = BotWarpOrder { target: BotWarpSubject::All, where_: BotWarpDestination::Above { height: 64.0 } };
        let second = BotWarpOrder {
            target: BotWarpSubject::Id(ClientId(3)),
            where_: BotWarpDestination::Absolute { at_point: [1.0, 2.0, 3.0], turn_yaw: Some(90.0), pitch: None },
        };
        tps.enqueue(first);
        tps.enqueue(second);
        assert_eq!(tps.take(), vec![first, second]);
        assert!(tps.take().is_empty());
    }
}
