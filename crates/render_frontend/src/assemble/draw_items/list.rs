//! The list of world draws for the frame, in sort order, as the visibility
//! pass produced them; plus the hash helpers other modules use to give a
//! draw set a content id.

use bevy::prelude::*;

use crate::prepare::scene::cull::SightFrameStats;
use crate::prepare::scene::world::{LevelHide, LevelDrawItemKind, LevelStage};

/// One entry in the sorted list: its sort key, its first surface and how many
/// surfaces the run covers, where those sit in the shared surface list, and
/// what kind of world draw it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FacetDrawItem {
    /// The packed draw-surf sort key.
    pub key: u64,
    /// First surface of the run.
    pub facet_ix: u16,
    /// Surfaces in the run.
    pub span_run: u16,
    /// The run's surfaces are `FacetDrawList::surfs[list_start..][..run]`.
    pub list_start: u32,
    pub entry_first: u16,
    pub kind: LevelDrawItemKind,
}

/// The frame's sorted world draws, and the counts used to cross-check them.
#[derive(Resource, Debug, Clone, Default)]
pub struct FacetDrawList {
    pub items: Vec<FacetDrawItem>,
    pub surfs: Vec<u16>,
    /// Number of items.
    pub keys: u32,
    /// How many times the material changes walking the list.
    pub rebind_count: u32,

    pub draw_batch_list_id: u64,
}

impl FacetDrawList {
    /// Rebuild the list from the visibility pass's output.
    pub fn ingest_from_hide(&mut self, cull: &LevelHide) {
        self.surfs.clone_from(&cull.item_surfs);
        let copied = cull.draw_batch_list.iter().map(|d| FacetDrawItem {
            key: d.key,
            facet_ix: d.facet_ix,
            span_run: d.batch_run,
            list_start: d.list_start,
            entry_first: d.entry_first,
            kind: d.kind,
        });
        self.items.truncate(0);
        self.items.extend(copied);

        // A material bind for the first item, then one more each time the
        // material differs from the item before it.
        let material = |item: &FacetDrawItem| dpvs_t6::SortedDraw::unpack_from_bits(item.key).material_draw_rank_slot();
        let changes = self.items.windows(2).filter(|pair| material(&pair[0]) != material(&pair[1])).count();
        let binds = if self.items.is_empty() { 0 } else { changes + 1 };

        let count = self.items.len();
        self.keys = count as u32;
        self.rebind_count = u32::try_from(binds).unwrap_or(u32::MAX);
        self.draw_batch_list_id = cull.draw_batch_list_id;
    }
}

/// Take the frame's draws from the visibility pass, and complain if the
/// counts it kept disagree with the ones just derived.
pub(crate) fn take_drawsurf_list(
    scene: Res<LevelStage>,
    stats: Option<Res<SightFrameStats>>,
    mut list: ResMut<FacetDrawList>,
) {
    let Some(hide) = scene.culling.as_ref() else { return };
    list.ingest_from_hide(hide);
    let ours = (list.keys, list.rebind_count);
    if let Some(dpvs) = stats.filter(|counted| (counted.keys, counted.rebinds) != ours) {
        diag::warn!(
            World,
            "drawsurf counts disagree: the list has {} items and {} material binds, dpvs counted {} and {} \
             (one of the two is wrong)",
            ours.0,
            ours.1,
            dpvs.keys,
            dpvs.rebinds
        );
    }
}

/// Starting value for a content id (the 64-bit FNV offset basis).
pub(crate) const CONTENT_KEY_SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// The 64-bit FNV prime.
const CONTENT_ID_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold one word into a content id.
pub(crate) fn blend_content_id(id: &mut u64, word: u64) {
    *id = (*id ^ word).wrapping_mul(CONTENT_ID_PRIME);
}

/// Fold a byte string into a content id: its length, then its bytes eight at
/// a time (low byte first, a short last chunk zero-padded).
pub(crate) fn fold_content_bytes(ident: &mut u64, data: &[u8]) {
    blend_content_id(ident, data.len() as u64);
    let words = data.chunks(8).map(|piece| {
        let mut padded = [0u8; 8];
        padded[..piece.len()].copy_from_slice(piece);
        u64::from_le_bytes(padded)
    });
    words.for_each(|word| blend_content_id(ident, word));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_zero_into_the_seed_multiplies_by_the_prime() {
        let mut id = CONTENT_KEY_SEED;
        blend_content_id(&mut id, 0);
        assert_eq!(id, CONTENT_KEY_SEED.wrapping_mul(CONTENT_ID_PRIME));
    }

    #[test]
    fn order_matters() {
        let (mut a, mut b) = (CONTENT_KEY_SEED, CONTENT_KEY_SEED);
        blend_content_id(&mut a, 1);
        blend_content_id(&mut a, 2);
        blend_content_id(&mut b, 2);
        blend_content_id(&mut b, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn bytes_are_the_length_then_little_endian_words() {
        let mut by_bytes = CONTENT_KEY_SEED;
        fold_content_bytes(&mut by_bytes, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut by_words = CONTENT_KEY_SEED;
        blend_content_id(&mut by_words, 9);
        blend_content_id(&mut by_words, 0x0807_0605_0403_0201);
        blend_content_id(&mut by_words, 9);
        assert_eq!(by_bytes, by_words);
    }

    #[test]
    fn byte_strings_of_different_length_differ_even_when_padding_matches() {
        let (mut short, mut long) = (CONTENT_KEY_SEED, CONTENT_KEY_SEED);
        fold_content_bytes(&mut short, &[7]);
        fold_content_bytes(&mut long, &[7, 0]);
        assert_ne!(short, long);
    }

    #[test]
    fn an_empty_byte_string_mixes_only_its_length() {
        let mut id = CONTENT_KEY_SEED;
        fold_content_bytes(&mut id, &[]);
        let mut expected = CONTENT_KEY_SEED;
        blend_content_id(&mut expected, 0);
        assert_eq!(id, expected);
    }

    #[test]
    fn an_empty_list_has_no_items_and_no_rebinds() {
        let list = FacetDrawList::default();
        assert_eq!((list.keys, list.rebind_count, list.items.len()), (0, 0, 0));
    }
}
