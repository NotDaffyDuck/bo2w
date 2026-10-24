//! Where players start, as the map's entity text places them.

/// One authored player spawn.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnPoint {
    /// What kind of spawn it is, e.g. `mp_tdm_spawn`.
    pub classname: String,
    pub origin: [f32; 3],
    /// Pitch, yaw, roll in degrees.
    pub angles: [f32; 3],
    /// The `script_linkto` key, for spawns a script groups together.
    pub script_linkto: String,
    /// The `script_destructable_area` key; empty when the spawn is not tied
    /// to a destructible area.
    pub script_destructable_area: String,
}

/// The fixed camera the end-of-match scoreboard is shown from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntermissionView {
    pub origin: [f32; 3],
    /// Pitch, yaw, roll in degrees.
    pub angles: [f32; 3],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spawn_compares_by_every_key() {
        let spawn = SpawnPoint {
            classname: "mp_dm_spawn".to_owned(),
            origin: [1.0, 2.0, 3.0],
            angles: [0.0, 90.0, 0.0],
            script_linkto: String::new(),
            script_destructable_area: String::new(),
        };
        let mut moved = spawn.clone();
        assert_eq!(spawn, moved);
        moved.origin[2] = 4.0;
        assert_ne!(spawn, moved);
    }
}
