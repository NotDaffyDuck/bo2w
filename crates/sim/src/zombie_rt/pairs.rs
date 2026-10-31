//! Keeping the (key, value) pairs of a map or list walk by what the value
//! holds.

/// A (key, value) pair, owned or borrowed.
pub(crate) trait Pair {
    type Value;
    fn value(&self) -> &Self::Value;
}

impl<K, V> Pair for (K, V) {
    type Value = V;
    fn value(&self) -> &V {
        &self.1
    }
}

impl<K, V> Pair for &(K, V) {
    type Value = V;
    fn value(&self) -> &V {
        &self.1
    }
}

/// `.where_value(|v| ...)` on any iterator of pairs: the pairs whose value
/// passes, in the order they came.
pub(crate) trait KeepByValue: Iterator<Item: Pair> + Sized {
    fn where_value(self, mut keep: impl FnMut(&<Self::Item as Pair>::Value) -> bool) -> impl Iterator<Item = Self::Item> {
        self.filter(move |pair| keep(pair.value()))
    }
}

impl<I: Iterator<Item: Pair>> KeepByValue for I {}
