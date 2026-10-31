//! Every game button a client's keys can hold.

use crate::HeldKeyLatch;

/// The hold buttons, each named after the command that drives it (`+attack` drives `attack`).
#[derive(Clone, Copy, Debug, Default)]
pub struct HeldKeyLatchBank {
    pub attack: HeldKeyLatch,
    pub melee: HeldKeyLatch,
    pub frag: HeldKeyLatch,
    pub smoke: HeldKeyLatch,
    pub usereload: HeldKeyLatch,
    /// `+speed_throw`: aims while held.
    pub speed: HeldKeyLatch,
    pub gostand: HeldKeyLatch,
    pub forward: HeldKeyLatch,
    pub back: HeldKeyLatch,
    pub moveleft: HeldKeyLatch,
    pub moveright: HeldKeyLatch,
    pub movedown: HeldKeyLatch,
    pub left: HeldKeyLatch,
    pub right: HeldKeyLatch,
    pub lookup: HeldKeyLatch,
    pub lookdown: HeldKeyLatch,
    pub strafe: HeldKeyLatch,
    pub holdbreath: HeldKeyLatch,
    pub activate: HeldKeyLatch,
    pub reload: HeldKeyLatch,
    pub prone: HeldKeyLatch,
    pub mlook: HeldKeyLatch,
    pub throw_btn: HeldKeyLatch,
    pub sprint: HeldKeyLatch,
    pub scores: HeldKeyLatch,
    pub talk: HeldKeyLatch,
    pub stance: HeldKeyLatch,
    /// Pushed down by the fire key's jam roll; comes up with the fire key.
    pub jam: HeldKeyLatch,
}

impl HeldKeyLatchBank {
    /// Calls `visit` with the hold command of each button that is down or was pressed since the
    /// last command, for the movement debug overlay. Buttons that neither move nor act
    /// (strafe, mlook, the menus, the stance press, the throw and jam halves) are not shown.
    pub fn each_held_command(&self, mut visit: impl FnMut(&'static str)) {
        let shown = [
            ("+attack", &self.attack),
            ("+melee", &self.melee),
            ("+frag", &self.frag),
            ("+smoke", &self.smoke),
            ("+usereload", &self.usereload),
            ("+speed_throw", &self.speed),
            ("+gostand", &self.gostand),
            ("+forward", &self.forward),
            ("+back", &self.back),
            ("+moveleft", &self.moveleft),
            ("+moveright", &self.moveright),
            ("+movedown", &self.movedown),
            ("+left", &self.left),
            ("+right", &self.right),
            ("+lookup", &self.lookup),
            ("+lookdown", &self.lookdown),
            ("+activate", &self.activate),
            ("+reload", &self.reload),
            ("+prone", &self.prone),
            ("+sprint", &self.sprint),
        ];
        for (name, held) in shown {
            if held.down_or_tapped() {
                visit(name);
            }
        }
    }

    /// Forgets the presses the last command sent; buttons still held stay active.
    pub fn forget_tap(&mut self) {
        let every = [
            &mut self.attack,
            &mut self.melee,
            &mut self.frag,
            &mut self.smoke,
            &mut self.usereload,
            &mut self.speed,
            &mut self.gostand,
            &mut self.forward,
            &mut self.back,
            &mut self.moveleft,
            &mut self.moveright,
            &mut self.movedown,
            &mut self.left,
            &mut self.right,
            &mut self.lookup,
            &mut self.lookdown,
            &mut self.strafe,
            &mut self.holdbreath,
            &mut self.activate,
            &mut self.reload,
            &mut self.prone,
            &mut self.mlook,
            &mut self.throw_btn,
            &mut self.sprint,
            &mut self.scores,
            &mut self.talk,
            &mut self.stance,
            &mut self.jam,
        ];
        for held in every {
            held.tapped_since_clear = false;
        }
    }
}
