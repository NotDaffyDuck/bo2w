//! The list of world draws for the frame, in sort order, as the visibility
//! pass produced them; plus the hash helpers other modules use to give a
//! draw set a content id.

use bevy::prelude::*;

use crate::prepare::scene::cull::DpvsFrameStats;
use crate::prepare::scene::world::{WorldCull, WorldDrawItemKind, WorldScene};

/// One entry in the sorted list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawSurfItem {
    pub key: u64,

    pub surf: u16,
    pub run: u16,
    /// The run's surfaces are `DrawSurfList::surfs[list_start..][..run]`.
    pub list_start: u32,
    pub entry_first: u16,
    pub kind: WorldDrawItemKind,
}

/// The frame's sorted world draws, and the counts used to cross-check them.
#[derive(Resource, Debug, Clone, Default)]
pub struct DrawSurfList {
    pub items: Vec<DrawSurfItem>,
    pub surfs: Vec<u16>,
    /// Number of items.
    pub keys: u32,
    /// How many times the material changes walking the list.
    pub rebinds: u32,

    pub draw_items_id: u64,
}

impl DrawSurfList {
    /// Rebuild the list from the visibility pass's output.
    pub fn ingest_from_cull(&mut self, cull: &WorldCull) {
        self.surfs.clone_from(&cull.item_surfs);
        self.items.clear();
        self.items
            .extend(cull.draw_items.iter().map(|item| DrawSurfItem {
                key: item.key,
                surf: item.surf,
                run: item.run,
                list_start: item.list_start,
                entry_first: item.entry_first,
                kind: item.kind,
            }));
        let material_of = |key: u64| dpvs_t6::GfxDrawSurf::from_packed(key).material_sorted_index();
        let mut previous = None;
        let mut rebinds = 0u32;
        for item in &self.items {
            let material = material_of(item.key);
            if previous != Some(material) {
                rebinds = rebinds.saturating_add(1);
            }
            previous = Some(material);
        }
        self.keys = self.items.len() as u32;
        self.rebinds = rebinds;
        self.draw_items_id = cull.draw_items_id;
    }
}

/// Take the frame's draws from the visibility pass, and complain if the
/// counts it kept disagree with the ones just derived.
pub(crate) fn ingest_drawsurf_list(
    scene: Res<WorldScene>,
    stats: Option<Res<DpvsFrameStats>>,
    mut list: ResMut<DrawSurfList>,
) {
    let Some(cull) = scene.cull.as_ref() else {
        return;
    };
    list.ingest_from_cull(cull);
    let Some(stats) = stats else {
        return;
    };
    if (list.keys, list.rebinds) != (stats.keys, stats.rebinds) {
        diag::warn!(
            World,
            "drawsurf census mismatch: list keys={} rebinds={} vs dpvs keys={} rebinds={} \
             (one side lies; the drawsurf list and dpvs disagree)",
            list.keys,
            list.rebinds,
            stats.keys,
            stats.rebinds
        );
    }
}

/// Starting value for a content id (the 64-bit FNV offset basis).
pub(crate) const CONTENT_ID_SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// The 64-bit FNV prime.
const CONTENT_ID_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold one word into a content id.
pub(crate) fn mix_content_id(id: &mut u64, word: u64) {
    *id = (*id ^ word).wrapping_mul(CONTENT_ID_PRIME);
}

/// Fold a byte string into a content id: its length, then its bytes eight at
/// a time (low byte first, a short last chunk zero-padded).
pub(crate) fn mix_content_bytes(id: &mut u64, bytes: &[u8]) {
    mix_content_id(id, bytes.len() as u64);
    for chunk in bytes.chunks(8) {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        mix_content_id(id, u64::from_le_bytes(word));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_zero_into_the_seed_multiplies_by_the_prime() {
        let mut id = CONTENT_ID_SEED;
        mix_content_id(&mut id, 0);
        assert_eq!(id, CONTENT_ID_SEED.wrapping_mul(CONTENT_ID_PRIME));
    }

    #[test]
    fn order_matters() {
        let (mut a, mut b) = (CONTENT_ID_SEED, CONTENT_ID_SEED);
        mix_content_id(&mut a, 1);
        mix_content_id(&mut a, 2);
        mix_content_id(&mut b, 2);
        mix_content_id(&mut b, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn bytes_are_the_length_then_little_endian_words() {
        let mut by_bytes = CONTENT_ID_SEED;
        mix_content_bytes(&mut by_bytes, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut by_words = CONTENT_ID_SEED;
        mix_content_id(&mut by_words, 9);
        mix_content_id(&mut by_words, 0x0807_0605_0403_0201);
        mix_content_id(&mut by_words, 9);
        assert_eq!(by_bytes, by_words);
    }

    #[test]
    fn byte_strings_of_different_length_differ_even_when_padding_matches() {
        let (mut short, mut long) = (CONTENT_ID_SEED, CONTENT_ID_SEED);
        mix_content_bytes(&mut short, &[7]);
        mix_content_bytes(&mut long, &[7, 0]);
        assert_ne!(short, long);
    }

    #[test]
    fn an_empty_byte_string_mixes_only_its_length() {
        let mut id = CONTENT_ID_SEED;
        mix_content_bytes(&mut id, &[]);
        let mut expected = CONTENT_ID_SEED;
        mix_content_id(&mut expected, 0);
        assert_eq!(id, expected);
    }

    #[test]
    fn an_empty_list_has_no_items_and_no_rebinds() {
        let list = DrawSurfList::default();
        assert_eq!((list.keys, list.rebinds, list.items.len()), (0, 0, 0));
    }
}
