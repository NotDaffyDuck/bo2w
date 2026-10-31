use crate::shot_trace::{EntityHitAbilities, coll_tris_clip_available, contents_match_mask};

/// How many models the "no clip" sample names at most.
const SAMPLE_LIMIT: usize = 12;

/// What shots can do to the world's entities, one count per way they are (or are not) clipped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntityClipCensus {
    pub lines: u32,
    /// Clipped by collision triangles.
    pub colltris: u32,
    /// Clipped by bone boxes alone.
    pub boxes_only: u32,
    /// Clipped by linked brushes alone.
    pub brush_only: u32,
    /// Skipped: the model's contents are not ones a bullet stops at.
    pub not_bullet_solid: u32,
    /// Bullet-solid, but the model has no collision authored.
    pub no_collision_authored: u32,
    pub no_dobj: u32,
    pub no_capability: u32,
    pub materialize_failed: u32,
    /// Linked brushes over all rows.
    pub attached_solids: u32,
    /// Some of the models that got no clip at all.
    pub no_clip_sample: Vec<String>,
}

/// Which way one entity is clipped.
enum Clip {
    Triangles,
    Boxes,
    Brushes,
    NoBody,
    NotSolid,
    /// No usable clip, and why.
    Missing(Gap),
}

enum Gap {
    NoCapability,
    MaterializeFailed,
    NothingAuthored,
}

fn classify(owner: &EntityHitAbilities, mask: u32) -> Clip {
    let Some(state) = owner.dobj.as_ref() else {
        return if owner.attached_solids.is_empty() { Clip::NoBody } else { Clip::Brushes };
    };
    if let Some(collision) = state.live_hitbox.as_ref() {
        if coll_tris_clip_available(collision.coll.as_ref(), mask) {
            return Clip::Triangles;
        }
        if !collision.bones.is_empty() {
            return Clip::Boxes;
        }
    }
    let contents = state.can_do.as_ref().and_then(|cap| cap.contents);
    if !contents_match_mask(contents, mask) {
        return Clip::NotSolid;
    }
    if state.can_do.is_none() {
        Clip::Missing(Gap::NoCapability)
    } else if state.bake_failure.is_some() {
        Clip::Missing(Gap::MaterializeFailed)
    } else if !owner.attached_solids.is_empty() {
        Clip::Brushes
    } else {
        Clip::Missing(Gap::NothingAuthored)
    }
}

/// Counts how each of `owners` is clipped against a shot with `mask`.
pub(crate) fn entity_clip_census(owners: &[EntityHitAbilities], mask: u32) -> EntityClipCensus {
    let mut census = EntityClipCensus::default();
    for owner in owners {
        census.lines += 1;
        census.attached_solids += owner.attached_solids.len() as u32;
        match classify(owner, mask) {
            Clip::Triangles => census.colltris += 1,
            Clip::Boxes => census.boxes_only += 1,
            Clip::Brushes => census.brush_only += 1,
            Clip::NoBody => census.no_dobj += 1,
            Clip::NotSolid => census.not_bullet_solid += 1,
            Clip::Missing(gap) => {
                match gap {
                    Gap::NoCapability => census.no_capability += 1,
                    Gap::MaterializeFailed => census.materialize_failed += 1,
                    Gap::NothingAuthored => census.no_collision_authored += 1,
                }
                let model = owner.dobj.as_ref().map(|state| &state.active_mesh);
                if let Some(model) = model.filter(|m| !census.no_clip_sample.contains(m)) {
                    if census.no_clip_sample.len() < SAMPLE_LIMIT {
                        census.no_clip_sample.push(model.clone());
                    }
                }
            }
        }
    }
    census
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_count_gives_an_empty_report() {
        assert_eq!(entity_clip_census(&[], 0), EntityClipCensus::default());
    }
}
