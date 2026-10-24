//! The effects a weapon fires: muzzle flash, brass, explosion and tracer.
//!
//! Each effect is a name the capture read (`*_hint`) and the edge that says
//! whether the effect table held it.

use asset_core::{AssetEdge, FxSpace, TracerSpace};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponCombatFx {
    pub view_flash: AssetEdge<FxSpace>,
    pub view_flash_hint: Option<String>,
    pub world_flash: AssetEdge<FxSpace>,
    pub world_flash_hint: Option<String>,
    pub view_shell_eject: AssetEdge<FxSpace>,
    pub view_shell_eject_hint: Option<String>,
    pub world_shell_eject: AssetEdge<FxSpace>,
    pub world_shell_eject_hint: Option<String>,
    pub explosion: AssetEdge<FxSpace>,
    pub explosion_hint: Option<String>,
    pub tracer: AssetEdge<TracerSpace>,
    pub tracer_hint: Option<String>,
    /// The namespace the names above are looked up in.
    pub namespace: crate::AssetNamespace,
}

impl WeaponCombatFx {
    /// The effect's name, when its edge is bound and a name was read.
    fn bound_name<'a>(
        &self,
        edge: AssetEdge<FxSpace>,
        hint: &'a Option<String>,
    ) -> Option<crate::FxName<'a>> {
        if !edge.is_bound() {
            return None;
        }
        let name = hint.as_deref().filter(|name| !name.is_empty())?;
        Some(crate::FxName::new(self.namespace, name))
    }

    pub fn view_flash_present(&self) -> Option<crate::FxName<'_>> {
        self.bound_name(self.view_flash, &self.view_flash_hint)
    }

    pub fn world_flash_present(&self) -> Option<crate::FxName<'_>> {
        self.bound_name(self.world_flash, &self.world_flash_hint)
    }

    pub fn flash_present(&self, player_view: bool) -> Option<crate::FxName<'_>> {
        if player_view {
            self.view_flash_present()
        } else {
            self.world_flash_present()
        }
    }

    pub fn flash_edge(&self, player_view: bool) -> AssetEdge<FxSpace> {
        if player_view { self.view_flash } else { self.world_flash }
    }

    pub fn view_shell_eject_present(&self) -> Option<crate::FxName<'_>> {
        self.bound_name(self.view_shell_eject, &self.view_shell_eject_hint)
    }

    pub fn world_shell_eject_present(&self) -> Option<crate::FxName<'_>> {
        self.bound_name(self.world_shell_eject, &self.world_shell_eject_hint)
    }

    pub fn brass_present(&self, player_view: bool) -> Option<crate::FxName<'_>> {
        if player_view {
            self.view_shell_eject_present()
        } else {
            self.world_shell_eject_present()
        }
    }

    pub fn brass_edge(&self, player_view: bool) -> AssetEdge<FxSpace> {
        if player_view { self.view_shell_eject } else { self.world_shell_eject }
    }

    /// Black Ops II has a separate last-round brass effect, but nothing reads
    /// it yet, so no weapon ever has one.
    pub fn last_shot_eject_pair_authored(&self) -> bool {
        false
    }

    pub fn brass_present_for_event(
        &self,
        player_view: bool,
        _last_shot: bool,
    ) -> Option<crate::FxName<'_>> {
        self.brass_present(player_view)
    }

    pub fn brass_edge_for_event(&self, player_view: bool, _last_shot: bool) -> AssetEdge<FxSpace> {
        self.brass_edge(player_view)
    }

    pub fn explosion_present(&self) -> Option<crate::FxName<'_>> {
        self.bound_name(self.explosion, &self.explosion_hint)
    }

    /// Fill every effect this weapon leaves unauthored from `src`.
    pub(super) fn fill_gaps_from(&mut self, src: &Self) {
        fn fill(
            edge: &mut AssetEdge<FxSpace>,
            hint: &mut Option<String>,
            src_edge: AssetEdge<FxSpace>,
            src_hint: &Option<String>,
        ) {
            if edge.is_absent() {
                *edge = src_edge;
                if hint.is_none() {
                    hint.clone_from(src_hint);
                }
            }
        }
        fill(&mut self.view_flash, &mut self.view_flash_hint, src.view_flash, &src.view_flash_hint);
        fill(&mut self.world_flash, &mut self.world_flash_hint, src.world_flash, &src.world_flash_hint);
        fill(
            &mut self.view_shell_eject,
            &mut self.view_shell_eject_hint,
            src.view_shell_eject,
            &src.view_shell_eject_hint,
        );
        fill(
            &mut self.world_shell_eject,
            &mut self.world_shell_eject_hint,
            src.world_shell_eject,
            &src.world_shell_eject_hint,
        );
        fill(&mut self.explosion, &mut self.explosion_hint, src.explosion, &src.explosion_hint);
        if self.tracer.is_absent() {
            self.tracer = src.tracer;
            if self.tracer_hint.is_none() {
                self.tracer_hint.clone_from(&src.tracer_hint);
            }
        }
    }
}
