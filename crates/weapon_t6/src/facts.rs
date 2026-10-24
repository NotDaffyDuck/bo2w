//! Per-weapon facts the brain reads, pulled from the BO2 fastfile weapon assets.
//!
//! Each mechanic window owns one sub-struct here (its fields and its reader); window 1 owns
//! the composition. Values are read little-endian at `fastfile_t6::layout` offsets, from the
//! weapon's variant bytes and its WeaponDef bytes.

use fastfile_t6::layout::{WeaponDef as D, WeaponVariantDef as V};

/// BO2 `weapType_t`.
pub mod weap_type {
    pub const BULLET: u8 = 0;
    pub const GRENADE: u8 = 1;
    pub const PROJECTILE: u8 = 2;
    pub const BINOCULARS: u8 = 3;
    pub const GAS: u8 = 4;
    pub const BOMB: u8 = 5;
    pub const MINE: u8 = 6;
    pub const MELEE: u8 = 7;
    pub const RIOTSHIELD: u8 = 8;
}

/// BO2 `weapClass_t`.
pub mod weap_class {
    pub const RIFLE: u8 = 0;
    pub const MG: u8 = 1;
    pub const SMG: u8 = 2;
    pub const SPREAD: u8 = 3;
    pub const PISTOL: u8 = 4;
    pub const GRENADE: u8 = 5;
    pub const ROCKETLAUNCHER: u8 = 6;
    pub const TURRET: u8 = 7;
    pub const NON_PLAYER: u8 = 8;
    pub const GAS: u8 = 9;
    pub const ITEM: u8 = 10;
    pub const MELEE: u8 = 11;
    pub const KILLSTREAK_ALT_STORED_WEAPON: u8 = 12;
    pub const PISTOL_SPREAD: u8 = 13;
}

/// BO2 `weapInventoryType_t`.
pub mod inventory {
    pub const PRIMARY: u8 = 0;
    pub const OFFHAND: u8 = 1;
    pub const ITEM: u8 = 2;
    pub const ALTMODE: u8 = 3;
    pub const MELEE: u8 = 4;
    pub const DWLEFTHAND: u8 = 5;
}

/// BO2 `weapFireType_t`.
pub mod fire_type {
    pub const FULLAUTO: u8 = 0;
    pub const SINGLESHOT: u8 = 1;
    pub const BURSTFIRE2: u8 = 2;
    pub const BURSTFIRE3: u8 = 3;
    pub const BURSTFIRE4: u8 = 4;
    pub const BURSTFIRE5: u8 = 5;
    pub const STACKED: u8 = 6;
    pub const MINIGUN: u8 = 7;
    pub const CHARGESHOT: u8 = 8;
    pub const JETGUN: u8 = 9;

    /// BG_WeaponUsesBurstFire: the burst types and the charge shot.
    pub fn uses_burst(t: u8) -> bool {
        (BURSTFIRE2..=BURSTFIRE5).contains(&t) || t == CHARGESHOT
    }
}

/// BO2 `OffhandClass`.
pub mod offhand_class {
    pub const NONE: u8 = 0;
    pub const FRAG_GRENADE: u8 = 1;
    pub const SMOKE_GRENADE: u8 = 2;
    pub const FLASH_GRENADE: u8 = 3;
    pub const GEAR: u8 = 4;
    pub const SUPPLYDROP_MARKER: u8 = 5;
}

/// BO2 `OffhandSlot`.
pub mod offhand_slot {
    pub const NONE: u8 = 0;
    pub const LETHAL_GRENADE: u8 = 1;
    pub const TACTICAL_GRENADE: u8 = 2;
    pub const EQUIPMENT: u8 = 3;
    pub const SPECIFIC_USE: u8 = 4;
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    rd_i32(b, at) as u32
}

fn rd_u8(b: &[u8], at: usize) -> u8 {
    b.get(at).copied().unwrap_or(0)
}

fn rd_bool(b: &[u8], at: usize) -> bool {
    rd_u8(b, at) != 0
}

/// Window 1: what every mechanic needs to know about a weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreFacts {
    pub weap_type: u8,
    pub weap_class: u8,
    pub inventory_type: u8,
    pub fire_type: u8,
    pub offhand_class: u8,
    pub offhand_slot: u8,
    pub unlimited_ammo: bool,
    pub no_drops_or_raises: bool,
    pub can_use_in_vehicle: bool,
    pub bolt_action: bool,
    pub dual_wield: bool,
    pub alt_weapon_ads_only: bool,
    /// Variant `altWeaponIndex` (0 = none).
    pub alt_weapon: u32,
    /// Def `dualWieldWeaponIndex`: the left-hand gun (0 = none).
    pub dual_wield_weapon: u32,
    pub clip_size: i32,
    pub burst_delay_time: i32,
    /// `bClipOnly`: the clip is the whole weapon (no stock).
    pub clip_only: bool,
    pub has_detonator: bool,
    pub plantable: bool,
    pub hold_button_to_throw: bool,
    /// `bAltWeaponDisableSwitching`.
    pub alt_weapon_disable_switching: bool,
    /// `guidedMissileType_t` (0 = none, 3 = javelin, 6 = TV guided, 7 = drone ...).
    pub guided_missile_type: i32,
    /// Filled by the loader (follows pointers): bit N set = variant anim slot N (0-87) names an
    /// anim. `read` leaves it 0.
    pub anim_slots: u128,
    /// Filled by the loader: the def has a gun model (`gunXModel[0]` set).
    pub has_gun_model: bool,
    /// Variant `bIgnoreAttachments`: Fast Mag and the other attachment flags pass this gun by.
    pub ignore_attachments: bool,
    /// Filled by the table: the weapon at the start of the alternate chain that reaches this
    /// charge-shot weapon (BO2's load stores it as a negative `iVariantCount`), 0 for none.
    /// Origins' upgraded staffs chain their charge levels this way.
    pub charge_root: u32,
    /// Def `bIsCarriedKillstreakWeapon` (a minigun or launcher a streak hands him): BO2's count
    /// of his primaries passes it by.
    pub carried_killstreak: bool,
    /// Def `bBulletImpactExplode`: the bullet blows up where it lands (a riot shield never
    /// turns one aside).
    pub bullet_impact_explode: bool,
    /// Def `armorPiercing`: Bullet_Process marks its hits no-armor (BO2's iDFlags 2).
    pub armor_piercing: bool,
}

impl CoreFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            weap_type: rd_u8(def, D::weapType),
            weap_class: rd_u8(def, D::weapClass),
            inventory_type: rd_u8(def, D::inventoryType),
            fire_type: rd_u8(def, D::fireType),
            offhand_class: rd_u8(def, D::offhandClass),
            offhand_slot: rd_u8(def, D::offhandSlot),
            unlimited_ammo: rd_bool(def, D::unlimitedAmmo),
            no_drops_or_raises: rd_bool(def, D::bNoDropsOrRaises),
            can_use_in_vehicle: rd_bool(def, D::bCanUseInVehicle),
            bolt_action: rd_bool(def, D::bBoltAction),
            dual_wield: rd_bool(def, D::bDualWield),
            alt_weapon_ads_only: rd_bool(def, D::bAltWeaponAdsOnly),
            alt_weapon: rd_u32(variant, V::altWeaponIndex),
            dual_wield_weapon: rd_u32(def, D::dualWieldWeaponIndex),
            clip_size: rd_i32(variant, V::iClipSize),
            burst_delay_time: rd_i32(def, D::iBurstDelayTime),
            clip_only: rd_bool(def, D::bClipOnly),
            has_detonator: rd_bool(def, D::hasDetonator),
            plantable: rd_bool(def, D::plantable),
            hold_button_to_throw: rd_bool(def, D::holdButtonToThrow),
            alt_weapon_disable_switching: rd_bool(def, D::bAltWeaponDisableSwitching),
            guided_missile_type: rd_i32(def, D::guidedMissileType),
            anim_slots: 0,
            has_gun_model: false,
            ignore_attachments: rd_bool(variant, V::bIgnoreAttachments),
            charge_root: 0,
            carried_killstreak: rd_bool(def, D::bIsCarriedKillstreakWeapon),
            bullet_impact_explode: rd_bool(def, D::bBulletImpactExplode),
            armor_piercing: rd_bool(def, D::armorPiercing),
        }
    }
}

/// A rocket's flight: its speed and how it wobbles (BO2's RunMissile_Destabilize).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MissileFacts {
    /// `iProjectileSpeed`: units a second.
    pub projectile_speed: i32,
    /// `destabilizationRateTime`: seconds between turns; 0 = it flies straight.
    pub destabilize_rate_time: f32,
    /// `destabilizationCurvatureMax`: the most it turns, degrees a second on each axis.
    pub destabilize_curvature_max: f32,
    /// `destabilizeDistance`: how far it flies before its first turn.
    pub destabilize_distance: i32,
}

impl MissileFacts {
    pub fn read(_variant: &[u8], def: &[u8]) -> Self {
        Self {
            projectile_speed: rd_i32(def, D::iProjectileSpeed),
            destabilize_rate_time: f32::from_bits(rd_u32(def, D::destabilizationRateTime)),
            destabilize_curvature_max: f32::from_bits(rd_u32(def, D::destabilizationCurvatureMax)),
            destabilize_distance: rd_i32(def, D::destabilizeDistance),
        }
    }
}

/// A turret's turn limits (BO2's G_SpawnTurret reads them into its arcs):
/// degrees each way from the facing it was set down with. The Sentry Gun's
/// are 65 left and right, 45 up, 20 down; the Guardian's 0 left and right.
/// With no one to shoot it sweeps across them: up to `scan_speed` degrees a
/// second, gaining `scan_accel` a second each second, waiting
/// `scan_pause_ms` at each end. A vehicle's gun fires from its barrels by
/// `barrel_type` (`barrels`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurretFacts {
    pub left_arc: f32,
    pub right_arc: f32,
    pub top_arc: f32,
    pub bottom_arc: f32,
    pub scan_speed: f32,
    pub scan_accel: f32,
    pub scan_pause_ms: i32,
    /// BO2's `barrelType`: 0 single, 1 dual, 2 dual alternating, 3 quad,
    /// 4 quad alternating, 5 quad double alternating.
    pub barrel_type: u32,
}

impl TurretFacts {
    pub fn read(_variant: &[u8], def: &[u8]) -> Self {
        let f = |at| f32::from_bits(rd_u32(def, at));
        Self {
            left_arc: f(D::leftArc),
            right_arc: f(D::rightArc),
            top_arc: f(D::topArc),
            bottom_arc: f(D::bottomArc),
            scan_speed: f(D::scanSpeed),
            scan_accel: f(D::scanAccel),
            scan_pause_ms: rd_i32(def, D::scanPauseTime),
            barrel_type: rd_u32(def, D::barrelType),
        }
    }

    /// `BG_GetWeaponFireBarrelInfo`: the barrel a vehicle gun's next shot
    /// starts at and how many fire at once, after barrel `last` fired: a
    /// dual or quad gun fires all of them, an alternating one goes round.
    #[must_use]
    pub fn barrels(&self, last: usize) -> (usize, usize) {
        match self.barrel_type {
            1 => (0, 2),
            2 => ((last + 1) & 1, 1),
            3 => (0, 4),
            4 => ((last + 1) & 3, 1),
            5 => ((last + 1) & 3, 2),
            _ => (0, 1),
        }
    }
}

/// One of a weapon's camera shakes: how hard, for how long, and out to how
/// far from where it starts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraShake {
    pub scale: f32,
    pub duration_ms: i32,
    pub radius: i32,
}

impl CameraShake {
    /// BO2 starts one only when it has both a length and a radius.
    pub fn is_set(&self) -> bool {
        self.duration_ms != 0 && self.radius != 0
    }
}

/// A weapon's camera shakes (BO2's client weapon events): `fire` from the
/// shooter each time it fires (CG_FireWeapon), `explosion` from where its
/// projectile goes off (CG_ProjectileExplosionEvent). Most explosives' is 0.5
/// for half a second out to 512 (the frag's, semtex's, sensor's, bolt's and
/// Lightning Strike's to 800); the scorestreaks' guns fire with 0.05 for a
/// tenth of a second out to 100. Players' guns have none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShakeFacts {
    pub fire: CameraShake,
    pub explosion: CameraShake,
}

impl ShakeFacts {
    pub fn read(_variant: &[u8], def: &[u8]) -> Self {
        let f = |at| f32::from_bits(rd_u32(def, at));
        Self {
            fire: CameraShake {
                scale: f(D::cameraShakeScale),
                duration_ms: rd_i32(def, D::cameraShakeDuration),
                radius: rd_i32(def, D::cameraShakeRadius),
            },
            explosion: CameraShake {
                scale: f(D::explosionCameraShakeScale),
                duration_ms: rd_i32(def, D::explosionCameraShakeDuration),
                radius: rd_i32(def, D::explosionCameraShakeRadius),
            },
        }
    }
}

/// Window 2 (switching): every raise/drop time.
///
/// `first_raise`, `alt_drop` and `alt_raise` hold the value after BO2's attachment-unique
/// override: the reader stores the gun's own, and a `<gun>+<att>` row gets its attachments'
/// (see [`FactsTable::from_dump`], `#attu` lines).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchFacts {
    pub drop: i32,
    pub raise: i32,
    pub alt_drop: i32,
    pub alt_raise: i32,
    pub quick_drop: i32,
    pub quick_raise: i32,
    pub first_raise: i32,
    pub empty_raise: i32,
    pub empty_drop: i32,
    pub no_quick_drop_when_empty: bool,
    pub cancel_auto_holster_when_empty: bool,
}

impl SwitchFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            drop: rd_i32(def, D::iDropTime),
            raise: rd_i32(def, D::iRaiseTime),
            alt_drop: rd_i32(def, D::iAltDropTime),
            alt_raise: rd_i32(variant, V::iAltRaiseTime),
            quick_drop: rd_i32(def, D::quickDropTime),
            quick_raise: rd_i32(def, D::quickRaiseTime),
            first_raise: rd_i32(def, D::iFirstRaiseTime),
            empty_raise: rd_i32(def, D::iEmptyRaiseTime),
            empty_drop: rd_i32(def, D::iEmptyDropTime),
            no_quick_drop_when_empty: rd_bool(def, D::bNoQuickDropWhenEmpty),
            cancel_auto_holster_when_empty: rd_bool(def, D::cancelAutoHolsterWhenEmpty),
        }
    }
}

/// A weapon's attachment time scales: BO2 multiplies every attachment's scale (up to 3) at each
/// read (`BG_GetFireTime`, `BG_GetADSTransTimeScales`, `BG_GetReloadTime` and the other reload
/// getters); 1 with no attachments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttachmentScales {
    /// Product of `fFireTimeScale`.
    pub fire_time: f32,
    /// Product of `fAdsTransInTimeScale`.
    pub ads_in: f32,
    /// Product of `fAdsTransOutTimeScale`.
    pub ads_out: f32,
    /// Products of `fReloadTimeScale`, `fReloadEmptyTimeScale`, `fReloadAddTimeScale`,
    /// `fReloadQuickTimeScale`, `fReloadQuickEmptyTimeScale`, `fReloadQuickAddTimeScale`.
    pub reload: f32,
    pub reload_empty: f32,
    pub reload_add: f32,
    pub reload_quick: f32,
    pub reload_quick_empty: f32,
    pub reload_quick_add: f32,
}

impl Default for AttachmentScales {
    fn default() -> Self {
        Self {
            fire_time: 1.0,
            ads_in: 1.0,
            ads_out: 1.0,
            reload: 1.0,
            reload_empty: 1.0,
            reload_add: 1.0,
            reload_quick: 1.0,
            reload_quick_empty: 1.0,
            reload_quick_add: 1.0,
        }
    }
}

impl AttachmentScales {
    pub fn fold(atts: &[&[u8]]) -> Self {
        use fastfile_t6::layout::WeaponAttachment as A;
        let rd = |a: &[u8], at: usize| f32::from_le_bytes(a[at..at + 4].try_into().unwrap());
        let mut s = Self::default();
        for a in &atts[..atts.len().min(3)] {
            s.fire_time *= rd(a, A::fFireTimeScale);
            s.ads_in *= rd(a, A::fAdsTransInTimeScale);
            s.ads_out *= rd(a, A::fAdsTransOutTimeScale);
            s.reload *= rd(a, A::fReloadTimeScale);
            s.reload_empty *= rd(a, A::fReloadEmptyTimeScale);
            s.reload_add *= rd(a, A::fReloadAddTimeScale);
            s.reload_quick *= rd(a, A::fReloadQuickTimeScale);
            s.reload_quick_empty *= rd(a, A::fReloadQuickEmptyTimeScale);
            s.reload_quick_add *= rd(a, A::fReloadQuickAddTimeScale);
        }
        s
    }
}

/// `ATTACHMENT_STACKFIRE` (`eAttachment`): Tri-Bolt, which sets the clip and fire type outright.
pub const ATTACHMENT_STACKFIRE: i32 = 24;

/// `player_clipSizeMultiplier` (BO2 default).
pub const PLAYER_CLIP_SIZE_MULTIPLIER: f32 = 1.0;

/// BG_GetClipSize: a stack-fire attachment's own clip; otherwise the gun's clip times the
/// dvar and every attachment's `clipSizeScale`, rounded, at least 1 (a one-round clip stays 1).
pub fn attachment_clip_size(clip: i32, atts: &[&[u8]]) -> i32 {
    use fastfile_t6::layout::WeaponAttachment as A;
    let rd_i = |a: &[u8], at: usize| i32::from_le_bytes(a[at..at + 4].try_into().unwrap());
    let rd_f = |a: &[u8], at: usize| f32::from_le_bytes(a[at..at + 4].try_into().unwrap());
    let mut scale = PLAYER_CLIP_SIZE_MULTIPLIER;
    for a in &atts[..atts.len().min(3)] {
        if rd_i(a, A::attachmentType) == ATTACHMENT_STACKFIRE {
            return rd_i(a, A::iClipSize);
        }
        // BO2 asserts the scale is above 0; a blank one leaves the clip alone here.
        let s = rd_f(a, A::clipSizeScale);
        if s > 0.0 {
            scale *= s;
        }
    }
    if clip > 1 {
        ((clip as f32 * scale + 9.313_226e-10).round_ties_even() as i32).max(1)
    } else {
        clip
    }
}

/// BG_GetFireType: a stack-fire attachment's fire type, else the gun's.
pub fn attachment_fire_type(fire_type: u8, atts: &[&[u8]]) -> u8 {
    use fastfile_t6::layout::WeaponAttachment as A;
    let rd_i = |a: &[u8], at: usize| i32::from_le_bytes(a[at..at + 4].try_into().unwrap());
    atts[..atts.len().min(3)]
        .iter()
        .find(|a| rd_i(a, A::attachmentType) == ATTACHMENT_STACKFIRE)
        .map_or(fire_type, |a| rd_i(a, A::fireType) as u8)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6WeaponFacts {
    pub core: CoreFacts,
    pub switch: SwitchFacts,
    pub fire: crate::pm::fire::FireFacts,
    pub reload: crate::pm::reload::ReloadFacts,
    pub offhand: crate::pm::offhand::OffhandFacts,
    pub melee: crate::pm::melee::MeleeFacts,
    pub moves: crate::pm::moves::MoveFacts,
    pub ads: crate::pm::ads::AdsFacts,
    pub shield: crate::pm::shield::ShieldFacts,
    pub spread: crate::pm::spread::SpreadFacts,
    pub dual: crate::pm::dual::DualFacts,
    pub att: AttachmentScales,
    pub missile: MissileFacts,
    pub turret: TurretFacts,
    pub shake: ShakeFacts,
}

impl T6WeaponFacts {
    /// The jam window the client's fire key reads (BO2's IN_Attack_Down): `iJamFireTime` for a
    /// single-shot gun (after its attachments), 0 for any other.
    pub fn jam_window(&self) -> i32 {
        if self.core.fire_type == fire_type::SINGLESHOT {
            self.fire.jam_fire_time.max(0)
        } else {
            0
        }
    }
}

impl T6WeaponFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            core: CoreFacts::read(variant, def),
            switch: SwitchFacts::read(variant, def),
            fire: crate::pm::fire::FireFacts::read(variant, def),
            reload: crate::pm::reload::ReloadFacts::read(variant, def),
            offhand: crate::pm::offhand::OffhandFacts::read(variant, def),
            melee: crate::pm::melee::MeleeFacts::read(variant, def),
            moves: crate::pm::moves::MoveFacts::read(variant, def),
            ads: crate::pm::ads::AdsFacts::read(variant, def),
            shield: crate::pm::shield::ShieldFacts::read(variant, def),
            spread: crate::pm::spread::SpreadFacts::read(variant, def),
            dual: crate::pm::dual::DualFacts::read(variant, def),
            att: AttachmentScales::default(),
            missile: MissileFacts::read(variant, def),
            turret: TurretFacts::read(variant, def),
            shake: ShakeFacts::read(variant, def),
        }
    }
}

/// Facts by weapon index. Index 0 (no weapon) and unknown indexes read as all-zero facts.
#[derive(Clone, Debug, Default)]
pub struct FactsTable {
    rows: Vec<T6WeaponFacts>,
    names: Vec<String>,
    /// Attachment bytes by name (`#att` dump lines), folded into `<gun>+<att>` rows.
    attachments: std::collections::HashMap<String, Vec<u8>>,
    /// Attachment-unique bytes by name (`#attu` lines: `au_<gun>_<att>`, `au_<gun>_none` the
    /// gun's base), up to [`UNIQUE_DUMP_LEN`].
    uniques: std::collections::HashMap<String, Vec<u8>>,
}

/// BO2's default attachment (`bg_weaponAttachmentDefault`): an attachment "overrides" a value
/// when it differs from this one.
pub const DEFAULT_ATTACHMENT: &str = "defaultattachment";

/// How much of each attachment unique the dump keeps: through `bSegmentedReload`, the last
/// field the gun brain reads.
pub const UNIQUE_DUMP_LEN: usize = fastfile_t6::layout::WeaponAttachmentUnique::bSegmentedReload + 1;

impl FactsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, weapon: u32, name: &str, facts: T6WeaponFacts) {
        let i = weapon as usize;
        if self.rows.len() <= i {
            self.rows.resize(i + 1, T6WeaponFacts::default());
            self.names.resize(i + 1, String::new());
        }
        self.rows[i] = facts;
        self.names[i] = name.to_owned();
    }

    pub fn get(&self, weapon: u32) -> T6WeaponFacts {
        // Unknown index: no weapon, all-zero facts.
        self.rows
            .get(weapon as usize)
            .copied()
            .unwrap_or(T6WeaponFacts::default())
    }

    pub fn name(&self, weapon: u32) -> &str {
        self.names
            .get(weapon as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn find(&self, name: &str) -> Option<u32> {
        self.names.iter().position(|n| n == name).map(|i| i as u32)
    }

    /// Parse the dump format, one weapon per line; line N (from 1) is weapon index N:
    /// `name<TAB>variant hex<TAB>def hex` then optional `key=value` columns the bytes cannot
    /// carry (pointers and load-time indexes): `slots=HEX` ([`CoreFacts::anim_slots`]),
    /// `gun=1` ([`CoreFacts::has_gun_model`]), `alt=NAME` and `dual=NAME` (the alternate
    /// weapon and the left-hand gun, resolved to this table's indexes). After the weapons,
    /// `#att<TAB>name<TAB>hex` lines hold attachments and `#attu<TAB>name<TAB>hex` lines
    /// attachment uniques.
    pub fn from_dump(text: &str) -> Self {
        let mut t = Self::new();
        let mut links: Vec<(u32, String, String)> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if let Some(rest) = line.strip_prefix("#att\t") {
                if let Some((name, bytes)) = rest
                    .split_once('\t')
                    .and_then(|(n, h)| Some((n, unhex(h)?)))
                {
                    t.attachments.insert(name.to_owned(), bytes);
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("#attu\t") {
                if let Some((name, bytes)) = rest
                    .split_once('\t')
                    .and_then(|(n, h)| Some((n, unhex(h)?)))
                {
                    t.uniques.insert(name.to_owned(), bytes);
                }
                continue;
            }
            let mut parts = line.split('\t');
            let (Some(name), Some(v), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let (Some(v), Some(d)) = (unhex(v), unhex(d)) else {
                continue;
            };
            let index = i as u32 + 1;
            let mut f = T6WeaponFacts::read(&v, &d);
            // Indexes in the bytes are the fastfile's, not ours.
            f.core.alt_weapon = 0;
            f.core.dual_wield_weapon = 0;
            let (mut alt, mut dual) = (String::new(), String::new());
            for col in parts {
                match col.split_once('=') {
                    Some(("slots", x)) => {
                        f.core.anim_slots = u128::from_str_radix(x, 16).unwrap_or(0)
                    }
                    Some(("gun", x)) => f.core.has_gun_model = x == "1",
                    Some(("alt", x)) => alt = x.to_owned(),
                    Some(("dual", x)) => dual = x.to_owned(),
                    _ => {}
                }
            }
            if !alt.is_empty() || !dual.is_empty() {
                links.push((index, alt, dual));
            }
            t.set(index, name, f);
        }
        for (index, alt, dual) in links {
            let alt = t.find(&alt).unwrap_or(0);
            let dual = t.find(&dual).unwrap_or(0);
            let row = &mut t.rows[index as usize];
            row.core.alt_weapon = alt;
            row.core.dual_wield_weapon = dual;
        }
        t.link_charge_roots();
        t
    }

    /// The same facts under the game's weapon numbers: `rows` gives each game index and its
    /// name. A name missing here falls back to its base gun (the part before `+`, the game's
    /// attachment rows). Alternate and left-hand weapon links are renumbered by name.
    pub fn reindexed<'a>(&self, rows: impl IntoIterator<Item = (u32, &'a str)>) -> Self {
        let rows: Vec<(u32, &str)> = rows.into_iter().collect();
        let game_index = |name: &str| rows.iter().find(|(_, n)| *n == name).map(|(i, _)| *i);
        let mut t = Self::new();
        for &(index, name) in &rows {
            let Some(src) = self
                .find(name)
                .or_else(|| name.split_once('+').and_then(|(base, _)| self.find(base)))
            else {
                continue;
            };
            let mut f = self.get(src);
            // BO2 reads a gun's own alt first, then the one its attachment names (a launcher,
            // the Hybrid's other optic: the `<gun>+<attachment>` rows).
            if f.core.alt_weapon == 0
                && let Some((base, atts)) = name.split_once('+')
            {
                f.core.alt_weapon = atts
                    .split('+')
                    .filter_map(|att| self.find(&format!("{base}+{att}")))
                    .map(|i| self.get(i).core.alt_weapon)
                    .find(|&alt| alt != 0)
                    .unwrap_or(0);
            }
            // BO2's alternate and left-hand weapons keep the gun's attachments (only the base
            // index changes): `an94_mp+gl` goes to `gl_an94_mp+gl`, and back to `an94_mp+gl`.
            let atts = name.split_once('+').map(|(_, a)| a);
            let relink = |i: u32| {
                if i == 0 {
                    return 0;
                }
                let other = self.name(i);
                atts.and_then(|a| game_index(&format!("{other}+{a}")))
                    .or_else(|| game_index(other))
                    .unwrap_or(0)
            };
            f.core.alt_weapon = relink(f.core.alt_weapon);
            f.core.dual_wield_weapon = relink(f.core.dual_wield_weapon);
            self.fold_attachments(name, &mut f);
            self.fold_uniques(name, &mut f);
            t.set(index, name, f);
        }
        t.attachments = self.attachments.clone();
        t.uniques = self.uniques.clone();
        t.link_charge_roots();
        t
    }

    /// BO2's load marks each weapon on a gun's alternate chain with the gun it came from
    /// (a negative `iVariantCount`); BG_GetWeaponForAmmoClip / Pool walk it back for a
    /// charge-shot weapon. Fills [`CoreFacts::charge_root`] with the end of that walk. An
    /// alt-mode weapon's own link back to its gun is not part of a chain (BO2 stops there).
    fn link_charge_roots(&mut self) {
        let n = self.rows.len();
        let mut parent = vec![0usize; n];
        for i in 1..n {
            let c = self.rows[i].core;
            let a = c.alt_weapon as usize;
            if c.inventory_type != inventory::ALTMODE && a != 0 && a != i && a < n && parent[a] == 0 {
                parent[a] = i;
            }
        }
        for i in 1..n {
            self.rows[i].core.charge_root = 0;
            if self.rows[i].core.fire_type != fire_type::CHARGESHOT {
                continue;
            }
            let mut w = i;
            for _ in 0..n {
                if parent[w] == 0 {
                    break;
                }
                w = parent[w];
            }
            if w != i && parent[w] == 0 {
                self.rows[i].core.charge_root = w as u32;
            }
        }
    }

    /// BG_GetWeaponForAmmoClip: the weapon whose magazine `weapon` fires from. An alt mode that
    /// keeps the gun's attachments (Select Fire, the Hybrid's other optic) and is not dual wield
    /// uses its gun's; a launcher (`gl_`, which ignores attachments) has its own. A charge-shot
    /// weapon uses the one at the start of its chain.
    pub fn ammo_clip_weapon(&self, weapon: u32) -> u32 {
        let c = self.get(weapon).core;
        let mut w = weapon;
        if c.inventory_type == inventory::ALTMODE
            && !c.ignore_attachments
            && !c.dual_wield
            && c.alt_weapon != 0
        {
            w = c.alt_weapon;
        }
        if c.fire_type == fire_type::CHARGESHOT && c.charge_root != 0 {
            w = c.charge_root;
        }
        w
    }

    /// BG_GetWeaponForAmmoPool: the weapon whose reserve `weapon` draws from. The alt-mode rule
    /// of [`Self::ammo_clip_weapon`] (dual wield or not), then a dual-wield left-hand gun draws
    /// from its right-hand partner's, then the charge-shot chain.
    pub fn ammo_pool_weapon(&self, weapon: u32) -> u32 {
        let c = self.get(weapon).core;
        let mut w = weapon;
        if c.inventory_type == inventory::ALTMODE && !c.ignore_attachments && c.alt_weapon != 0 {
            w = c.alt_weapon;
        }
        if c.dual_wield && c.inventory_type == inventory::DWLEFTHAND && c.dual_wield_weapon != 0 {
            w = c.dual_wield_weapon;
        }
        if c.fire_type == fire_type::CHARGESHOT && c.charge_root != 0 {
            w = c.charge_root;
        }
        w
    }

    /// A `<gun>+<att>[+<att>...]` row's spread, sway, idle and perk values with its attachments
    /// folded in (BO2's getters read the variant's attachments; without the default
    /// attachment asset they compare against an all-zero one, as BO2's no-fastfile loader).
    fn fold_attachments(&self, name: &str, f: &mut T6WeaponFacts) {
        let Some((_, atts)) = name.split_once('+') else {
            return;
        };
        let atts: Vec<&[u8]> = atts
            .split('+')
            .filter_map(|a| self.attachments.get(a).map(Vec::as_slice))
            .collect();
        if atts.is_empty() {
            return;
        }
        let zero = [0u8; fastfile_t6::layout::WeaponAttachment::SIZE];
        let default = self
            .attachments
            .get(DEFAULT_ATTACHMENT)
            .map_or(&zero[..], Vec::as_slice);
        f.spread.apply_attachments(&atts, default);
        f.att = AttachmentScales::fold(&atts);
        f.reload.scale_by_attachments(&f.att);
        f.ads.apply_attachments(&atts, default, f.att.ads_in, f.att.ads_out);
        f.core.clip_size = attachment_clip_size(f.core.clip_size, &atts);
        f.core.fire_type = attachment_fire_type(f.core.fire_type, &atts);
        // BG_IsDualMag: Fast Mag's quick reloads, unless the gun ignores its attachments.
        if !f.core.ignore_attachments {
            use fastfile_t6::layout::WeaponAttachment as A;
            f.reload.dual_mag |= atts[..atts.len().min(3)].iter().any(|a| a[A::bDualMag] != 0);
        }
        // BO2's alt-mode flag getters also take any of the gun's attachments' flags (select fire, Hybrid).
        use fastfile_t6::layout::WeaponAttachment as A;
        for a in &atts[..atts.len().min(3)] {
            f.core.alt_weapon_ads_only |= a[A::bAltWeaponAdsOnly] != 0;
            f.core.alt_weapon_disable_switching |= a[A::bAltWeaponDisableSwitching] != 0;
        }
    }

    /// BO2's attachment-unique overrides (BG_GetFirstRaiseTime, BG_GetAltRaiseTime and
    /// AltDropTime, BG_GetReloadAmmoAdd and StartAdd, BG_IsSegmentedReload). Each attachment
    /// slot has its unique (`au_<gun>_<att>`; the first two slots fall back to the gun's base
    /// unique `au_<gun>_none`, the third stops the search). The first slot whose value differs
    /// from the base unique's wins; else the gun's own value stays. The segmented flag is the
    /// gun's or any slot's, unless the gun ignores its attachments.
    fn fold_uniques(&self, name: &str, f: &mut T6WeaponFacts) {
        use fastfile_t6::layout::WeaponAttachmentUnique as U;
        let Some((gun, atts)) = name.split_once('+') else {
            return;
        };
        let stem = gun.strip_suffix("_mp").unwrap_or(gun);
        let Some(base) = self.uniques.get(&format!("au_{stem}_none")).map(Vec::as_slice) else {
            return;
        };
        let slots: Vec<&[u8]> = atts
            .split('+')
            .take(3)
            .enumerate()
            .map_while(|(i, a)| {
                let own = self.uniques.get(&format!("au_{stem}_{a}")).map(Vec::as_slice);
                if i < 2 { Some(own.unwrap_or(base)) } else { own }
            })
            .collect();
        let pick = |at: usize, own: i32| {
            slots
                .iter()
                .map(|u| rd_i32(u, at))
                .find(|&x| x != rd_i32(base, at))
                .unwrap_or(own)
        };
        f.switch.first_raise = pick(U::iFirstRaiseTime, f.switch.first_raise);
        if f.core.inventory_type != inventory::ALTMODE {
            f.switch.alt_raise = pick(U::iAltRaiseTime, f.switch.alt_raise);
            f.switch.alt_drop = pick(U::iAltDropTime, f.switch.alt_drop);
        }
        f.reload.ammo_add = pick(U::iReloadAmmoAdd, f.reload.ammo_add);
        f.reload.start_add = pick(U::iReloadStartAdd, f.reload.start_add);
        if !f.core.ignore_attachments {
            f.reload.segmented |= slots.iter().any(|u| rd_bool(u, U::bSegmentedReload));
        }
    }
}

/// One attachment unique as a dump line (see [`FactsTable::from_dump`]): its first
/// [`UNIQUE_DUMP_LEN`] bytes.
pub fn unique_line(name: &str, bytes: &[u8]) -> String {
    format!("#attu\t{name}\t{}", hex(&bytes[..bytes.len().min(UNIQUE_DUMP_LEN)]))
}

/// One attachment's bytes as a dump line (see [`FactsTable::from_dump`]).
pub fn attachment_line(name: &str, bytes: &[u8]) -> String {
    format!("#att\t{name}\t{}", hex(bytes))
}

/// One dump line (see [`FactsTable::from_dump`]); empty `alt`/`dual` are left out.
pub fn dump_line(
    name: &str,
    variant: &[u8],
    def: &[u8],
    anim_slots: u128,
    has_gun_model: bool,
    alt: &str,
    dual: &str,
) -> String {
    let mut line = format!(
        "{name}\t{}\t{}\tslots={anim_slots:x}",
        hex(variant),
        hex(def)
    );
    if has_gun_model {
        line.push_str("\tgun=1");
    }
    if !alt.is_empty() {
        line.push_str(&format!("\talt={alt}"));
    }
    if !dual.is_empty() {
        line.push_str(&format!("\tdual={dual}"));
    }
    line
}

pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rockets_wobble_reads_from_its_def() {
        // smaw_mp's: 2500 units a second, a turn every 0.4 s after 300 units, 10 degrees.
        let mut d = vec![0u8; D::SIZE];
        d[D::iProjectileSpeed..D::iProjectileSpeed + 4].copy_from_slice(&2500i32.to_le_bytes());
        d[D::destabilizationRateTime..D::destabilizationRateTime + 4]
            .copy_from_slice(&0.4f32.to_le_bytes());
        d[D::destabilizationCurvatureMax..D::destabilizationCurvatureMax + 4]
            .copy_from_slice(&10.0f32.to_le_bytes());
        d[D::destabilizeDistance..D::destabilizeDistance + 4].copy_from_slice(&300i32.to_le_bytes());
        let t = FactsTable::from_dump(&dump_line("smaw_mp", &[0u8; 600], &d, 0, true, "", ""));
        assert_eq!(
            t.get(1).missile,
            MissileFacts {
                projectile_speed: 2500,
                destabilize_rate_time: 0.4,
                destabilize_curvature_max: 10.0,
                destabilize_distance: 300,
            }
        );
    }

    #[test]
    fn a_turrets_arcs_read_from_its_def() {
        // auto_gun_turret_mp's (the Sentry Gun).
        let mut d = vec![0u8; D::SIZE];
        for (at, x) in [
            (D::leftArc, 65.0f32),
            (D::rightArc, 65.0),
            (D::topArc, 45.0),
            (D::bottomArc, 20.0),
            (D::scanSpeed, 60.0),
            (D::scanAccel, 30.0),
        ] {
            d[at..at + 4].copy_from_slice(&x.to_le_bytes());
        }
        d[D::scanPauseTime..D::scanPauseTime + 4].copy_from_slice(&500i32.to_le_bytes());
        let t = FactsTable::from_dump(&dump_line("auto_gun_turret_mp", &[0u8; 600], &d, 0, true, "", ""));
        assert_eq!(
            t.get(1).turret,
            TurretFacts {
                left_arc: 65.0,
                right_arc: 65.0,
                top_arc: 45.0,
                bottom_arc: 20.0,
                scan_speed: 60.0,
                scan_accel: 30.0,
                scan_pause_ms: 500,
                barrel_type: 0,
            }
        );
    }

    /// A vehicle gun's barrels: the VTOL Warship's minigun (dual) fires
    /// both each shot, its rockets (dual alternating) go side to side, the
    /// AGR's gun (quad alternating) goes round its four; one barrel always
    /// fires itself. Its type reads from its def.
    #[test]
    fn a_vehicle_guns_barrels_go_round_by_its_type() {
        let gun = |barrel_type| TurretFacts { barrel_type, ..Default::default() };
        let round = |barrel_type, shots| {
            let mut last = 0;
            (0..shots)
                .map(|_| {
                    let (b, n) = gun(barrel_type).barrels(last);
                    assert_eq!(n, 1);
                    last = b;
                    b
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(gun(0).barrels(0), (0, 1));
        assert_eq!(gun(0).barrels(3), (0, 1));
        assert_eq!(gun(1).barrels(1), (0, 2));
        assert_eq!(round(2, 3), [1, 0, 1]);
        assert_eq!(gun(3).barrels(2), (0, 4));
        assert_eq!(round(4, 5), [1, 2, 3, 0, 1]);
        assert_eq!(gun(5).barrels(2), (3, 2));
        let mut d = vec![0u8; D::SIZE];
        d[D::barrelType..D::barrelType + 4].copy_from_slice(&4u32.to_le_bytes());
        let t = FactsTable::from_dump(&dump_line("ai_tank_drone_gun_mp", &[0u8; 600], &d, 0, true, "", ""));
        assert_eq!(t.get(1).turret.barrel_type, 4);
    }

    #[test]
    fn a_weapons_camera_shakes_read_from_its_def() {
        // cobra_20mm_mp's: fires with 0.05 for 100 ms out to 100, goes off
        // with 0.5 for 500 ms out to 512.
        let mut d = vec![0u8; D::SIZE];
        for (at, x) in [(D::cameraShakeScale, 0.05f32), (D::explosionCameraShakeScale, 0.5)] {
            d[at..at + 4].copy_from_slice(&x.to_le_bytes());
        }
        for (at, x) in [
            (D::cameraShakeDuration, 100i32),
            (D::cameraShakeRadius, 100),
            (D::explosionCameraShakeDuration, 500),
            (D::explosionCameraShakeRadius, 512),
        ] {
            d[at..at + 4].copy_from_slice(&x.to_le_bytes());
        }
        let t = FactsTable::from_dump(&dump_line("cobra_20mm_mp", &[0u8; 600], &d, 0, true, "", ""));
        let shake = t.get(1).shake;
        assert_eq!(shake.fire, CameraShake { scale: 0.05, duration_ms: 100, radius: 100 });
        assert_eq!(shake.explosion, CameraShake { scale: 0.5, duration_ms: 500, radius: 512 });
        assert!(shake.fire.is_set() && shake.explosion.is_set());
        // Without a radius, or a length, it does not shake.
        assert!(!CameraShake { scale: 0.5, duration_ms: 500, radius: 0 }.is_set());
        assert!(!CameraShake { scale: 0.5, duration_ms: 0, radius: 512 }.is_set());
    }

    #[test]
    fn attachment_rows_fold_their_attachments() {
        use fastfile_t6::layout::WeaponAttachment as A;
        let (v, d) = ([0u8; 600], vec![0u8; 2048]);
        let mut laser = vec![0u8; A::SIZE];
        laser[A::laserSight] = 1;
        laser[A::perks..A::perks + 8].copy_from_slice(&(1u64 << 3).to_le_bytes());
        for (at, x) in [
            (A::fFireTimeScale, 0.8f32),
            (A::fAdsTransInTimeScale, 0.5),
            (A::fAdsTransOutTimeScale, 1.0),
        ] {
            laser[at..at + 4].copy_from_slice(&x.to_le_bytes());
        }
        let dump = [
            dump_line("gun_mp", &v, &d, 0, true, "", ""),
            attachment_line("steadyaim", &laser),
            attachment_line(DEFAULT_ATTACHMENT, &[0u8; A::SIZE]),
        ]
        .join("\n");
        let t = FactsTable::from_dump(&dump).reindexed([(4, "gun_mp"), (9, "gun_mp+steadyaim")]);
        assert!(!t.get(4).spread.laser_sight);
        assert!(t.get(9).spread.laser_sight);
        assert!(t.get(9).spread.has_perk(3));
        assert_eq!(t.get(4).att, AttachmentScales::default());
        assert_eq!((t.get(9).att.fire_time, t.get(9).att.ads_in), (0.8, 0.5));
        assert_eq!(t.find("gun_mp+steadyaim"), Some(9));
    }

    #[test]
    fn sight_attachments_set_the_aim_speed_and_zoom() {
        use fastfile_t6::layout::WeaponAttachment as A;
        let (v, d) = ([0u8; 600], vec![0u8; 2048]);
        let att = |kind: i32, fields: &[(usize, f32)]| {
            let mut a = vec![0u8; A::SIZE];
            a[A::attachmentType..A::attachmentType + 4].copy_from_slice(&kind.to_le_bytes());
            for &(at, x) in fields {
                a[at..at + 4].copy_from_slice(&x.to_le_bytes());
            }
            a
        };
        let acog = att(1, &[
            (A::fAdsTransInTimeScale, 0.5),
            (A::fAdsTransOutTimeScale, 0.8),
            (A::fAdsZoomFov1, 40.0),
            (A::fAdsZoomInFrac, 0.5),
        ]);
        let vzoom = att(crate::pm::ads::ATTACHMENT_VZOOM, &[
            (A::fAdsTransInTimeScale, 0.9),
            (A::fAdsTransOutTimeScale, 1.0),
            (A::fAdsZoomFov1, 30.0),
            (A::fAdsZoomFov2, 20.0),
            (A::fAdsZoomFov3, 10.0),
            (A::fAdsZoomInFrac, 0.4),
        ]);
        let dump = [
            dump_line("gun_mp", &v, &d, 0, true, "", ""),
            attachment_line("acog", &acog),
            attachment_line("vzoom", &vzoom),
            attachment_line(DEFAULT_ATTACHMENT, &[0u8; A::SIZE]),
        ]
        .join("
");
        let t = FactsTable::from_dump(&dump).reindexed([(4, "gun_mp"), (9, "gun_mp+acog+vzoom")]);
        let (bare, both) = (t.get(4).ads, t.get(9).ads);
        assert_eq!((bare.in_scale, bare.out_scale, bare.variable_zoom), (1.0, 1.0, false));
        assert_eq!((both.in_scale, both.out_scale), (0.5 * 0.9, 0.8));
        // The first differing zoom fraction wins; each field of view takes the last differing one.
        assert_eq!(both.zoom_in_frac, 0.5);
        assert_eq!(both.zoom_fov, [30.0, 20.0, 10.0]);
        assert!(both.variable_zoom);
    }

    #[test]
    fn reads_switch_times_at_layout_offsets() {
        let mut def = vec![0u8; 2048];
        def[D::iDropTime..D::iDropTime + 4].copy_from_slice(&450i32.to_le_bytes());
        def[D::iFirstRaiseTime..D::iFirstRaiseTime + 4].copy_from_slice(&1200i32.to_le_bytes());
        def[D::bNoDropsOrRaises] = 1;
        let f = T6WeaponFacts::read(&[0u8; 600], &def);
        assert_eq!(f.switch.drop, 450);
        assert_eq!(f.switch.first_raise, 1200);
        assert!(f.core.no_drops_or_raises);
        let dump = format!("gun_mp\t{}\t{}\n", hex(&[0u8; 600]), hex(&def));
        let t = FactsTable::from_dump(&dump);
        assert_eq!(t.get(1).switch.drop, 450);
        assert_eq!(t.find("gun_mp"), Some(1));
        assert_eq!(t.get(9), T6WeaponFacts::default());
    }

    /// Select Fire / Hybrid (`sf_`, `dualoptic_`): the gun's magazine and reserve. Launcher
    /// (`gl_`, ignores attachments): its own. Left-hand pistol: its own clip, the right gun's
    /// reserve. Origins staff charge levels: the first staff's.
    pub fn ammo_remap_dump() -> FactsTable {
        let v = [0u8; 600];
        let mut v_own = [0u8; 600];
        v_own[V::bIgnoreAttachments] = 1;
        let d = vec![0u8; 2048];
        let mut alt = d.clone();
        alt[D::inventoryType] = inventory::ALTMODE;
        let mut right = d.clone();
        right[D::bDualWield] = 1;
        let mut left = right.clone();
        left[D::inventoryType] = inventory::DWLEFTHAND;
        let mut charge = d.clone();
        charge[D::fireType] = fire_type::CHARGESHOT;
        let dump = [
            dump_line("ar_mp", &v, &d, 0, true, "sf_ar_mp", ""),
            dump_line("sf_ar_mp", &v, &alt, 0, true, "ar_mp", ""),
            dump_line("gun_mp", &v, &d, 0, true, "gl_gun_mp", ""),
            dump_line("gl_gun_mp", &v_own, &alt, 0, true, "gun_mp", ""),
            dump_line("pistol_dw_mp", &v, &right, 0, true, "", "pistol_lh_mp"),
            dump_line("pistol_lh_mp", &v, &left, 0, true, "", "pistol_dw_mp"),
            dump_line("staff_zm", &v, &charge, 0, true, "staff2_zm", ""),
            dump_line("staff2_zm", &v, &charge, 0, true, "staff3_zm", ""),
            dump_line("staff3_zm", &v, &charge, 0, true, "", ""),
        ]
        .join("
");
        FactsTable::from_dump(&dump)
    }

    #[test]
    fn ammo_owners_follow_bo2s_alt_dual_and_charge_rules() {
        let t = ammo_remap_dump();
        let owners = |w| (t.ammo_clip_weapon(w), t.ammo_pool_weapon(w));
        assert_eq!(owners(1), (1, 1));
        assert_eq!(owners(2), (1, 1), "select fire shares the gun's");
        assert_eq!(owners(4), (4, 4), "a launcher keeps its own");
        assert_eq!(owners(5), (5, 5));
        assert_eq!(owners(6), (6, 5), "left hand: own clip, partner's reserve");
        assert_eq!(owners(7), (7, 7));
        assert_eq!(owners(8), (7, 7), "charge levels use the first staff's");
        assert_eq!(owners(9), (7, 7));
        // The game's own numbering keeps the links.
        let g = t.reindexed([(3, "staff3_zm"), (4, "staff_zm"), (5, "staff2_zm"), (8, "ar_mp"), (9, "sf_ar_mp")]);
        assert_eq!(g.ammo_clip_weapon(3), 4);
        assert_eq!(g.ammo_pool_weapon(5), 4);
        assert_eq!(g.ammo_clip_weapon(9), 8);
    }

    #[test]
    fn shared_magazine_is_one_count_and_never_negative() {
        let t = ammo_remap_dump();
        let mut ps = crate::player::T6WeaponPs::default();
        for w in [1, 2, 3, 4, 5, 6] {
            ps.give(w);
        }
        ps.set_ammo_in_clip(&t, 1, 30);
        ps.set_ammo_not_in_clip(&t, 1, 90);
        ps.set_ammo_in_clip(&t, 2, 29);
        assert_eq!(ps.ammo_in_clip(&t, 1), 29, "a select-fire shot comes out of the gun's magazine");
        assert_eq!(ps.ammo_not_in_clip(&t, 2), 90);
        assert_eq!(ps.clip_slot(2), 0, "the alt mode has no clip of its own");
        ps.set_ammo_in_clip(&t, 3, 6);
        ps.set_ammo_in_clip(&t, 4, 1);
        assert_eq!((ps.ammo_in_clip(&t, 3), ps.ammo_in_clip(&t, 4)), (6, 1));
        ps.set_ammo_not_in_clip(&t, 6, 40);
        assert_eq!(ps.ammo_not_in_clip(&t, 5), 40, "both pistols share one reserve");
        ps.set_ammo_in_clip(&t, 1, -3);
        ps.set_ammo_not_in_clip(&t, 1, -1);
        assert_eq!((ps.ammo_in_clip(&t, 2), ps.ammo_not_in_clip(&t, 2)), (0, 0));
    }

    #[test]
    fn dump_links_and_loader_columns_follow_names() {
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let dump = [
            dump_line("gun_mp", &v, &d, 0x5, true, "gun_gl_mp", ""),
            dump_line("gun_gl_mp", &v, &d, 0, false, "gun_mp", ""),
            dump_line("pistol_dw_mp", &v, &d, 0, true, "", "pistol_lh_mp"),
            dump_line("pistol_lh_mp", &v, &d, 0, true, "", ""),
        ]
        .join("\n");
        let t = FactsTable::from_dump(&dump);
        assert_eq!(t.get(1).core.anim_slots, 0x5);
        assert!(t.get(1).core.has_gun_model);
        assert!(!t.get(2).core.has_gun_model);
        assert_eq!(t.get(1).core.alt_weapon, 2);
        assert_eq!(t.get(2).core.alt_weapon, 1);
        assert_eq!(t.get(3).core.dual_wield_weapon, 4);
        // The game numbers them differently and adds attachment rows.
        let g = t.reindexed([
            (7, "pistol_lh_mp"),
            (3, "gun_mp"),
            (4, "gun_gl_mp"),
            (5, "gun_mp+extclip"),
            (9, "pistol_dw_mp"),
        ]);
        assert_eq!(g.get(3).core.alt_weapon, 4);
        assert_eq!(g.get(4).core.alt_weapon, 3);
        assert_eq!(g.get(5).core.anim_slots, 0x5);
        assert_eq!(g.get(9).core.dual_wield_weapon, 7);
        assert_eq!(g.find("gun_mp+extclip"), Some(5));
    }

    #[test]
    fn an_attachment_row_gives_its_gun_the_alt() {
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let dump = [
            dump_line("rifle_mp", &v, &d, 0x7, true, "", ""),
            dump_line("gl_rifle_mp", &v, &d, 0, true, "rifle_mp", ""),
            dump_line("rifle_mp+gl", &v, &d, 0x7, true, "gl_rifle_mp", ""),
        ]
        .join(
            "
",
        );
        let g = FactsTable::from_dump(&dump).reindexed([
            (2, "rifle_mp"),
            (3, "gl_rifle_mp"),
            (4, "rifle_mp+gl"),
            (5, "rifle_mp+extclip+gl"),
            (6, "rifle_mp+extclip"),
        ]);
        // The bare gun has no alt; with the launcher on, alone or with others, it is the launcher.
        assert_eq!(g.get(2).core.alt_weapon, 0);
        assert_eq!(g.get(4).core.alt_weapon, 3);
        assert_eq!(g.get(5).core.alt_weapon, 3);
        assert_eq!(g.get(5).core.anim_slots, 0x7);
        assert_eq!(g.get(6).core.alt_weapon, 0);
        assert_eq!(g.get(3).core.alt_weapon, 2);
    }

    #[test]
    fn an_alt_keeps_the_guns_attachments() {
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let dump = [
            dump_line("rifle_mp", &v, &d, 0x7, true, "", ""),
            dump_line("gl_rifle_mp", &v, &d, 0, true, "rifle_mp", ""),
            dump_line("rifle_mp+gl", &v, &d, 0x7, true, "gl_rifle_mp", ""),
            dump_line("pistol_dw_mp", &v, &d, 0, true, "", "pistol_lh_mp"),
            dump_line("pistol_lh_mp", &v, &d, 0, true, "", ""),
        ]
        .join("\n");
        let g = FactsTable::from_dump(&dump).reindexed([
            (2, "rifle_mp"),
            (3, "gl_rifle_mp"),
            (4, "rifle_mp+gl"),
            (5, "gl_rifle_mp+gl"),
            (6, "rifle_mp+extclip+gl"),
            (7, "gl_rifle_mp+extclip+gl"),
            (8, "pistol_dw_mp+extclip"),
            (9, "pistol_lh_mp+extclip"),
            (10, "pistol_lh_mp"),
        ]);
        // The game's launcher rows carry the gun's attachments, both ways (BG_GetAltWeapon
        // swaps only the base index).
        assert_eq!(g.get(4).core.alt_weapon, 5);
        assert_eq!(g.get(5).core.alt_weapon, 4);
        assert_eq!(g.get(6).core.alt_weapon, 7);
        assert_eq!(g.get(7).core.alt_weapon, 6);
        // Without its own `+att` row the bare launcher still answers.
        assert_eq!(g.get(3).core.alt_weapon, 2);
        assert_eq!(g.get(8).core.dual_wield_weapon, 9);
    }

    #[test]
    fn an_attachment_sets_the_alt_mode_flags() {
        use fastfile_t6::layout::WeaponAttachment as A;
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let mut sf = vec![0u8; A::SIZE];
        sf[A::bAltWeaponDisableSwitching] = 1;
        let mut optic = vec![0u8; A::SIZE];
        optic[A::bAltWeaponAdsOnly] = 1;
        optic[A::bAltWeaponDisableSwitching] = 1;
        let dump = [
            attachment_line("sf", &sf),
            attachment_line("dualoptic", &optic),
            dump_line("rifle_mp", &v, &d, 0x7, true, "", ""),
        ]
        .join("\n");
        let g = FactsTable::from_dump(&dump).reindexed([
            (2, "rifle_mp"),
            (3, "rifle_mp+sf"),
            (4, "rifle_mp+dualoptic"),
        ]);
        let flags = |i| (g.get(i).core.alt_weapon_ads_only, g.get(i).core.alt_weapon_disable_switching);
        assert_eq!(flags(2), (false, false));
        assert_eq!(flags(3), (false, true));
        assert_eq!(flags(4), (true, true));
    }

    /// An attachment that changes nothing: every scale 1, as BO2's own attachments are unless
    /// they say otherwise.
    fn plain_attachment() -> Vec<u8> {
        use fastfile_t6::layout::WeaponAttachment as A;
        let mut a = vec![0u8; A::SIZE];
        for at in [
            A::fFireTimeScale,
            A::fAdsTransInTimeScale,
            A::fAdsTransOutTimeScale,
            A::fReloadTimeScale,
            A::fReloadEmptyTimeScale,
            A::fReloadAddTimeScale,
            A::fReloadQuickTimeScale,
            A::fReloadQuickEmptyTimeScale,
            A::fReloadQuickAddTimeScale,
            A::clipSizeScale,
        ] {
            a[at..at + 4].copy_from_slice(&1f32.to_le_bytes());
        }
        a
    }

    fn put_f32(b: &mut [u8], at: usize, x: f32) {
        b[at..at + 4].copy_from_slice(&x.to_le_bytes());
    }

    fn put_i32(b: &mut [u8], at: usize, x: i32) {
        b[at..at + 4].copy_from_slice(&x.to_le_bytes());
    }

    /// A 30-round rifle (reload 2475 ms, empty 3000, quick 1500, add 500) with BO2 MP's
    /// Fast Mag, Extended Mag, Rapid Fire and Tri-Bolt numbers (patch weapon assets).
    fn rifle_with_mags(ignore_attachments: bool) -> FactsTable {
        use fastfile_t6::layout::WeaponAttachment as A;
        let mut v = vec![0u8; V::SIZE];
        let mut d = vec![0u8; D::SIZE];
        put_i32(&mut v, V::iClipSize, 30);
        put_i32(&mut v, V::iReloadTime, 2475);
        put_i32(&mut v, V::iReloadEmptyTime, 3000);
        put_i32(&mut v, V::iReloadQuickTime, 1500);
        put_i32(&mut d, D::iReloadAddTime, 500);
        put_i32(&mut d, D::iFireTime, 100);
        v[V::bIgnoreAttachments] = ignore_attachments as u8;
        let mut dualclip = plain_attachment();
        for at in [
            A::fReloadTimeScale,
            A::fReloadEmptyTimeScale,
            A::fReloadAddTimeScale,
            A::fReloadQuickTimeScale,
            A::fReloadQuickEmptyTimeScale,
        ] {
            put_f32(&mut dualclip, at, 0.675);
        }
        dualclip[A::bDualMag] = 1;
        let mut extclip = plain_attachment();
        put_f32(&mut extclip, A::fReloadTimeScale, 1.1);
        put_f32(&mut extclip, A::clipSizeScale, 1.345);
        let mut rf = plain_attachment();
        put_f32(&mut rf, A::fFireTimeScale, 0.75);
        let mut stackfire = plain_attachment();
        put_i32(&mut stackfire, A::attachmentType, ATTACHMENT_STACKFIRE);
        put_i32(&mut stackfire, A::fireType, fire_type::STACKED as i32);
        put_i32(&mut stackfire, A::iClipSize, 3);
        put_f32(&mut stackfire, A::stackFireSpread, 4.0);
        put_f32(&mut stackfire, A::stackFireAccuracyDecay, 0.9);
        let dump = [
            dump_line("rifle_mp", &v, &d, 0, true, "", ""),
            attachment_line("dualclip", &dualclip),
            attachment_line("extclip", &extclip),
            attachment_line("rf", &rf),
            attachment_line("stackfire", &stackfire),
        ]
        .join("\n");
        FactsTable::from_dump(&dump).reindexed([
            (1, "rifle_mp"),
            (2, "rifle_mp+dualclip"),
            (3, "rifle_mp+extclip"),
            (4, "rifle_mp+rf"),
            (5, "rifle_mp+stackfire"),
            (6, "rifle_mp+extclip+rf"),
        ])
    }

    #[test]
    fn fast_mag_shortens_every_reload_and_gives_the_quick_one() {
        let t = rifle_with_mags(false);
        let (base, fast) = (t.get(1).reload, t.get(2).reload);
        assert!(!base.dual_mag);
        assert!(fast.dual_mag);
        // BG_GetReloadTime: (int)(2475 * 0.675) = 1670 (1670.6 cut toward zero).
        assert_eq!((base.reload, fast.reload), (2475, 1670));
        assert_eq!(fast.reload_empty, 2025);
        assert_eq!(fast.reload_quick, 1012);
        assert_eq!(fast.reload_add, 337);
        assert_eq!(t.get(2).core.clip_size, 30);
    }

    #[test]
    fn a_gun_that_ignores_attachments_gets_no_fast_mag_flag() {
        let t = rifle_with_mags(true);
        assert!(!t.get(2).reload.dual_mag);
        // The time scales still apply: BO2's reload getters do not read the flag.
        assert_eq!(t.get(2).reload.reload, 1670);
    }

    #[test]
    fn extended_mag_grows_the_clip_and_slows_the_reload() {
        let t = rifle_with_mags(false);
        // BG_GetClipSize: round(30 * 1.345) = 40.
        assert_eq!(t.get(3).core.clip_size, 40);
        assert_eq!(t.get(3).reload.reload, 2722);
        assert_eq!(t.get(3).reload.reload_empty, 3000);
        assert!(!t.get(3).reload.dual_mag);
        // Two attachments fold together.
        assert_eq!(t.get(6).core.clip_size, 40);
        assert_eq!(t.get(6).att.fire_time, 0.75);
    }

    #[test]
    fn tri_bolt_sets_its_own_clip_and_fire_type() {
        let t = rifle_with_mags(false);
        assert_eq!(t.get(1).core.fire_type, fire_type::FULLAUTO);
        assert_eq!(t.get(5).core.fire_type, fire_type::STACKED);
        assert_eq!(t.get(5).core.clip_size, 3);
        assert_eq!(t.get(4).core.fire_type, fire_type::FULLAUTO);
        // BG_GetStackFireSpread / BG_GetStackFireAccuracyDecay: the attachment's own values.
        assert_eq!(t.get(5).spread.stack_fire_spread, 4.0);
        assert_eq!(t.get(5).spread.stack_fire_accuracy_decay, 0.9);
        assert_eq!(t.get(1).spread.stack_fire_accuracy_decay, 0.0);
    }

    #[test]
    fn a_one_round_clip_stays_one() {
        use fastfile_t6::layout::WeaponAttachment as A;
        let mut ext = plain_attachment();
        put_f32(&mut ext, A::clipSizeScale, 1.4);
        assert_eq!(attachment_clip_size(1, &[&ext]), 1);
        assert_eq!(attachment_clip_size(5, &[&ext]), 7);
        // A blank scale (0) leaves the clip alone instead of emptying it.
        assert_eq!(attachment_clip_size(30, &[&[0u8; A::SIZE]]), 30);
    }

    /// An attachment unique with BO2 MP's override fields:
    /// [first raise, alt raise, alt drop, reload ammo add, start add, segmented].
    fn unique(vals: [i32; 6]) -> Vec<u8> {
        use fastfile_t6::layout::WeaponAttachmentUnique as U;
        let mut u = vec![0u8; U::SIZE];
        for (at, x) in [
            U::iFirstRaiseTime,
            U::iAltRaiseTime,
            U::iAltDropTime,
            U::iReloadAmmoAdd,
            U::iReloadStartAdd,
        ]
        .into_iter()
        .zip(vals)
        {
            put_i32(&mut u, at, x);
        }
        u[U::bSegmentedReload] = vals[5] as u8;
        u
    }

    /// The KSG (shell by shell, one at a time) and AN-94 with their real attachment uniques:
    /// Fast Mag loads two shells, MMS and Laser change the first raise, Select Fire's drop.
    fn guns_with_uniques(alt_mode: bool) -> FactsTable {
        let (mut v, mut d) = (vec![0u8; V::SIZE], vec![0u8; D::SIZE]);
        put_i32(&mut d, D::iFirstRaiseTime, 1150);
        put_i32(&mut d, D::iReloadAmmoAdd, 1);
        put_i32(&mut d, D::iReloadStartAdd, 1);
        d[D::bSegmentedReload] = 1;
        let ksg = dump_line("ksg_mp", &v, &d, 0, true, "", "");
        let mut d = vec![0u8; D::SIZE];
        put_i32(&mut d, D::iFirstRaiseTime, 1600);
        put_i32(&mut v, V::iAltRaiseTime, 700);
        if alt_mode {
            d[D::inventoryType] = inventory::ALTMODE;
        }
        let an94 = dump_line("an94_mp", &v, &d, 0, true, "", "");
        let dump = [
            ksg,
            an94,
            unique_line("au_ksg_none", &unique([0, 500, 0, 1, 1, 1])),
            unique_line("au_ksg_dualclip", &unique([0, 500, 0, 2, 2, 1])),
            unique_line("au_ksg_mms", &unique([1250, 500, 0, 1, 1, 1])),
            unique_line("au_an94_none", &unique([0, 500, 0, 0, 0, 0])),
            unique_line("au_an94_sf", &unique([0, 500, 500, 0, 0, 0])),
            unique_line("au_an94_steadyaim", &unique([1150, 500, 0, 0, 0, 0])),
            unique_line("au_an94_mms", &unique([1250, 500, 0, 0, 0, 0])),
        ]
        .join("\n");
        FactsTable::from_dump(&dump).reindexed([
            (1, "ksg_mp"),
            (2, "ksg_mp+dualclip"),
            (3, "ksg_mp+mms"),
            (4, "ksg_mp+silencer"),
            (5, "an94_mp+sf"),
            (6, "an94_mp+steadyaim+mms"),
            (7, "an94_mp+grip+mms"),
            (8, "an94_mp"),
        ])
    }

    #[test]
    fn fast_mag_loads_two_shells_in_the_ksg() {
        let t = guns_with_uniques(false);
        let r = |i| (t.get(i).reload.ammo_add, t.get(i).reload.start_add, t.get(i).reload.segmented);
        assert_eq!(r(1), (1, 1, true));
        assert_eq!(r(2), (2, 2, true));
        // An attachment without its own unique reads the gun's base one: nothing changes.
        assert_eq!(r(4), (1, 1, true));
    }

    #[test]
    fn an_attachment_unique_sets_the_first_raise() {
        let t = guns_with_uniques(false);
        assert_eq!(t.get(1).switch.first_raise, 1150);
        assert_eq!(t.get(3).switch.first_raise, 1250);
        assert_eq!(t.get(8).switch.first_raise, 1600);
        // The first slot that differs from the base wins: Laser before MMS.
        assert_eq!(t.get(6).switch.first_raise, 1150);
        // A slot without its own unique is skipped over (the base matches the base).
        assert_eq!(t.get(7).switch.first_raise, 1250);
    }

    #[test]
    fn select_fire_takes_its_drop_and_keeps_the_guns_raise() {
        let t = guns_with_uniques(false);
        // Alt raise 500 matches the base unique's, so the gun's own 700 stays; the drop differs.
        assert_eq!((t.get(5).switch.alt_raise, t.get(5).switch.alt_drop), (700, 500));
        // An alt-mode weapon reads only its own times.
        let t = guns_with_uniques(true);
        assert_eq!((t.get(5).switch.alt_raise, t.get(5).switch.alt_drop), (700, 0));
    }

    #[test]
    fn unique_lines_keep_only_what_the_brain_reads() {
        let line = unique_line("au_ksg_none", &unique([0, 500, 0, 1, 1, 1]));
        assert_eq!(line.split('\t').nth(2).map(str::len), Some(UNIQUE_DUMP_LEN * 2));
    }

    /// Prints a few real guns with Fast Mag, Extended Mag, Rapid Fire and Tri-Bolt folded in.
    /// `T6_FACTS_DUMP=<dump> cargo test -p weapon_t6 real_attachment_rows -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_attachment_rows() {
        let Ok(path) = std::env::var("T6_FACTS_DUMP") else {
            println!("T6_FACTS_DUMP not set");
            return;
        };
        let dump = FactsTable::from_dump(&std::fs::read_to_string(path).expect("dump"));
        let names: Vec<String> = ["an94_zm", "hk416_zm", "svu_zm", "type95_zm"]
            .iter()
            .flat_map(|g| {
                ["", "+dualclip", "+extclip", "+rf", "+stackfire", "+extclip+rf"]
                    .iter()
                    .map(move |a| format!("{g}{a}"))
            })
            .collect();
        let t = dump.reindexed(names.iter().enumerate().map(|(i, n)| (i as u32 + 1, n.as_str())));
        for (i, n) in names.iter().enumerate() {
            let f = t.get(i as u32 + 1);
            println!(
                "{n:24} clip {:3} fire_type {} fire x{:.3} reload {:5} empty {:5} quick {:5} add {:4} dual_mag {}",
                f.core.clip_size,
                f.core.fire_type,
                f.att.fire_time,
                f.reload.reload,
                f.reload.reload_empty,
                f.reload.reload_quick,
                f.reload.reload_add,
                f.reload.dual_mag
            );
        }
    }
}
