//! One mark in the pool: making it from a staged request, and picking which mark to drop
//! when the pool is full.

use crate::GfxMarkContext;

/// How many marks were made before the first frame.
pub const DECAL_INITIAL_CLAIMED: u32 = 0;

/// Why no mark was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalClaimDenial {
    /// The cut found no pieces at all.
    NoFragments,
    /// Pieces, but no triangles among them.
    NoTriangles,
}

/// How long a mark lasts and when it starts to fade, from its effect. Both 0 means it
/// stays until pushed out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxMarkAlphaFade {
    pub age_limit_msec: i32,
    pub fade_start_age_msec: i32,
}

impl FxMarkAlphaFade {
    pub const NEVER: Self = Self {
        age_limit_msec: 0,
        fade_start_age_msec: 0,
    };

    /// Past its age limit, when it has one.
    pub const fn expired(self, lived_ms: i32) -> bool {
        self.age_limit_msec > 0 && lived_ms > self.age_limit_msec
    }
}

/// A mark in the pool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuiltDecalRecord {
    /// The last frame it was drawn; a new mark counts as drawn the frame before it was made.
    pub frames_since_shown: i32,
    pub frames_since_claimed: i32,
    pub lived_ms: i32,
    pub alpha_fade: FxMarkAlphaFade,
    pub origin: [f32; 3],
    pub radius: f32,
    /// The texture's u direction; v is this crossed with each point's normal.
    pub uv_projection_axis: [f32; 3],
    pub authored_tint: u32,
    pub look_id: u32,
    pub context: GfxMarkContext,
    pub tri_tally: u8,
    pub vertex_total_in_mark: i16,
    /// The first group of its triangle chain.
    pub triangle_list: u16,
    /// The first group of its point chain.
    pub points: u16,
}

/// What a new mark is made from: the staged counts and the mark's look.
#[derive(Clone, Copy, Debug)]
pub struct DecalAllocationAsk {
    /// The cut produced pieces.
    pub any_stains: bool,
    pub tri_tally: u32,
    pub vertex_total_in_mark: u32,
    pub origin: [f32; 3],
    pub radius: f32,
    pub uv_projection_axis: [f32; 3],
    pub authored_tint: u32,
    pub material: u32,
    pub anim_steps: i32,
    /// The context of the first staged triangle, which the mark takes as its own.
    pub leading_triangle_mark_info: GfxMarkContext,
}

/// Makes the mark for `ask` whose triangle and point chains start at `triangle_list` and `points`.
pub fn alloc_and_construct_mark(
    ask: &DecalAllocationAsk,
    triangle_list: u16,
    points: u16,
) -> Result<BuiltDecalRecord, DecalClaimDenial> {
    if !ask.any_stains {
        return Err(DecalClaimDenial::NoFragments);
    }
    if ask.tri_tally == 0 {
        return Err(DecalClaimDenial::NoTriangles);
    }
    Ok(BuiltDecalRecord {
        frames_since_shown: ask.anim_steps.wrapping_sub(1),
        frames_since_claimed: ask.anim_steps,
        lived_ms: 0,
        alpha_fade: FxMarkAlphaFade::NEVER,
        origin: ask.origin,
        radius: ask.radius,
        uv_projection_axis: ask.uv_projection_axis,
        authored_tint: ask.authored_tint,
        look_id: ask.material,
        context: ask.leading_triangle_mark_info,
        tri_tally: ask.tri_tally as u8,
        vertex_total_in_mark: ask.vertex_total_in_mark as i16,
        triangle_list,
        points,
    })
}

/// The mark to drop to make room: the first one past its age limit, else the one drawn
/// longest ago (on a tie, made longest ago; then the first).
pub fn lru_mark_pick<'a, I>(marks: I) -> Option<u16>
where
    I: IntoIterator<Item = (u16, &'a BuiltDecalRecord)>,
{
    let mut oldest: Option<(u16, (i32, i32))> = None;
    for (slot_no, live) in marks {
        if live.alpha_fade.expired(live.lived_ms) {
            return Some(slot_no);
        }
        let age = (live.frames_since_shown, live.frames_since_claimed);
        if oldest.is_none_or(|(_, best)| age < best) {
            oldest = Some((slot_no, age));
        }
    }
    oldest.map(|(handle, _)| handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(frame_count: i32) -> DecalAllocationAsk {
        DecalAllocationAsk {
            any_stains: true,
            tri_tally: 4,
            vertex_total_in_mark: 6,
            origin: [1.0, 2.0, 3.0],
            radius: 8.0,
            uv_projection_axis: [0.0, 1.0, 0.0],
            authored_tint: 0x8040_20ff,
            material: 7,
            anim_steps: frame_count,
            leading_triangle_mark_info: GfxMarkContext::world_surface(3, 0, 0),
        }
    }

    #[test]
    fn a_new_mark_counts_as_drawn_last_frame() {
        let mark = alloc_and_construct_mark(&request(10), 5, 9).unwrap();
        assert_eq!(
            (
                mark.frames_since_shown,
                mark.frames_since_claimed,
                mark.lived_ms
            ),
            (9, 10, 0)
        );
        assert_eq!(
            (mark.tri_tally, mark.vertex_total_in_mark, mark.triangle_list, mark.points),
            (4, 6, 5, 9)
        );
        assert_eq!(mark.alpha_fade, FxMarkAlphaFade::NEVER);
        assert_eq!(mark.context, GfxMarkContext::world_surface(3, 0, 0));
        assert_eq!(
            alloc_and_construct_mark(&request(i32::MIN), 0, 0)
                .unwrap()
                .frames_since_shown,
            i32::MAX
        );
    }

    #[test]
    fn no_pieces_or_no_triangles_make_no_mark() {
        let empty = DecalAllocationAsk {
            any_stains: false,
            ..request(1)
        };
        let flat = DecalAllocationAsk {
            tri_tally: 0,
            ..request(1)
        };
        assert_eq!(
            alloc_and_construct_mark(&empty, 0, 0),
            Err(DecalClaimDenial::NoFragments)
        );
        assert_eq!(
            alloc_and_construct_mark(&flat, 0, 0),
            Err(DecalClaimDenial::NoTriangles)
        );
    }

    #[test]
    fn expiry_needs_a_limit() {
        assert!(!FxMarkAlphaFade::NEVER.expired(i32::MAX));
        let fade = FxMarkAlphaFade {
            age_limit_msec: 1000,
            fade_start_age_msec: 500,
        };
        assert!(!fade.expired(1000));
        assert!(fade.expired(1001));
    }

    #[test]
    fn the_pick_prefers_expired_then_least_recently_drawn() {
        let mut marks = [request(10), request(4), request(4), request(20)]
            .map(|r| alloc_and_construct_mark(&r, 0, 0).unwrap());
        marks[2].frames_since_claimed = 2;
        let listed = |m: &[BuiltDecalRecord; 4]| {
            lru_mark_pick(m.iter().enumerate().map(|(k, mark)| (k as u16, mark)))
        };
        assert_eq!(listed(&marks), Some(2));
        marks[2].frames_since_claimed = 4;
        assert_eq!(listed(&marks), Some(1));
        marks[3].alpha_fade = FxMarkAlphaFade {
            age_limit_msec: 100,
            fade_start_age_msec: 0,
        };
        marks[3].lived_ms = 101;
        assert_eq!(listed(&marks), Some(3));
        assert_eq!(lru_mark_pick(core::iter::empty()), None);
    }
}
