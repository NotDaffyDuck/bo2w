use super::PlayerIntent;
use crate::sim_parts::SeatNo;
use playerstate_t6::game::InputCommand;

/// All the input of one tick, each item tagged with the client it came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepInputs {
    pub cmds: Vec<(SeatNo, InputCommand)>,
    pub actions: Vec<(SeatNo, PlayerIntent)>,
}

impl StepInputs {
    /// Movement commands and no actions.
    pub fn from_cmds(commands: Vec<(SeatNo, InputCommand)>) -> Self {
        Self { cmds: commands, ..Self::default() }
    }

    /// Puts the input in the same order on every machine: commands by client; actions by client, then request id.
    /// Equal keys keep the order they arrived in.
    pub fn put_in_order(&mut self) {
        self.cmds.sort_by_key(|&(who, _)| who.0);
        self.actions.sort_by_key(|(who, action)| (who.0, action.ticket()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(authority_clock_ms: i32) -> InputCommand {
        InputCommand { authority_clock_ms, ..InputCommand::default() }
    }

    #[test]
    fn commands_sort_by_client() {
        let mut input = StepInputs::from_cmds(vec![(SeatNo(3), cmd(30)), (SeatNo(1), cmd(10)), (SeatNo(2), cmd(20))]);
        input.put_in_order();
        let order: Vec<u32> = input.cmds.iter().map(|(client, _)| client.0).collect();
        assert_eq!(order, [1, 2, 3]);
        assert!(input.actions.is_empty());
    }

    #[test]
    fn actions_sort_by_client_then_request_and_ties_keep_arrival_order() {
        let give = |weapon| PlayerIntent::HandOutGun { request_id: 4, gun: weapon };
        let mut input = StepInputs {
            actions: vec![
                (SeatNo(2), PlayerIntent::ForceDeath { request_id: 1 }),
                (SeatNo(1), PlayerIntent::SpawnClient { request_id: 9 }),
                (SeatNo(1), give(11)),
                (SeatNo(1), give(12)),
            ],
            ..StepInputs::default()
        };
        input.put_in_order();
        let keys: Vec<(u32, u32)> = input.actions.iter().map(|(c, a)| (c.0, a.ticket())).collect();
        assert_eq!(keys, [(1, 4), (1, 4), (1, 9), (2, 1)]);
        assert_eq!([input.actions[0].1, input.actions[1].1], [give(11), give(12)]);
    }
}
