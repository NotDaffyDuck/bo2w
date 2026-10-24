//! Controller vibration: a rumble asset is a pair of motor curves over a
//! duration; the systems here mix the ones that are playing and hand the
//! strongest level to the active gamepad.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use asset_audio::SoundCatalog;
use asset_core::AssetNamespace;
use bevy::input::gamepad::{GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::prelude::*;
use net::{LocalPresentClient, PresentedSnapshot};

/// Motor level over normalised time: (position 0..=1, level) points in order.
type Curve = Vec<(f32, f32)>;

/// Black Ops II's client keeps this many rumbles at once.
const RUMBLE_SLOTS_T6: usize = 32;
/// The most points a motor curve may have.
const MAX_CURVE_POINTS: usize = 16;
/// Each request to the pad lasts this long; it is renewed well before.
const PAD_REQUEST: Duration = Duration::from_millis(160);
const PAD_RENEW_MS: i32 = 80;
/// The most rumbles the plain (non-slot) mixer holds.
const MIXER_LIMIT: usize = 32;

const SILENT: GamepadRumbleIntensity = GamepadRumbleIntensity {
    strong_motor: 0.0,
    weak_motor: 0.0,
};

/// One prepared rumble asset.
#[derive(Clone, Debug)]
pub(crate) struct Rumble {
    name: String,
    duration_ms: i32,
    /// The strong (low-frequency) motor.
    low: Curve,
    /// The weak (high-frequency) motor.
    high: Curve,
    /// How far a broadcast rumble is felt.
    range: f32,
    fade_with_distance: bool,
    /// Whether people other than the one it is on feel it.
    broadcast: bool,
}

/// The text of a rumble-folder rawfile out of a bank.
fn rumble_text<'a>(
    bank: &'a SoundCatalog,
    namespace: AssetNamespace,
    name: &str,
) -> Result<&'a str, String> {
    let mut key = format!("rumble/{name}");
    if namespace == AssetNamespace::T6 {
        key.make_ascii_lowercase();
    }
    let bytes = bank
        .rawfiles
        .get(&(namespace, key.clone()))
        .ok_or_else(|| format!("missing {key}"))?;
    std::str::from_utf8(bytes).map_err(|_| format!("invalid text in {key}"))
}

/// Reads one `RUMBLEGRAPHFILE` motor curve.
fn read_curve(
    bank: &SoundCatalog,
    namespace: AssetNamespace,
    file: Option<&str>,
) -> Result<Curve, String> {
    let file = file
        .filter(|file| !file.is_empty())
        .ok_or_else(|| "missing rumble graph name".to_owned())?;
    let text = rumble_text(bank, namespace, file)?.trim_end_matches('\0');
    let mut words = text.split_whitespace();
    if words.next() != Some("RUMBLEGRAPHFILE") {
        return Err(format!("invalid rumble graph header: {file}"));
    }
    let count = words
        .next()
        .and_then(|word| word.parse::<usize>().ok())
        .filter(|count| (1..=MAX_CURVE_POINTS).contains(count))
        .ok_or_else(|| format!("invalid rumble graph count: {file}"))?;

    let mut next_number = || {
        words
            .next()
            .and_then(|word| word.parse::<f32>().ok())
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("invalid rumble graph point: {file}"))
    };
    let mut curve: Curve = Vec::with_capacity(count);
    for _ in 0..count {
        let position = next_number()?;
        let level = next_number()?;
        if curve.last().is_some_and(|&(last, _)| last > position) {
            return Err(format!("unordered rumble graph: {file}"));
        }
        curve.push((position, level));
    }
    Ok(curve)
}

impl Rumble {
    pub(crate) fn prepare(
        bank: &SoundCatalog,
        namespace: AssetNamespace,
        name: &str,
    ) -> Result<Arc<Self>, String> {
        let definition = rumble_text(bank, namespace, name)?.trim_end_matches('\0');
        let mut parts = definition.split('\\');
        if parts.next() != Some("RUMBLE") {
            return Err(format!("invalid rumble header: {name}"));
        }

        let (mut duration, mut low, mut high) = (None, None, None);
        let (mut range, mut fade_with_distance, mut broadcast) = (0.0, false, false);
        let flag = |value: &str| value.parse::<i32>().is_ok_and(|number| number != 0);
        while let Some(field) = parts.next() {
            let value = parts
                .next()
                .ok_or_else(|| format!("missing {field} value: {name}"))?;
            match field {
                "duration" => duration = value.parse::<f32>().ok(),
                "lowRumbleFile" => low = Some(value),
                "highRumbleFile" => high = Some(value),
                "range" => range = value.parse::<f32>().unwrap_or(0.0),
                "fadeWithDistance" => fade_with_distance = flag(value),
                "broadcast" => broadcast = flag(value),
                _ => {}
            }
        }

        let seconds = duration
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .ok_or_else(|| format!("invalid rumble duration: {name}"))?;
        Ok(Arc::new(Self {
            name: name.to_owned(),
            duration_ms: (seconds * 1000.0).max(1.0) as i32,
            low: read_curve(bank, namespace, low)?,
            high: read_curve(bank, namespace, high)?,
            range,
            fade_with_distance,
            broadcast,
        }))
    }

    /// Both motors `elapsed_ms` into the rumble.
    fn intensity(&self, elapsed_ms: i32) -> GamepadRumbleIntensity {
        let at = (elapsed_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0);
        GamepadRumbleIntensity {
            strong_motor: level_at(&self.low, at),
            weak_motor: level_at(&self.high, at),
        }
    }
}

/// The curve's level at `at`, interpolated between its neighbours and held
/// at the first and last point outside them.
fn level_at(curve: &[(f32, f32)], at: f32) -> f32 {
    let (first, last) = (curve[0], curve[curve.len() - 1]);
    let level = if at <= first.0 {
        first.1
    } else if let Some(pair) = curve.windows(2).find(|pair| at <= pair[1].0) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        let along = if x1 > x0 { (at - x0) / (x1 - x0) } else { 1.0 };
        y0 + (y1 - y0) * along
    } else {
        last.1
    };
    level.clamp(0.0, 1.0)
}

#[derive(Message)]
pub(crate) struct PlayRumble {
    pub bank_revision: u64,
    pub rumble: Arc<Rumble>,
}

/// A rumble Black Ops II's own client code starts on a player (the
/// dive-to-prone landing, a hit on a riot shield). It plays as a script's
/// rumble on that entity would.
#[derive(Message, Clone, Copy, Debug)]
pub struct CodeRumbleT6 {
    pub name: &'static str,
    pub number: u16,
}

/// Black Ops II's own client code stopping a rumble it started on a player
/// (the slide's `slide_rumble` as the slide ends). The first rumble of that
/// name on the player stops; a looping flag a script set stays.
#[derive(Message, Clone, Copy, Debug)]
pub struct CodeRumbleStopT6 {
    pub name: &'static str,
    pub number: u16,
}

/// The plain mixer: what is playing on the pad right now.
#[derive(Default)]
struct RumblePlayback {
    owner: Option<(frame::WorldGeneration, sim::ClientId, sim::LifeSequence, u64)>,
    last_time: i32,
    /// (start ms, rumble) for each one playing.
    active: Vec<(i32, Arc<Rumble>)>,
    /// What was last sent to the pad.
    output: Option<(Entity, GamepadRumbleIntensity)>,
    selected: Option<Entity>,
    next_refresh_ms: i32,
}

impl RumblePlayback {
    /// Starts `rumble`, first dropping the one nearest its end if the mixer is full.
    fn push(&mut self, now: i32, rumble: Arc<Rumble>) {
        if self.active.len() >= MIXER_LIMIT {
            let soonest = self
                .active
                .iter()
                .enumerate()
                .min_by_key(|(_, (start, rumble))| start.saturating_add(rumble.duration_ms))
                .map(|(index, _)| index);
            if let Some(index) = soonest {
                self.active.swap_remove(index);
            }
        }
        self.active.push((now, rumble));
    }

    /// Stops whatever was last sent to the pad.
    fn silence(&mut self, requests: &mut MessageWriter<GamepadRumbleRequest>) {
        if let Some((gamepad, _)) = self.output.take() {
            requests.write(GamepadRumbleRequest::Stop { gamepad });
        }
    }
}

pub(crate) fn register(app: &mut App) {
    app.add_message::<PlayRumble>()
        .add_message::<CodeRumbleT6>()
        .add_message::<CodeRumbleStopT6>()
        .init_resource::<RumblesT6>()
        .init_resource::<CuesT6>()
        .add_observer(queue_rumble_t6)
        .add_systems(
            Update,
            (
                update_t6
                    .run_if(crate::shellshock::bank_is_t6)
                    .before(update),
                update,
            )
                .in_set(net::ClientSet::Effects)
                .after(crate::entity_events::play_viewmodel_notetrack_messages),
        );
}

fn now_ms(time: &Time) -> i32 {
    (time.elapsed_secs_f64() * 1000.0) as i32
}

fn update(
    mut requests: MessageReader<PlayRumble>,
    bank: Option<Res<crate::SoundBank>>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    generation: Res<frame::WorldGeneration>,
    time: Res<Time>,
    devices: Res<frame::InputDevices>,
    mut tests: MessageReader<frame::TestControllerRumble>,
    gamepads: Query<Entity, With<Gamepad>>,
    mut state: Local<RumblePlayback>,
    mut output: MessageWriter<GamepadRumbleRequest>,
    settings: Res<frame::GameSettings>,
    active: Option<Res<frame::ActivePad>>,
    (t6, namespace): (
        Option<Res<RumblesT6>>,
        Option<Res<crate::ambient::SoundBankNamespace>>,
    ),
) {
    // Black Ops II plays its rumbles through its own slots (`update_t6`).
    let t6 = t6.filter(|_| crate::shellshock::bank_is_t6(namespace));
    let now = now_ms(&time);

    let selected = active
        .and_then(|active| active.0)
        .filter(|pad| gamepads.contains(*pad));
    if state.selected != selected {
        state.active.clear();
        state.silence(&mut output);
        state.selected = selected;
        state.next_refresh_ms = 0;
    }
    let pad = match selected {
        Some(pad) if settings.pad_vibration && devices.focused => pad,
        _ => {
            requests.clear();
            tests.clear();
            state.active.clear();
            state.silence(&mut output);
            return;
        }
    };

    // Whose rumbles these are: this client, this life, this bank.
    let owner = bank.as_ref().and_then(|bank| {
        let meta = presented.snapshot()?.meta.for_client(local.0)?;
        (meta.lifecycle == sim::ClientLifecycle::Alive).then_some((
            *generation,
            local.0,
            meta.life_sequence,
            bank.0.revision(),
        ))
    });
    if owner != state.owner || now < state.last_time {
        state.active.clear();
        state.silence(&mut output);
        state.owner = owner;
    }
    state.last_time = now;
    state
        .active
        .retain(|(start, rumble)| now.saturating_sub(*start) < rumble.duration_ms);

    if t6.is_some() {
        requests.clear();
    }
    for request in requests.read() {
        if owner.is_some_and(|owner| owner.3 == request.bank_revision) {
            state.push(now, Arc::clone(&request.rumble));
        }
    }
    if tests.read().next().is_some() {
        state.push(
            now,
            Arc::new(Rumble {
                name: String::new(),
                duration_ms: 400,
                low: vec![(0.0, 0.6), (1.0, 0.0)],
                high: vec![(0.0, 0.4), (1.0, 0.0)],
                range: 0.0,
                fade_with_distance: false,
                broadcast: false,
            }),
        );
        tests.clear();
    }

    // The strongest of every playing rumble drives each motor.
    let mut level = SILENT;
    for (start, rumble) in &state.active {
        let now_level = rumble.intensity(now.saturating_sub(*start));
        level.strong_motor = level.strong_motor.max(now_level.strong_motor);
        level.weak_motor = level.weak_motor.max(now_level.weak_motor);
    }
    if let Some(t6) = &t6 {
        level.strong_motor = level.strong_motor.max(t6.motors.strong_motor);
        level.weak_motor = level.weak_motor.max(t6.motors.weak_motor);
    }

    if level.strong_motor == 0.0 && level.weak_motor == 0.0 {
        state.silence(&mut output);
        state.next_refresh_ms = 0;
    } else if state.output.is_none() || now >= state.next_refresh_ms {
        output.write(GamepadRumbleRequest::Stop { gamepad: pad });
        output.write(GamepadRumbleRequest::Add {
            gamepad: pad,
            duration: PAD_REQUEST,
            intensity: level,
        });
        state.output = Some((pad, level));
        state.next_refresh_ms = now.saturating_add(PAD_RENEW_MS);
    }
}

/// What a Black Ops II rumble shakes from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SourceT6 {
    /// An entity: its number, and the entity it was when the rumble began.
    Entity(u16, Option<Entity>),
    /// A spot in the world (whole units).
    Position([f32; 3]),
}

impl SourceT6 {
    fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Entity(a, _), Self::Entity(b, _)) => a == b,
            (Self::Position(a), Self::Position(b)) => a == b,
            _ => false,
        }
    }

    fn number(self) -> Option<u16> {
        match self {
            Self::Entity(number, _) => Some(number),
            Self::Position(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
struct SlotT6 {
    rumble: Arc<Rumble>,
    source: SourceT6,
    looping: bool,
    start: i32,
}

impl SlotT6 {
    fn ends_at(&self) -> i32 {
        self.start.saturating_add(self.rumble.duration_ms)
    }
}

/// A script's rumble command, as its event reached this client.
#[derive(Clone, Debug)]
enum CueT6 {
    Play {
        name: String,
        source: SourceT6,
        looping: bool,
    },
    Stop {
        name: String,
        number: u16,
    },
    StopAll,
}

#[derive(Resource, Default)]
pub(crate) struct CuesT6(Vec<CueT6>);

/// Black Ops II's rumbles on this client: its slots, the entities whose
/// looping flag is set, and the motor levels they drive this frame.
#[derive(Resource)]
pub(crate) struct RumblesT6 {
    slots: Vec<Option<SlotT6>>,
    loop_flags: HashSet<u16>,
    owner: Option<(frame::WorldGeneration, sim::ClientId, u64)>,
    last_time: i32,
    motors: GamepadRumbleIntensity,
}

impl Default for RumblesT6 {
    fn default() -> Self {
        Self {
            slots: vec![None; RUMBLE_SLOTS_T6],
            loop_flags: HashSet::new(),
            owner: None,
            last_time: 0,
            motors: SILENT,
        }
    }
}

impl RumblesT6 {
    fn clear(&mut self) {
        self.slots.fill(None);
        self.motors = SILENT;
    }

    /// Starts a rumble the way the game's `CG_PlayRumble` does. One already
    /// started this frame from the same source the same way is not started
    /// twice. A free slot takes it; with none free, the rumble nearest its end
    /// goes (never a loop, never one on the viewer), else the first slot.
    fn play(
        &mut self,
        rumble: Arc<Rumble>,
        source: SourceT6,
        looping: bool,
        now: i32,
        own: Option<u16>,
    ) {
        let repeat = self.slots.iter().flatten().any(|slot| {
            slot.start == now
                && slot.looping == looping
                && slot.source.same(source)
                && slot.rumble.name == rumble.name
        });
        if repeat {
            return;
        }

        let free = self.slots.iter().position(Option::is_none);
        let index = free.unwrap_or_else(|| {
            let replaceable = |slot: &SlotT6| {
                !slot.looping && (own.is_none() || slot.source.number() != own)
            };
            self.slots
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| Some((index, slot.as_ref()?)))
                .filter(|(_, slot)| replaceable(slot))
                .min_by_key(|(_, slot)| slot.ends_at().saturating_sub(now))
                .map_or(0, |(index, _)| index)
        });
        self.slots[index] = Some(SlotT6 {
            rumble,
            source,
            looping,
            start: now,
        });
    }

    /// The game's `CG_StopRumble`: the first rumble of that name on the entity stops.
    fn stop(&mut self, name: &str, number: u16) {
        let hit = self.slots.iter_mut().find(|slot| {
            slot.as_ref()
                .is_some_and(|slot| slot.source.number() == Some(number) && slot.rumble.name == name)
        });
        if let Some(slot) = hit {
            *slot = None;
        }
    }

    /// One frame. Rumbles that ran their length stop (a loop starts over); a
    /// loop on an entity whose looping flag is clear stops, and so does a
    /// rumble on an entity that is gone. The motors take the strongest of the
    /// rest, a broadcast rumble weaker with distance from the view (or
    /// nothing past its range).
    fn tick(
        &mut self,
        now: i32,
        ear: Option<Vec3>,
        entity_now: impl Fn(u16) -> Option<Entity>,
        origin_of: impl Fn(Entity) -> Option<[f32; 3]>,
    ) {
        let mut motors = SILENT;
        for entry in &mut self.slots {
            let Some(slot) = entry else {
                continue;
            };
            let elapsed = now.saturating_sub(slot.start);
            let source_gone = match slot.source {
                SourceT6::Entity(number, began_as) => {
                    entity_now(number) != began_as
                        || (slot.looping && !self.loop_flags.contains(&number))
                }
                SourceT6::Position(_) => false,
            };
            let finished = !slot.looping && elapsed >= slot.rumble.duration_ms;
            if source_gone || finished {
                *entry = None;
                continue;
            }

            let into = if slot.looping {
                elapsed.rem_euclid(slot.rumble.duration_ms.max(1))
            } else {
                elapsed
            };
            let scale = if slot.rumble.broadcast {
                let origin = match slot.source {
                    SourceT6::Entity(_, began_as) => began_as.and_then(&origin_of),
                    SourceT6::Position(point) => Some(point),
                };
                match origin.zip(ear) {
                    Some((origin, ear)) => {
                        broadcast_scale(&slot.rumble, Vec3::from_array(origin).distance(ear))
                    }
                    None => 0.0,
                }
            } else {
                1.0
            };
            if scale <= 0.0 {
                continue;
            }
            let level = slot.rumble.intensity(into);
            motors.strong_motor = motors.strong_motor.max(level.strong_motor * scale);
            motors.weak_motor = motors.weak_motor.max(level.weak_motor * scale);
        }
        self.motors = motors;
    }
}

/// A broadcast rumble past its range is not felt; inside it, fully, or
/// fading to nothing at the range when it fades with distance.
fn broadcast_scale(rumble: &Rumble, distance: f32) -> f32 {
    if rumble.range <= 0.0 || distance > rumble.range {
        0.0
    } else if rumble.fade_with_distance {
        1.0 - distance / rumble.range
    } else {
        1.0
    }
}

/// A script's rumble event, its name read now (the alias slot it rides may be
/// reused later).
fn queue_rumble_t6(
    ev: On<net::EntityRumble>,
    adopted: Option<Res<net::LastAdoptedSnapshot>>,
    namespace: Option<Res<crate::ambient::SoundBankNamespace>>,
    mut cues: ResMut<CuesT6>,
) {
    use entity_t6::EntityEventKind as Kind;

    if ev.in_killcam || !crate::shellshock::bank_is_t6(namespace) {
        return;
    }
    let event = ev.event;
    let rumble_name = || {
        let index = u8::try_from(event.payload.event_parm).ok()?;
        let alias = adopted.as_ref()?.sound_alias_name(index)?;
        alias.strip_prefix("rumble:").map(str::to_ascii_lowercase)
    };
    let number = u16::try_from(event.payload.number).ok();
    let entity = (ev.entity != Entity::PLACEHOLDER).then_some(ev.entity);

    let cue = match event.event {
        Kind::STOP_ALL_RUMBLES => Some(CueT6::StopAll),
        Kind::STOP_RUMBLE => rumble_name()
            .zip(number)
            .map(|(name, number)| CueT6::Stop { name, number }),
        Kind::PLAY_RUMBLE_ON_ENT | Kind::PLAY_RUMBLELOOP_ON_ENT => {
            rumble_name().zip(number).map(|(name, number)| CueT6::Play {
                name,
                source: SourceT6::Entity(number, entity),
                looping: event.event == Kind::PLAY_RUMBLELOOP_ON_ENT,
            })
        }
        Kind::PLAY_RUMBLE_ON_POS | Kind::PLAY_RUMBLELOOP_ON_POS => {
            rumble_name().map(|name| CueT6::Play {
                name,
                source: SourceT6::Position(event.payload.origin.map(f32::trunc)),
                looping: event.event == Kind::PLAY_RUMBLELOOP_ON_POS,
            })
        }
        _ => None,
    };
    match cue {
        Some(cue) => cues.0.push(cue),
        None => diag::warn!(
            Audio,
            "audio: rumble event {:#x} on {} names no rumble (parm {}; typed gap)",
            event.event.0,
            event.payload.number,
            event.payload.event_parm
        ),
    }
}

/// Loads a named Black Ops II rumble, remembering a failure as `None`.
fn load_rumble_t6(bank: &SoundCatalog, name: &str) -> Option<Arc<Rumble>> {
    match Rumble::prepare(bank, AssetNamespace::T6, name) {
        Ok(rumble) if rumble.broadcast && rumble.range <= 0.0 => {
            diag::warn!(Audio, "audio: rumble `{name}` is broadcast with no range");
            None
        }
        Ok(rumble) => Some(rumble),
        Err(error) => {
            diag::warn!(Audio, "audio: rumble `{name}`: {error} (typed gap)");
            None
        }
    }
}

/// Black Ops II's rumbles (its `CG_UpdateRumble`). They are felt only while
/// this client sees through its own eyes and is not a spectator: the killcam
/// or spectating clears every one, loops too, and none comes back. A rumble
/// that is not broadcast is felt only from the viewer; a broadcast one from
/// anywhere in its range. The weapon's own (notetrack) rumbles share the slots.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_t6(
    (mut requests, mut code, mut code_stops): (
        MessageReader<PlayRumble>,
        MessageReader<CodeRumbleT6>,
        MessageReader<CodeRumbleStopT6>,
    ),
    (mut cues, mut rumbles): (ResMut<CuesT6>, ResMut<RumblesT6>),
    bank: Option<Res<crate::SoundBank>>,
    (presented, local, generation, view): (
        Res<PresentedSnapshot>,
        Res<LocalPresentClient>,
        Res<frame::WorldGeneration>,
        Res<frame::ViewSubject>,
    ),
    time: Res<Time>,
    (slots, runtimes): (Option<Res<net::CEntitySlots>>, Query<&net::CEntityRuntime>),
    listeners: Query<&Transform, With<crate::playback::AmbientListener>>,
    mut loaded: Local<(u64, HashMap<String, Option<Arc<Rumble>>>)>,
) {
    let now = now_ms(&time);
    let owner = bank
        .as_ref()
        .map(|bank| (*generation, local.0, bank.0.revision()));
    if owner != rumbles.owner || now < rumbles.last_time {
        rumbles.clear();
        rumbles.loop_flags.clear();
        rumbles.owner = owner;
    }
    rumbles.last_time = now;

    let own = u16::try_from(local.0.0).ok();
    let own_view = matches!(*view, frame::ViewSubject::Own { client } if client == local.0.0)
        && presented
            .player(local.0)
            .is_some_and(|state| state.pm_type != playerstate_t6::game::PM_TYPE_SPECTATOR);
    let Some(bank) = bank.filter(|_| own_view) else {
        requests.clear();
        code.clear();
        code_stops.clear();
        cues.0.clear();
        rumbles.clear();
        return;
    };
    if loaded.0 != bank.0.revision() {
        *loaded = (bank.0.revision(), HashMap::new());
    }

    let entity_now = |number: u16| slots.as_ref().and_then(|slots| slots.entity_for_number(number));

    // The weapon's own rumbles go on the viewer.
    if let Some(own) = own {
        let own_entity = entity_now(own);
        for request in requests.read() {
            if request.bank_revision == bank.0.revision() {
                let rumble = Arc::clone(&request.rumble);
                rumbles.play(rumble, SourceT6::Entity(own, own_entity), false, now, Some(own));
            }
        }
    } else {
        requests.clear();
    }
    for request in code.read() {
        cues.0.push(CueT6::Play {
            name: request.name.to_owned(),
            source: SourceT6::Entity(request.number, entity_now(request.number)),
            looping: false,
        });
    }

    for cue in std::mem::take(&mut cues.0) {
        match cue {
            CueT6::StopAll => rumbles.clear(),
            CueT6::Stop { name, number } => {
                rumbles.loop_flags.remove(&number);
                rumbles.stop(&name, number);
            }
            CueT6::Play {
                name,
                source,
                looping,
            } => {
                // The entity's looping flag is set whether or not this client feels it.
                if let Some(number) = source.number().filter(|_| looping) {
                    rumbles.loop_flags.insert(number);
                }
                let rumble = loaded
                    .1
                    .entry(name.clone())
                    .or_insert_with(|| load_rumble_t6(&bank.0, &name))
                    .clone();
                let Some(rumble) = rumble else {
                    continue;
                };
                // Not broadcast: felt only when it is on the viewer.
                let on_viewer = source.number().is_some() && source.number() == own;
                if rumble.broadcast || on_viewer {
                    rumbles.play(rumble, source, looping, now, own);
                }
            }
        }
    }
    for stop in code_stops.read() {
        rumbles.stop(stop.name, stop.number);
    }

    let ear = listeners.iter().next().map(|at| at.translation);
    rumbles.tick(now, ear, entity_now, |entity| {
        runtimes.get(entity).ok().map(|runtime| runtime.origin)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rumble(name: &str, ms: i32, broadcast: bool, range: f32, fade: bool) -> Arc<Rumble> {
        Arc::new(Rumble {
            name: name.to_owned(),
            duration_ms: ms,
            low: vec![(0.0, 1.0), (1.0, 1.0)],
            high: vec![(0.0, 0.5), (1.0, 0.5)],
            range,
            fade_with_distance: fade,
            broadcast,
        })
    }

    fn entity(raw: u32) -> Option<Entity> {
        Some(Entity::from_raw_u32(raw).unwrap())
    }

    /// Ticks with entity 3 present (as entity 7) and everything at the origin.
    fn tick_at(slots: &mut RumblesT6, now: i32, ear: Vec3) {
        let seven = entity(7).unwrap();
        slots.tick(now, Some(ear), |n| (n == 3).then_some(seven), |_| Some([0.0; 3]));
    }

    fn occupied(slots: &RumblesT6) -> usize {
        slots.slots.iter().flatten().count()
    }

    #[test]
    fn curves_interpolate_and_hold_their_ends() {
        let curve = [(0.2, 0.0), (0.6, 1.0)];
        assert_eq!(level_at(&curve, 0.0), 0.0);
        assert!((level_at(&curve, 0.4) - 0.5).abs() < 1e-5);
        assert_eq!(level_at(&curve, 0.9), 1.0);
        // Levels beyond the motor's range are clamped.
        assert_eq!(level_at(&[(0.0, 3.0)], 0.5), 1.0);
    }

    #[test]
    fn a_broadcast_rumble_fades_to_its_range() {
        let fading = rumble("x", 1000, true, 400.0, true);
        assert_eq!(broadcast_scale(&fading, 100.0), 0.75);
        assert_eq!(broadcast_scale(&fading, 401.0), 0.0);
        let flat = rumble("x", 1000, true, 400.0, false);
        assert_eq!(broadcast_scale(&flat, 399.0), 1.0);
    }

    #[test]
    fn a_rumble_ends_and_a_loop_starts_over() {
        let mut slots = RumblesT6::default();
        slots.play(rumble("once", 500, false, 0.0, false), SourceT6::Entity(3, entity(7)), false, 0, Some(3));
        slots.play(rumble("loop", 500, true, 1000.0, false), SourceT6::Position([0.0; 3]), true, 0, Some(3));
        tick_at(&mut slots, 499, Vec3::ZERO);
        assert_eq!(occupied(&slots), 2);
        tick_at(&mut slots, 500, Vec3::ZERO);
        assert_eq!(occupied(&slots), 1);
        tick_at(&mut slots, 5000, Vec3::ZERO);
        assert_eq!(slots.motors.strong_motor, 1.0);
        // Out of range a broadcast rumble keeps its slot but is not felt.
        tick_at(&mut slots, 5001, Vec3::new(2000.0, 0.0, 0.0));
        assert_eq!(occupied(&slots), 1);
        assert_eq!(slots.motors.strong_motor, 0.0);
    }

    #[test]
    fn a_loop_on_an_entity_lasts_while_its_flag_is_set() {
        let mut slots = RumblesT6::default();
        slots.loop_flags.insert(3);
        for name in ["a", "b"] {
            slots.play(rumble(name, 500, true, 1000.0, false), SourceT6::Entity(3, entity(7)), true, 0, Some(1));
        }
        tick_at(&mut slots, 2000, Vec3::ZERO);
        assert_eq!(occupied(&slots), 2);
        // Stopping frees the first of that name; clearing the flag ends the other.
        slots.loop_flags.remove(&3);
        slots.stop("a", 3);
        assert_eq!(occupied(&slots), 1);
        tick_at(&mut slots, 2001, Vec3::ZERO);
        assert_eq!(occupied(&slots), 0);
    }

    #[test]
    fn a_rumble_on_an_entity_that_is_gone_stops() {
        let mut slots = RumblesT6::default();
        slots.play(rumble("a", 500, true, 1000.0, false), SourceT6::Entity(3, entity(9)), false, 0, Some(1));
        tick_at(&mut slots, 10, Vec3::ZERO);
        assert_eq!(occupied(&slots), 0);
    }

    #[test]
    fn a_full_set_gives_up_the_nearest_to_ending_but_not_loops_or_the_viewers() {
        let mut slots = RumblesT6::default();
        for index in 0..RUMBLE_SLOTS_T6 {
            let source = if index == 0 {
                SourceT6::Entity(3, entity(7))
            } else {
                SourceT6::Position([index as f32, 0.0, 0.0])
            };
            let length = 1000 + index as i32 * 10;
            slots.play(
                rumble(&format!("r{index}"), length, true, 100.0, false),
                source,
                index == 1,
                0,
                Some(3),
            );
        }
        // Slot 0 (the viewer's, shortest) and slot 1 (a loop) are passed over; slot 2 goes.
        let fresh = || rumble("new", 1000, true, 100.0, false);
        slots.play(fresh(), SourceT6::Position([0.0; 3]), false, 5, Some(3));
        assert_eq!(slots.slots[2].as_ref().unwrap().rumble.name, "new");
        // The same rumble from the same spot in the same frame is not started twice.
        slots.play(fresh(), SourceT6::Position([0.0; 3]), false, 5, Some(3));
        let copies = slots.slots.iter().flatten().filter(|s| s.rumble.name == "new");
        assert_eq!(copies.count(), 1);
    }
}
