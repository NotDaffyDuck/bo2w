/// What a mark triangle was clipped from (BO2 `GfxMarkContext`, 6 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxMarkContext {
    pub baked_light_slot_number: u8,
    pub key_light_index: u8,
    pub env_probe_index: u8,
    /// Top two bits are the model type (`MARK_MODEL_TYPE_*`).
    pub model_type_and_surf: u8,
    pub mesh_slot_number: u16,
}

const _: () = assert!(core::mem::size_of::<GfxMarkContext>() == 6);

pub const MARK_MODEL_TYPE_MASK: u8 = 0xc0;
pub const MARK_MODEL_TYPE_WORLD_BRUSH: u8 = 0x00;
pub const MARK_MODEL_TYPE_STATIC_MODEL: u8 = 0x40;
/// An entity's brush model (BO2 ENT_BRUSH); the per-entity list holds it.
pub const MARK_MODEL_TYPE_ENTITY_BRUSH: u8 = 0x80;
/// An entity's DObj model (BO2 ENT_MODEL): 0xc0 | viewmodel bit | DObj model index (0-31).
pub const MARK_MODEL_TYPE_ENTITY_MODEL: u8 = 0xc0;
/// Set on an ENT_MODEL mark of the viewmodel: it goes to the viewmodel's mark list and skips
/// the entity-list checks when added.
pub const MARK_MODEL_VIEWMODEL_BIT: u8 = 0x20;
/// The DObj model index in an ENT_MODEL mark's low five bits.
pub const MARK_MODEL_DOBJ_MODEL_MASK: u8 = 0x1f;

impl GfxMarkContext {
    pub const ZERO: Self = Self {
        baked_light_slot_number: 0,
        key_light_index: 0,
        env_probe_index: 0,
        model_type_and_surf: 0,
        mesh_slot_number: 0,
    };

    /// A world brush surface's context: type 0 and modelIndex 0, as BO2's world-brush
    /// marks always are.
    #[inline]
    pub const fn world_surface(baked_light_slot: u8, dominant_light: u8, probe: u8) -> Self {
        Self {
            baked_light_slot_number: baked_light_slot,
            key_light_index: dominant_light,
            env_probe_index: probe,
            model_type_and_surf: MARK_MODEL_TYPE_WORLD_BRUSH,
            mesh_slot_number: 0,
        }
    }

    /// A glass pane's context: a world mark whose modelIndex is the pane's glass index,
    /// on the same world list as world-brush marks (BO2 glass marks).
    #[inline]
    pub const fn glass_pane(baked_light_slot: u8, dominant_light: u8, probe: u8, glass_index: u16) -> Self {
        Self {
            mesh_slot_number: glass_index,
            ..Self::world_surface(baked_light_slot, dominant_light, probe)
        }
    }

    /// A mark on one of an entity's DObj models; it has no lightmap. The model's index
    /// (0-31) goes in the low bits of the type byte, as in BO2.
    #[inline]
    pub const fn entity_model(ent: u16, dobj_model_index: u8) -> Self {
        Self {
            baked_light_slot_number: crate::SURFACE_BAKED_LIGHT_ABSENT,
            key_light_index: 0,
            env_probe_index: 0,
            model_type_and_surf: MARK_MODEL_TYPE_ENTITY_MODEL
                | (dobj_model_index & MARK_MODEL_DOBJ_MODEL_MASK),
            mesh_slot_number: ent,
        }
    }

    #[inline]
    pub const fn model_type(self) -> u8 {
        self.model_type_and_surf & MARK_MODEL_TYPE_MASK
    }

    /// 0x80 and 0xc0: entity models and brushes.
    #[inline]
    pub const fn is_entity(self) -> bool {
        self.model_type_and_surf & 0x80 != 0
    }

    #[inline]
    pub const fn is_viewmodel(self) -> bool {
        const BITS: u8 = MARK_MODEL_TYPE_ENTITY_MODEL | MARK_MODEL_VIEWMODEL_BIT;
        self.model_type_and_surf & BITS == BITS
    }

    /// An ENT_MODEL mark's DObj model index; `None` for other types.
    #[inline]
    pub const fn dobj_model_index(self) -> Option<u8> {
        if self.model_type() == MARK_MODEL_TYPE_ENTITY_MODEL {
            Some(self.model_type_and_surf & MARK_MODEL_DOBJ_MODEL_MASK)
        } else {
            None
        }
    }

    /// World brushes and static models: the marks the world list holds and the grid counts.
    #[inline]
    pub const fn is_world_list(self) -> bool {
        !self.is_entity()
    }

    /// World and brush marks keep world vertices; model and viewmodel marks are packed.
    #[inline]
    pub const fn uses_world_vertex(self) -> bool {
        let ty = self.model_type();
        ty == MARK_MODEL_TYPE_WORLD_BRUSH || ty == MARK_MODEL_TYPE_ENTITY_BRUSH
    }

    #[inline]
    pub const fn to_bytes(self) -> [u8; 6] {
        let m = self.mesh_slot_number.to_le_bytes();
        [
            self.baked_light_slot_number,
            self.key_light_index,
            self.env_probe_index,
            self.model_type_and_surf,
            m[0],
            m[1],
        ]
    }

    #[inline]
    pub const fn from_bytes(b: [u8; 6]) -> Self {
        Self {
            baked_light_slot_number: b[0],
            key_light_index: b[1],
            env_probe_index: b[2],
            model_type_and_surf: b[3],
            mesh_slot_number: u16::from_le_bytes([b[4], b[5]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_bytes_round_trip() {
        let c = GfxMarkContext {
            baked_light_slot_number: 3,
            key_light_index: 7,
            env_probe_index: 9,
            model_type_and_surf: 0x45,
            mesh_slot_number: 0x1234,
        };
        assert_eq!(c.to_bytes(), [3, 7, 9, 0x45, 0x34, 0x12]);
        assert_eq!(GfxMarkContext::from_bytes(c.to_bytes()), c);
    }

    #[test]
    fn model_type_bits() {
        let w = GfxMarkContext::world_surface(1, 2, 3);
        assert_eq!(w.model_type(), MARK_MODEL_TYPE_WORLD_BRUSH);
        assert!(!w.is_entity() && !w.is_viewmodel());
        let s = GfxMarkContext { model_type_and_surf: 0x41, ..w };
        assert_eq!(s.model_type(), MARK_MODEL_TYPE_STATIC_MODEL);
        let e = GfxMarkContext { model_type_and_surf: 0xc2, ..w };
        assert_eq!(e.model_type(), MARK_MODEL_TYPE_ENTITY_MODEL);
        assert!(e.is_entity() && !e.is_viewmodel());
        assert_eq!(e.dobj_model_index(), Some(2));
        let b = GfxMarkContext { model_type_and_surf: 0x81, ..w };
        assert_eq!(b.model_type(), MARK_MODEL_TYPE_ENTITY_BRUSH);
        assert!(b.is_entity() && !b.is_viewmodel());
        assert_eq!(b.dobj_model_index(), None);
        let v = GfxMarkContext { model_type_and_surf: 0xe3, ..w };
        assert_eq!(v.model_type(), MARK_MODEL_TYPE_ENTITY_MODEL);
        assert!(v.is_entity() && v.is_viewmodel());
        assert_eq!(v.dobj_model_index(), Some(3));
    }

    #[test]
    fn world_list_and_vertex_kind() {
        let w = GfxMarkContext::world_surface(1, 2, 3);
        assert!(w.is_world_list() && w.uses_world_vertex());
        let s = GfxMarkContext { model_type_and_surf: MARK_MODEL_TYPE_STATIC_MODEL, ..w };
        assert!(s.is_world_list() && !s.uses_world_vertex());
        let e = GfxMarkContext::entity_model(0x123, 0);
        assert_eq!(e.model_type_and_surf, 0xc0);
        assert!(!e.is_world_list() && !e.uses_world_vertex() && !e.is_viewmodel());
        let e5 = GfxMarkContext::entity_model(0x123, 5);
        assert_eq!(e5.model_type_and_surf, 0xc5);
        assert_eq!(e5.dobj_model_index(), Some(5));
        assert!(!e5.is_viewmodel() && e5.model_type() == MARK_MODEL_TYPE_ENTITY_MODEL);
        assert!(!crate::mark_contexts_equal(&e, &e5));
        assert_eq!((e.mesh_slot_number, e.baked_light_slot_number), (0x123, crate::SURFACE_BAKED_LIGHT_ABSENT));
        let b = GfxMarkContext { model_type_and_surf: MARK_MODEL_TYPE_ENTITY_BRUSH, ..w };
        assert!(!b.is_world_list() && b.uses_world_vertex());
        let v = GfxMarkContext {
            model_type_and_surf: MARK_MODEL_TYPE_ENTITY_MODEL | MARK_MODEL_VIEWMODEL_BIT,
            ..w
        };
        assert!(!v.is_world_list() && !v.uses_world_vertex());
    }
}
