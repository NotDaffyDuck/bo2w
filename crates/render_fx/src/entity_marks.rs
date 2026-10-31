//! Shared queue of decal-style marks that gameplay code wants stuck onto entities.

use std::sync::{Mutex, MutexGuard};

use bevy::prelude::Resource;

/// Where a mark lands and how far it reaches.
#[derive(Clone, Copy)]
pub struct MarkPlacement {
    pub spot: [f32; 3],
    pub turn: [[f32; 3]; 3],
    pub reach_r: f32,
}

/// What a mark looks like.
#[derive(Clone)]
pub struct MarkPaint {
    pub shader: String,
    pub tint: u32,
}

/// A mark to be projected onto an entity, waiting to be picked up by the renderer.
pub struct PendingEntityMark {
    pub target: u16,
    pub placement: MarkPlacement,
    pub paint: MarkPaint,
}

/// The spot on an entity's composed model that carries a mark.
pub struct BoneSlot {
    pub prop: String,
    pub edit_no: u32,
    pub joint: usize,
}

/// A mark that has already been bound to a bone of an entity's model.
pub struct BoundEntityMark {
    pub target: u16,
    pub slot: BoneSlot,
    pub handle: u16,
}

/// How many marks could not be placed, by what stopped them.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Refusals {
    pub tri_total: u64,
    pub receivers: u64,
}

/// Contents behind the lock: queued requests, live attachments and refusal counters.
#[derive(Default)]
pub struct EntityMarkState {
    queued: Vec<PendingEntityMark>,
    bound: Vec<BoundEntityMark>,
    refused: Refusals,
}

impl EntityMarkState {
    pub fn enqueue(&mut self, mark: PendingEntityMark) {
        self.queued.push(mark);
    }

    /// Hands over every request made since the last call.
    pub fn drain_queue(&mut self) -> Vec<PendingEntityMark> {
        std::mem::take(&mut self.queued)
    }

    pub fn bind(&mut self, mark: BoundEntityMark) {
        self.bound.push(mark);
    }

    /// The marks already carried by `target`.
    pub fn bound_to(&self, target: u16) -> impl Iterator<Item = &BoundEntityMark> {
        self.bound.iter().filter(move |mark| mark.target == target)
    }

    pub fn refuse_receivers(&mut self, total: usize) {
        self.refused.receivers += total as u64;
    }

    pub fn refuse_triangle(&mut self) {
        self.refused.tri_total += 1;
    }

    pub fn refusals(&self) -> Refusals {
        self.refused
    }
}

/// Thread-safe holder for the mark state.
#[derive(Default, Resource)]
pub struct EntityMarkBoard {
    guarded: Mutex<EntityMarkState>,
}

impl EntityMarkBoard {
    /// Locks the state; panics if an earlier holder panicked while inside it.
    pub fn hold_marks(&self) -> MutexGuard<'_, EntityMarkState> {
        self.guarded
            .lock()
            .expect("entity mark state lock poisoned")
    }
}
