//! Game audio: alias resolution, clip storage and decoding, voice management,
//! and the Bevy systems that play world, weapon, ambient and menu sounds.

mod ads_sounds;
mod cue_names;
mod map_sound_set;
mod attenuation;
mod backend;
mod bus_gain;
mod clip_bank;
mod destructible_loops;
mod dive_vox; // bo2zm
mod duck;
mod noise_out;
mod ent_sounds;
mod menu_track;
mod mix_probe;
mod guest;
mod laststand; // bo2zm
mod loop_emitter;
mod master;
mod match_sounds;
mod match_voices;
mod menu_volume;
mod alerts;
mod sample_feed;
mod play_core;
mod sound_out;
mod room_echo;
mod rumble;
mod shock_sound;
mod slide_loop;
mod distance;
mod staffloop; // bo2zm
mod playback_gate;
mod voice_slots;
mod weapon_lock;
mod weapon_loops;
mod zm_music;

pub use cue_names::{
    FootfallPace, footfall_cue_names, gear_alias, gear_jingle_cue, touchdown_cue_names, hushed_ground_cue,
    pick_view_gunshot_cue, pick_gunshot_cue, stride_prefix, surface_cue_list,
    t6_movement_alias, surface_cue_for,
};
pub use map_sound_set::{MAP_EMITTER_CAP, EMITTER_GAIN_FLOOR, SoundArchive};
pub use map_sound_set::{LevelBackdropSound, AmbientBedStarted, LevelSoundSource};
pub use map_sound_set::{end_map_ambience, map_emitter_gain_tick};
pub use clip_bank::{ClipPath, ClipPathCost, ClipPrepCost, ClipRepository, PREP_BATCH, clip_prep_cost};
pub use duck::{DuckGroup, DuckMixer};
pub use noise_out::{StrideRhythmWatch, send_footfall_at_stride_end, send_gunshot_noise};
pub use menu_track::FrontendAudio;
pub use guest::{GuestSoundQueue, GuestSoundRequest};
pub use match_sounds::{SoundSystemLive, AudioSilent};
pub use menu_volume::MenuVolume;
pub use alerts::{
    AliasCommand, BoundWeaponSound, FootfallMsg, TouchdownNoiseMsg, TriggerCueMsg, SOUND_SOURCE_SELF,
    FirstPersonAnimCues, GunNoiseMsg, ent_from_number,
};
pub use sample_feed::{PanHandle, LoopedPcm, ClipPcm, decode_clip_bytes};
pub use play_core::{BackgroundHearer, SpatialVoice, UnknownCueHoles};
pub use play_core::{AudioCatalogRes, CuePickState};
pub use play_core::{oneshot_lr_gains, oneshot_pan};
pub use sound_out::SoundOutPlugin;
pub use rumble::{CodeRumbleStopT6, CodeRumbleT6};
pub use distance::{gap_in_inches, inches_of};
pub use playback_gate::{NoiseCategory, PlaybackBlocker, PlaybackVerdict, MuteCause};
pub use playback_gate::{PlaybackVerdictEntry, PlaybackVerdictLog};
pub use voice_slots::ChannelUsageTally;
pub use weapon_loops::{WeaponLoopRow, WeaponLoops};
