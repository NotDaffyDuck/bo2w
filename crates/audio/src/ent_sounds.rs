//! Turns the sound events other systems raise on entities (weapon handling,
//! footsteps, landings, grenade bounces, viewmodel notetracks) into the
//! messages that start sounds.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use asset_audio::AudioRoster;
use asset_core::AssetCategoryScope;
use asset_game::{WeaponLedger, WeaponAudioRole};
use assets::StagedWeapons;
use bevy::prelude::*;
use entity_t6::{EventCode, LOCAL_SOUND_ENTITY};
use net::{ClientEntityRecord, HostViewClient, ShownWorldFrame};

use crate::SOUND_SOURCE_SELF;
use crate::{AliasCommand, FootfallMsg, TouchdownNoiseMsg, TriggerCueMsg, AudioCatalogRes, FootfallPace, GunNoiseMsg};
use crate::{ent_from_number, footfall_cue_names, gear_alias, gear_jingle_cue, touchdown_cue_names};

/// The knife ringing off a riot shield (`meleeKnifeHitShield`).
const KNIFE_ON_SHIELD: &str = "fly_riotshield_impact_knife";

/// Foliage rustle, indexed by "heard in first person".
const FOLIAGE_RUSTLE: [&str; 2] = ["fly_movement_foliage_npc", "fly_movement_foliage_plr"];

/// A jump's launch exertion, indexed by "heard in first person".
const LAUNCH_EXERTION: [&str; 2] = ["chr_launch_exert_npc", "chr_launch_exert_plr"];

/// A sound that sits where the listener is, not out in the world.
/// The notetrack that only marks the end of an animation.
const END_NOTE: &str = "end";

/// The sound entity of the listener's own view.
const LISTENER_ENT: Option<u32> = Some(SOUND_SOURCE_SELF);

const NO_ORIGIN: Option<[f32; 3]> = None;

/// Viewmodel notetracks that name a night-vision sound the client has no alias for.
const NVG_NOTES: [&str; 2] = ["NVG_on_powerup", "NVG_off_powerdown"];

/// Each plain weapon event with the (world, first-person) slot it plays.
const PLAIN_WEAPON_EVENTS: &[(EventCode, WeaponAudioRole, WeaponAudioRole)] = &[
    (EventCode::ITEM_PICKUP, WeaponAudioRole::Pickup, WeaponAudioRole::PickupPlayer),
    (EventCode::AMMO_PICKUP, WeaponAudioRole::AmmoPickup, WeaponAudioRole::AmmoPickupPlayer),
    (EventCode::NOAMMO, WeaponAudioRole::EmptyFire, WeaponAudioRole::EmptyFirePlayer),
    (EventCode::RELOAD, WeaponAudioRole::Reload, WeaponAudioRole::ReloadPlayer),
    (EventCode::RELOAD_FROM_EMPTY, WeaponAudioRole::DryReload, WeaponAudioRole::ReloadEmptyPlayer),
    (EventCode::RELOAD_START, WeaponAudioRole::ReloadKickoff, WeaponAudioRole::ReloadStartPlayer),
    (EventCode::RELOAD_END, WeaponAudioRole::ReloadEnd, WeaponAudioRole::ReloadEndPlayer),
    (EventCode::RAISE_WEAPON, WeaponAudioRole::Raise, WeaponAudioRole::RaisePlayer),
    (EventCode::FIRST_RAISE_WEAPON, WeaponAudioRole::FirstRaise, WeaponAudioRole::FirstRaisePlayer),
    (EventCode::PUTAWAY_WEAPON, WeaponAudioRole::Putaway, WeaponAudioRole::PutawayPlayer),
    (EventCode::WEAPON_ALT, WeaponAudioRole::AltSwitch, WeaponAudioRole::AltSwitchPlayer),
    (EventCode::PULLBACK_WEAPON, WeaponAudioRole::Pullback, WeaponAudioRole::PullbackPlayer),
    (EventCode::PREP_OFFHAND, WeaponAudioRole::Pullback, WeaponAudioRole::PullbackPlayer),
    (EventCode::USE_OFFHAND, WeaponAudioRole::Fire, WeaponAudioRole::FirePlayer),
    (EventCode::RECHAMBER_WEAPON, WeaponAudioRole::Rechamber, WeaponAudioRole::RechamberPlayer),
    (EventCode::MELEE_SWIPE, WeaponAudioRole::MeleeSwipe, WeaponAudioRole::MeleeSwipePlayer),
];

/// Each footstep-like event with the gait it sounds as.
const GAIT_EVENTS: [(EventCode, FootfallPace); 8] = [
    (EventCode::FOOTSTEP_SPRINT, FootfallPace::Sprint),
    (EventCode::FOOTSTEP_RUN, FootfallPace::Run),
    (EventCode::LEAP, FootfallPace::Run),
    (EventCode::FOOTSTEP_WALK, FootfallPace::Walking),
    (EventCode::FOOTSTEP_PRONE, FootfallPace::Prone),
    (EventCode::FOOTSTEP_CROUCHRUN, FootfallPace::CrouchRun),
    (EventCode::FOOTSTEP_CROUCHWALK, FootfallPace::CrouchWalk),
    (EventCode::LADDER_HAND_GRAB, FootfallPace::LadderHand),
];

/// The (world, first-person) slots a plain weapon event plays.
fn plain_slots(event: EventCode) -> Option<(WeaponAudioRole, WeaponAudioRole)> {
    PLAIN_WEAPON_EVENTS
        .iter()
        .find(|row| row.0 == event)
        .map(|&(_, world, first_person)| (world, first_person))
}

/// Events this module turns into weapon sounds: the plain ones, and a melee
/// hit or miss.
fn is_weapon_event(event: EventCode) -> bool {
    [EventCode::MELEE_HIT, EventCode::MELEE_MISS].contains(&event) || plain_slots(event).is_some()
}

/// Which gait a movement event steps as, when it is a step at all.
fn gait_of(event: EventCode) -> Option<FootfallPace> {
    GAIT_EVENTS
        .iter()
        .find_map(|&(kind, gait)| (kind == event).then_some(gait))
}

/// Events that are dropped on purpose: a bash into a riot shield makes no
/// impact sound, and a gun that never clicks empty stays quiet (its event
/// still reaches the client for the out-of-ammo switch).
fn is_silent_by_design(event: EventCode, parm: i32) -> bool {
    match event {
        EventCode::MELEE_HIT => parm == sim::MELEE_PARM_SHIELD,
        EventCode::NOAMMO => parm & sim::NOAMMO_PARM_SILENT != 0,
        _ => false,
    }
}

/// Whether the sound catalog (if there is one) has a Black Ops II alias by this name.
fn bank_has(bank: Option<&AudioCatalogRes>, cue_name: &str) -> bool {
    bank.is_some_and(|held| held.0.slot_in(AssetCategoryScope::T6, cue_name).is_some())
}

/// A play command for one alias.
fn play_alias(
    namespace: AssetCategoryScope,
    cue_id: &str,
    second_choice: Option<&str>,
    position_in_inches: Option<[f32; 3]>,
    source_entity_no: Option<u32>,
) -> AliasCommand {
    AliasCommand::Play(TriggerCueMsg {
        source_entity_no,
        position_in_inches,
        fallback: second_choice.map(str::to_owned),
        alias: cue_id.to_owned(),
        cue_ns: namespace,
    })
}

fn weapon_sound(
    namespace: AssetCategoryScope,
    cue_id: &str,
    position_in_inches: Option<[f32; 3]>,
    source_entity_no: Option<u32>,
) -> GunNoiseMsg {
    GunNoiseMsg {
        source_entity_no,
        position_in_inches,
        alias: cue_id.to_owned(),
        cue_ns: namespace,
    }
}

/// Who an entity sound comes from, as the local player hears it.
#[derive(Clone, Copy)]
struct Emitter {
    first_person: bool,
    number: u32,
}

impl Emitter {
    fn from_identity(identity: &ClientEntityRecord, local: &HostViewClient) -> Self {
        Self {
            first_person: identity.client() == Some(local.0),
            number: u32::from(identity.number()),
        }
    }

    fn source_entity_no(self) -> Option<u32> {
        Some(self.number)
    }

    /// Where the sound plays: other players are heard where they stand.
    fn origin(self, at: [f32; 3]) -> Option<[f32; 3]> {
        (!self.first_person).then_some(at)
    }
}

/// What a weapon event asks of the catalog.
#[derive(Clone, Copy)]
struct WeaponAsk {
    weapon_ix: u32,
    ev_kind: EventCode,
    first_person: bool,
    parm: i32,
}

impl WeaponAsk {
    fn is_melee(self) -> bool {
        matches!(self.ev_kind, EventCode::MELEE_SWIPE | EventCode::MELEE_HIT | EventCode::MELEE_MISS)
    }

    /// Bit 0 of a melee parm: a knife rather than a bash.
    fn with_knife(self) -> bool {
        self.parm & 1 != 0
    }

    fn struck_shield(self) -> bool {
        self.ev_kind == EventCode::MELEE_HIT && self.parm & sim::MELEE_PARM_SHIELD != 0
    }
}

/// Where a melee impact alias can come from, in the order tried.
#[derive(Clone, Copy)]
enum ImpactSource {
    Own,
    KnifeGeneric,
    PlainGeneric,
}

/// The generic melee alias for a hit or miss, with a knife or without.
fn melee_generic_name(prefixed: bool, knife: bool, hit: bool) -> String {
    const BASES: [[&str; 2]; 2] = [
        ["melee_hit_other", "melee_hit"],
        ["melee_knife_hit_other", "melee_knife_hit_body"],
    ];
    let base = BASES[usize::from(knife)][usize::from(hit)];
    if prefixed {
        ["wpn_", base].concat()
    } else {
        base.to_owned()
    }
}

/// The weapon registry and the sound catalog an alias is looked up in.
struct Lookup<'a> {
    registry: &'a WeaponLedger,
    cat_ref: &'a AudioRoster,
}

impl<'a> Lookup<'a> {
    /// The alias in `namespace` by that name, with the game it resolves to.
    fn entry(&self, namespace: AssetCategoryScope, name: &str) -> Option<(AssetCategoryScope, &'a str)> {
        let at = self.cat_ref.slot_in(namespace, name)?;
        let resolved = self.cat_ref.label_at(at)?;
        Some((self.cat_ref.space_of_slot(at), resolved))
    }

    /// A Black Ops II alias none of whose variants names a sound file.
    fn is_silent(&self, namespace: AssetCategoryScope, cue_name: &str) -> bool {
        if namespace != AssetCategoryScope::T6 {
            return false;
        }
        let Some(found) = self.cat_ref.entry_in(namespace, cue_name) else {
            return false;
        };
        let variants = &found.aliases;
        !variants.is_empty()
            && variants
                .iter()
                .all(|v| v.streamed.is_none() && v.clip_file.is_none())
    }

    /// The alias (and its game) a weapon event plays, if the weapon has one.
    fn alias_for(&self, ask: WeaponAsk) -> Option<(AssetCategoryScope, &'a str)> {
        let weapon = match ask.is_melee() {
            true => self.registry.melee_weapon_of(ask.weapon_ix),
            false => ask.weapon_ix,
        };
        if ask.struck_shield() {
            // Only a knife rings off a shield; a bash makes nothing.
            return if ask.with_knife() {
                self.entry(AssetCategoryScope::T6, KNIFE_ON_SHIELD)
            } else {
                None
            };
        }
        match ask.ev_kind {
            EventCode::MELEE_HIT => return self.melee_impact(weapon, ask.with_knife(), WeaponAudioRole::MeleeHit),
            EventCode::MELEE_MISS => {
                return self.melee_impact(weapon, ask.with_knife(), WeaponAudioRole::MeleeMiss);
            }
            _ => {}
        }

        let (world, first_person) = plain_slots(ask.ev_kind)?;
        let heard = self.registry.weapon_sound_key(
            weapon,
            if ask.first_person { first_person } else { world },
            self.cat_ref,
        );
        // The first-person knife swing is a placeholder whose one variant
        // names no sound file; the swing the player really hears is the
        // world one.
        match heard {
            Some((cue_ns, name))
                if ask.first_person
                    && ask.ev_kind == EventCode::MELEE_SWIPE
                    && self.is_silent(cue_ns, name) =>
            {
                self.registry.weapon_sound_key(weapon, world, self.cat_ref)
            }
            unmatched => unmatched,
        }
    }

    /// The alias a melee hit or miss plays: the weapon's own, or a generic one.
    fn melee_impact(
        &self,
        wpn: u32,
        knife: bool,
        bay: WeaponAudioRole,
    ) -> Option<(AssetCategoryScope, &'a str)> {
        let namespace = self.registry.scope_for(wpn).unwrap_or_default();
        // Black Ops II names its melee sounds the way Black Ops does.
        let prefixed = matches!(namespace, AssetCategoryScope::T5 | AssetCategoryScope::T6);
        let hit = bay == WeaponAudioRole::MeleeHit;
        let generic = |with_knife: bool| {
            self.entry(namespace, &melee_generic_name(prefixed, with_knife, hit))
        };

        let order = if prefixed {
            [ImpactSource::Own, ImpactSource::KnifeGeneric, ImpactSource::PlainGeneric]
        } else {
            [ImpactSource::KnifeGeneric, ImpactSource::Own, ImpactSource::PlainGeneric]
        };
        order.into_iter().find_map(|source| match source {
            ImpactSource::Own => self.registry.weapon_sound_key(wpn, bay, self.cat_ref),
            ImpactSource::KnifeGeneric if knife => generic(true),
            ImpactSource::KnifeGeneric => None,
            ImpactSource::PlainGeneric => generic(false),
        })
    }
}

fn entity_event_sound(
    trigger: On<net::SoundCueAt>,
    local: Res<HostViewClient>,
    identities: Query<&ClientEntityRecord>,
    bank: Option<Res<AudioCatalogRes>>,
    armory: Option<Res<StagedWeapons>>,
    latest_snapshot: Option<Res<net::NewestTakenSnapshot>>,
    mut alias_out: MessageWriter<AliasCommand>,
    mut weapon_out: MessageWriter<GunNoiseMsg>,
    mut unvoiced: Local<HashSet<(i32, u32)>>,
) {
    let dispatched = trigger.event;
    let kind = dispatched.event;
    let data = dispatched.payload;
    if kind == EventCode::SOUND_ALIAS {
        play_configstring_alias(&dispatched, latest_snapshot.as_deref(), &mut alias_out);
        return;
    }
    if is_silent_by_design(kind, data.event_parm) {
        return;
    }
    let Ok(identity) = identities.get(trigger.entity) else {
        diag::warn!(Audio, "sound: entity cue for an ent without a CEntity (typed gap)");
        return;
    };
    let who = Emitter::from_identity(identity, &local);

    // Pushing through foliage: the player's own rustle in their ears, another
    // player's where they are.
    if kind == EventCode::FOLIAGE_SOUND {
        let rustle = FOLIAGE_RUSTLE[usize::from(who.first_person)];
        if bank_has(bank.as_deref(), rustle) {
            let command = play_alias(
                AssetCategoryScope::T6,
                rustle,
                None,
                who.origin(data.origin),
                who.source_entity_no(),
            );
            alias_out.write(command);
        }
        return;
    }

    if !is_weapon_event(kind) {
        diag::warn!(Audio, "sound: entity cue kind {} is not handled (typed gap)", kind.0);
        return;
    }
    let registry = armory
        .as_deref()
        .map(|prepared| &*prepared.0)
        .filter(|registry| registry.audio_for(data.weapon).is_some());
    let Some(registry) = registry else {
        diag::warn!(Audio, "sound: entity cue names a weapon that is not loaded (typed gap)");
        return;
    };

    // Offhand view sounds may be authored entirely as animation notetracks. A
    // missing player alias must not fall back to the world throw sound.
    if kind == EventCode::USE_OFFHAND {
        let wanted = if who.first_person { WeaponAudioRole::FirePlayer } else { WeaponAudioRole::Fire };
        if registry.authored_weapon_sound(data.weapon, wanted).is_none() {
            return;
        }
    }

    let ask = WeaponAsk {
        weapon_ix: data.weapon,
        ev_kind: kind,
        first_person: who.first_person,
        parm: data.event_parm,
    };
    let chosen = bank.as_deref().and_then(|held| {
        Lookup {
            registry,
            cat_ref: &held.0,
        }
        .alias_for(ask)
    });
    let Some((snd_ns, alias)) = chosen else {
        // Once for each event and gun: it fired hundreds of times a match.
        if unvoiced.insert((kind.0, data.weapon)) {
            diag::warn!(
                Audio,
                "audio: weapon {} has no authored alias for entity sound event {}",
                data.weapon,
                kind.0
            );
        }
        return;
    };
    // A melee contact is heard where it lands, even by the one who swung.
    let origin = if who.first_person && !matches!(kind, EventCode::MELEE_HIT | EventCode::MELEE_MISS) {
        NO_ORIGIN
    } else {
        Some(data.origin)
    };
    weapon_out.write(weapon_sound(snd_ns, alias, origin, who.source_entity_no()));
}

/// An alias a configstring names, as (game, alias). `game:name` picks the
/// game; a bare name is Black Ops II's.
fn qualified_alias(name: &str) -> (AssetCategoryScope, &str) {
    if let Some((game, rest)) = name.split_once(':')
        && let Some(namespace) = AssetCategoryScope::parse(game)
    {
        return (namespace, rest);
    }
    (AssetCategoryScope::T6, name)
}

fn play_configstring_alias(
    dispatched: &net::RoutedEntityEvent,
    latest: Option<&net::NewestTakenSnapshot>,
    alias_out: &mut MessageWriter<AliasCommand>,
) {
    let data = &dispatched.payload;
    let config_index = u8::try_from(data.event_parm).unwrap_or_default();
    let Some(name) = latest.and_then(|snap| snap.sound_cue_name(config_index)) else {
        diag::warn!(Audio, "sound: configstring {config_index} has no cue name (typed gap)");
        return;
    };
    let on_listener = data.number == LOCAL_SOUND_ENTITY;
    diag::event!(Audio, Debug, "cs_sound", "audio: configstring sound `{name}` on_listener={on_listener}");

    let (namespace, alias) = qualified_alias(name);
    let (eye_pos, source_entity_no) = match on_listener {
        true => (NO_ORIGIN, None),
        false => (Some(data.origin), ent_from_number(data.number)),
    };
    alias_out.write(play_alias(namespace, alias, None, eye_pos, source_entity_no));
}

/// What a movement event turns out to be.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Movement {
    Landing(i32),
    Step(FootfallPace),
    Mantle,
    Slide { starting: bool },
}

fn classify_movement(event: EventCode) -> Option<Movement> {
    if let Some(surface) = event.touchdown_material_index() {
        return Some(Movement::Landing(surface));
    }
    if let Some(gait) = gait_of(event) {
        return Some(Movement::Step(gait));
    }
    match event {
        EventCode::MANTLE => Some(Movement::Mantle),
        EventCode::SLIDE_START => Some(Movement::Slide { starting: true }),
        EventCode::SLIDE_STOP => Some(Movement::Slide { starting: false }),
        _ => None,
    }
}

fn movement_sound(
    trigger: On<net::StepSoundAt>,
    local: Res<HostViewClient>,
    presented: Res<ShownWorldFrame>,
    identities: Query<&ClientEntityRecord>,
    bank: Option<Res<AudioCatalogRes>>,
    mut slide: ResMut<crate::slide_loop::SlideLoop>,
    mut alias_out: MessageWriter<AliasCommand>,
    mut step_out: MessageWriter<FootfallMsg>,
    mut gear_out: MessageWriter<GunNoiseMsg>,
    mut land_out: MessageWriter<TouchdownNoiseMsg>,
) {
    let Ok(identity) = identities.get(trigger.entity) else {
        diag::warn!(Audio, "sound: move cue for an ent without a CEntity (typed gap)");
        return;
    };
    let kind = trigger.event.event;
    let data = &trigger.event.payload;
    let who = Emitter::from_identity(identity, &local);
    let mover = identity.client().and_then(|client| presented.player(client));
    let quieter = matches!(mover, Some(ps) if ps.has_perk_bit(playerstate_t6::perks::PERK_QUIETMOVE));
    let heard_at = who.origin(data.origin);
    let Some(movement) = classify_movement(kind) else {
        unreachable!("a movement sound carries a movement event, not {kind:?}");
    };

    match movement {
        Movement::Landing(surface) => {
            let (alias, fallback) =
                touchdown_cue_names(surface.cast_unsigned() << 20, who.first_person, quieter);
            land_out.write(TouchdownNoiseMsg {
                source_entity_no: who.source_entity_no(),
                position_in_inches: heard_at,
                fallback,
                alias,
            });
        }
        Movement::Mantle => {
            let rattle = gear_alias(who.first_person);
            let plain = who.first_person.then(|| gear_alias(false));
            alias_out.write(play_alias(AssetCategoryScope::T6, rattle, plain, heard_at, who.source_entity_no()));
        }
        Movement::Slide { starting } => {
            let started = crate::slide_loop::slide_event(
                &mut slide,
                starting,
                who.first_person,
                data.event_parm,
                data.origin,
                who.number,
            );
            if let Some(command) = started {
                alias_out.write(AliasCommand::Play(command));
            }
        }
        Movement::Step(gait) => {
            // Crouched and ladder-hand steps are Black Ops II's own; without
            // its movement sounds they sound as the fallback run and walk.
            let held = bank.as_deref();
            let gait = match bank_has(held, "fly_step_run_plr_default") {
                true => gait,
                false => gait.fallback(),
            };
            // A jump is also the player's launch exertion.
            if kind == EventCode::LEAP && bank_has(held, LAUNCH_EXERTION[0]) {
                let exertion = LAUNCH_EXERTION[usize::from(who.first_person)];
                alias_out.write(play_alias(
                    AssetCategoryScope::T6,
                    exertion,
                    None,
                    heard_at,
                    who.source_entity_no(),
                ));
            }
            let (alias, fallback) = footfall_cue_names(
                gait,
                u32::from(data.surf_type) << 20,
                who.first_person,
                quieter,
            );
            step_out.write(FootfallMsg {
                alias,
                fallback,
                position_in_inches: heard_at,
                source_entity_no: who.source_entity_no(),
            });
            // bo2zm: inside a step trigger (Nuketown's porches, the truck bed) the map's client
            // script sets a sound each of his steps also plays. Only the T6 sim sets this dvar,
            // and the alias is in BO2's bank.
            let step = presented
                .snapshot()
                .and_then(|snap| snap.meta.script_dvars(local.0).string("bo2zm_step"))
                .filter(|alias| !alias.is_empty() && *alias != "none");
            if let Some(step) = step.filter(|_| who.first_person) {
                alias_out.write(play_alias(AssetCategoryScope::T6, step, None, heard_at, who.source_entity_no()));
            }
            let rattle = gear_jingle_cue(gait, who.first_person);
            gear_out.write(weapon_sound(AssetCategoryScope::T6, rattle, heard_at, who.source_entity_no()));
        }
    }
}

pub(crate) fn register_entity_event_audio(target: &mut App) {
    target.add_observer(entity_event_sound);
    target.add_observer(movement_sound);
    target.add_observer(grenade_contact);
}

/// What a viewmodel notetrack's sound is, once looked up in a sound bank.
#[derive(Clone, Debug)]
enum NoteVoice {
    InBank(usize),
    Missing(String),
}

/// A notetrack's sound and rumble as bound against one bank.
#[derive(Clone, Debug)]
struct NoteBinding {
    rumble: Option<Result<Arc<crate::rumble::Rumble>, String>>,
    voice: Option<NoteVoice>,
}

/// What playing a bound notetrack does.
#[derive(Debug, Default)]
struct NoteOutcome {
    rumble: Option<Arc<crate::rumble::Rumble>>,
    sound_index: Option<usize>,
    /// The first thing worth a warning, if anything.
    complaint: Option<String>,
}

fn note_outcome(note: &str, binding: &NoteBinding) -> NoteOutcome {
    let rumble_error = match &binding.rumble {
        Some(Err(error)) => Some(format!("audio: notetrack `{note}` rumble was refused: {error}")),
        _ => None,
    };
    let voice_error = match &binding.voice {
        Some(NoteVoice::Missing(alias)) => Some(format!(
            "audio: notetrack `{note}` sound `{alias}` was not in the sound bank when bound"
        )),
        _ => None,
    };
    let nothing_bound = binding.rumble.is_none() && binding.voice.is_none();
    NoteOutcome {
        rumble: binding.rumble.as_ref().and_then(|r| r.as_ref().ok().cloned()),
        sound_index: match binding.voice {
            Some(NoteVoice::InBank(index)) => Some(index),
            _ => None,
        },
        complaint: rumble_error.or(voice_error).or_else(|| {
            nothing_bound.then(|| format!("audio: notetrack `{note}` maps to no sound or rumble"))
        }),
    }
}

/// Every weapon notetrack's sound and rumble, bound once per (bank, weapons) pair.
#[derive(Resource, Default)]
pub(crate) struct NotetrackSoundTable {
    bound_to: Option<(Arc<AudioRoster>, Arc<WeaponLedger>)>,
    by_note: HashMap<(u32, String), NoteBinding>,
    warned: HashSet<(u32, String)>,
}

impl NotetrackSoundTable {
    fn owns(&self, bank: &Arc<AudioRoster>, weapons: &Arc<WeaponLedger>) -> bool {
        self.bound_to
            .as_ref()
            .is_some_and(|(b, w)| Arc::ptr_eq(b, bank) && Arc::ptr_eq(w, weapons))
    }

    fn bind(bank: &Arc<AudioRoster>, weapons: &Arc<WeaponLedger>) -> Self {
        // One rumble_cache rumble per (game, alias), shared by every note that names it.
        let mut rumble_cache: HashMap<(AssetCategoryScope, String), Result<Arc<crate::rumble::Rumble>, String>> = HashMap::new();
        let mut missing_sounds = 0usize;
        let mut by_note = HashMap::new();

        for weapon in (1u32..).take(weapons.len()) {
            let namespace = weapons.scope_for(weapon).unwrap_or(AssetCategoryScope::T6);
            by_note.extend(weapons.notetrack_actions_of(weapon).map(|(note, action)| {
                let voice = action.sound_alias.as_deref().map(|alias| {
                    match bank.slot_in(namespace, alias) {
                        Some(index) => NoteVoice::InBank(index),
                        None => {
                            missing_sounds += 1;
                            NoteVoice::Missing(alias.to_owned())
                        }
                    }
                });
                let rumble = action.rumble_alias.as_ref().map(|alias| {
                    rumble_cache
                        .entry((namespace, alias.clone()))
                        .or_insert_with(|| crate::rumble::Rumble::stage(bank, namespace, alias))
                        .clone()
                });
                ((weapon, note.to_owned()), NoteBinding { rumble, voice })
            }));
        }

        let refusals: Vec<_> = rumble_cache
            .iter()
            .filter_map(|((namespace, alias), outcome)| {
                outcome.as_ref().err().map(|error| (namespace, alias, error))
            })
            .collect();
        for (cat_ns, alias, error) in &refusals {
            diag::warn!(
                Audio,
                "audio: {cat_ns:?} rumble `{alias}` was refused while binding: {error}"
            );
        }
        diag::info!(
            Audio,
            "audio: bound viewmodel notetracks: {} actions, {missing_sounds} sounds missing, {} rumbles ({} refused)",
            by_note.len(),
            rumble_cache.len(),
            refusals.len(),
        );
        Self {
            bound_to: Some((Arc::clone(bank), Arc::clone(weapons))),
            by_note,
            warned: HashSet::new(),
        }
    }

    /// Warns the first time a (weapon, note) pair has something to say.
    /// Returns whether this call was that first time.
    fn warn_first(&mut self, gun_ix: u32, note: &str, message: impl FnOnce() -> String) -> bool {
        let first = self.warned.insert((gun_ix, note.to_owned()));
        if first {
            diag::warn!(Audio, "{}", message());
        }
        first
    }
}

pub(crate) fn bind_notetrack_sounds(
    mut table: ResMut<NotetrackSoundTable>,
    weapons: Option<Res<StagedWeapons>>,
    bank: Option<Res<AudioCatalogRes>>,
) {
    match bank.zip(weapons) {
        Some((bank, weapons)) => {
            if !table.owns(&bank.0, &weapons.0) {
                *table = NotetrackSoundTable::bind(&bank.0, &weapons.0);
            }
        }
        None => {
            if table.bound_to.is_some() {
                *table = NotetrackSoundTable::default();
            }
        }
    }
}

/// Whether the local player is alive in the given life.
fn alive_in_life(presented: &ShownWorldFrame, cl_id: sim::ClientId, life: sim::LifeSequence) -> bool {
    let entry = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(cl_id));
    entry.is_some_and(|m| m.life_sequence == life && m.lifecycle == sim::PlayerSessionStage::Alive)
}

pub(crate) fn voice_first_person_anim_cues(
    mut incoming: MessageReader<crate::FirstPersonAnimCues>,
    mut table: ResMut<NotetrackSoundTable>,
    mut sound_out: MessageWriter<crate::BoundWeaponSound>,
    mut rumble_out: MessageWriter<crate::rumble::PlayRumble>,
    generation: Res<frame::LevelGeneration>,
    snapshots: Res<net::ShownWorldFrame>,
    me: Res<net::HostViewClient>,
    armory: Option<Res<StagedWeapons>>,
    bank: Option<Res<AudioCatalogRes>>,
) {
    // Notes only play against the bank their table was bound to.
    let bound_bank = bank
        .as_deref()
        .zip(armory.as_deref())
        .filter(|(held, prepared)| table.owns(&held.0, &prepared.0))
        .map(|(held, _)| Arc::clone(&held.0));

    let live: Vec<_> = incoming
        .read()
        .filter(|batch| batch.generation == *generation && batch.client == me.0)
        .filter(|batch| alive_in_life(&snapshots, me.0, batch.life))
        .collect();
    let pending = live
        .iter()
        .flat_map(|batch| batch.cue_list.iter().map(move |name| (batch.cue_wpn, name)));
    for (wpn_slot, name) in pending {
        play_one_notetrack(
            &mut table,
            bound_bank.as_deref(),
            wpn_slot,
            name,
            &mut sound_out,
            &mut rumble_out,
        );
    }
}

fn play_one_notetrack(
    table: &mut NotetrackSoundTable,
    bank: Option<&AudioRoster>,
    held_wpn: u32,
    note: &str,
    sound_out: &mut MessageWriter<crate::BoundWeaponSound>,
    rumble_out: &mut MessageWriter<crate::rumble::PlayRumble>,
) {
    if note.eq_ignore_ascii_case(END_NOTE) {
        return;
    }
    if NVG_NOTES.iter().any(|nvg| nvg.eq_ignore_ascii_case(note)) {
        table.warn_first(held_wpn, note, || {
            format!("audio: night-vision notetrack `{note}` has no cgMedia alias (typed gap)")
        });
    }
    let Some(snd_bank) = bank else {
        table.warn_first(held_wpn, note, || {
            format!("audio: notetrack `{note}` dropped: the notetracks are not bound to this sound bank")
        });
        return;
    };
    let outcome = table
        .by_note
        .get(&(held_wpn, note.to_ascii_lowercase()))
        .map(|binding| note_outcome(note, binding));
    let Some(outcome) = outcome else {
        table.warn_first(held_wpn, note, || format!("audio: notetrack `{note}` is not mapped to anything"));
        return;
    };

    if let Some(rumble) = outcome.rumble {
        rumble_out.write(crate::rumble::PlayRumble {
            rumble,
            catalog_edition: snd_bank.revision(),
        });
    }
    if let Some(index) = outcome.sound_index {
        sound_out.write(crate::BoundWeaponSound {
            source_entity_no: LISTENER_ENT,
            position_in_inches: NO_ORIGIN,
            index,
            catalog_edition: snd_bank.revision(),
        });
    }
    if let Some(complaint) = outcome.complaint {
        table.warn_first(held_wpn, note, || complaint);
    }
}

fn grenade_contact(
    trigger: On<net::GrenadeBounceAt>,
    armory: Option<Res<StagedWeapons>>,
    bank: Option<Res<AudioCatalogRes>>,
    mut weapon_out: MessageWriter<GunNoiseMsg>,
) {
    let landed = &trigger.event.payload;
    let surface = usize::try_from(landed.event_parm).unwrap_or_else(|_| usize::from(landed.surf_type));

    let found = armory.as_deref().zip(bank.as_deref()).and_then(|(prepared, held)| {
        let registry = &*prepared.0;
        let alias = registry.bounce_sound_alias(landed.weapon, surface, &held.0)?;
        let namespace = registry.scope_for(landed.weapon);
        Some((namespace.unwrap_or(AssetCategoryScope::T6), alias))
    });
    let Some((found_ns, alias)) = found else {
        // What the weapon names for that surface, if anything: either no name,
        // or one the loaded sound banks lack.
        let named = armory
            .as_deref()
            .and_then(|prepared| prepared.0.bounce_cue_for(landed.weapon, surface));
        diag::warn!(
            Audio,
            "audio: no grenade bounce alias for eventParm {} (weapon {} names {:?}) (typed gap)",
            landed.event_parm,
            landed.weapon,
            named
        );
        return;
    };
    weapon_out.write(weapon_sound(
        found_ns,
        alias,
        Some(landed.origin),
        ent_from_number(landed.number),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_audio::{SnapshotSoundAlias, SnapshotSoundEntry};

    fn catalog_of(names: &[&str]) -> AudioRoster {
        let mut catalog = AudioRoster::default();
        for &known in names {
            catalog.add_sound(SnapshotSoundEntry {
                name: known.to_owned(),
                aliases: vec![SnapshotSoundAlias {
                    clip_file: Some(format!("{known}.wav")),
                    ..SnapshotSoundAlias::default()
                }],
            });
        }
        catalog
    }

    fn lookup_in<'a>(registry: &'a WeaponLedger, catalog: &'a AudioRoster) -> Lookup<'a> {
        Lookup { cat_ref: catalog, registry }
    }

    fn ask(event: EventCode, first_person: bool, parm: i32) -> WeaponAsk {
        WeaponAsk {
            weapon_ix: 7,
            ev_kind: event,
            first_person,
            parm,
        }
    }

    fn settle(catalog: &AudioRoster, ask: WeaponAsk) -> Option<(AssetCategoryScope, String)> {
        let registry = WeaponLedger::default();
        lookup_in(&registry, catalog)
            .alias_for(ask)
            .map(|(ns, cue_name)| (ns, cue_name.to_owned()))
    }

    #[test]
    fn plain_events_pick_world_then_first_person_slots() {
        assert_eq!(plain_slots(EventCode::RELOAD), Some((WeaponAudioRole::Reload, WeaponAudioRole::ReloadPlayer)));
        assert_eq!(plain_slots(EventCode::USE_OFFHAND), Some((WeaponAudioRole::Fire, WeaponAudioRole::FirePlayer)));
        assert_eq!(plain_slots(EventCode::MELEE_SWIPE), Some((WeaponAudioRole::MeleeSwipe, WeaponAudioRole::MeleeSwipePlayer)));
        assert_eq!(plain_slots(EventCode::PREP_OFFHAND), plain_slots(EventCode::PULLBACK_WEAPON));
        assert_eq!(plain_slots(EventCode::FOLIAGE_SOUND), None);
        assert_eq!(plain_slots(EventCode::MELEE_HIT), None);
    }

    #[test]
    fn every_plain_row_is_listed_once_and_found() {
        let mut seen_keys = HashSet::new();
        for &(kind, _, _) in PLAIN_WEAPON_EVENTS {
            assert!(seen_keys.insert(kind.0), "event {} listed twice", kind.0);
            assert_ne!(plain_slots(kind), None);
            assert!(is_weapon_event(kind));
        }
    }

    #[test]
    fn weapon_events_are_the_plain_ones_and_melee_contacts() {
        assert!(is_weapon_event(EventCode::MELEE_HIT));
        assert!(is_weapon_event(EventCode::MELEE_MISS));
        assert!(is_weapon_event(EventCode::NOAMMO));
        assert!(!is_weapon_event(EventCode::SOUND_ALIAS));
        assert!(!is_weapon_event(EventCode::FOOTSTEP_RUN));
    }

    #[test]
    fn gaits_follow_the_movement_event() {
        assert_eq!(gait_of(EventCode::LEAP), Some(FootfallPace::Run));
        assert_eq!(gait_of(EventCode::FOOTSTEP_RUN), Some(FootfallPace::Run));
        assert_eq!(gait_of(EventCode::FOOTSTEP_CROUCHWALK), Some(FootfallPace::CrouchWalk));
        assert_eq!(gait_of(EventCode::LADDER_HAND_GRAB), Some(FootfallPace::LadderHand));
        assert_eq!(gait_of(EventCode::MANTLE), None);
        assert_eq!(gait_of(EventCode::RELOAD), None);
    }

    #[test]
    fn movement_events_classify_by_kind() {
        assert_eq!(classify_movement(EventCode::MANTLE), Some(Movement::Mantle));
        assert_eq!(classify_movement(EventCode::SLIDE_START), Some(Movement::Slide { starting: true }));
        assert_eq!(classify_movement(EventCode::SLIDE_STOP), Some(Movement::Slide { starting: false }));
        assert_eq!(classify_movement(EventCode::FOOTSTEP_SPRINT), Some(Movement::Step(FootfallPace::Sprint)));
        assert_eq!(classify_movement(EventCode::RELOAD), None);
        let landing = (0..=u8::MAX)
            .map(|n| EventCode(i32::from(n)))
            .find(|kind| kind.touchdown_material_index().is_some())
            .expect("some event is a landing");
        assert_eq!(
            classify_movement(landing),
            landing.touchdown_material_index().map(Movement::Landing)
        );
    }

    #[test]
    fn some_events_are_dropped_on_purpose() {
        assert!(is_silent_by_design(EventCode::MELEE_HIT, sim::MELEE_PARM_SHIELD));
        assert!(!is_silent_by_design(EventCode::MELEE_HIT, sim::MELEE_PARM_SHIELD | 1));
        assert!(!is_silent_by_design(EventCode::MELEE_MISS, sim::MELEE_PARM_SHIELD));
        assert!(is_silent_by_design(EventCode::NOAMMO, sim::NOAMMO_PARM_SILENT));
        assert!(!is_silent_by_design(EventCode::NOAMMO, 0));
        assert!(!is_silent_by_design(EventCode::RELOAD, sim::NOAMMO_PARM_SILENT));
    }

    #[test]
    fn generic_melee_names_by_knife_hit_and_game() {
        assert_eq!(melee_generic_name(true, true, true), "wpn_melee_knife_hit_body");
        assert_eq!(melee_generic_name(true, true, false), "wpn_melee_knife_hit_other");
        assert_eq!(melee_generic_name(true, false, true), "wpn_melee_hit");
        assert_eq!(melee_generic_name(true, false, false), "wpn_melee_hit_other");
        assert_eq!(melee_generic_name(false, true, true), "melee_knife_hit_body");
        assert_eq!(melee_generic_name(false, false, false), "melee_hit_other");
    }

    #[test]
    fn configstring_aliases_may_name_their_game() {
        assert_eq!(qualified_alias("t5:ring"), (AssetCategoryScope::T5, "ring"));
        assert_eq!(qualified_alias("t6:ring"), (AssetCategoryScope::T6, "ring"));
        assert_eq!(qualified_alias("ring"), (AssetCategoryScope::T6, "ring"));
        assert_eq!(qualified_alias("nope:ring"), (AssetCategoryScope::T6, "nope:ring"));
    }

    #[test]
    fn a_knife_into_a_shield_rings_and_a_bash_does_not() {
        let catalog = catalog_of(&[KNIFE_ON_SHIELD]);
        let knife = settle(&catalog, ask(EventCode::MELEE_HIT, true, sim::MELEE_PARM_SHIELD | 1));
        assert_eq!(knife, Some((AssetCategoryScope::T6, KNIFE_ON_SHIELD.to_owned())));
        assert_eq!(settle(&catalog, ask(EventCode::MELEE_HIT, true, sim::MELEE_PARM_SHIELD)), None);
        assert_eq!(settle(&AudioRoster::default(), ask(EventCode::MELEE_HIT, true, sim::MELEE_PARM_SHIELD | 1)), None);
    }

    #[test]
    fn melee_contacts_fall_back_to_the_generic_aliases() {
        let catalog = catalog_of(&["wpn_melee_knife_hit_body", "wpn_melee_hit_other", "wpn_melee_hit"]);
        let named = |hit_or_miss: EventCode, parm: i32| settle(&catalog, ask(hit_or_miss, true, parm)).map(|r| r.1);
        assert_eq!(named(EventCode::MELEE_HIT, 1).as_deref(), Some("wpn_melee_knife_hit_body"));
        assert_eq!(named(EventCode::MELEE_HIT, 0).as_deref(), Some("wpn_melee_hit"));
        assert_eq!(named(EventCode::MELEE_MISS, 0).as_deref(), Some("wpn_melee_hit_other"));
        // No knife-miss alias exists, so a knife miss uses the plain miss one.
        assert_eq!(named(EventCode::MELEE_MISS, 1).as_deref(), Some("wpn_melee_hit_other"));
        assert_eq!(settle(&AudioRoster::default(), ask(EventCode::MELEE_MISS, false, 0)), None);
    }

    #[test]
    fn a_weapon_with_no_sounds_plays_no_plain_alias() {
        let catalog = catalog_of(&["wpn_melee_hit"]);
        assert_eq!(settle(&catalog, ask(EventCode::RELOAD, false, 0)), None);
        assert_eq!(settle(&catalog, ask(EventCode::MELEE_SWIPE, true, 0)), None);
        assert_eq!(settle(&catalog, ask(EventCode::FOLIAGE_SOUND, false, 0)), None);
    }

    #[test]
    fn an_alias_is_silent_only_when_no_variant_names_a_file() {
        let mut catalog = catalog_of(&["loud"]);
        catalog.add_sound(SnapshotSoundEntry {
            name: "quiet".to_owned(),
            aliases: vec![SnapshotSoundAlias::default(), SnapshotSoundAlias::default()],
        });
        catalog.add_sound(SnapshotSoundEntry {
            name: "empty".to_owned(),
            aliases: vec![],
        });
        let registry = WeaponLedger::default();
        let lookup = lookup_in(&registry, &catalog);
        let t6 = AssetCategoryScope::T6;
        assert!(lookup.is_silent(t6, "quiet"));
        assert!(!lookup.is_silent(t6, "loud"));
        assert!(!lookup.is_silent(t6, "empty"));
        assert!(!lookup.is_silent(t6, "absent"));
        assert!(!lookup.is_silent(AssetCategoryScope::T5, "quiet"));
    }

    #[test]
    fn entries_resolve_to_the_catalog_spelling() {
        let catalog = catalog_of(&["Mixed_Case"]);
        let registry = WeaponLedger::default();
        let lookup = lookup_in(&registry, &catalog);
        assert_eq!(
            lookup.entry(AssetCategoryScope::T6, "mixed_case"),
            Some((AssetCategoryScope::T6, "Mixed_Case"))
        );
        assert_eq!(lookup.entry(AssetCategoryScope::T6, "other"), None);
        assert_eq!(lookup.entry(AssetCategoryScope::T5, "Mixed_Case"), None);
    }

    #[test]
    fn bank_checks_use_black_ops_ii_names() {
        let held = AudioCatalogRes(Arc::new(catalog_of(&["fly_step_run_plr_default"])));
        assert!(bank_has(Some(&held), "fly_step_run_plr_default"));
        assert!(!bank_has(Some(&held), "chr_launch_exert_npc"));
        assert!(!bank_has(None, "fly_step_run_plr_default"));
    }

    #[test]
    fn play_commands_carry_their_fields() {
        let command = play_alias(AssetCategoryScope::T5, "a", Some("b"), Some([1.0, 2.0, 3.0]), Some(9));
        let AliasCommand::Play(play) = command else {
            panic!("expected a play command");
        };
        assert_eq!(play.cue_ns, AssetCategoryScope::T5);
        assert_eq!(play.alias, "a");
        assert_eq!(play.fallback.as_deref(), Some("b"));
        assert_eq!(play.position_in_inches, Some([1.0, 2.0, 3.0]));
        assert_eq!(play.source_entity_no, Some(9));
        let heard = weapon_sound(AssetCategoryScope::T6, "gear", NO_ORIGIN, None);
        assert_eq!((heard.alias.as_str(), heard.position_in_inches, heard.source_entity_no), ("gear", None, None));
    }

    #[test]
    fn notetrack_outcomes_play_what_is_bound_and_name_the_first_problem() {
        let bound = NoteBinding {
            rumble: None,
            voice: Some(NoteVoice::InBank(4)),
        };
        let played = note_outcome("fire", &bound);
        assert_eq!((played.sound_index, played.complaint), (Some(4), None));
        assert!(played.rumble.is_none());

        let unbound = NoteBinding {
            rumble: Some(Err("bad curve".to_owned())),
            voice: Some(NoteVoice::Missing("snd".to_owned())),
        };
        let refused = note_outcome("fire", &unbound);
        assert_eq!(refused.sound_index, None);
        assert!(refused.complaint.is_some_and(|c| c.contains("bad curve")));

        let sound_only = NoteBinding {
            rumble: None,
            voice: Some(NoteVoice::Missing("snd".to_owned())),
        };
        assert!(note_outcome("fire", &sound_only).complaint.is_some_and(|c| c.contains("`snd`")));

        let nothing = NoteBinding {
            rumble: None,
            voice: None,
        };
        assert!(note_outcome("idle", &nothing).complaint.is_some_and(|c| c.contains("`idle`")));
    }

    #[test]
    fn the_table_warns_once_per_weapon_and_note() {
        let mut table = NotetrackSoundTable::default();
        assert!(table.warn_first(3, "fire", || "x".to_owned()));
        assert!(!table.warn_first(3, "fire", || unreachable!("not asked twice")));
        assert!(table.warn_first(3, "reload", || "y".to_owned()));
        assert!(table.warn_first(4, "fire", || "z".to_owned()));
    }

    #[test]
    fn a_table_belongs_to_the_pair_it_was_bound_against() {
        let bank = Arc::new(AudioRoster::default());
        let weapons = Arc::new(WeaponLedger::default());
        let table = NotetrackSoundTable::bind(&bank, &weapons);
        assert!(table.owns(&bank, &weapons));
        assert!(!table.owns(&Arc::new(AudioRoster::default()), &weapons));
        assert!(!table.owns(&bank, &Arc::new(WeaponLedger::default())));
        assert!(!NotetrackSoundTable::default().owns(&bank, &weapons));
        assert!(table.by_note.is_empty());
    }

    #[test]
    fn binding_follows_the_installed_bank_and_weapons() {
        let mut harness = App::new();
        harness.init_resource::<NotetrackSoundTable>();
        harness.add_systems(Update, bind_notetrack_sounds);
        harness.update();
        assert!(harness.world().resource::<NotetrackSoundTable>().bound_to.is_none());

        let bank = AudioCatalogRes(Arc::new(AudioRoster::default()));
        harness.insert_resource(bank.clone());
        harness.update();
        assert!(harness.world().resource::<NotetrackSoundTable>().bound_to.is_none());

        let weapons = StagedWeapons::default();
        harness.insert_resource(weapons.clone());
        harness.update();
        let table = harness.world().resource::<NotetrackSoundTable>();
        assert!(table.owns(&bank.0, &weapons.0));

        harness.world_mut().remove_resource::<AudioCatalogRes>();
        harness.update();
        assert!(harness.world().resource::<NotetrackSoundTable>().bound_to.is_none());
    }
}
