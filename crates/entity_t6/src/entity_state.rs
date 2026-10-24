//! BO2's `entityState_s` and `LerpEntityState`, in BO2's field order.
//!
//! BO2 keeps several fields as C unions read by entity type. Here each
//! union is its raw words or bytes, with a typed view per member that
//! reads and writes those bytes in BO2's layout (little-endian, C
//! alignment). No `unsafe`.

use playerstate_t6::{RenderOptions, Weapon};

use crate::trajectory::Trajectory;

fn rd_i32(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn rd_f32(b: &[u8], o: usize) -> f32 {
    f32::from_bits(rd_i32(b, o) as u32)
}

fn rd_i16(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([b[o], b[o + 1]])
}

fn wr_i32(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn wr_f32(b: &mut [u8], o: usize, v: f32) {
    wr_i32(b, o, v.to_bits() as i32);
}

fn wr_i16(b: &mut [u8], o: usize, v: i16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// `LerpEntityState::u`: BO2's `LerpEntityStateAnonymous` (`int data[9]`),
/// the largest member. Read it through the view for the entity's type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LerpUnion(pub [i32; 9]);

impl LerpUnion {
    fn bytes(&self) -> [u8; 36] {
        let mut b = [0u8; 36];
        for (i, w) in self.0.iter().enumerate() {
            wr_i32(&mut b, i * 4, *w);
        }
        b
    }

    fn from_bytes(b: &[u8; 36]) -> Self {
        let mut w = [0i32; 9];
        for (i, v) in w.iter_mut().enumerate() {
            *v = rd_i32(b, i * 4);
        }
        Self(w)
    }

    #[must_use]
    pub fn player(&self) -> LerpPlayer {
        LerpPlayer::read(&self.bytes())
    }

    pub fn set_player(&mut self, p: &LerpPlayer) {
        let mut b = self.bytes();
        p.write(&mut b);
        *self = Self::from_bytes(&b);
    }

    #[must_use]
    pub fn missile(&self) -> LerpMissile {
        LerpMissile::read(&self.bytes())
    }

    pub fn set_missile(&mut self, m: &LerpMissile) {
        let mut b = self.bytes();
        m.write(&mut b);
        *self = Self::from_bytes(&b);
    }

    #[must_use]
    pub fn script_mover(&self) -> LerpScriptMover {
        LerpScriptMover::read(&self.bytes())
    }

    pub fn set_script_mover(&mut self, s: &LerpScriptMover) {
        let mut b = self.bytes();
        s.write(&mut b);
        *self = Self::from_bytes(&b);
    }
}

/// BO2's `LerpEntityStatePlayer`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LerpPlayer {
    pub leanf: f32,
    pub movement_dir: i32,
    pub move_type: i32,
    pub primary_weapon: Weapon,
    pub stowed_weapon: Weapon,
    pub offhand_weapon: Weapon,
    pub melee_weapon: Weapon,
    pub melee_weapon_camo: u8,
    pub melee_weapon_model: u8,
    pub vehicle_type: u8,
    pub vehicle_anim_bone_index: u8,
    pub vehicle_seat: u8,
    pub stowed_weapon_camo: u8,
    pub weapon_heat: u8,
    pub prone_legs_angle: u8,
}

impl LerpPlayer {
    fn read(b: &[u8; 36]) -> Self {
        Self {
            leanf: rd_f32(b, 0),
            movement_dir: rd_i32(b, 4),
            move_type: rd_i32(b, 8),
            primary_weapon: Weapon(rd_i32(b, 12) as u32),
            stowed_weapon: Weapon(rd_i32(b, 16) as u32),
            offhand_weapon: Weapon(rd_i32(b, 20) as u32),
            melee_weapon: Weapon(rd_i32(b, 24) as u32),
            melee_weapon_camo: b[28],
            melee_weapon_model: b[29],
            vehicle_type: b[30],
            vehicle_anim_bone_index: b[31],
            vehicle_seat: b[32],
            stowed_weapon_camo: b[33],
            weapon_heat: b[34],
            prone_legs_angle: b[35],
        }
    }

    fn write(&self, b: &mut [u8; 36]) {
        wr_f32(b, 0, self.leanf);
        wr_i32(b, 4, self.movement_dir);
        wr_i32(b, 8, self.move_type);
        wr_i32(b, 12, self.primary_weapon.0 as i32);
        wr_i32(b, 16, self.stowed_weapon.0 as i32);
        wr_i32(b, 20, self.offhand_weapon.0 as i32);
        wr_i32(b, 24, self.melee_weapon.0 as i32);
        b[28] = self.melee_weapon_camo;
        b[29] = self.melee_weapon_model;
        b[30] = self.vehicle_type;
        b[31] = self.vehicle_anim_bone_index;
        b[32] = self.vehicle_seat;
        b[33] = self.stowed_weapon_camo;
        b[34] = self.weapon_heat;
        b[35] = self.prone_legs_angle;
    }
}

/// BO2's `LerpEntityStateMissile`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LerpMissile {
    pub launch_time: i32,
    pub parent_client_num: i32,
    pub fuse_time: i32,
    pub forced_dud: i32,
    pub auto_detonate_time: f32,
}

impl LerpMissile {
    fn read(b: &[u8; 36]) -> Self {
        Self {
            launch_time: rd_i32(b, 0),
            parent_client_num: rd_i32(b, 4),
            fuse_time: rd_i32(b, 8),
            forced_dud: rd_i32(b, 12),
            auto_detonate_time: rd_f32(b, 16),
        }
    }

    fn write(&self, b: &mut [u8; 36]) {
        wr_i32(b, 0, self.launch_time);
        wr_i32(b, 4, self.parent_client_num);
        wr_i32(b, 8, self.fuse_time);
        wr_i32(b, 12, self.forced_dud);
        wr_f32(b, 16, self.auto_detonate_time);
    }
}

/// BO2's `LerpEntityStateScriptMover`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LerpScriptMover {
    pub attach_tag_index: [u8; 4],
    pub attached_tag_index: i32,
    pub attach_model_index: [i16; 4],
    pub anim_scripted_anim: i16,
    pub attached_ent_num: i16,
    pub attached_ent_time: i32,
    pub exploder_index: i16,
    pub scale: u16,
    pub flags: u8,
    pub ai_type: u16,
}

impl LerpScriptMover {
    fn read(b: &[u8; 36]) -> Self {
        Self {
            attach_tag_index: [b[0], b[1], b[2], b[3]],
            attached_tag_index: rd_i32(b, 4),
            attach_model_index: [rd_i16(b, 8), rd_i16(b, 10), rd_i16(b, 12), rd_i16(b, 14)],
            anim_scripted_anim: rd_i16(b, 16),
            attached_ent_num: rd_i16(b, 18),
            attached_ent_time: rd_i32(b, 20),
            exploder_index: rd_i16(b, 24),
            scale: rd_i16(b, 26) as u16,
            flags: b[28],
            ai_type: rd_i16(b, 30) as u16,
        }
    }

    fn write(&self, b: &mut [u8; 36]) {
        b[0..4].copy_from_slice(&self.attach_tag_index);
        wr_i32(b, 4, self.attached_tag_index);
        for (i, m) in self.attach_model_index.iter().enumerate() {
            wr_i16(b, 8 + i * 2, *m);
        }
        wr_i16(b, 16, self.anim_scripted_anim);
        wr_i16(b, 18, self.attached_ent_num);
        wr_i32(b, 20, self.attached_ent_time);
        wr_i16(b, 24, self.exploder_index);
        wr_i16(b, 26, self.scale as i16);
        b[28] = self.flags;
        wr_i16(b, 30, self.ai_type as i16);
    }
}

/// BO2's `LerpEntityState`: the part of an entity the client lerps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LerpEntityState {
    /// [`playerstate_t6::flags::e_flags`].
    pub e_flags: i32,
    /// [`playerstate_t6::flags::e_flags2`].
    pub e_flags2: i32,
    pub pos: Trajectory,
    pub apos: Trajectory,
    pub u: LerpUnion,
    pub use_count: i16,
    /// BO2's faction union: `teamAndOwnerIndex`, or `iHeadIconTeam` in its
    /// low byte.
    pub faction: i16,
    pub client_fields: u32,
}

impl LerpEntityState {
    #[must_use]
    pub const fn head_icon_team(&self) -> u8 {
        self.faction as u8
    }
}

/// `entityState_s::un2`: 16 bytes, read by entity type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Un2(pub [u8; 16]);

/// BO2's `playerAnimState_t` (players and corpses).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerAnimState {
    pub legs_anim: i32,
    pub torso_anim: i32,
    pub torso_pitch: f32,
    pub waist_pitch: f32,
}

/// BO2's `actorAnimState_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActorAnimState {
    pub state: i16,
    pub sub_state: u8,
    pub lean_amount: f32,
    pub aim_up_down: f32,
    pub aim_left_right: f32,
}

/// BO2's `vehicleState_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VehicleState {
    pub flags: i16,
    pub vehicle_def_index: u8,
    pub tree_id: u8,
    pub anim_id: i16,
    pub attach_model_index: [i16; 2],
    pub attach_tag_index: [u8; 2],
}

/// BO2's `hardlineHint_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardlineHint {
    pub team: u8,
    pub perk: u8,
    pub hint: u8,
}

/// BO2's `scriptMoverState_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptMoverState {
    pub fov: u8,
    pub tree_id: u8,
    pub anim_id: i16,
}

/// BO2's `fxLightingState_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxLightingState {
    pub primary_light_fraction: f32,
    pub lighting_origin_offset: i32,
}

impl Un2 {
    #[must_use]
    pub fn player_anim(&self) -> PlayerAnimState {
        let b = &self.0;
        PlayerAnimState {
            legs_anim: rd_i32(b, 0),
            torso_anim: rd_i32(b, 4),
            torso_pitch: rd_f32(b, 8),
            waist_pitch: rd_f32(b, 12),
        }
    }

    pub fn set_player_anim(&mut self, a: &PlayerAnimState) {
        let b = &mut self.0;
        wr_i32(b, 0, a.legs_anim);
        wr_i32(b, 4, a.torso_anim);
        wr_f32(b, 8, a.torso_pitch);
        wr_f32(b, 12, a.waist_pitch);
    }

    #[must_use]
    pub fn actor_anim(&self) -> ActorAnimState {
        let b = &self.0;
        ActorAnimState {
            state: rd_i16(b, 0),
            sub_state: b[2],
            lean_amount: rd_f32(b, 4),
            aim_up_down: rd_f32(b, 8),
            aim_left_right: rd_f32(b, 12),
        }
    }

    pub fn set_actor_anim(&mut self, a: &ActorAnimState) {
        let b = &mut self.0;
        wr_i16(b, 0, a.state);
        b[2] = a.sub_state;
        wr_f32(b, 4, a.lean_amount);
        wr_f32(b, 8, a.aim_up_down);
        wr_f32(b, 12, a.aim_left_right);
    }

    #[must_use]
    pub fn vehicle(&self) -> VehicleState {
        let b = &self.0;
        VehicleState {
            flags: rd_i16(b, 0),
            vehicle_def_index: b[2],
            tree_id: b[3],
            anim_id: rd_i16(b, 4),
            attach_model_index: [rd_i16(b, 6), rd_i16(b, 8)],
            attach_tag_index: [b[10], b[11]],
        }
    }

    pub fn set_vehicle(&mut self, v: &VehicleState) {
        let b = &mut self.0;
        wr_i16(b, 0, v.flags);
        b[2] = v.vehicle_def_index;
        b[3] = v.tree_id;
        wr_i16(b, 4, v.anim_id);
        wr_i16(b, 6, v.attach_model_index[0]);
        wr_i16(b, 8, v.attach_model_index[1]);
        b[10] = v.attach_tag_index[0];
        b[11] = v.attach_tag_index[1];
    }

    #[must_use]
    pub fn hardline_hint(&self) -> HardlineHint {
        HardlineHint {
            team: self.0[0],
            perk: self.0[1],
            hint: self.0[2],
        }
    }

    pub fn set_hardline_hint(&mut self, h: &HardlineHint) {
        self.0[0] = h.team;
        self.0[1] = h.perk;
        self.0[2] = h.hint;
    }

    #[must_use]
    pub fn script_mover(&self) -> ScriptMoverState {
        ScriptMoverState {
            fov: self.0[0],
            tree_id: self.0[1],
            anim_id: rd_i16(&self.0, 2),
        }
    }

    pub fn set_script_mover(&mut self, s: &ScriptMoverState) {
        self.0[0] = s.fov;
        self.0[1] = s.tree_id;
        wr_i16(&mut self.0, 2, s.anim_id);
    }

    #[must_use]
    pub fn fx_lighting(&self) -> FxLightingState {
        FxLightingState {
            primary_light_fraction: rd_f32(&self.0, 0),
            lighting_origin_offset: rd_i32(&self.0, 4),
        }
    }

    pub fn set_fx_lighting(&mut self, f: &FxLightingState) {
        wr_f32(&mut self.0, 0, f.primary_light_fraction);
        wr_i32(&mut self.0, 4, f.lighting_origin_offset);
    }
}

/// BO2's `clientLinkInfo_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientLinkInfo {
    pub parent_ent: i16,
    pub tag_index: u8,
    pub flags: u8,
}

/// BO2's `entityState_s`, in BO2's field order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityState {
    pub number: i32,
    pub lerp: LerpEntityState,
    pub time2: i32,
    pub loop_sound_id: u32,
    pub solid: i32,
    pub render_options: RenderOptions,
    pub un2: Un2,
    /// BO2's `un3` union: `hintString`, `vehicleXModel`, `secondBcAlias` or
    /// `soundTag`, by entity type.
    pub un3: u32,
    pub part_bits: [u32; 5],
    pub client_link_info: ClientLinkInfo,
    pub client_mask: [i32; 1],
    /// The event ring: four slots, indexed by `event_sequence & 3`.
    pub events: [u8; 4],
    pub event_parms: [u32; 4],
    pub event_parm: u32,
    /// [`crate::entity_type::et`].
    pub e_type: i16,
    pub ground_entity_num: i16,
    /// BO2's `index` union: `brushmodel`, `xmodel` or `primaryLight`
    /// (signed), or `bone` (unsigned).
    pub index: i16,
    pub other_entity_num: i16,
    pub attacker_entity_num: i16,
    pub enemy_model: i16,
    pub weapon: Weapon,
    pub last_stand_prev_weapon: Weapon,
    pub targetname: u16,
    pub loop_sound_fade: i16,
    pub event_sequence: i16,
    pub surf_type: u8,
    pub client_num: u8,
    pub i_head_icon: u8,
    pub weapon_model: u8,
    /// BO2's `un1` union: `scale`, `eventParm2`, `helicopterStage`,
    /// `destructibleid` or `zombieShrinkOn`, by entity type.
    pub un1: u8,
}

/// BO2's event ring size in `entityState_s::events`.
pub const MAX_EVENTS_PER_ENTITY: usize = 4;

impl EntityState {
    #[must_use]
    pub const fn bone(&self) -> u16 {
        self.index as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_view_round_trips_in_bo2_layout() {
        let p = LerpPlayer {
            leanf: -0.5,
            movement_dir: 45,
            move_type: playerstate_t6::flags::MOVE_TYPE_SLIDE,
            primary_weapon: Weapon::new(12, 1, 2, 3),
            stowed_weapon: Weapon::new(7, 0, 0, 0),
            offhand_weapon: Weapon::new(3, 0, 0, 0),
            melee_weapon: Weapon::new(1, 0, 0, 0),
            melee_weapon_camo: 1,
            melee_weapon_model: 2,
            vehicle_type: 3,
            vehicle_anim_bone_index: 4,
            vehicle_seat: 5,
            stowed_weapon_camo: 6,
            weapon_heat: 7,
            prone_legs_angle: 8,
        };
        let mut u = LerpUnion::default();
        u.set_player(&p);
        assert_eq!(u.player(), p);
        assert_eq!(u.0[0], (-0.5f32).to_bits() as i32);
        assert_eq!(u.0[3], Weapon::new(12, 1, 2, 3).0 as i32);
        // The eight trailing bytes fill words 7 and 8, low byte first.
        assert_eq!(u.0[7], 0x0403_0201);
        assert_eq!(u.0[8], 0x0807_0605);
    }

    #[test]
    fn missile_and_mover_share_the_same_words() {
        let mut u = LerpUnion::default();
        u.set_missile(&LerpMissile {
            launch_time: 5000,
            parent_client_num: 3,
            fuse_time: 2500,
            forced_dud: 0,
            auto_detonate_time: 1.5,
        });
        assert_eq!(u.0[..3], [5000, 3, 2500]);
        let m = LerpScriptMover {
            attach_tag_index: [1, 2, 3, 4],
            attached_tag_index: -1,
            attach_model_index: [10, 11, 12, 13],
            anim_scripted_anim: 20,
            attached_ent_num: 1022,
            attached_ent_time: 9000,
            exploder_index: -2,
            scale: 128,
            flags: 9,
            ai_type: 300,
        };
        u.set_script_mover(&m);
        assert_eq!(u.script_mover(), m);
        assert_eq!(u.0[0], 0x0403_0201);
        assert_eq!(u.0[4], 1022 << 16 | 20);
    }

    #[test]
    fn un2_views() {
        let mut un2 = Un2::default();
        let a = PlayerAnimState {
            legs_anim: 100,
            torso_anim: 200,
            torso_pitch: 10.0,
            waist_pitch: -5.0,
        };
        un2.set_player_anim(&a);
        assert_eq!(un2.player_anim(), a);
        let v = VehicleState {
            flags: 3,
            vehicle_def_index: 4,
            tree_id: 5,
            anim_id: 6,
            attach_model_index: [7, 8],
            attach_tag_index: [9, 10],
        };
        un2.set_vehicle(&v);
        assert_eq!(un2.vehicle(), v);
        let s = ScriptMoverState {
            fov: 65,
            tree_id: 2,
            anim_id: -1,
        };
        un2.set_script_mover(&s);
        assert_eq!(un2.script_mover(), s);
    }
}
