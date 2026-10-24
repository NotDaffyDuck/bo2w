//! Turns the sound events other systems raise on entities (weapon handling,
//! footsteps, landings, grenade bounces, viewmodel notetracks) into the
//! messages that start sounds.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use asset_audio::SoundCatalog;
use asset_core::AssetNamespace;
use asset_game::{WeaponRegistry, WeaponSoundSlot};
use assets::PreparedWeapons;
use bevy::prelude::*;
use entity_t6::{EntityEventKind, LOCAL_SOUND_ENTITY};
use net::{CEntity, LocalPresentClient, PresentedSnapshot};

use crate::{
    AliasCommand, Footstep, LandSound, PlayAlias, SoundBank, StepGait, WeaponSound,
    ent_from_number, footstep_aliases, gear_alias, gear_rattle_alias, land_aliases,
};

/// Black Ops II's knife striking a riot shield (`meleeKnifeHitShield`).
const SHIELD_KNIFE_HIT: &str = "fly_riotshield_impact_knife";

/// The (world, first-person) weapon sound slots a plain weapon event plays.
fn plain_slots(event: EntityEventKind) -> Option<(WeaponSoundSlot, WeaponSoundSlot)> {
    use WeaponSoundSlot as Slot;
    Some(match event {
        EntityEventKind::ITEM_PICKUP => (Slot::Pickup, Slot::PickupPlayer),
        EntityEventKind::AMMO_PICKUP => (Slot::AmmoPickup, Slot::AmmoPickupPlayer),
        EntityEventKind::NOAMMO => (Slot::EmptyFire, Slot::EmptyFirePlayer),
        EntityEventKind::RELOAD => (Slot::Reload, Slot::ReloadPlayer),
        EntityEventKind::RELOAD_FROM_EMPTY => (Slot::ReloadEmpty, Slot::ReloadEmptyPlayer),
        EntityEventKind::RELOAD_START => (Slot::ReloadStart, Slot::ReloadStartPlayer),
        EntityEventKind::RELOAD_END => (Slot::ReloadEnd, Slot::ReloadEndPlayer),
        EntityEventKind::RAISE_WEAPON => (Slot::Raise, Slot::RaisePlayer),
        EntityEventKind::FIRST_RAISE_WEAPON => (Slot::FirstRaise, Slot::FirstRaisePlayer),
        EntityEventKind::PUTAWAY_WEAPON => (Slot::Putaway, Slot::PutawayPlayer),
        EntityEventKind::WEAPON_ALT => (Slot::AltSwitch, Slot::AltSwitchPlayer),
        EntityEventKind::PULLBACK_WEAPON | EntityEventKind::PREP_OFFHAND => {
            (Slot::Pullback, Slot::PullbackPlayer)
        }
        EntityEventKind::USE_OFFHAND => (Slot::Fire, Slot::FirePlayer),
        EntityEventKind::RECHAMBER_WEAPON => (Slot::Rechamber, Slot::RechamberPlayer),
        EntityEventKind::MELEE_SWIPE => (Slot::MeleeSwipe, Slot::MeleeSwipePlayer),
        _ => return None,
    })
}

/// Events this module turns into weapon sounds (everything `plain_slots`
/// knows, plus melee hits and misses).
fn is_weapon_event(event: EntityEventKind) -> bool {
    plain_slots(event).is_some()
        || matches!(event, EntityEventKind::MELEE_HIT | EntityEventKind::MELEE_MISS)
}

/// The alias (and the game it belongs to) a weapon event plays, if the weapon
/// has one.
fn selected_alias<'a>(
    weapons: &WeaponRegistry,
    bank: &'a SoundCatalog,
    weapon: u32,
    event: EntityEventKind,
    player_view: bool,
    melee_parm: i32,
) -> Option<(AssetNamespace, &'a str)> {
    // A melee hit or miss: bit 0 is a knife (not a shield bash), and a riot
    // shield took the blow when `MELEE_PARM_SHIELD` is set.
    let knife = melee_parm & 1 != 0;
    let melee = matches!(
        event,
        EntityEventKind::MELEE_SWIPE | EntityEventKind::MELEE_HIT | EntityEventKind::MELEE_MISS
    );
    let weapon = if melee {
        weapons.melee_weapon_of(weapon)
    } else {
        weapon
    };

    match event {
        // A knife into a riot shield (carried or planted) rings off it; a bash makes nothing.
        EntityEventKind::MELEE_HIT if melee_parm & sim::MELEE_PARM_SHIELD != 0 => {
            let index = knife
                .then(|| bank.index_in(AssetNamespace::T6, SHIELD_KNIFE_HIT))
                .flatten()?;
            return Some((bank.namespace_of_alias(index), bank.name_at(index)?));
        }
        EntityEventKind::MELEE_HIT => {
            return melee_impact_alias(weapons, bank, weapon, knife, WeaponSoundSlot::MeleeHit);
        }
        EntityEventKind::MELEE_MISS => {
            return melee_impact_alias(weapons, bank, weapon, knife, WeaponSoundSlot::MeleeMiss);
        }
        _ => {}
    }
    let (world, player) = plain_slots(event)?;

    // Black Ops II's own knife swing (`wpn_melee_whoosh_plr`) is a placeholder
    // whose one variant names no sound file; the swing it really has is the
    // world one (`wpn_melee_whoosh_npc`), so the player hears that instead.
    if player_view
        && event == EntityEventKind::MELEE_SWIPE
        && let Some((namespace, name)) = weapons.weapon_sound_key(weapon, player, bank)
    {
        if alias_is_silent(bank, namespace, name) {
            return weapons.weapon_sound_key(weapon, world, bank);
        }
        return Some((namespace, name));
    }
    weapons.weapon_sound_key(weapon, if player_view { player } else { world }, bank)
}

/// A Black Ops II alias none of whose variants names a sound file.
fn alias_is_silent(bank: &SoundCatalog, namespace: AssetNamespace, name: &str) -> bool {
    namespace == AssetNamespace::T6
        && bank.sound_in(namespace, name).is_some_and(|sound| {
            !sound.aliases.is_empty()
                && sound
                    .aliases
                    .iter()
                    .all(|variant| variant.streamed.is_none() && variant.file_name.is_none())
        })
}

fn melee_impact_alias<'a>(
    weapons: &WeaponRegistry,
    bank: &'a SoundCatalog,
    weapon: u32,
    knife: bool,
    slot: WeaponSoundSlot,
) -> Option<(AssetNamespace, &'a str)> {
    let namespace = weapons.namespace_of(weapon).unwrap_or_default();
    // Black Ops II names its melee sounds the way Black Ops does
    // (`wpn_melee_knife_hit_body`, `wpn_melee_hit_other`).
    let prefixed = matches!(namespace, AssetNamespace::T5 | AssetNamespace::T6);
    let hit = slot == WeaponSoundSlot::MeleeHit;

    let generic = |with_knife: bool| {
        let base = match (with_knife, hit) {
            (true, true) => "melee_knife_hit_body",
            (true, false) => "melee_knife_hit_other",
            (false, true) => "melee_hit",
            (false, false) => "melee_hit_other",
        };
        let name = if prefixed {
            format!("wpn_{base}")
        } else {
            base.to_owned()
        };
        let index = bank.index_in(namespace, &name)?;
        Some((bank.namespace_of_alias(index), bank.name_at(index)?))
    };
    let own = || weapons.weapon_sound_key(weapon, slot, bank);
    let knife_generic = || knife.then(|| generic(true)).flatten();

    let first = if prefixed {
        own().or_else(knife_generic)
    } else {
        knife_generic().or_else(own)
    };
    first.or_else(|| generic(false))
}

fn entity_event_sound(
    sound: On<net::EntityEventSound>,
    identities: Query<&CEntity>,
    local: Res<LocalPresentClient>,
    weapons: Option<Res<PreparedWeapons>>,
    bank: Option<Res<SoundBank>>,
    adopted: Option<Res<net::LastAdoptedSnapshot>>,
    mut output: MessageWriter<WeaponSound>,
    mut play: MessageWriter<AliasCommand>,
    mut unvoiced: Local<HashSet<(i32, u32)>>,
) {
    let event = sound.event.event;
    let payload = &sound.event.payload;
    if event == EntityEventKind::SOUND_ALIAS {
        play_cs_sound_alias(&sound.event, adopted.as_deref(), &mut play);
        return;
    }
    // A shield bash into a riot shield makes no impact sound.
    if event == EntityEventKind::MELEE_HIT && payload.event_parm == sim::MELEE_PARM_SHIELD {
        return;
    }
    // No dry-fire click for a clip-only gun or one that stays up when empty;
    // the event still arrives for the client's out-of-ammo switch.
    if event == EntityEventKind::NOAMMO && payload.event_parm & sim::NOAMMO_PARM_SILENT != 0 {
        return;
    }
    let Ok(identity) = identities.get(sound.entity) else {
        diag::warn!(
            Audio,
            "audio: entity sound has no CEntity identity (typed gap)"
        );
        return;
    };
    let player_view = identity.client() == Some(local.0);
    let snd_ent = Some(u32::from(identity.number()));

    // Pushing through foliage (EV_FOLIAGE_SOUND): the player's own rustle in
    // their ears, another player's where they are.
    if event == EntityEventKind::FOLIAGE_SOUND {
        let alias = if player_view {
            "fly_movement_foliage_plr"
        } else {
            "fly_movement_foliage_npc"
        };
        let known = bank
            .as_deref()
            .is_some_and(|bank| bank.0.index_in(AssetNamespace::T6, alias).is_some());
        if known {
            play.write(AliasCommand::Play(PlayAlias {
                namespace: AssetNamespace::T6,
                alias: alias.to_owned(),
                fallback: None,
                origin_inches: (!player_view).then_some(payload.origin),
                snd_ent,
            }));
        }
        return;
    }

    if !is_weapon_event(event) {
        diag::warn!(
            Audio,
            "audio: entity sound event {} has no handler (typed gap)",
            event.0
        );
        return;
    }
    let weapon_known = weapons
        .as_deref()
        .is_some_and(|weapons| weapons.0.sounds_of(payload.weapon).is_some());
    let (true, Some(weapons)) = (weapon_known, weapons.as_deref()) else {
        diag::warn!(
            Audio,
            "audio: entity sound weapon is unavailable (typed gap)"
        );
        return;
    };

    // Offhand view sounds may be authored entirely as animation notetracks. A
    // missing player alias must not fall back to the world throw sound.
    if event == EntityEventKind::USE_OFFHAND {
        let slot = if player_view {
            WeaponSoundSlot::FirePlayer
        } else {
            WeaponSoundSlot::Fire
        };
        if weapons.0.authored_weapon_sound(payload.weapon, slot).is_none() {
            return;
        }
    }

    let chosen = bank.as_deref().and_then(|bank| {
        selected_alias(
            &weapons.0,
            &bank.0,
            payload.weapon,
            event,
            player_view,
            payload.event_parm,
        )
    });
    let Some((namespace, alias)) = chosen else {
        // Once for each event and gun: it fired hundreds of times a match.
        if unvoiced.insert((event.0, payload.weapon)) {
            diag::warn!(
                Audio,
                "audio: entity sound event {} has no authored alias (weapon {})",
                event.0,
                payload.weapon
            );
        }
        return;
    };
    let melee_contact = matches!(
        event,
        EntityEventKind::MELEE_HIT | EntityEventKind::MELEE_MISS
    );
    output.write(WeaponSound {
        namespace,
        alias: alias.to_owned(),
        origin_inches: (!player_view || melee_contact).then_some(payload.origin),
        snd_ent,
    });
}

fn play_cs_sound_alias(
    dispatched: &net::DispatchedEntityEvent,
    adopted: Option<&net::LastAdoptedSnapshot>,
    play: &mut MessageWriter<AliasCommand>,
) {
    let payload = &dispatched.payload;
    let index = u8::try_from(payload.event_parm).unwrap_or(0);
    let Some(name) = adopted.and_then(|snapshot| snapshot.sound_alias_name(index)) else {
        diag::warn!(
            Audio,
            "audio: EV_SOUND_ALIAS CS index {index} is unresolved (typed gap)"
        );
        return;
    };
    let local = payload.number == LOCAL_SOUND_ENTITY;
    diag::event!(
        Audio,
        Debug,
        "cs_sound",
        "audio: cs sound `{name}` local={local}"
    );

    // A configstring alias may be written `game:name`; bare ones are Black Ops II's.
    let (namespace, alias) = name
        .split_once(':')
        .and_then(|(game, rest)| AssetNamespace::parse(game).map(|game| (game, rest)))
        .unwrap_or((AssetNamespace::T6, name));
    play.write(AliasCommand::Play(PlayAlias {
        namespace,
        alias: alias.to_owned(),
        fallback: None,
        origin_inches: (!local).then_some(payload.origin),
        snd_ent: if local {
            None
        } else {
            ent_from_number(payload.number)
        },
    }));
}

/// Which footstep gait a movement event is, when it is one.
fn gait_of(event: EntityEventKind) -> Option<StepGait> {
    Some(match event {
        EntityEventKind::FOOTSTEP_SPRINT => StepGait::Sprint,
        EntityEventKind::FOOTSTEP_RUN | EntityEventKind::JUMP => StepGait::Run,
        EntityEventKind::FOOTSTEP_WALK => StepGait::Walk,
        EntityEventKind::FOOTSTEP_PRONE => StepGait::Prone,
        EntityEventKind::FOOTSTEP_CROUCHRUN => StepGait::CrouchRun,
        EntityEventKind::FOOTSTEP_CROUCHWALK => StepGait::CrouchWalk,
        EntityEventKind::LADDER_HAND_GRAB => StepGait::LadderHand,
        _ => return None,
    })
}

fn movement_sound(
    sound: On<net::EntityMovementSound>,
    identities: Query<&CEntity>,
    local: Res<LocalPresentClient>,
    presented: Res<PresentedSnapshot>,
    mut footsteps: MessageWriter<Footstep>,
    mut gear: MessageWriter<WeaponSound>,
    mut play: MessageWriter<AliasCommand>,
    mut land: MessageWriter<LandSound>,
    bank: Option<Res<SoundBank>>,
    mut slide: ResMut<crate::slide_loop::SlideLoop>,
) {
    let Ok(identity) = identities.get(sound.entity) else {
        diag::warn!(
            Audio,
            "audio: movement sound has no CEntity identity (typed gap)"
        );
        return;
    };
    let event = sound.event.event;
    let payload = &sound.event.payload;
    let player_view = identity.client() == Some(local.0);
    let snd_ent = Some(u32::from(identity.number()));
    let quieter = identity
        .client()
        .and_then(|id| presented.player(id))
        .is_some_and(|state| state.perks[0] & playerstate_t6::game::PERK_QUIETER != 0);
    let origin_inches = (!player_view).then_some(payload.origin);

    if let Some(surface) = event.landing_surface_index() {
        let (alias, fallback) = land_aliases((surface as u32) << 20, player_view, quieter);
        land.write(LandSound {
            alias,
            fallback,
            origin_inches,
            snd_ent,
        });
        return;
    }

    let Some(gait) = gait_of(event) else {
        match event {
            EntityEventKind::MANTLE => {
                play.write(AliasCommand::Play(PlayAlias {
                    namespace: AssetNamespace::T6,
                    alias: gear_alias(player_view).to_owned(),
                    fallback: player_view.then(|| gear_alias(false).to_owned()),
                    origin_inches,
                    snd_ent,
                }));
            }
            // Black Ops II's slide ground start and stop.
            EntityEventKind::SLIDE_START | EntityEventKind::SLIDE_STOP => {
                let starting = event == EntityEventKind::SLIDE_START;
                if let Some(start) = crate::slide_loop::slide_event(
                    &mut slide,
                    starting,
                    player_view,
                    payload.event_parm,
                    payload.origin,
                    u32::from(identity.number()),
                ) {
                    play.write(AliasCommand::Play(start));
                }
            }
            _ => unreachable!("EntityMovementSound must carry a movement event"),
        }
        return;
    };

    let has_t6_alias = |name: &str| {
        bank.as_deref()
            .is_some_and(|bank| bank.0.index_in(AssetNamespace::T6, name).is_some())
    };
    // Crouched and ladder-hand steps are Black Ops II's own; without its
    // movement sounds they sound as the fallback run and walk.
    let gait = if has_t6_alias("fly_step_run_plr_default") {
        gait
    } else {
        gait.fallback()
    };
    // A jump is also the player's launch exertion (its gear rides as the
    // alias's secondary).
    if event == EntityEventKind::JUMP && has_t6_alias("chr_launch_exert_npc") {
        let alias = if player_view {
            "chr_launch_exert_plr"
        } else {
            "chr_launch_exert_npc"
        };
        play.write(AliasCommand::Play(PlayAlias {
            namespace: AssetNamespace::T6,
            alias: alias.to_owned(),
            fallback: None,
            origin_inches,
            snd_ent,
        }));
    }

    let surface_flags = u32::from(payload.surf_type) << 20;
    let (alias, fallback) = footstep_aliases(gait, surface_flags, player_view, quieter);
    footsteps.write(Footstep {
        alias,
        fallback,
        origin_inches,
        snd_ent,
    });
    gear.write(WeaponSound {
        namespace: AssetNamespace::T6,
        alias: gear_rattle_alias(gait, player_view).to_owned(),
        origin_inches,
        snd_ent,
    });
}

pub(crate) fn register_entity_event_audio(app: &mut App) {
    app.add_observer(entity_event_sound)
        .add_observer(movement_sound)
        .add_observer(grenade_contact);
}

/// What a viewmodel notetrack's sound is, once bound against a sound bank.
#[derive(Clone, Debug)]
enum NotetrackSound {
    Bound(usize),
    Unbound(String),
}

#[derive(Clone, Debug)]
struct BoundNotetrack {
    sound: Option<NotetrackSound>,
    rumble: Option<Result<Arc<crate::rumble::Rumble>, String>>,
}

/// Every weapon notetrack's sound and rumble, bound once per (bank, weapons) pair.
#[derive(Resource, Default)]
pub(crate) struct NotetrackSoundTable {
    owner: Option<(Arc<SoundCatalog>, Arc<WeaponRegistry>)>,
    actions: HashMap<(u32, String), BoundNotetrack>,
    reported: HashSet<(u32, String)>,
}

impl NotetrackSoundTable {
    fn owns(&self, bank: &Arc<SoundCatalog>, weapons: &Arc<WeaponRegistry>) -> bool {
        match &self.owner {
            Some((owned_bank, owned_weapons)) => {
                Arc::ptr_eq(owned_bank, bank) && Arc::ptr_eq(owned_weapons, weapons)
            }
            None => false,
        }
    }

    fn bind(bank: &Arc<SoundCatalog>, weapons: &Arc<WeaponRegistry>) -> Self {
        let mut actions = HashMap::new();
        let mut unbound = 0usize;
        // One prepared rumble per (game, alias), shared by every note that names it.
        let mut rumbles: HashMap<(AssetNamespace, String), Result<Arc<crate::rumble::Rumble>, String>> =
            HashMap::new();

        for weapon in 1..=weapons.len() as u32 {
            let namespace = weapons
                .namespace_of(weapon)
                .unwrap_or(AssetNamespace::T6);
            for (note, action) in weapons.notetrack_actions_of(weapon) {
                let sound = action.sound_alias.as_deref().map(|alias| {
                    bank.index_in(namespace, alias).map_or_else(
                        || {
                            unbound += 1;
                            NotetrackSound::Unbound(alias.to_owned())
                        },
                        NotetrackSound::Bound,
                    )
                });
                let rumble = action.rumble_alias.as_ref().map(|alias| {
                    rumbles
                        .entry((namespace, alias.clone()))
                        .or_insert_with(|| crate::rumble::Rumble::prepare(bank, namespace, alias))
                        .clone()
                });
                actions.insert((weapon, note.to_owned()), BoundNotetrack { sound, rumble });
            }
        }

        let mut refused = 0usize;
        for ((namespace, alias), rumble) in &rumbles {
            if let Err(error) = rumble {
                refused += 1;
                diag::warn!(
                    Audio,
                    "audio: {namespace:?} rumble `{alias}` refused during binding: {error}"
                );
            }
        }
        diag::info!(
            Audio,
            "audio: viewmodel notetracks bound to the sound bank: actions={} unbound_sounds={unbound} rumbles={} refused_rumbles={refused}",
            actions.len(),
            rumbles.len(),
        );
        Self {
            owner: Some((Arc::clone(bank), Arc::clone(weapons))),
            actions,
            reported: HashSet::new(),
        }
    }

    /// Logs a warning the first time a (weapon, note) pair has one to give.
    fn report_once(&mut self, weapon: u32, note: &str, message: impl FnOnce() -> String) {
        if self.reported.insert((weapon, note.to_owned())) {
            diag::warn!(Audio, "{}", message());
        }
    }
}

pub(crate) fn bind_notetrack_sounds(
    bank: Option<Res<SoundBank>>,
    weapons: Option<Res<PreparedWeapons>>,
    mut table: ResMut<NotetrackSoundTable>,
) {
    let (Some(bank), Some(weapons)) = (bank, weapons) else {
        if table.owner.is_some() {
            *table = NotetrackSoundTable::default();
        }
        return;
    };
    if !table.owns(&bank.0, &weapons.0) {
        *table = NotetrackSoundTable::bind(&bank.0, &weapons.0);
    }
}

pub(crate) fn play_viewmodel_notetrack_messages(
    mut notes: MessageReader<crate::ViewmodelNotetracks>,
    generation: Res<frame::WorldGeneration>,
    presented: Res<net::PresentedSnapshot>,
    local: Res<net::LocalPresentClient>,
    weapons: Option<Res<PreparedWeapons>>,
    bank: Option<Res<SoundBank>>,
    mut table: ResMut<NotetrackSoundTable>,
    mut output: MessageWriter<crate::BoundWeaponSound>,
    mut rumbles: MessageWriter<crate::rumble::PlayRumble>,
) {
    // Notes only play against the bank their table was bound to.
    let bound_bank = bank
        .as_deref()
        .zip(weapons.as_deref())
        .filter(|(bank, weapons)| table.owns(&bank.0, &weapons.0))
        .map(|(bank, _)| Arc::clone(&bank.0));

    let alive_this_life = |life: sim::LifeSequence| {
        presented
            .snapshot()
            .and_then(|snapshot| snapshot.meta.for_client(local.0))
            .is_some_and(|meta| {
                meta.life_sequence == life && meta.lifecycle == sim::ClientLifecycle::Alive
            })
    };
    for batch in notes.read() {
        let current =
            batch.generation == *generation && batch.client == local.0 && alive_this_life(batch.life);
        if !current {
            continue;
        }
        for name in &batch.names {
            apply_viewmodel_notetrack(
                batch.weapon,
                name,
                bound_bank.as_deref(),
                &mut table,
                &mut output,
                &mut rumbles,
            );
        }
    }
}

fn apply_viewmodel_notetrack(
    weapon: u32,
    note: &str,
    bank: Option<&SoundCatalog>,
    table: &mut NotetrackSoundTable,
    output: &mut MessageWriter<crate::BoundWeaponSound>,
    rumbles: &mut MessageWriter<crate::rumble::PlayRumble>,
) {
    if note.eq_ignore_ascii_case("end") {
        return;
    }
    if ["NVG_on_powerup", "NVG_off_powerdown"]
        .iter()
        .any(|nvg| note.eq_ignore_ascii_case(nvg))
    {
        table.report_once(weapon, note, || {
            format!("audio: NVG notetrack `{note}` has no cgMedia alias (typed gap)")
        });
    }
    let Some(bank) = bank else {
        table.report_once(weapon, note, || {
            format!(
                "audio: notetrack `{note}` dropped — notetracks are not bound to the installed sound bank"
            )
        });
        return;
    };
    let Some(action) = table
        .actions
        .get(&(weapon, note.to_ascii_lowercase()))
        .cloned()
    else {
        table.report_once(weapon, note, || {
            format!("audio: notetrack `{note}` has no sound or rumble mapping")
        });
        return;
    };

    match &action.rumble {
        Some(Ok(rumble)) => {
            rumbles.write(crate::rumble::PlayRumble {
                bank_revision: bank.revision(),
                rumble: Arc::clone(rumble),
            });
        }
        Some(Err(error)) => table.report_once(weapon, note, || {
            format!("audio: notetrack `{note}` rumble refused: {error}")
        }),
        None => {}
    }

    match action.sound {
        Some(NotetrackSound::Bound(index)) => {
            output.write(crate::BoundWeaponSound {
                bank_revision: bank.revision(),
                index,
                origin_inches: None,
                snd_ent: Some(crate::SND_ENT_LOCAL),
            });
        }
        Some(NotetrackSound::Unbound(alias)) => table.report_once(weapon, note, || {
            format!(
                "audio: notetrack `{note}` sound `{alias}` is not in the sound bank (refused when bound)"
            )
        }),
        None if action.rumble.is_none() => table.report_once(weapon, note, || {
            format!("audio: notetrack `{note}` has no sound or rumble mapping")
        }),
        None => {}
    }
}

fn grenade_contact(
    contact: On<net::EntityGrenadeContact>,
    weapons: Option<Res<PreparedWeapons>>,
    bank: Option<Res<SoundBank>>,
    mut output: MessageWriter<WeaponSound>,
) {
    let payload = &contact.event.payload;
    let surface = usize::try_from(payload.event_parm).unwrap_or(usize::from(payload.surf_type));

    let bounce = weapons.as_deref().and_then(|weapons| {
        let bank = bank.as_deref()?;
        let alias = weapons
            .0
            .bounce_sound_alias(payload.weapon, surface, &bank.0)?;
        let namespace = weapons
            .0
            .namespace_of(payload.weapon)
            .unwrap_or(AssetNamespace::T6);
        Some((namespace, alias))
    });
    let Some((namespace, alias)) = bounce else {
        // What the weapon names for that surface, if anything: either no name,
        // or one the loaded sound banks lack.
        let named = weapons
            .as_deref()
            .and_then(|weapons| weapons.0.bounce_sound_of(payload.weapon, surface));
        diag::warn!(
            Audio,
            "audio: grenade bounce alias for eventParm {} is unavailable (weapon {} names {:?}) (typed gap)",
            payload.event_parm,
            payload.weapon,
            named
        );
        return;
    };
    output.write(WeaponSound {
        namespace,
        alias: alias.to_owned(),
        origin_inches: Some(payload.origin),
        snd_ent: ent_from_number(payload.number),
    });
}
