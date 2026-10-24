//! A hand-over queue for values whose drop is slow or must not happen on the
//! main thread. Producers move values in; the owner of the queue ships the
//! batch to a worker and counts the shipment until the worker settles it.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::ecs::component::Mutable;
use bevy::prelude::{Resource, World};

type Handed = Box<dyn Any + Send + Sync>;

#[derive(Resource, Default)]
pub struct Retiring {
    handed: Vec<Handed>,
    teardown_requested: bool,
    jobs: u64,
    values: u64,
    in_flight: Arc<AtomicUsize>,
    teardown_in_flight: Arc<AtomicUsize>,
}

impl Retiring {
    /// Queues `value` for the next batch.
    pub fn hand_over<T: Send + Sync + 'static>(&mut self, value: T) {
        self.handed.push(Box::new(value));
        self.values = self.values.saturating_add(1);
    }

    /// Marks the next batch as the one that finishes a teardown.
    pub fn mark_teardown(&mut self) {
        self.teardown_requested = true;
    }

    /// Empties the queue. The flag comes back set when a teardown was marked
    /// since the last call; an empty batch does not count as a job.
    pub fn take_batch(&mut self) -> (Vec<Handed>, bool) {
        let batch = std::mem::take(&mut self.handed);
        let teardown = std::mem::take(&mut self.teardown_requested);
        if !batch.is_empty() {
            self.jobs = self.jobs.saturating_add(1);
        }
        (batch, teardown)
    }

    /// Counts a shipment as in flight until the returned token is settled.
    pub fn ship(&self, teardown: bool) -> RetiringInFlight {
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        let teardown = teardown.then(|| {
            self.teardown_in_flight.fetch_add(1, Ordering::AcqRel);
            Arc::clone(&self.teardown_in_flight)
        });
        RetiringInFlight {
            all: Arc::clone(&self.in_flight),
            teardown,
        }
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Anything queued, marked or still being dropped.
    pub fn busy(&self) -> bool {
        self.teardown_requested || !self.handed.is_empty() || self.in_flight() != 0
    }

    /// `(jobs shipped, values handed over, values waiting)`.
    pub fn counts(&self) -> (u64, u64, usize) {
        (self.jobs, self.values, self.handed.len())
    }
}

/// Proof that a batch is being dropped somewhere.
pub struct RetiringInFlight {
    all: Arc<AtomicUsize>,
    teardown: Option<Arc<AtomicUsize>>,
}

impl RetiringInFlight {
    /// Ends the shipment. For a teardown shipment, returns how many teardown
    /// shipments are still outstanding afterwards.
    pub fn settle(self) -> Option<usize> {
        self.all.fetch_sub(1, Ordering::AcqRel);
        self.teardown
            .map(|counter| counter.fetch_sub(1, Ordering::AcqRel) - 1)
    }
}

/// Lets `take` pull resources out of `world` into the [`Retiring`] queue,
/// creating the queue first if the world has none.
pub fn retire_resources(world: &mut World, take: impl FnOnce(&mut RetireBatch<'_>)) {
    world.init_resource::<Retiring>();
    world.resource_scope(|world, mut retiring: bevy::prelude::Mut<Retiring>| {
        take(&mut RetireBatch {
            world,
            retiring: &mut retiring,
        });
    });
}

pub struct RetireBatch<'w> {
    world: &'w mut World,
    retiring: &'w mut Retiring,
}

impl RetireBatch<'_> {
    /// Removes `T` from the world (if present) and queues it.
    pub fn resource<T: Resource>(&mut self) -> &mut Self {
        if let Some(value) = self.world.remove_resource::<T>() {
            self.retiring.hand_over(value);
        }
        self
    }

    /// Queues the current `T` and leaves a default one in its place.
    pub fn reset<T: Resource<Mutability = Mutable> + Default>(&mut self) -> &mut Self {
        if let Some(mut slot) = self.world.get_resource_mut::<T>() {
            self.retiring.hand_over(std::mem::take(&mut *slot));
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default, Debug, PartialEq)]
    struct Big(Vec<u8>);

    #[derive(Resource, Default)]
    struct Other;

    #[test]
    fn a_new_queue_is_idle() {
        let queue = Retiring::default();
        assert!(!queue.busy());
        assert_eq!(queue.in_flight(), 0);
        assert_eq!(queue.counts(), (0, 0, 0));
    }

    #[test]
    fn handed_values_make_it_busy_until_taken() {
        let mut queue = Retiring::default();
        queue.hand_over(String::from("a"));
        queue.hand_over(7u32);
        assert!(queue.busy());
        assert_eq!(queue.counts(), (0, 2, 2));

        let (batch, teardown) = queue.take_batch();
        assert_eq!(batch.len(), 2);
        assert!(!teardown);
        assert!(!queue.busy());
        assert_eq!(queue.counts(), (1, 2, 0));
    }

    #[test]
    fn an_empty_batch_is_not_a_job() {
        let mut queue = Retiring::default();
        let (batch, _) = queue.take_batch();
        assert!(batch.is_empty());
        assert_eq!(queue.counts().0, 0);
    }

    #[test]
    fn the_teardown_mark_is_reported_once() {
        let mut queue = Retiring::default();
        queue.mark_teardown();
        assert!(queue.busy());
        assert!(queue.take_batch().1);
        assert!(!queue.busy());
        assert!(!queue.take_batch().1);
    }

    #[test]
    fn handed_values_come_back_downcastable() {
        let mut queue = Retiring::default();
        queue.hand_over(Big(vec![1, 2, 3]));
        let (mut batch, _) = queue.take_batch();
        let value = batch.pop().expect("one value");
        assert_eq!(value.downcast_ref::<Big>(), Some(&Big(vec![1, 2, 3])));
    }

    #[test]
    fn shipments_count_until_settled() {
        let queue = Retiring::default();
        let plain = queue.ship(false);
        let teardown = queue.ship(true);
        assert_eq!(queue.in_flight(), 2);
        assert!(queue.busy());

        assert_eq!(plain.settle(), None);
        assert_eq!(queue.in_flight(), 1);
        assert_eq!(teardown.settle(), Some(0));
        assert_eq!(queue.in_flight(), 0);
        assert!(!queue.busy());
    }

    #[test]
    fn settling_reports_the_teardowns_still_outstanding() {
        let queue = Retiring::default();
        let first = queue.ship(true);
        let second = queue.ship(true);
        assert_eq!(first.settle(), Some(1));
        assert_eq!(second.settle(), Some(0));
    }

    #[test]
    fn retiring_resources_removes_them_and_queues_the_values() {
        let mut world = World::new();
        world.insert_resource(Big(vec![9; 4]));
        world.insert_resource(Other);

        retire_resources(&mut world, |batch| {
            batch.resource::<Big>().resource::<Other>();
        });

        assert!(world.get_resource::<Big>().is_none());
        assert!(world.get_resource::<Other>().is_none());
        assert_eq!(world.resource::<Retiring>().counts(), (0, 2, 2));
    }

    #[test]
    fn retiring_a_missing_resource_queues_nothing() {
        let mut world = World::new();
        retire_resources(&mut world, |batch| {
            batch.resource::<Big>();
        });
        assert_eq!(world.resource::<Retiring>().counts(), (0, 0, 0));
    }

    #[test]
    fn reset_queues_the_old_value_and_leaves_a_default() {
        let mut world = World::new();
        world.insert_resource(Big(vec![5; 3]));

        retire_resources(&mut world, |batch| {
            batch.reset::<Big>();
        });

        assert_eq!(world.resource::<Big>(), &Big(Vec::new()));
        let mut queue = world.resource_mut::<Retiring>();
        let (mut batch, _) = queue.take_batch();
        let old = batch.pop().expect("the old value");
        assert_eq!(old.downcast_ref::<Big>(), Some(&Big(vec![5; 3])));
    }

    #[test]
    fn reset_of_a_missing_resource_does_not_create_one() {
        let mut world = World::new();
        retire_resources(&mut world, |batch| {
            batch.reset::<Big>();
        });
        assert!(world.get_resource::<Big>().is_none());
    }
}
