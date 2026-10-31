//! What kinds of dynamic entity a map can hold, and what each can do.

/// The kind a map gives a dynamic entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovableObjectKind {
    /// Not a real entity; a map row nobody filled in.
    Invalid,
    /// A loose prop that physics can knock about.
    Clutter,
    /// A prop that physics can knock about and weapons can break.
    Destruct,
}

/// What a kind of dynamic entity is allowed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynEntAbilities {
    pub is_simulated: bool,
    pub can_be_wrecked: bool,
}

/// What `ty` can do. Nothing for an invalid kind.
pub fn dyn_ent_props(ty: MovableObjectKind) -> Option<DynEntAbilities> {
    let (use_physics, can_be_wrecked) = match ty {
        MovableObjectKind::Invalid => return None,
        MovableObjectKind::Clutter => (true, false),
        MovableObjectKind::Destruct => (true, true),
    };
    Some(DynEntAbilities {
        is_simulated: use_physics,
        can_be_wrecked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clutter_moves_but_does_not_break() {
        let props = dyn_ent_props(MovableObjectKind::Clutter).expect("clutter");
        assert!(props.is_simulated && !props.can_be_wrecked);
    }

    #[test]
    fn a_destructible_moves_and_breaks() {
        let props = dyn_ent_props(MovableObjectKind::Destruct).expect("destruct");
        assert!(props.is_simulated && props.can_be_wrecked);
    }

    #[test]
    fn an_invalid_kind_can_do_nothing() {
        assert_eq!(dyn_ent_props(MovableObjectKind::Invalid), None);
    }
}
