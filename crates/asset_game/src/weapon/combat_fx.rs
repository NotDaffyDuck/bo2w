//! The effects a weapon fires: muzzle flash, brass, explosion and tracer.
//!
//! Each effect is a name the capture read (`*_hint`) and the edge that says
//! whether the effect table held it.

use asset_core::{AssetDependencyLink, FxRange, TracerRange};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponBattleEffects {
    pub view_flash: AssetDependencyLink<FxRange>,
    pub own_muzzle_name: Option<String>,
    pub world_flash: AssetDependencyLink<FxRange>,
    pub far_muzzle_name: Option<String>,
    pub view_shell_eject: AssetDependencyLink<FxRange>,
    pub own_casing_name: Option<String>,
    pub world_shell_eject: AssetDependencyLink<FxRange>,
    pub far_casing_name: Option<String>,
    pub explosion: AssetDependencyLink<FxRange>,
    pub blast_clue: Option<String>,
    pub tracer: AssetDependencyLink<TracerRange>,
    pub streak_name: Option<String>,
    /// The namespace the names above are looked up in.
    pub namespace: crate::AssetCategoryScope,
}

/// The five effects that come out of the fx table, in the order they are
/// listed when gaps are filled.
#[derive(Clone, Copy)]
enum FxSlot {
    ViewFlash,
    WorldFlash,
    ViewBrass,
    WorldBrass,
    Blast,
}

impl FxSlot {
    const ALL: [FxSlot; 5] = [
        FxSlot::ViewFlash,
        FxSlot::WorldFlash,
        FxSlot::ViewBrass,
        FxSlot::WorldBrass,
        FxSlot::Blast,
    ];

    /// The muzzle flash seen from the player's own view or from outside.
    fn flash(player_view: bool) -> FxSlot {
        [FxSlot::WorldFlash, FxSlot::ViewFlash][usize::from(player_view)]
    }

    /// The ejected brass seen from the player's own view or from outside.
    fn brass(player_view: bool) -> FxSlot {
        [FxSlot::WorldBrass, FxSlot::ViewBrass][usize::from(player_view)]
    }
}

impl WeaponBattleEffects {
    fn bay(&self, which: FxSlot) -> (AssetDependencyLink<FxRange>, Option<&str>) {
        let (edge, text) = match which {
            FxSlot::ViewFlash => (self.view_flash, &self.own_muzzle_name),
            FxSlot::WorldFlash => (self.world_flash, &self.far_muzzle_name),
            FxSlot::ViewBrass => (self.view_shell_eject, &self.own_casing_name),
            FxSlot::WorldBrass => (self.world_shell_eject, &self.far_casing_name),
            FxSlot::Blast => (self.explosion, &self.blast_clue),
        };
        (edge, text.as_deref())
    }

    fn slot_pair_mut(&mut self, which: FxSlot) -> (&mut AssetDependencyLink<FxRange>, &mut Option<String>) {
        match which {
            FxSlot::ViewFlash => (&mut self.view_flash, &mut self.own_muzzle_name),
            FxSlot::WorldFlash => (&mut self.world_flash, &mut self.far_muzzle_name),
            FxSlot::ViewBrass => (&mut self.view_shell_eject, &mut self.own_casing_name),
            FxSlot::WorldBrass => (
                &mut self.world_shell_eject,
                &mut self.far_casing_name,
            ),
            FxSlot::Blast => (&mut self.explosion, &mut self.blast_clue),
        }
    }

    /// The slot's effect name, when its edge is bound and a name was read.
    fn present(&self, which: FxSlot) -> Option<crate::FxName<'_>> {
        let (edge, text) = self.bay(which);
        let name = crate::FxName::new(self.namespace, text.filter(|text| !text.is_empty())?);
        Some(name).filter(|_| edge.is_attached())
    }

    /// The muzzle flash seen from the player's own view or from outside.
    pub fn has_muzzle_effect(&self, player_view: bool) -> Option<crate::FxName<'_>> {
        self.present(FxSlot::flash(player_view))
    }

    /// The brass a shot throws. Black Ops II has a separate last-round brass
    /// effect, but nothing reads it yet, so every shot throws the same.
    pub fn brass_present_for_event(
        &self,
        own_view: bool,
        _last_shot: bool,
    ) -> Option<crate::FxName<'_>> {
        self.present(FxSlot::brass(own_view))
    }

    pub fn explosion_present(&self) -> Option<crate::FxName<'_>> {
        self.present(FxSlot::Blast)
    }

    /// Fill every effect this weapon leaves unauthored from `donor`.
    pub(super) fn fill_gaps_from(&mut self, donor: &Self) {
        let gaps: Vec<FxSlot> = FxSlot::ALL
            .into_iter()
            .filter(|&which| self.bay(which).0.lacks_entry())
            .collect();
        for which in gaps {
            let (donor_edge, donor_text) = donor.bay(which);
            let (edge, text) = self.slot_pair_mut(which);
            *edge = donor_edge;
            keep_or_borrow(text, donor_text);
        }
        if self.tracer.lacks_entry() {
            self.tracer = donor.tracer;
            keep_or_borrow(&mut self.streak_name, donor.streak_name.as_deref());
        }
    }
}

/// Leaves a name that was read alone, and takes the donor's when there is none.
fn keep_or_borrow(words: &mut Option<String>, donor: Option<&str>) {
    *words = words.take().or_else(|| donor.map(str::to_owned));
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_core::ZoneCustodian;

    fn bound() -> AssetDependencyLink<FxRange> {
        AssetDependencyLink::bind_order(3, ZoneCustodian::default())
    }

    fn sample() -> WeaponBattleEffects {
        WeaponBattleEffects {
            view_flash: bound(),
            own_muzzle_name: Some("flash_view".into()),
            world_flash: bound(),
            far_muzzle_name: Some("flash_world".into()),
            view_shell_eject: bound(),
            own_casing_name: Some("brass_view".into()),
            world_shell_eject: AssetDependencyLink::NothingThere,
            far_casing_name: Some("brass_world".into()),
            ..WeaponBattleEffects::default()
        }
    }

    #[test]
    fn the_view_or_world_copy_is_picked_by_player_view() {
        let fx = sample();
        let named = |found: Option<crate::FxName<'_>>| found.map(|n| n.name.to_owned());
        assert_eq!(named(fx.has_muzzle_effect(true)), Some("flash_view".to_owned()));
        assert_eq!(named(fx.has_muzzle_effect(false)), Some("flash_world".to_owned()));
        assert_eq!(named(fx.brass_present_for_event(true, false)), Some("brass_view".to_owned()));
        // The world brass has a name but its edge is absent.
        assert!(fx.brass_present_for_event(false, true).is_none());
        assert!(fx.brass_present_for_event(false, false).is_none());
    }

    #[test]
    fn an_empty_name_is_not_present_even_when_bound() {
        let mut fx = sample();
        fx.own_muzzle_name = Some(String::new());
        assert!(fx.has_muzzle_effect(true).is_none());
        fx.own_muzzle_name = None;
        assert!(fx.has_muzzle_effect(true).is_none());
        assert!(fx.explosion_present().is_none());
    }

    #[test]
    fn gaps_take_the_donor_edge_and_keep_a_name_already_read() {
        let mut gun = sample();
        gun.blast_clue = Some("mine".into());
        let mut donor = sample();
        donor.explosion = bound();
        donor.blast_clue = Some("theirs".into());
        donor.world_shell_eject = bound();
        donor.far_casing_name = Some("donor_brass".into());
        donor.tracer = AssetDependencyLink::bind_order(1, ZoneCustodian::default());
        donor.streak_name = Some("tracer_d".into());
        gun.fill_gaps_from(&donor);
        assert!(gun.explosion.is_attached());
        assert_eq!(gun.blast_clue.as_deref(), Some("mine"));
        assert!(gun.world_shell_eject.is_attached());
        assert_eq!(gun.far_casing_name.as_deref(), Some("brass_world"));
        assert!(gun.tracer.is_attached());
        assert_eq!(gun.streak_name.as_deref(), Some("tracer_d"));
        assert_eq!(gun.own_muzzle_name.as_deref(), Some("flash_view"));
    }
}
