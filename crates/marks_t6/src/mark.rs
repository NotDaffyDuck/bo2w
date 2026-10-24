//! One mark in the pool: making it from a staged request, and picking which mark to drop
//! when the pool is full.

use crate::GfxMarkContext;

/// How many marks were made before the first frame.
pub const FX_MARKS_INIT_ALLOCED_COUNT: u32 = 0;

/// Why no mark was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxAllocMarkRefuse {
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
    pub const fn expired(self, age_msec: i32) -> bool {
        self.age_limit_msec > 0 && age_msec > self.age_limit_msec
    }
}

/// A mark in the pool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxMarkConstructed {
    /// The last frame it was drawn; a new mark counts as drawn the frame before it was made.
    pub frame_count_drawn: i32,
    pub frame_count_alloced: i32,
    pub age_msec: i32,
    pub alpha_fade: FxMarkAlphaFade,
    pub origin: [f32; 3],
    pub radius: f32,
    /// The texture's u direction; v is this crossed with each point's normal.
    pub tex_coord_axis: [f32; 3],
    pub native_color: u32,
    pub material: u32,
    pub context: GfxMarkContext,
    pub tri_count: u8,
    pub point_count: i16,
    /// The first group of its triangle chain.
    pub tris: u16,
    /// The first group of its point chain.
    pub points: u16,
}

/// What a new mark is made from: the staged counts and the mark's look.
#[derive(Clone, Copy, Debug)]
pub struct FxAllocMarkRequest {
    /// The cut produced pieces.
    pub any_marks: bool,
    pub tri_count: u32,
    pub point_count: u32,
    pub origin: [f32; 3],
    pub radius: f32,
    pub tex_coord_axis: [f32; 3],
    pub native_color: u32,
    pub material: u32,
    pub frame_count: i32,
    /// The context of the first staged triangle, which the mark takes as its own.
    pub first_tri_context: GfxMarkContext,
}

/// Makes the mark for `req` whose triangle and point chains start at `tris` and `points`.
pub fn alloc_and_construct_mark(
    req: &FxAllocMarkRequest,
    tris: u16,
    points: u16,
) -> Result<FxMarkConstructed, FxAllocMarkRefuse> {
    if !req.any_marks {
        return Err(FxAllocMarkRefuse::NoFragments);
    }
    if req.tri_count == 0 {
        return Err(FxAllocMarkRefuse::NoTriangles);
    }
    Ok(FxMarkConstructed {
        frame_count_drawn: req.frame_count.wrapping_sub(1),
        frame_count_alloced: req.frame_count,
        age_msec: 0,
        alpha_fade: FxMarkAlphaFade::NEVER,
        origin: req.origin,
        radius: req.radius,
        tex_coord_axis: req.tex_coord_axis,
        native_color: req.native_color,
        material: req.material,
        context: req.first_tri_context,
        tri_count: req.tri_count as u8,
        point_count: req.point_count as i16,
        tris,
        points,
    })
}

/// The mark to drop to make room: the first one past its age limit, else the one drawn
/// longest ago (on a tie, made longest ago; then the first).
pub fn lru_mark_pick<'a, I>(marks: I) -> Option<u16>
where
    I: IntoIterator<Item = (u16, &'a FxMarkConstructed)>,
{
    let mut oldest: Option<(u16, (i32, i32))> = None;
    for (handle, mark) in marks {
        if mark.alpha_fade.expired(mark.age_msec) {
            return Some(handle);
        }
        let age = (mark.frame_count_drawn, mark.frame_count_alloced);
        if oldest.is_none_or(|(_, best)| age < best) {
            oldest = Some((handle, age));
        }
    }
    oldest.map(|(handle, _)| handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(frame_count: i32) -> FxAllocMarkRequest {
        FxAllocMarkRequest {
            any_marks: true,
            tri_count: 4,
            point_count: 6,
            origin: [1.0, 2.0, 3.0],
            radius: 8.0,
            tex_coord_axis: [0.0, 1.0, 0.0],
            native_color: 0x8040_20ff,
            material: 7,
            frame_count,
            first_tri_context: GfxMarkContext::world_surface(3, 0, 0),
        }
    }

    #[test]
    fn a_new_mark_counts_as_drawn_last_frame() {
        let mark = alloc_and_construct_mark(&request(10), 5, 9).unwrap();
        assert_eq!(
            (
                mark.frame_count_drawn,
                mark.frame_count_alloced,
                mark.age_msec
            ),
            (9, 10, 0)
        );
        assert_eq!(
            (mark.tri_count, mark.point_count, mark.tris, mark.points),
            (4, 6, 5, 9)
        );
        assert_eq!(mark.alpha_fade, FxMarkAlphaFade::NEVER);
        assert_eq!(mark.context, GfxMarkContext::world_surface(3, 0, 0));
        assert_eq!(
            alloc_and_construct_mark(&request(i32::MIN), 0, 0)
                .unwrap()
                .frame_count_drawn,
            i32::MAX
        );
    }

    #[test]
    fn no_pieces_or_no_triangles_make_no_mark() {
        let empty = FxAllocMarkRequest {
            any_marks: false,
            ..request(1)
        };
        let flat = FxAllocMarkRequest {
            tri_count: 0,
            ..request(1)
        };
        assert_eq!(
            alloc_and_construct_mark(&empty, 0, 0),
            Err(FxAllocMarkRefuse::NoFragments)
        );
        assert_eq!(
            alloc_and_construct_mark(&flat, 0, 0),
            Err(FxAllocMarkRefuse::NoTriangles)
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
        marks[2].frame_count_alloced = 2;
        let listed = |m: &[FxMarkConstructed; 4]| {
            lru_mark_pick(m.iter().enumerate().map(|(k, mark)| (k as u16, mark)))
        };
        assert_eq!(listed(&marks), Some(2));
        marks[2].frame_count_alloced = 4;
        assert_eq!(listed(&marks), Some(1));
        marks[3].alpha_fade = FxMarkAlphaFade {
            age_limit_msec: 100,
            fade_start_age_msec: 0,
        };
        marks[3].age_msec = 101;
        assert_eq!(listed(&marks), Some(3));
        assert_eq!(lru_mark_pick(core::iter::empty()), None);
    }
}
