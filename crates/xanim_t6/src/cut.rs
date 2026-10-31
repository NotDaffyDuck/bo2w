//! One XAnimParts decoded for playback: a rotation and a translation track per bone, the
//! notetracks, and the root-motion delta.

use crate::{
    fired_notes, frame_position, jumpcut_frame, blend_triple, lerp4, quat2, quat4, snap_jumpcut,
    time_index, time_index_dense, unpack_trans,
};

/// Flag bit: the clip loops.
const FLAG_LOOP: u8 = 1;
/// Flag bit: the clip moves its root (it has a delta track).
const FLAG_DELTA: u8 = 2;

/// An XAnimParts as stored: the pools its tracks are read from, front to back, in bone
/// order.
#[derive(Debug, Clone, Default)]
pub struct StoredClipParts {
    pub name: String,
    pub payload_u8: Vec<u8>,
    pub payload_i16: Vec<u16>,
    pub payload_i32: Vec<u32>,
    pub noise_octet: Vec<u8>,
    pub noise_word16: Vec<u16>,
    pub noise_word32: Vec<u32>,
    /// The last frame's number: a clip of `numframes + 1` frames.
    pub frame_total: u16,
    pub flags: u8,
    /// How many bones use each track kind: five rotation kinds, then four translation
    /// kinds.
    pub joint_total: [u8; 10],
    pub fps: f32,
    /// One name per bone.
    pub names: Vec<String>,
    pub notifies: Vec<ClipEventMark>,
    /// The key frames of long keyed tracks (64 keys or more) in long clips (256 frames or
    /// more).
    pub indices: Vec<u16>,
    pub delta_trans: Option<StoredRootMotion>,
}

/// The root-motion track as stored: `size + 1` keys, each `mins + step * key`, packed as
/// three bytes (`small`) or three little-endian u16.
#[derive(Debug, Clone, Default)]
pub struct StoredRootMotion {
    pub size: u16,
    pub packed16: bool,
    pub base: [f32; 3],
    pub scale: [f32; 3],
    pub indices: Vec<u16>,
    pub packed: Vec<u8>,
}

/// Why an XAnimParts did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipLoadFault {
    /// A read ran past the end of a pool.
    Exhausted {
        source: &'static str,
        want: usize,
        left_bytes: usize,
    },
    /// The tracks were read and a pool still had data left.
    Trailing {
        name: String,
        pool: &'static str,
        left_bytes: usize,
    },
    /// The rotation or translation kinds do not cover each named bone once.
    KindCountMismatch {
        name: String,
        names: usize,
        rotation_quaternions: usize,
        shift_offset: usize,
    },
    TrackBoneOutOfRange {
        name: String,
        joint: usize,
        lanes: usize,
    },
    TranslationTwice {
        name: String,
        bone: usize,
    },
    TranslationAbsent {
        name: String,
        bone: usize,
    },
    /// A keyed track's key frames go backwards.
    KeyFramesBackwards {
        name: String,
    },
}

/// A keyed track: one value per key and the frame each key sits on.
#[derive(Debug, Clone)]
pub struct TimedValue<T> {
    pub values: Vec<T>,
    pub frames: KeyframeIds,
}

/// The frames a track's keys sit on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyframeIds {
    /// Key `i` is on frame `i`.
    Dense,
    Sparse(Vec<u16>),
}

/// A bone's rotation track. Half rotations turn about z only.
#[derive(Debug, Clone)]
pub enum Rotation {
    /// The clip leaves the bone's rotation alone.
    Default,
    HalfKeyed(TimedValue<[f32; 4]>),
    FullKeyed(TimedValue<[f32; 4]>),
    HalfStatic([f32; 4]),
    FullStatic([f32; 4]),
}

/// A bone's translation track.
#[derive(Debug, Clone)]
pub enum RootShift {
    /// The clip leaves the bone where it is.
    Default,
    /// Keys stored as bytes.
    SmallKeyed(TimedValue<[f32; 3]>),
    /// Keys stored as u16.
    FullKeyed(TimedValue<[f32; 3]>),
    Fixed([f32; 3]),
}

/// One bone the clip drives.
#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub rotation: Rotation,
    pub move_track: RootShift,
}

/// An XAnimParts ready to play.
#[derive(Debug, Clone)]
pub struct SkeletalClip {
    pub name: String,
    pub fps: f32,
    pub frame_total: u16,
    pub looping: bool,
    pub tracks: Vec<Track>,
    pub notifies: Vec<ClipEventMark>,
    pub root_motion_shift: RootShift,
}

/// A notetrack: its name and where it sits, 0 at the start of the clip and 1 at the end.
#[derive(Debug, Clone)]
pub struct ClipEventMark {
    pub name: String,
    pub time: f32,
}

/// One track at one time. `None` where the track leaves the bone alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct EvaluatedChannel {
    pub rotation: Option<[f32; 4]>,
    pub move_track: Option<[f32; 3]>,
}

impl<T: Copy> TimedValue<T> {
    fn new(samples: Vec<T>, keyed_at: Vec<u16>, reel: &str) -> Result<Self, ClipLoadFault> {
        let rising = keyed_at.iter().zip(&keyed_at[keyed_at.len().min(1)..]).all(|(lo, hi)| lo <= hi);
        if !rising {
            let name = reel.to_owned();
            return Err(ClipLoadFault::KeyFramesBackwards { name });
        }
        let on_every_frame = keyed_at.iter().zip(0usize..).all(|(&f, i)| usize::from(f) == i);
        let frames = match on_every_frame {
            true => KeyframeIds::Dense,
            false => KeyframeIds::Sparse(keyed_at),
        };
        Ok(Self { frames, values: samples })
    }

    /// The track at frame position `at_frac`, between the two keys around it.
    fn value_at(&self, at_frac: f32, lerp: fn(T, T, f32) -> T) -> Option<T> {
        let (a, b, t) = match &self.frames {
            KeyframeIds::Dense => time_index_dense(self.values.len(), at_frac),
            KeyframeIds::Sparse(keys) => time_index(keys, at_frac),
        };
        Some(lerp(*self.values.get(a)?, *self.values.get(b)?, t))
    }
}

/// A track kind that can be read at a frame position.
trait AtFrame {
    type Value;
    fn eval_at(&self, frac_at: f32) -> Option<Self::Value>;
}

impl AtFrame for Rotation {
    type Value = [f32; 4];

    fn eval_at(&self, u: f32) -> Option<[f32; 4]> {
        match self {
            Rotation::HalfKeyed(keys) | Rotation::FullKeyed(keys) => keys.value_at(u, lerp4),
            Rotation::HalfStatic(q) | Rotation::FullStatic(q) => Some(*q),
            Rotation::Default => None,
        }
    }
}

impl AtFrame for RootShift {
    type Value = [f32; 3];

    fn eval_at(&self, s_frac: f32) -> Option<[f32; 3]> {
        match self {
            RootShift::SmallKeyed(keys) | RootShift::FullKeyed(keys) => keys.value_at(s_frac, blend_triple),
            RootShift::Fixed(v) => Some(*v),
            RootShift::Default => None,
        }
    }
}

/// A bone with no translation track of its own.
const UNMOVED: RootShift = RootShift::Default;

impl SkeletalClip {
    /// Decodes `parts`. Rotations are stored in bone order, grouped by kind; each
    /// translation names its bone. Every pool must be read to its end.
    pub fn from_parts(pieces: &StoredClipParts) -> Result<Self, ClipLoadFault> {
        let counts = pieces.joint_total.map(usize::from);
        let (rotation_counts, translation_counts) = (&counts[..5], &counts[5..9]);
        let bones = pieces.names.len();
        let (rotation_quaternions, shift_offset) = (
            rotation_counts.iter().sum::<usize>(),
            translation_counts.iter().sum::<usize>(),
        );
        if bones != rotation_quaternions || bones != shift_offset {
            let name = pieces.name.clone();
            return Err(ClipLoadFault::KindCountMismatch { name, names: bones, rotation_quaternions, shift_offset });
        }

        // A clip under 256 frames stores its key frames as single bytes.
        let byte_frames = pieces.frame_total < 256;
        let mut pools = Pools::start(pieces);
        let rotations = read_rotations(&mut pools, rotation_counts, byte_frames)?;
        let mut tracks: Vec<Track> = rotations
            .into_iter()
            .zip(&pieces.names)
            .map(|(rotation, bone)| Track {
                rotation,
                move_track: UNMOVED,
                name: String::clone(bone),
            })
            .collect();
        read_translations(
            &mut pools,
            translation_counts,
            byte_frames,
            &mut tracks,
            &pieces.name,
        )?;
        pools.finish()?;

        let name = pieces.name.clone();
        let notifies = pieces.notifies.clone();
        let (framerate, numframes) = (pieces.fps, pieces.frame_total);
        let looping = pieces.flags & FLAG_LOOP != 0;
        let root_motion_shift = root_motion_shift(pieces);
        Ok(Self {
            looping,
            tracks,
            root_motion_shift,
            name,
            fps: framerate,
            frame_total: numframes,
            notifies,
        })
    }

    /// Seconds from the first frame to the last.
    pub fn duration(&self) -> f32 {
        match self.fps {
            rate if rate > 0.0 => f32::from(self.frame_total) / rate,
            _ => 0.0,
        }
    }

    /// Plays of the whole clip per second.
    pub fn plays_per_second(&self) -> f32 {
        match self.duration() {
            length if length > 0.0 => length.recip(),
            _ => 0.0,
        }
    }

    /// `t_secs` seconds in as a 0..1 clip time: wrapped when the clip loops, else held at
    /// the ends.
    fn time_fraction(&self, t_secs: f32) -> f32 {
        let phase = t_secs * self.plays_per_second();
        match self.looping {
            true => phase - phase.floor(),
            false => phase.clamp(0.0, 1.0),
        }
    }

    fn time_to_frame(&self, at_secs: f32) -> f32 {
        frame_position(self.time_fraction(at_secs), self.frame_total)
    }

    /// The frames that carry a "jumpcut" notetrack.
    fn jumpcut_frames(&self) -> Vec<i32> {
        let is_cut = |note: &&ClipEventMark| note.name.eq_ignore_ascii_case("jumpcut");
        self.notifies
            .iter()
            .filter(is_cut)
            .map(|note| jumpcut_frame(note.time, self.frame_total))
            .collect()
    }

    /// Track `lane` at `time` seconds. A frame with a "jumpcut" notetrack is held, not
    /// blended into the next.
    pub fn lane_at(&self, lane: usize, time: f32) -> Option<EvaluatedChannel> {
        let bone = self.tracks.get(lane)?;
        let frame = snap_jumpcut(self.time_to_frame(time), &self.jumpcut_frames());
        let sampled = EvaluatedChannel {
            move_track: bone.move_track.eval_at(frame),
            rotation: bone.rotation.eval_at(frame),
        };
        Some(sampled)
    }

    /// Where the root has moved to at clip time `t01` (0..1); nowhere without a delta.
    #[must_use]
    pub fn absolute_root_shift(&self, t01: f32) -> [f32; 3] {
        let frame = frame_position(t01.clamp(0.0, 1.0), self.frame_total);
        self.root_motion_shift.eval_at(frame).unwrap_or_default()
    }

    #[must_use]
    pub fn span_in_ms(&self) -> i32 {
        (self.duration() * 1_000.0) as i32
    }

    /// The clip moves its root.
    #[must_use]
    pub fn moves_root(&self) -> bool {
        match self.root_motion_shift {
            RootShift::Default => false,
            _ => true,
        }
    }

    /// The notetracks passed going from `from_secs` to `to_secs` seconds, in the order they
    /// fire (across the loop seam when the clip loops). Nameless notes and "end" are left
    /// out.
    pub fn notes_between(&self, from_secs: f32, to_secs: f32) -> Vec<String> {
        let length = self.duration();
        if length <= f32::EPSILON || self.notifies.is_empty() {
            return Vec::new();
        }
        let as_fraction = |seconds: f32| (seconds / length).clamp(0.0, 1.0);
        let (from, to) = (as_fraction(from_secs), as_fraction(to_secs));
        if from == to {
            return Vec::new();
        }
        let at: Vec<f32> = self
            .notifies
            .iter()
            .map(|note| note.time.clamp(0.0, 1.0))
            .collect();
        let fired = fired_notes(&at, from, to, self.looping, false);
        fired
            .notes
            .into_iter()
            .map(|index| self.notifies[index].name.as_str())
            .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("end"))
            .map(str::to_owned)
            .collect()
    }
}

/// The rotation of every bone, in bone order. `counts` is how many bones use each of
/// the five kinds: unchanged, half-keyed, full-keyed, half-constant, full-constant.
fn read_rotations(
    pools: &mut Pools<'_>,
    counts: &[usize],
    byte_frames: bool,
) -> Result<Vec<Rotation>, ClipLoadFault> {
    let mut rotations = Vec::with_capacity(counts.iter().sum());
    for (kind, &bones) in counts.iter().enumerate() {
        for _ in 0..bones {
            let rotation = match kind {
                0 => Rotation::Default,
                1 => {
                    let size = pools.short()?;
                    let keys = pools.keys(size, byte_frames, |p| Ok(quat2(p.random_signed()?)))?;
                    Rotation::HalfKeyed(keys)
                }
                2 => {
                    let size = pools.short()?;
                    let keys = pools.keys(size, byte_frames, |p| Ok(quat4(p.random_signed()?)))?;
                    Rotation::FullKeyed(keys)
                }
                3 => Rotation::HalfStatic(quat2(pools.signed()?)),
                _ => Rotation::FullStatic(quat4(pools.signed()?)),
            };
            rotations.push(rotation);
        }
    }
    Ok(rotations)
}

/// Gives each bone its translation. `counts` is how many bones use each of the four
/// kinds: small-keyed, full-keyed, constant, unchanged. A bone is named once, and
/// every bone has to be named.
fn read_translations(
    pools: &mut Pools<'_>,
    counts: &[usize],
    byte_frames: bool,
    lanes: &mut [Track],
    reel: &str,
) -> Result<(), ClipLoadFault> {
    let mut named = vec![false; lanes.len()];
    for (kind, &bones) in counts.iter().enumerate() {
        for _ in 0..bones {
            let bone = usize::from(pools.next_u8()?);
            let total = lanes.len();
            if bone >= total {
                let name = reel.to_owned();
                return Err(ClipLoadFault::TrackBoneOutOfRange { name, joint: bone, lanes: total });
            }
            let seen_before = std::mem::replace(&mut named[bone], true);
            if seen_before {
                let name = reel.to_owned();
                return Err(ClipLoadFault::TranslationTwice { name, bone });
            }
            lanes[bone].move_track = match kind {
                0 | 1 => {
                    let size = pools.short()?;
                    let (mins, step) = (pools.float3()?, pools.float3()?);
                    if kind == 0 {
                        RootShift::SmallKeyed(pools.keys(size, byte_frames, |p| {
                            Ok(unpack_trans(mins, step, p.random_bytes()?))
                        })?)
                    } else {
                        RootShift::FullKeyed(pools.keys(size, byte_frames, |p| {
                            Ok(unpack_trans(mins, step, p.random_signed()?.map(|v| v as u16)))
                        })?)
                    }
                }
                2 => RootShift::Fixed(pools.float3()?),
                _ => RootShift::Default,
            };
        }
    }
    if let Some(bone) = named.iter().position(|&done| !done) {
        let name = reel.to_owned();
        return Err(ClipLoadFault::TranslationAbsent { name, bone });
    }
    Ok(())
}

/// The root-motion track. A delta that is flagged off, missing or malformed is none.
fn root_motion_shift(pieces: &StoredClipParts) -> RootShift {
    let flagged = pieces.flags & FLAG_DELTA != 0;
    match pieces.delta_trans.as_ref().filter(|_| flagged) {
        Some(delta) => keyed_delta(pieces, delta).unwrap_or(RootShift::Default),
        None => RootShift::Default,
    }
}

/// The track of a flagged delta, or `None` when its key list or packed bytes do not fit.
fn keyed_delta(pieces: &StoredClipParts, lane_src: &StoredRootMotion) -> Option<RootShift> {
    let last_key = lane_src.size;
    if last_key == 0 {
        return Some(RootShift::Fixed(lane_src.base));
    }
    let keys = usize::from(last_key) + 1;
    if lane_src.indices.len() != keys {
        return None;
    }
    let values = delta_values(lane_src, keys)?;
    let keyed_lane = TimedValue::new(values, lane_src.indices.clone(), &pieces.name).ok()?;
    Some(if lane_src.packed16 {
        RootShift::SmallKeyed(keyed_lane)
    } else {
        RootShift::FullKeyed(keyed_lane)
    })
}

/// `keys` positions unpacked from the packed bytes of a delta: three bytes per key
/// when it is small, else three little-endian shorts. `None` if the bytes run short.
fn delta_values(delta: &StoredRootMotion, keys: usize) -> Option<Vec<[f32; 3]>> {
    let (mins, step) = (delta.base, delta.scale);
    if delta.packed16 {
        let bytes = delta.packed.get(..keys * 3)?;
        return Some(
            bytes
                .chunks_exact(3)
                .map(|key| unpack_trans(mins, step, [key[0], key[1], key[2]]))
                .collect(),
        );
    }
    let bytes = delta.packed.get(..keys * 6)?;
    Some(
        bytes
            .chunks_exact(6)
            .map(|key| {
                let component = |axis: usize| u16::from_le_bytes([key[axis * 2], key[axis * 2 + 1]]);
                unpack_trans(mins, step, [component(0), component(1), component(2)])
            })
            .collect(),
    )
}

/// The pools of an XAnimParts, in the order their reads are checked at the end.
#[derive(Clone, Copy)]
enum Pool {
    Byte,
    Short,
    Int,
    RandomByte,
    RandomShort,
    Index,
}

impl Pool {
    const ALL: [Pool; 6] = [
        Pool::Byte,
        Pool::Short,
        Pool::Int,
        Pool::RandomByte,
        Pool::RandomShort,
        Pool::Index,
    ];

    fn title(self) -> &'static str {
        match self {
            Pool::Byte => "dataByte",
            Pool::Short => "dataShort",
            Pool::Int => "dataInt",
            Pool::RandomByte => "randomDataByte",
            Pool::RandomShort => "randomDataShort",
            Pool::Index => "indices",
        }
    }

    fn size(self, pieces: &StoredClipParts) -> usize {
        match self {
            Pool::Byte => pieces.payload_u8.len(),
            Pool::Short => pieces.payload_i16.len(),
            Pool::Int => pieces.payload_i32.len(),
            Pool::RandomByte => pieces.noise_octet.len(),
            Pool::RandomShort => pieces.noise_word16.len(),
            Pool::Index => pieces.indices.len(),
        }
    }
}

/// How far each pool of an XAnimParts has been read.
struct Pools<'a> {
    source: &'a StoredClipParts,
    read: [usize; 6],
}

impl<'a> Pools<'a> {
    fn start(pieces: &'a StoredClipParts) -> Self {
        Self { read: [0; 6], source: pieces }
    }

    /// Claims the next `count` entries of `pool` and says where they are.
    fn claim(&mut self, pool: Pool, count: usize) -> Result<std::ops::Range<usize>, ClipLoadFault> {
        let start = self.read[pool as usize];
        let remaining = pool.size(self.source).saturating_sub(start);
        if remaining < count {
            return Err(ClipLoadFault::Exhausted { source: pool.title(), want: count, left_bytes: remaining });
        }
        self.read[pool as usize] = start + count;
        Ok(start..start + count)
    }

    fn next_u8(&mut self) -> Result<u8, ClipLoadFault> {
        let at = self.claim(Pool::Byte, 1)?.start;
        Ok(self.source.payload_u8[at])
    }

    fn short(&mut self) -> Result<u16, ClipLoadFault> {
        let at = self.claim(Pool::Short, 1)?.start;
        Ok(self.source.payload_i16[at])
    }

    /// `N` consecutive dataShort entries, read as signed.
    fn signed<const N: usize>(&mut self) -> Result<[i16; N], ClipLoadFault> {
        let mut out = [0i16; N];
        for slot in &mut out {
            *slot = self.short()? as i16;
        }
        Ok(out)
    }

    fn float3(&mut self) -> Result<[f32; 3], ClipLoadFault> {
        let at = self.claim(Pool::Int, 3)?;
        let bits = &self.source.payload_i32[at];
        Ok(std::array::from_fn(|axis| f32::from_bits(bits[axis])))
    }

    /// `N` consecutive randomDataByte entries, claimed one at a time.
    fn random_bytes<const N: usize>(&mut self) -> Result<[u8; N], ClipLoadFault> {
        let mut read_bytes = [0u8; N];
        for slot in &mut read_bytes {
            let at = self.claim(Pool::RandomByte, 1)?.start;
            *slot = self.source.noise_octet[at];
        }
        Ok(read_bytes)
    }

    /// `N` consecutive randomDataShort entries, claimed one at a time and read as signed.
    fn random_signed<const N: usize>(&mut self) -> Result<[i16; N], ClipLoadFault> {
        let mut out = [0i16; N];
        for slot in &mut out {
            let at = self.claim(Pool::RandomShort, 1)?.start;
            *slot = self.source.noise_word16[at] as i16;
        }
        Ok(out)
    }

    /// The key frames of a track whose last key is `size`. Long lists live in `indices`,
    /// with (keys - 2) / 256 + 2 lookup entries for them in dataShort, which are skipped.
    fn frames(&mut self, size: u16, bytes: bool) -> Result<Vec<u16>, ClipLoadFault> {
        let keys = usize::from(size) + 1;
        if bytes {
            return (0..keys).map(|_| self.next_u8().map(u16::from)).collect();
        }
        if size < 64 {
            return (0..keys).map(|_| self.short()).collect();
        }
        let listed = self.claim(Pool::Index, keys)?;
        let frames = self.source.indices[listed].to_vec();
        self.claim(Pool::Short, (keys - 2) / 256 + 2)?;
        Ok(frames)
    }

    /// A keyed track whose last key is `size`: its frames, then a value per key.
    fn keys<T: Copy>(
        &mut self,
        size: u16,
        byte_frames: bool,
        mut value: impl FnMut(&mut Self) -> Result<T, ClipLoadFault>,
    ) -> Result<TimedValue<T>, ClipLoadFault> {
        let frames = self.frames(size, byte_frames)?;
        let mut values = Vec::with_capacity(frames.len());
        for _ in 0..=size {
            values.push(value(self)?);
        }
        TimedValue::new(values, frames, &self.source.name)
    }

    /// Every pool read to its end, checked in storage order.
    fn finish(&self) -> Result<(), ClipLoadFault> {
        let unread = |pool: &Pool| pool.size(self.source) != self.read[*pool as usize];
        if let Some(pool) = Pool::ALL.iter().copied().find(unread) {
            let left = pool.size(self.source).saturating_sub(self.read[pool as usize]);
            let name = self.source.name.clone();
            return Err(ClipLoadFault::Trailing { name, pool: pool.title(), left_bytes: left });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floats(v: &[f32]) -> Vec<u32> {
        v.iter().map(|f| f.to_bits()).collect()
    }

    /// Two bones over frames 0..=2 at 2 fps: "root" with no rotation and a constant
    /// translation, "arm" with a keyed full rotation and keyed byte translation.
    fn two_bones() -> StoredClipParts {
        StoredClipParts {
            name: "test_anim".into(),
            frame_total: 2,
            fps: 2.0,
            flags: FLAG_LOOP,
            names: vec!["root".into(), "arm".into()],
            joint_total: [1, 0, 1, 0, 0, 1, 0, 1, 0, 0],
            // arm's rotation frames, arm's translation bone and frames, root's bone.
            payload_u8: vec![0, 1, 2, 1, 0, 2, 0],
            // arm's last rotation key, arm's last translation key.
            payload_i16: vec![2, 1],
            // arm's translation mins and step, root's constant.
            payload_i32: floats(&[0.0, 0.0, 0.0, 1.0, 2.0, 0.5, 4.0, 5.0, 6.0]),
            noise_word16: vec![0, 0, 0, 32767, 0, 0, 32767, 0, 0, 0, 0, 32767],
            noise_octet: vec![0, 0, 0, 10, 10, 10],
            ..Default::default()
        }
    }

    #[test]
    fn a_two_bone_clip_decodes() {
        let clip = SkeletalClip::from_parts(&two_bones()).unwrap();
        assert!(clip.looping);
        assert_eq!(clip.duration(), 1.0);
        assert_eq!(clip.plays_per_second(), 1.0);
        assert_eq!(clip.span_in_ms(), 1000);
        assert!(!clip.moves_root());

        let root = &clip.tracks[0];
        assert_eq!(root.name, "root");
        assert!(matches!(root.rotation, Rotation::Default));
        assert!(matches!(root.move_track, RootShift::Fixed(v) if v == [4.0, 5.0, 6.0]));

        let arm = &clip.tracks[1];
        let Rotation::FullKeyed(turns) = &arm.rotation else {
            panic!("{:?}", arm.rotation);
        };
        assert_eq!(turns.frames, KeyframeIds::Dense);
        assert_eq!(turns.values[1], quat4([0, 0, 32767, 0]));
        let RootShift::SmallKeyed(moves) = &arm.move_track else {
            panic!("{:?}", arm.move_track);
        };
        assert_eq!(moves.frames, KeyframeIds::Sparse(vec![0, 2]));
        assert_eq!(moves.values, vec![[0.0; 3], [10.0, 20.0, 5.0]]);
    }

    #[test]
    fn sampling_blends_between_keys() {
        let clip = SkeletalClip::from_parts(&two_bones()).unwrap();
        // 0.25 s is frame 0.5: halfway between the first two rotation keys, a quarter of
        // the way along the translation's one span.
        let arm = clip.lane_at(1, 0.25).unwrap();
        let half_turn = lerp4(quat4([0, 0, 0, 32767]), quat4([0, 0, 32767, 0]), 0.5);
        assert_eq!(arm.rotation, Some(half_turn));
        assert_eq!(arm.move_track, Some([2.5, 5.0, 1.25]));
        let root = clip.lane_at(0, 0.25).unwrap();
        assert_eq!(
            (root.rotation, root.move_track),
            (None, Some([4.0, 5.0, 6.0]))
        );
        assert!(clip.lane_at(2, 0.0).is_none());
        // Looping wraps 1.25 s back to 0.25 s.
        assert_eq!(
            clip.lane_at(1, 1.25).unwrap().move_track,
            arm.move_track
        );
    }

    #[test]
    fn a_jumpcut_holds_its_frame() {
        let mut clip = SkeletalClip::from_parts(&two_bones()).unwrap();
        clip.notifies.push(ClipEventMark {
            name: "JumpCut".into(),
            time: 0.5,
        });
        // The cut is after frame 0, so frame 0.5 is held at frame 0.
        let arm = clip.lane_at(1, 0.25).unwrap();
        assert_eq!(arm.move_track, Some([0.0; 3]));
    }

    #[test]
    fn counts_must_cover_each_bone_once() {
        let mut parts = two_bones();
        parts.joint_total[0] = 2;
        assert_eq!(
            SkeletalClip::from_parts(&parts).unwrap_err(),
            ClipLoadFault::KindCountMismatch {
                name: "test_anim".into(),
                names: 2,
                rotation_quaternions: 3,
                shift_offset: 2,
            }
        );
    }

    #[test]
    fn translations_name_each_bone_once() {
        let mut parts = two_bones();
        parts.payload_u8[6] = 1;
        assert_eq!(
            SkeletalClip::from_parts(&parts).unwrap_err(),
            ClipLoadFault::TranslationTwice {
                name: "test_anim".into(),
                bone: 1
            }
        );
        parts.payload_u8[6] = 7;
        assert_eq!(
            SkeletalClip::from_parts(&parts).unwrap_err(),
            ClipLoadFault::TrackBoneOutOfRange {
                name: "test_anim".into(),
                joint: 7,
                lanes: 2
            }
        );
        // Root's constant becomes a default that names arm again.
        let mut parts = two_bones();
        parts.joint_total[7] = 0;
        parts.joint_total[8] = 1;
        parts.payload_i32.truncate(6);
        parts.payload_u8[6] = 1;
        assert!(matches!(
            SkeletalClip::from_parts(&parts),
            Err(ClipLoadFault::TranslationTwice { bone: 1, .. })
        ));
    }

    #[test]
    fn short_and_leftover_pools() {
        let mut parts = two_bones();
        parts.noise_word16.truncate(11);
        let short_pool = ClipLoadFault::Exhausted {
            source: "randomDataShort",
            want: 1,
            left_bytes: 0,
        };
        assert_eq!(SkeletalClip::from_parts(&parts).unwrap_err(), short_pool);
        let mut parts = two_bones();
        parts.payload_i32.push(0);
        parts.noise_octet.push(0);
        assert_eq!(
            SkeletalClip::from_parts(&parts).unwrap_err(),
            ClipLoadFault::Trailing {
                name: "test_anim".into(),
                pool: "dataInt",
                left_bytes: 1
            }
        );
    }

    #[test]
    fn key_frames_must_rise() {
        let mut parts = two_bones();
        parts.payload_u8[1..3].copy_from_slice(&[2, 1]);
        assert_eq!(
            SkeletalClip::from_parts(&parts).unwrap_err(),
            ClipLoadFault::KeyFramesBackwards {
                name: "test_anim".into()
            }
        );
    }

    #[test]
    fn long_clips_keep_long_key_lists_apart() {
        // 300 frames, one bone with a half rotation of 65 keys: its frames come from
        // `indices`, and dataShort holds the size and two lookup entries.
        let parts = StoredClipParts {
            name: "long".into(),
            frame_total: 300,
            fps: 30.0,
            names: vec!["root".into()],
            joint_total: [0, 1, 0, 0, 0, 0, 0, 0, 1, 0],
            payload_i16: vec![64, 0, 0],
            indices: (0..65).map(|i| i * 4).collect(),
            noise_word16: vec![0; 130],
            payload_u8: vec![0],
            ..Default::default()
        };
        let clip = SkeletalClip::from_parts(&parts).unwrap();
        let Rotation::HalfKeyed(keys) = &clip.tracks[0].rotation else {
            panic!();
        };
        assert_eq!(keys.values.len(), 65);
        assert!(matches!(&keys.frames, KeyframeIds::Sparse(f) if f[64] == 256));
        let mut short = parts.clone();
        short.payload_i16.pop();
        let lookup_missing = ClipLoadFault::Exhausted {
            source: "dataShort",
            want: 2,
            left_bytes: 1,
        };
        assert_eq!(SkeletalClip::from_parts(&short).unwrap_err(), lookup_missing);
    }

    fn with_delta(small: bool, packed: Vec<u8>) -> SkeletalClip {
        let mut parts = two_bones();
        parts.flags |= FLAG_DELTA;
        parts.delta_trans = Some(StoredRootMotion {
            size: 1,
            packed16: small,
            base: [1.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            indices: vec![0, 2],
            packed,
        });
        SkeletalClip::from_parts(&parts).unwrap()
    }

    #[test]
    fn the_root_motion_track() {
        let small = with_delta(true, vec![0, 0, 0, 4, 8, 0]);
        assert!(small.moves_root());
        assert_eq!(small.absolute_root_shift(0.0), [1.0, 0.0, 0.0]);
        assert_eq!(small.absolute_root_shift(0.5), [3.0, 4.0, 0.0]);
        assert_eq!(small.absolute_root_shift(9.0), [5.0, 8.0, 0.0]);
        let full = with_delta(false, vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0]);
        assert!(matches!(full.root_motion_shift, RootShift::FullKeyed(_)));
        assert_eq!(full.absolute_root_shift(1.0), [257.0, 0.0, 2.0]);
        // Too few packed bytes, or the flag off: no root motion.
        assert!(!with_delta(true, vec![0; 5]).moves_root());
        let mut parts = two_bones();
        parts.delta_trans = one_key_delta();
        assert!(!SkeletalClip::from_parts(&parts).unwrap().moves_root());
        assert_eq!(
            SkeletalClip::from_parts(&parts).unwrap().absolute_root_shift(0.5),
            [0.0; 3]
        );
    }

    fn one_key_delta() -> Option<StoredRootMotion> {
        let delta = StoredRootMotion {
            size: 0,
            base: [1.0, 2.0, 3.0],
            ..Default::default()
        };
        Some(delta)
    }

    #[test]
    fn a_malformed_delta_is_none() {
        let mut parts = two_bones();
        parts.flags |= FLAG_DELTA;
        let mut delta = StoredRootMotion {
            size: 1,
            packed16: true,
            indices: vec![0, 2],
            packed: vec![0; 6],
            ..Default::default()
        };
        parts.delta_trans = Some(delta.clone());
        assert!(SkeletalClip::from_parts(&parts).unwrap().moves_root());
        delta.indices = vec![0];
        parts.delta_trans = Some(delta.clone());
        assert!(!SkeletalClip::from_parts(&parts).unwrap().moves_root());
        delta.indices = vec![2, 0];
        parts.delta_trans = Some(delta);
        assert!(!SkeletalClip::from_parts(&parts).unwrap().moves_root());
    }

    #[test]
    fn a_one_key_delta_is_constant() {
        let mut parts = two_bones();
        parts.flags |= FLAG_DELTA;
        parts.delta_trans = one_key_delta();
        let clip = SkeletalClip::from_parts(&parts).unwrap();
        assert_eq!(clip.absolute_root_shift(0.7), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn notetracks_crossed() {
        let mut clip = SkeletalClip::from_parts(&two_bones()).unwrap();
        clip.notifies = [("fire", 0.25), ("", 0.5), ("END", 1.0), ("reload", 0.75)]
            .map(|(label, at_s)| ClipEventMark {
                name: label.into(),
                time: at_s,
            })
            .into();
        assert_eq!(clip.notes_between(0.1, 0.8), ["fire", "reload"]);
        assert!(clip.notes_between(0.3, 0.3).is_empty());
        // Looping past the end wraps to the start.
        assert_eq!(clip.notes_between(0.8, 0.3), ["fire"]);
        clip.fps = 0.0;
        assert!(clip.notes_between(0.1, 0.8).is_empty());
    }
}
