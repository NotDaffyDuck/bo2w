//! Every game button a client's keys can hold.

use crate::Kbutton;

/// The hold buttons, each named after the command that drives it (`+attack` drives `attack`).
#[derive(Clone, Copy, Debug, Default)]
pub struct KbuttonSet {
    pub attack: Kbutton,
    pub melee: Kbutton,
    pub frag: Kbutton,
    pub smoke: Kbutton,
    pub usereload: Kbutton,
    /// `+speed_throw`: aims while held.
    pub speed: Kbutton,
    pub gostand: Kbutton,
    pub forward: Kbutton,
    pub back: Kbutton,
    pub moveleft: Kbutton,
    pub moveright: Kbutton,
    pub movedown: Kbutton,
    pub left: Kbutton,
    pub right: Kbutton,
    pub lookup: Kbutton,
    pub lookdown: Kbutton,
    pub strafe: Kbutton,
    pub holdbreath: Kbutton,
    pub activate: Kbutton,
    pub reload: Kbutton,
    pub prone: Kbutton,
    pub mlook: Kbutton,
    pub throw_btn: Kbutton,
    pub sprint: Kbutton,
    pub scores: Kbutton,
    pub talk: Kbutton,
    pub stance: Kbutton,
    /// Pushed down by the fire key's jam roll; comes up with the fire key.
    pub jam: Kbutton,
}

impl KbuttonSet {
    /// Calls `f` with the hold command of each button that is down or was pressed since the
    /// last command, for the movement debug overlay. Buttons that neither move nor act
    /// (strafe, mlook, the menus, the stance press, the throw and jam halves) are not shown.
    pub fn visit_active_names(&self, mut f: impl FnMut(&'static str)) {
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
                f(name);
            }
        }
    }

    /// Forgets the presses the last command sent; buttons still held stay active.
    pub fn clear_was_pressed(&mut self) {
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
            held.was_pressed = false;
        }
    }
}
