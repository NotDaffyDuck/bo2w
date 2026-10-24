//! bo2mp: Black Ops II's camera shakes (its CG_StartShakeCamera,
//! CG_UpdateCameraShake and CG_ShakeCamera). Four slots; each shake fades
//! out over its length and, given a radius, is weaker the farther the view
//! is from where it started. The strongest one turns the view by three sine
//! waves. The Hellstorm's boost (EV_REMOTE_MISSILE_BOOST) starts one while he
//! rides a missile; a script's earthquake (EV_EARTHQUAKE) starts one for all
//! (or the one player it names). A weapon's own shakes start one where it
//! fires and where its projectile goes off; a vehicle gunner's shot starts
//! one more where the vehicle stands. A bullet, a missile or a blast
//! striking the riot shield he bears starts one at his view.

use bevy::prelude::*;
use entity_t6::event as ev;
use net::{FrameClock, LocalPresentClient, PresentedSnapshot};

const SLOTS: usize = 4;
/// BO2's boost shake: 0.3 for a second.
const BOOST_SCALE: f32 = 0.3;
const BOOST_MS: i32 = 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shake {
    time: i32,
    scale: f32,
    length: i32,
    radius: f32,
    src: [f32; 3],
    size: f32,
    rumble_scale: f32,
}

impl Shake {
    /// Whether it still shakes at `now` from `view`; if so, how hard
    /// (`size`, negative once the view is past its radius) and its faded
    /// strength (`rumble_scale`).
    fn update(&mut self, now: i32, view: [f32; 3]) -> bool {
        let elapsed = now.wrapping_sub(self.time);
        if elapsed < 0 || elapsed >= self.length {
            return false;
        }
        let spatial = if self.radius > 0.0 {
            let d = [view[0] - self.src[0], view[1] - self.src[1], view[2] - self.src[2]];
            1.0 - game_math::vec3_length(d) / self.radius
        } else {
            1.0
        };
        let amp = (1.0 - elapsed as f32 / self.length as f32) * self.scale;
        if amp <= 0.0 {
            return false;
        }
        self.size = if spatial < 0.0 { spatial / amp } else { amp * spatial };
        self.rumble_scale = amp;
        true
    }

    fn free(&self, now: i32) -> bool {
        self.time > now || self.time.wrapping_add(self.length) <= now
    }
}

#[derive(Resource, Clone, Debug)]
pub struct CameraShakes {
    shakes: [Shake; SLOTS],
    phase: f32,
    seed: u32,
    /// The view the shakes last turned: BO2 starts one from its last
    /// frame's view (its refdef).
    view: [f32; 3],
}

impl Default for CameraShakes {
    fn default() -> Self {
        Self {
            shakes: [Shake::default(); SLOTS],
            phase: 0.0,
            seed: 0x9e37_79b9,
            view: [0.0; 3],
        }
    }
}

impl CameraShakes {
    /// A random number from -1 to 1.
    fn crandom(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        x as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    /// Starts a shake of `scale` for `duration_ms` from `src` (radius 0:
    /// as hard everywhere). It takes a free slot; with none free it replaces
    /// the one farthest past its radius, or is dropped if none is.
    pub(crate) fn start(
        &mut self,
        scale: f32,
        duration_ms: i32,
        src: [f32; 3],
        radius: f32,
        now: i32,
        view: [f32; 3],
    ) {
        if scale <= 0.0 || duration_ms < 1 {
            return;
        }
        for shake in &mut self.shakes {
            shake.update(now, view);
        }
        let slot = self.shakes.iter().position(|s| s.free(now)).or_else(|| {
            self.shakes
                .iter()
                .enumerate()
                .filter(|(_, s)| s.size < 0.0)
                .min_by(|a, b| a.1.size.total_cmp(&b.1.size))
                .map(|(i, _)| i)
        });
        let Some(slot) = slot else {
            return;
        };
        self.shakes[slot] = Shake {
            time: now,
            scale,
            length: duration_ms,
            radius,
            src,
            ..Shake::default()
        };
    }

    /// The view's angles turned by the strongest shake at `now` from `view`.
    /// While nothing shakes the waves' phase is picked anew.
    pub(crate) fn apply(&mut self, angles: &mut [f32; 3], now: i32, view: [f32; 3]) {
        self.view = view;
        let mut best = 0.0f32;
        let mut amp = 0.0f32;
        for shake in &mut self.shakes {
            if shake.update(now, view) && shake.size > best {
                best = shake.size;
                amp = shake.rumble_scale;
            }
        }
        if best <= 0.0 {
            self.phase = self.crandom() * std::f32::consts::PI;
            return;
        }
        let best = best.min(1.0);
        let t = now as f32 * 0.001_666_666_7;
        angles[0] += (t * 25.132_742 + self.phase).sin() * amp * 18.0 * best;
        angles[1] += (t * 47.123_89 + self.phase).sin() * amp * 16.0 * best;
        angles[2] += (t * 37.699_112 + self.phase).sin() * amp * 10.0 * best;
    }
}

/// The Hellstorm's rider boosted it, or it began coasting still armed: if
/// he rides a missile his view shakes (BO2 starts it on any missile he
/// rides, from his view, as hard everywhere).
pub(crate) fn remote_missile_boost(
    _boost: On<net::EntityRemoteMissileBoost>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    clock: Res<FrameClock>,
    mut shakes: ResMut<CameraShakes>,
) {
    let now = clock.time();
    if let Some(pose) = super::third_person::remote_missile_camera(&presented, local.0, now) {
        shakes.start(BOOST_SCALE, BOOST_MS, pose.origin, 0.0, now, pose.origin);
    }
}

/// A script's earthquake: a shake of its scale for its length from where
/// it went off, weaker out to its radius, started from the last view.
pub(crate) fn earthquake(
    quake: On<net::EntityEarthquake>,
    clock: Res<FrameClock>,
    mut shakes: ResMut<CameraShakes>,
) {
    let p = quake.event.payload;
    shakes.start_earthquake(p.origin, p.origin2, p.event_parm, clock.time());
}

/// A projectile went off (BO2's CG_ProjectileExplosionEvent): its weapon's
/// explosion shake from the blast. A flashbang's own event starts none.
pub(crate) fn explosion(
    boom: On<net::EntityExplosion>,
    prediction: Option<Res<net::ClientPredictionState>>,
    clock: Res<FrameClock>,
    mut shakes: ResMut<CameraShakes>,
) {
    let e = boom.event;
    if e.event.0 == i32::from(ev::FLASHBANG_EXPLODE) {
        return;
    }
    let Some(prediction) = prediction else {
        return;
    };
    let shake = prediction.0.world().gun_facts().get(e.payload.weapon).shake;
    shakes.start_weapon(shake.explosion, e.payload.origin, clock.time());
}

/// A gun fired (BO2's CG_FireWeapon): its fire shake from the shooter. A
/// vehicle gunner's adds BO2's own small one from where the vehicle stands.
pub(crate) fn weapon_fire(
    fire: On<net::EntityWeaponFire>,
    prediction: Option<Res<net::ClientPredictionState>>,
    clock: Res<FrameClock>,
    mut shakes: ResMut<CameraShakes>,
) {
    let Some(prediction) = prediction else {
        return;
    };
    let p = fire.event.payload;
    let shake = prediction.0.world().gun_facts().get(p.weapon).shake;
    shakes.start_weapon(shake.fire, p.origin, clock.time());
    if u8::try_from(fire.event.event.0).is_ok_and(|e| ev::gunner_of_fire_event(e).is_some()) {
        shakes.start_gunner(p.origin2, clock.time());
    }
}

/// The shake every vehicle gunner's shot starts (CG_EntityEvent's gunner
/// fire: 0.05 for 100 ms, out to 100 from the vehicle).
const GUNNER_SCALE: f32 = 0.05;
const GUNNER_MS: i32 = 100;
const GUNNER_RADIUS: f32 = 100.0;

/// A bullet struck the riot shield he bears (BO2's
/// CG_BulletHitClientShieldEvent; the event comes to him alone): a shake
/// from his view, and the riotshield_impact rumble.
pub(crate) fn shield_hit(
    hit: On<net::EntityBulletHit>,
    local: Res<LocalPresentClient>,
    clock: Res<FrameClock>,
    mut shakes: ResMut<CameraShakes>,
    mut rumbles: MessageWriter<audio::CodeRumbleT6>,
) {
    let e = hit.event;
    if shakes.start_shield(e.event.0, e.payload.event_parm, false, clock.time()) {
        shield_rumble(&local, &mut rumbles);
    }
}

/// BO2 rumbles his pad (riotshield_impact) as each shield shake starts.
fn shield_rumble(local: &LocalPresentClient, rumbles: &mut MessageWriter<audio::CodeRumbleT6>) {
    if let Ok(number) = u16::try_from(local.0.0) {
        rumbles.write(audio::CodeRumbleT6 { name: "riotshield_impact", number });
    }
}

/// A missile struck a riot shield, or a blast it partly stopped (BO2's
/// CG_ExplosiveImpactOnShieldEvent and CG_ExplosiveSplashOnShieldEvent):
/// the event goes to all, and shakes only the bearer it names.
pub(crate) fn shield_blast(
    blast: On<net::EntityShieldBlast>,
    local: Res<LocalPresentClient>,
    clock: Res<FrameClock>,
    mut shakes: ResMut<CameraShakes>,
    mut rumbles: MessageWriter<audio::CodeRumbleT6>,
) {
    let e = blast.event;
    let his = e.payload.number == local.0.0 as i32;
    if shakes.start_shield(e.event.0, e.payload.event_parm, his, clock.time()) {
        shield_rumble(&local, &mut rumbles);
    }
}

/// The shieldImpact*Shake* dvars' defaults: every shield shake starts at the
/// view and fades out to 100 from it.
const SHIELD_RADIUS: f32 = 100.0;
/// A blast that would have done more than this unshielded shakes harder
/// (shieldImpactExplosionThreshold).
const SHIELD_BLAST_THRESHOLD: i32 = 85;

/// The shake (scale, milliseconds) a hit on his shield starts: a bullet's,
/// a missile striking it, or a blast's (`parm`, what it would have done
/// unshielded). `his`: the event names him.
fn shield_shake(event: i32, parm: i32, his: bool) -> Option<(f32, i32)> {
    match u8::try_from(event).ok()? {
        ev::BULLET_HIT_CLIENT_SHIELD => Some((0.25, 500)),
        ev::EXPLOSIVE_IMPACT_ON_SHIELD if his => Some((0.5, 1_500)),
        ev::EXPLOSIVE_SPLASH_ON_SHIELD if his && parm > SHIELD_BLAST_THRESHOLD => Some((0.5, 1_250)),
        ev::EXPLOSIVE_SPLASH_ON_SHIELD if his => Some((0.4, 800)),
        _ => None,
    }
}

impl CameraShakes {
    /// A hit on his shield: its shake from the last view. True when it shook.
    fn start_shield(&mut self, event: i32, parm: i32, his: bool, now: i32) -> bool {
        let shake = shield_shake(event, parm, his);
        if std::env::var_os("BO2MP_SHIELDLOG").is_some() {
            diag::info!(Fpv, "bo2mp shield: event {event:?} parm {parm} his {his}: shake {shake:?}");
        }
        if let Some((scale, ms)) = shake {
            let view = self.view;
            self.start(scale, ms, view, SHIELD_RADIUS, now, view);
        }
        shake.is_some()
    }

    /// The earthquake event's fields: the scale and the radius ride in
    /// origin2, the length in milliseconds in event_parm.
    fn start_earthquake(&mut self, origin: [f32; 3], origin2: [f32; 3], duration_ms: i32, now: i32) {
        let view = self.view;
        self.start(origin2[0], duration_ms, origin, origin2[1], now, view);
    }

    /// A vehicle gunner's shot: its shake from where the vehicle stands.
    fn start_gunner(&mut self, vehicle: [f32; 3], now: i32) {
        let view = self.view;
        self.start(GUNNER_SCALE, GUNNER_MS, vehicle, GUNNER_RADIUS, now, view);
    }

    /// One of a weapon's shakes from `src`, started from the last view; BO2
    /// skips one without a length or a radius.
    fn start_weapon(&mut self, shake: weapon_t6::facts::CameraShake, src: [f32; 3], now: i32) {
        if !shake.is_set() {
            return;
        }
        let view = self.view;
        self.start(shake.scale, shake.duration_ms, src, shake.radius as f32, now, view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: [f32; 3] = [0.0; 3];

    fn shaken(shakes: &mut CameraShakes, now: i32, view: [f32; 3]) -> [f32; 3] {
        let mut angles = [0.0; 3];
        shakes.apply(&mut angles, now, view);
        angles
    }

    #[test]
    fn a_shake_fades_out_over_its_length() {
        let mut slot = Shake { time: 1_000, scale: 0.3, length: 1_000, ..Shake::default() };
        assert!(slot.update(1_000, AT));
        assert_eq!(slot.rumble_scale, 0.3);
        assert!(slot.update(1_500, AT));
        assert!((slot.rumble_scale - 0.15).abs() < 1e-6);
        assert!((slot.size - 0.15).abs() < 1e-6);
        assert!(!slot.update(2_000, AT));
        assert!(!slot.update(999, AT));
    }

    #[test]
    fn a_radius_weakens_it_with_distance() {
        let mut slot = Shake { time: 0, scale: 1.0, length: 1_000, radius: 100.0, ..Shake::default() };
        assert!(slot.update(0, [50.0, 0.0, 0.0]));
        assert!((slot.size - 0.5).abs() < 1e-6);
        // Past its radius it still counts, but below nothing.
        assert!(slot.update(500, [150.0, 0.0, 0.0]));
        assert!((slot.size - -1.0).abs() < 1e-6);
    }

    #[test]
    fn a_shake_takes_the_first_free_slot() {
        let mut shakes = CameraShakes::default();
        shakes.start(0.3, 1_000, AT, 0.0, 5_000, AT);
        assert_eq!(shakes.shakes[0].time, 5_000);
        shakes.start(0.5, 200, AT, 0.0, 5_100, AT);
        assert_eq!(shakes.shakes[1].scale, 0.5);
        // The second one is over: its slot is free again.
        shakes.start(0.2, 100, AT, 0.0, 5_400, AT);
        assert_eq!(shakes.shakes[1].scale, 0.2);
        // Nothing for no scale or no length.
        shakes.start(0.0, 100, AT, 0.0, 5_400, AT);
        shakes.start(0.4, 0, AT, 0.0, 5_400, AT);
        assert_eq!(shakes.shakes[2], Shake::default());
    }

    #[test]
    fn a_full_set_replaces_the_one_farthest_out_or_drops_it() {
        let mut shakes = CameraShakes::default();
        for _ in 0..SLOTS {
            shakes.start(1.0, 1_000, AT, 0.0, 0, AT);
        }
        shakes.start(0.7, 1_000, AT, 0.0, 10, AT);
        assert!(shakes.shakes.iter().all(|s| s.scale == 1.0));
        // Two out of range: the one farther out goes.
        shakes.shakes[1].radius = 10.0;
        shakes.shakes[3].radius = 10.0;
        shakes.shakes[3].src = [100.0, 0.0, 0.0];
        shakes.start(0.7, 1_000, AT, 0.0, 10, [30.0, 0.0, 0.0]);
        assert_eq!(shakes.shakes[3].scale, 0.7);
        assert_eq!(shakes.shakes[1].scale, 1.0);
    }

    #[test]
    fn the_strongest_shake_turns_the_view_by_its_waves() {
        let mut shakes = CameraShakes::default();
        shakes.start(0.3, 1_000, AT, 0.0, 0, AT);
        shakes.start(0.1, 1_000, AT, 0.0, 0, AT);
        let phase = shakes.phase;
        let now = 250;
        let t = now as f32 * 0.001_666_666_7;
        let amp = 0.3 * 0.75;
        let angles = shaken(&mut shakes, now, AT);
        let want = [
            (t * 25.132_742 + phase).sin() * amp * 18.0 * amp,
            (t * 47.123_89 + phase).sin() * amp * 16.0 * amp,
            (t * 37.699_112 + phase).sin() * amp * 10.0 * amp,
        ];
        for (got, want) in angles.iter().zip(want) {
            assert!((got - want).abs() < 1e-4, "{angles:?} {want}");
        }
        // The phase holds while it shakes, and is picked anew once it stops.
        assert_eq!(shakes.phase, phase);
        assert_eq!(shaken(&mut shakes, 1_000, AT), [0.0; 3]);
        assert_ne!(shakes.phase, phase);
    }

    #[test]
    fn an_earthquake_shakes_by_its_distance_from_the_last_view() {
        let mut shakes = CameraShakes::default();
        // The view was last turned 50 out from where a 200-wide quake
        // goes off.
        shaken(&mut shakes, 900, [50.0, 0.0, 0.0]);
        shakes.start_earthquake([0.0; 3], [0.2, 200.0, 0.0], 200, 1_000);
        assert_eq!(shakes.shakes[0].length, 200);
        assert_eq!(shakes.shakes[0].radius, 200.0);
        let phase = shakes.phase;
        let angles = shaken(&mut shakes, 1_100, [50.0, 0.0, 0.0]);
        // Half way through, three quarters in: 0.1 faded, 0.075 strong.
        let t = 1_100.0f32 * 0.001_666_666_7;
        let want = (t * 25.132_742 + phase).sin() * 0.1 * 18.0 * 0.075;
        assert!((angles[0] - want).abs() < 1e-5, "{angles:?} {want}");
        // Past its radius nothing turns.
        assert_eq!(shaken(&mut shakes, 1_150, [300.0, 0.0, 0.0]), [0.0; 3]);
    }

    #[test]
    fn a_weapons_shake_needs_a_length_and_a_radius() {
        use weapon_t6::facts::CameraShake;
        let mut shakes = CameraShakes::default();
        shaken(&mut shakes, 900, [100.0, 0.0, 0.0]);
        // A frag going off 100 out: 0.5 for 500 ms out to 800.
        let frag = CameraShake { scale: 0.5, duration_ms: 500, radius: 800 };
        shakes.start_weapon(frag, AT, 1_000);
        assert_eq!(shakes.shakes[0].radius, 800.0);
        assert_eq!(shakes.shakes[0].length, 500);
        assert_ne!(shaken(&mut shakes, 1_100, [100.0, 0.0, 0.0]), [0.0; 3]);
        // A player's gun: no length, no radius, no shake.
        let mut calm = CameraShakes::default();
        calm.start_weapon(CameraShake { scale: 0.5, duration_ms: 500, radius: 0 }, AT, 0);
        calm.start_weapon(CameraShake { scale: 0.5, duration_ms: 0, radius: 512 }, AT, 0);
        assert_eq!(calm.shakes, [Shake::default(); SLOTS]);
    }

    #[test]
    fn a_gunners_shot_shakes_those_near_the_vehicle() {
        let mut near = CameraShakes::default();
        near.start_gunner([40.0, 0.0, 0.0], 0);
        let a = shaken(&mut near, 20, AT);
        assert!(a.iter().any(|v| v.abs() > 0.0), "{a:?}");
        assert!(shaken(&mut near, 101, AT) == [0.0; 3]);
        let mut far = CameraShakes::default();
        far.start_gunner([400.0, 0.0, 0.0], 0);
        assert_eq!(shaken(&mut far, 20, AT), [0.0; 3]);
    }

    #[test]
    fn a_hit_on_his_shield_shakes_his_view() {
        // A bullet's comes to him alone: it always shakes.
        assert_eq!(shield_shake(i32::from(ev::BULLET_HIT_CLIENT_SHIELD), 0, false), Some((0.25, 500)));
        // A missile's and a blast's only shake the one they name.
        assert_eq!(shield_shake(i32::from(ev::EXPLOSIVE_IMPACT_ON_SHIELD), 0, true), Some((0.5, 1_500)));
        assert_eq!(shield_shake(i32::from(ev::EXPLOSIVE_IMPACT_ON_SHIELD), 0, false), None);
        assert_eq!(shield_shake(i32::from(ev::EXPLOSIVE_SPLASH_ON_SHIELD), 86, true), Some((0.5, 1_250)));
        assert_eq!(shield_shake(i32::from(ev::EXPLOSIVE_SPLASH_ON_SHIELD), 85, true), Some((0.4, 800)));
        assert_eq!(shield_shake(i32::from(ev::EXPLOSIVE_SPLASH_ON_SHIELD), 200, false), None);
        assert_eq!(shield_shake(i32::from(ev::BULLET_HIT_CLIENT_SMALL), 0, true), None);
        // It starts where the view last was, and fades out 100 from it.
        let mut shakes = CameraShakes::default();
        shaken(&mut shakes, 900, [40.0, 0.0, 0.0]);
        // A shake that starts also rumbles his pad; a blast on another's shield neither.
        assert!(!shakes.start_shield(i32::from(ev::EXPLOSIVE_IMPACT_ON_SHIELD), 0, false, 1_000));
        assert!(shakes.start_shield(i32::from(ev::BULLET_HIT_CLIENT_SHIELD), 0, false, 1_000));
        assert_eq!(shakes.shakes[0].src, [40.0, 0.0, 0.0]);
        assert_eq!(shakes.shakes[0].radius, 100.0);
        assert_ne!(shaken(&mut shakes, 1_100, [40.0, 0.0, 0.0]), [0.0; 3]);
        assert_eq!(shaken(&mut shakes, 1_200, [160.0, 0.0, 0.0]), [0.0; 3]);
    }

    #[test]
    fn a_shake_past_full_strength_turns_it_as_full() {
        let mut shakes = CameraShakes::default();
        shakes.start(4.0, 1_000, AT, 0.0, 0, AT);
        let phase = shakes.phase;
        let t = 100.0f32 * 0.001_666_666_7;
        let angles = shaken(&mut shakes, 100, AT);
        // 3.6 strong, counted as 1.
        assert!((angles[0] - (t * 25.132_742 + phase).sin() * 3.6 * 18.0).abs() < 1e-3);
    }
}
