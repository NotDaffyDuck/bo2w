//! `SimWorld` operations on the script side: GSC programs, the Black Ops II install, bots and
//! the seats scripts use to drive the camera.

use super::SimWorld;
use crate::script::{Fault, LevelData, NativeRegistry, Program, Runtime, Value};
use crate::world::ClientId;

impl SimWorld {
    /// Loads a compiled GSC program, the natives it may call, and the level's own data.
    pub fn install_gsc_program(
        &mut self,
        program: Program,
        natives: NativeRegistry,
        level: LevelData,
    ) -> Result<(), Fault> {
        crate::script::install(&mut self.ecs, program, natives, level)
    }

    /// Starts the function `name` on `receiver` and returns the thread's id.
    pub fn start_gsc(
        &mut self,
        name: &str,
        receiver: Value,
        arguments: Vec<Value>,
    ) -> Result<u64, Fault> {
        crate::script::start(&mut self.ecs, name, receiver, arguments)
    }

    pub fn set_gsc_dvar(&mut self, name: &str, value: &str) {
        crate::script::set_dvar(&mut self.ecs, name, value);
    }

    /// Hash of the installed program, or `None` while no program is loaded.
    pub fn gsc_program_fingerprint(&self) -> Option<[u8; 32]> {
        self.ecs.resource::<Runtime>().program_fingerprint()
    }

    /// Installs a Black Ops II map's own scripts; they start on the first authority frame.
    pub fn install_t6(&mut self, install: crate::T6Install) -> Result<(), String> {
        crate::t6::install(&mut self.ecs, install)
    }

    /// Bot clients the Black Ops II scripts asked for (`addtestclient`) since the last call, by
    /// client number. The host's bot roster joins each one as a client its commands come from.
    pub fn take_t6_test_clients(&mut self) -> Vec<u32> {
        crate::t6::take_test_clients(&mut self.ecs)
    }

    /// Bots the scripts sent away (`botleavegame`) since the last call.
    pub fn take_t6_bot_leaves(&mut self) -> Vec<u32> {
        crate::t6::take_bot_leaves(&mut self.ecs)
    }

    /// The console's `bot add`: `count` more bots from Black Ops II's own bot script, spawned
    /// on the next frame.
    pub fn request_t6_bots(&mut self, count: u32) {
        crate::t6::console_bots::ask(&mut self.ecs, count);
    }

    /// The console's `bot hold`: while on, the bots keep their view and neither move nor press
    /// anything.
    pub fn hold_t6_bots(&mut self, held: bool) {
        crate::t6::console_bots::hold(&mut self.ecs, held);
    }

    /// Every seat a script (or the Black Ops II killcam) has put a client in.
    pub fn script_seats(&self) -> Vec<(ClientId, crate::ScriptSeat)> {
        let mut seats = crate::script::script_seats(&self.ecs);
        seats.extend(crate::t6::killcam_seats(&self.ecs));
        seats
    }

    /// The engine's killcam stepped this seat's `archivetime` (ms) nearer the present. It stays
    /// stepped, like the engine's own `sess.archiveTime`.
    pub fn store_script_seat_archive(&mut self, client: ClientId, archive_ms: i32) {
        crate::script::store_seat_archive(&mut self.ecs, client, archive_ms);
        crate::t6::store_killcam_archive(&mut self.ecs, client, archive_ms);
    }

    /// The script fault, once: later calls return `None` until a new fault happens.
    pub fn take_script_fault(&mut self) -> Option<Fault> {
        let mut runtime = self.ecs.resource_mut::<Runtime>();
        if runtime.fault_reported {
            return None;
        }
        runtime.fault_reported = runtime.fault.is_some();
        runtime.fault.clone()
    }

    /// True once after a script asked to end the level.
    pub fn take_script_exit_level(&mut self) -> bool {
        core::mem::take(&mut self.ecs.resource_mut::<Runtime>().exit_level)
    }
}
