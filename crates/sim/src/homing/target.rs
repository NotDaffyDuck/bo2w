//! What a guided missile is homing on, and where that thing is right now.

use crate::step_view::StepWorldView;

/// A homing goal: a fixed point, a scripted mover, or a living player. The offset is in the goal's own frame
/// (forward, right-negated, up), so a missile can aim at a spot on the body instead of its origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MissileTarget {
    Point([f32; 3]),
    Entity {
        entity: crate::EntHandle,
        offset: [f32; 3],
    },
    Player {
        client: crate::SeatNo,
        life: crate::LifeCount,
        offset: [f32; 3],
    },
}

impl MissileTarget {
    pub fn is_point(self) -> bool {
        match self {
            Self::Point(_) => true,
            _ => false,
        }
    }

    /// The world position of the goal this frame, or `None` when it has gone away (entity freed, player dead or
    /// respawned into a new life).
    pub(crate) fn resolve(self, world: &StepWorldView) -> Option<[f32; 3]> {
        match self {
            Self::Point(point) => Some(point),
            Self::Entity { entity, offset } => {
                world.entity_kernel().look_up(entity).ok()?;
                let mover = world.script_mover_by_number(entity.slot())?;
                let frame = &mover.state.lerp;
                Some(offset_point(frame.pos.base_point, frame.apos.base_point, offset))
            }
            Self::Player {
                client,
                life,
                offset,
            } => {
                let meta = world.client_meta(client)?;
                let same_life = meta.life_sequence == life;
                if !same_life || meta.lifecycle != crate::PlayerSessionStage::Alive {
                    return None;
                }
                let view = world.avatar(client)?;
                Some(offset_point(view.origin, view.look_angles, offset))
            }
        }
    }
}

/// `base` pushed by `shift`, where `shift` is measured along the axes the `turn` faces.
fn offset_point(base: [f32; 3], turn: [f32; 3], shift: [f32; 3]) -> [f32; 3] {
    let axis = game_math::euler_to_rows(turn);
    core::array::from_fn(|i| base[i] + shift[0] * axis[0][i] + shift[1] * axis[1][i] + shift[2] * axis[2][i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn a_zero_offset_is_the_origin() {
        let spot = offset_point([5.0, -6.0, 7.0], [10.0, 200.0, 0.0], [0.0; 3]);
        assert_eq!(spot, [5.0, -6.0, 7.0]);
    }

    #[test]
    fn a_forward_offset_follows_the_yaw() {
        assert!(close(offset_point([0.0; 3], [0.0; 3], [10.0, 0.0, 0.0]), [10.0, 0.0, 0.0]));
        assert!(close(offset_point([1.0, 1.0, 1.0], [0.0, 90.0, 0.0], [10.0, 0.0, 0.0]), [1.0, 11.0, 1.0]));
    }

    #[test]
    fn an_up_offset_stays_up_when_level() {
        assert!(close(offset_point([0.0; 3], [0.0, 45.0, 0.0], [0.0, 0.0, 8.0]), [0.0, 0.0, 8.0]));
    }

    #[test]
    fn only_a_point_says_it_is_one() {
        assert!(MissileTarget::Point([0.0; 3]).is_point());
        let player = MissileTarget::Player {
            client: crate::SeatNo(1),
            life: crate::LifeCount::default(),
            offset: [0.0; 3],
        };
        assert!(!player.is_point());
    }
}
