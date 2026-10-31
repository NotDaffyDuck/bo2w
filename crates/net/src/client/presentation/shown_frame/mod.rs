//! What the client shows: the snapshot being presented, the poses laid over it, and the small
//! resources the presentation systems share.

mod cues;
mod interpolate;
mod local;
mod local_choice;
mod throw_path;

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::Resource;
use playerstate_t6::game::AvatarState;
use sim::{ClientId, Snapshot};

pub use cues::{FirstPersonEventHints, ViewHandEntry, ViewHandRawReading};
pub(crate) use interpolate::blend_participant_state;
pub use local::{HostViewClient, OwnViewerTally, ViewweaponAim};

use crate::client::presentation::seen_throws::ShownProjectile;
use crate::client::remote_buffer::SmoothingSampleOrigin;

/// What the player sees: the newest snapshot with the pose and projectile rows computed for it.
#[derive(Resource, Clone, Debug, Default)]
pub struct ShownWorldFrame {
    projectile_rows: Vec<ShownProjectile>,
    /// Camera drop below the eye while a step eases out.
    eye_drop: f32,
    remote_origin: HashMap<ClientId, SmoothingSampleOrigin>,
    /// View values predicted for the local player; `None` means the snapshot's are used.
    predicted_view: Option<sim::T6ViewInputs>,
    sample_next: Option<Arc<Snapshot>>,
    /// Player states that stand in for the snapshot's own (the predicted local one and the
    /// proxied remote ones), by client.
    overrides: HashMap<ClientId, AvatarState>,
    frames: FramePair,
    eye_shift: [f32; 3],
    sample_clock_ms: Option<i32>,
}

/// The two snapshots interpolation runs between, and how far along it is.
#[derive(Clone, Debug, Default)]
struct FramePair {
    blend: f32,
    older: Option<Arc<Snapshot>>,
    newest: Option<Arc<Snapshot>>,
}

impl ShownWorldFrame {
    /// The state shown for `id`: an override when one is laid over it, otherwise the snapshot's.
    pub fn player(&self, peer: ClientId) -> Option<&AvatarState> {
        if let Some(over) = self.overrides.get(&peer) {
            return Some(over);
        }
        let newest = self.frames.newest.as_deref()?;
        newest
            .players
            .iter()
            .find_map(|(seat, state)| (*seat == peer).then_some(state))
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.frames.newest.as_deref()
    }

    pub fn tick(&self) -> Option<sim::Tick> {
        self.snapshot().map(|frame| frame.tick)
    }

    pub fn presented_projectiles(&self) -> &[ShownProjectile] {
        &self.projectile_rows
    }

    pub fn view_z_drop(&self) -> f32 {
        self.eye_drop
    }

    pub fn eye_displacement(&self) -> [f32; 3] {
        self.eye_shift
    }

    pub fn local_t6_view(&self) -> Option<sim::T6ViewInputs> {
        self.predicted_view
    }

    pub fn peer_reading_origin(&self, id: ClientId) -> Option<SmoothingSampleOrigin> {
        self.remote_origin.get(&id).copied()
    }

    pub fn blend_endpoints(&self) -> Option<(&Snapshot, &Snapshot, f32)> {
        let FramePair {
            newest,
            older,
            blend,
        } = &self.frames;
        Some((older.as_deref()?, newest.as_deref()?, *blend))
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Drops what belonged to the previous frame before either publish fills in the new one.
    fn begin_publish(&mut self, projectiles: Vec<ShownProjectile>) {
        self.frames.blend = 0.0;
        self.frames.older = None;
        self.overrides.clear();
        self.sample_clock_ms = None;
        self.sample_next = None;
        self.projectile_rows = projectiles;
    }

    /// Show a snapshot exactly as it was decoded.
    pub fn announce_parsed(&mut self, decoded: Snapshot) {
        self.begin_publish(authoritative_projectiles(&decoded));
        self.predicted_view = None;
        self.remote_origin.clear();
        self.frames.newest = Some(Arc::new(decoded));
    }

    /// Show a snapshot with the local player's predicted state and the proxied remote poses on it.
    pub fn announce_standin(
        &mut self,
        base_frame: Arc<Snapshot>,
        own_peer: ClientId,
        own_guess: AvatarState,
        peer_poses: HashMap<ClientId, AvatarState>,
        peer_sources: HashMap<ClientId, SmoothingSampleOrigin>,
    ) {
        self.begin_publish(authoritative_projectiles(&base_frame));
        self.remote_origin = peer_sources;
        self.frames.newest = Some(base_frame);
        self.overrides.insert(own_peer, own_guess);
        self.overrides.extend(peer_poses);
    }

    pub(crate) fn assign_state_blending(
        &mut self,
        prior_frame: Option<Arc<Snapshot>>,
        share: f32,
    ) {
        self.frames.blend = share;
        self.frames.older = prior_frame;
    }

    pub fn store_projectile_rows(&mut self, merged: Vec<ShownProjectile>) {
        self.projectile_rows = merged;
    }

    pub fn set_view_z_drop(&mut self, drop: f32) {
        self.eye_drop = drop;
    }

    pub fn store_eye_shift(&mut self, shift: [f32; 3]) {
        self.eye_shift = shift;
    }

    pub fn set_local_t6_view(&mut self, view: Option<sim::T6ViewInputs>) {
        self.predicted_view = view;
    }
}

fn authoritative_projectiles(frame: &Snapshot) -> Vec<ShownProjectile> {
    frame
        .projectiles
        .iter()
        .map(|row| ShownProjectile::HostDecided(*row))
        .collect()
}
