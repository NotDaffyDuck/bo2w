//! `WholeSim` operations on the script side: GSC programs, the Black Ops II install, bots and
//! the seats scripts use to drive the camera.

use crate::gsc_host::{Fault, LevelData, NativeRegistry, Program, Runtime, Value};
use crate::sim_parts::SeatNo;

impl super::WholeSim {
    /// Loads a compiled GSC program, the natives it may call, and the level's own data.
    pub fn install_gsc_program(
        &mut self,
        program: Program,
        natives: NativeRegistry,
        level: LevelData,
    ) -> Result<(), Fault> {
        crate::gsc_host::seat_program(&mut self.bevy, program, natives, level)
    }

    /// Starts the function `name` on `receiver` and returns the thread's id.
    pub fn start_gsc(
        &mut self,
        name: &str,
        receiver: Value,
        arguments: Vec<Value>,
    ) -> Result<u64, Fault> {
        crate::gsc_host::start(&mut self.bevy, name, receiver, arguments)
    }

    pub fn set_gsc_dvar(&mut self, name: &str, setting: &str) {
        crate::gsc_host::set_dvar(&mut self.bevy, name, setting);
    }

    /// Hash of the installed program, or `None` while no program is loaded.
    pub fn gsc_program_fingerprint(&self) -> Option<[u8; 32]> {
        self.bevy.resource::<Runtime>().program_fingerprint()
    }

    /// Installs a Black Ops II map's own scripts; they start on the first authority frame.
    pub fn install_t6(&mut self, install: crate::T6Install) -> Result<(), String> {
        crate::zombie_rt::install(&mut self.bevy, install)
    }

    /// Bot clients the Black Ops II scripts asked for (`addtestclient`) since the last call, by
    /// client number. The host's bot roster joins each one as a client its commands come from.
    pub fn take_t6_test_clients(&mut self) -> Vec<u32> {
        crate::zombie_rt::take_test_clients(&mut self.bevy)
    }

    /// Bots the scripts sent away (`botleavegame`) since the last call.
    pub fn take_t6_bot_leaves(&mut self) -> Vec<u32> {
        crate::zombie_rt::take_bot_leaves(&mut self.bevy)
    }

    /// The console's `bot add`: `count` more bots from Black Ops II's own bot script, spawned
    /// on the next frame.
    pub fn request_t6_bots(&mut self, rounds: u32) {
        crate::zombie_rt::console_bots::ask(&mut self.bevy, rounds);
    }

    /// The console's `bot hold`: while on, the bots keep their view and neither move nor press
    /// anything.
    pub fn hold_t6_bots(&mut self, held: bool) {
        crate::zombie_rt::console_bots::hold(&mut self.bevy, held);
    }

    /// Every seat the Black Ops II killcam has put a client in.
    pub fn script_seats(&self) -> Vec<(SeatNo, crate::ScriptSeat)> {
        crate::zombie_rt::killcam_seats(&self.bevy)
    }

    /// The engine's killcam stepped this seat's `archivetime` (ms) nearer the present. It stays
    /// stepped, like the engine's own `sess.archiveTime`.
    pub fn store_script_seat_archive(&mut self, client: SeatNo, archive_ms: i32) {
        crate::zombie_rt::store_killcam_archive(&mut self.bevy, client, archive_ms);
    }

    /// The script fault, once: later calls return `None` until a new fault happens.
    pub fn take_script_fault(&mut self) -> Option<Fault> {
        let mut runtime = self.bevy.resource_mut::<Runtime>();
        if runtime.fault_reported {
            return None;
        }
        runtime.fault_reported = runtime.fault.is_some();
        runtime.fault.clone()
    }

    /// True once after a script asked to end the level.
    pub fn take_script_exit_level(&mut self) -> bool {
        core::mem::take(&mut self.bevy.resource_mut::<Runtime>().exit_level)
    }
}
