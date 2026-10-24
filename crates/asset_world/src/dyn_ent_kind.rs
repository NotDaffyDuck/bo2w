//! What kinds of dynamic entity a map can hold, and what each can do.

/// The kind a map gives a dynamic entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynEntType {
    /// Not a real entity; a map row nobody filled in.
    Invalid,
    /// A loose prop that physics can knock about.
    Clutter,
    /// A prop that physics can knock about and weapons can break.
    Destruct,
}

/// What a kind of dynamic entity is allowed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynEntProps {
    pub use_physics: bool,
    pub destroyable: bool,
}

/// What `ty` can do. Nothing for an invalid kind.
pub fn dyn_ent_props(ty: DynEntType) -> Option<DynEntProps> {
    let (use_physics, destroyable) = match ty {
        DynEntType::Invalid => return None,
        DynEntType::Clutter => (true, false),
        DynEntType::Destruct => (true, true),
    };
    Some(DynEntProps {
        use_physics,
        destroyable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clutter_moves_but_does_not_break() {
        let props = dyn_ent_props(DynEntType::Clutter).expect("clutter");
        assert!(props.use_physics && !props.destroyable);
    }

    #[test]
    fn a_destructible_moves_and_breaks() {
        let props = dyn_ent_props(DynEntType::Destruct).expect("destruct");
        assert!(props.use_physics && props.destroyable);
    }

    #[test]
    fn an_invalid_kind_can_do_nothing() {
        assert_eq!(dyn_ent_props(DynEntType::Invalid), None);
    }
}
