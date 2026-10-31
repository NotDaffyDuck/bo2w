use std::sync::Arc;

/// A share a plan published, or an empty one when it has published none.
pub(super) fn share<T>(rows: Option<&Arc<Vec<T>>>) -> Arc<Vec<T>> {
    rows.map_or_else(Arc::default, Arc::clone)
}

/// The decoded vertices, indices and surface ranges a plan published, as
/// three empty shares when there is no plan.
macro_rules! decoded_shares {
    ($plan:expr) => {
        match $plan {
            Some(found) => (
                $crate::intake::shares::share(found.decoded_share.as_ref()),
                $crate::intake::shares::share(found.index_portion.as_ref()),
                $crate::intake::shares::share(found.range_portion.as_ref()),
            ),
            None => Default::default(),
        }
    };
}
pub(super) use decoded_shares;
