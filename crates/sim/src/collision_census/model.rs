use xmodel_runtime::RetainedModelCapability;

/// What one model brings to collision.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelCollisionCensus {
    /// The model's name.
    pub key: String,
    /// False when no model of that name is loaded; the counts are then all zero.
    pub present: bool,
    pub bones: u32,
    /// Bones that carry a hit box.
    pub bone_boxes: u32,
    pub coll_lod: i16,
    pub coll_surfs: u32,
    pub coll_tris: u32,
    pub contents: Option<u32>,
}

impl ModelCollisionCensus {
    /// Counts `capability` (the model named `key`, if it is loaded).
    pub fn of(key: &str, capability: Option<&RetainedModelCapability>) -> Self {
        let key = key.to_owned();
        let Some(cap) = capability else {
            return Self { key, ..Self::default() };
        };
        let tris = cap.coll_surfs.iter().map(|surf| surf.tris.len()).sum::<usize>();
        Self {
            key,
            present: true,
            bones: cap.pose.num_bones as u32,
            bone_boxes: cap.bone_collision.iter().flatten().count() as u32,
            coll_lod: cap.coll_lod,
            coll_surfs: cap.coll_surfs.len() as u32,
            coll_tris: tris as u32,
            contents: cap.contents,
        }
    }

    /// The best clip a shot can use against this model: collision triangles, else bone boxes.
    pub fn clip(&self) -> &'static str {
        match (self.present, self.coll_tris, self.bone_boxes) {
            (false, ..) => "absent",
            (true, tris, _) if tris > 0 => "colltris",
            (true, _, boxes) if boxes > 0 => "boxes",
            _ => "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted(present: bool, coll_tris: u32, bone_boxes: u32) -> ModelCollisionCensus {
        ModelCollisionCensus {
            present,
            coll_tris,
            bone_boxes,
            ..Default::default()
        }
    }

    #[test]
    fn a_missing_model_counts_nothing() {
        let census = ModelCollisionCensus::of("gone", None);
        assert_eq!(census.key, "gone");
        assert!(!census.present);
        assert_eq!((census.bones, census.coll_tris), (0, 0));
    }

    #[test]
    fn triangles_beat_boxes_and_a_missing_model_is_absent() {
        assert_eq!(counted(false, 9, 9).clip(), "absent");
        assert_eq!(counted(true, 1, 5).clip(), "colltris");
        assert_eq!(counted(true, 0, 5).clip(), "boxes");
        assert_eq!(counted(true, 0, 0).clip(), "none");
    }
}
