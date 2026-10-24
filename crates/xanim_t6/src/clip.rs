//! One XAnimParts decoded for playback: a rotation and a translation track per bone, the
//! notetracks, and the root-motion delta.

use crate::{
    fired_notes, frame_position, jumpcut_frame, lerp3, lerp4, quat2, quat4, snap_jumpcut,
    time_index, time_index_dense, unpack_trans,
};

type Quat = [f32; 4];
type Vec3 = [f32; 3];

/// Flag bit: the clip loops.
const FLAG_LOOP: u8 = 1;
/// Flag bit: the clip moves its root (it has a delta track).
const FLAG_DELTA: u8 = 2;

/// An XAnimParts as stored: the pools its tracks are read from, front to back, in bone
/// order.
#[derive(Debug, Clone, Default)]
pub struct RawXAnimParts {
    pub name: String,
    pub data_byte: Vec<u8>,
    pub data_short: Vec<u16>,
    pub data_int: Vec<u32>,
    pub random_data_byte: Vec<u8>,
    pub random_data_short: Vec<u16>,
    pub random_data_int: Vec<u32>,
    /// The last frame's number: a clip of `numframes + 1` frames.
    pub numframes: u16,
    pub flags: u8,
    /// How many bones use each track kind: five rotation kinds, then four translation
    /// kinds.
    pub bone_count: [u8; 10],
    pub framerate: f32,
    /// One name per bone.
    pub names: Vec<String>,
    pub notifies: Vec<ClipNotify>,
    /// The key frames of long keyed tracks (64 keys or more) in long clips (256 frames or
    /// more).
    pub indices: Vec<u16>,
    pub delta_trans: Option<RawDeltaTrans>,
}

/// The root-motion track as stored: `size + 1` keys, each `mins + step * key`, packed as
/// three bytes (`small`) or three little-endian u16.
#[derive(Debug, Clone, Default)]
pub struct RawDeltaTrans {
    pub size: u16,
    pub small: bool,
    pub mins: [f32; 3],
    pub step: [f32; 3],
    pub indices: Vec<u16>,
    pub packed: Vec<u8>,
}

/// Why an XAnimParts did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipError {
    /// A read ran past the end of a pool.
    Exhausted {
        pool: &'static str,
        needed: usize,
        remaining: usize,
    },
    /// The tracks were read and a pool still had data left.
    Trailing {
        name: String,
        pool: &'static str,
        remaining: usize,
    },
    /// The rotation or translation kinds do not cover each named bone once.
    InvalidBoneCounts {
        name: String,
        names: usize,
        quats: usize,
        trans: usize,
    },
    InvalidBoneIndex {
        name: String,
        bone: usize,
        tracks: usize,
    },
    DuplicateTranslation {
        name: String,
        bone: usize,
    },
    MissingTranslation {
        name: String,
        bone: usize,
    },
    /// A keyed track's key frames go backwards.
    NonMonotonicIndices {
        name: String,
    },
}

/// A keyed track: one value per key and the frame each key sits on.
#[derive(Debug, Clone)]
pub struct Keyed<T> {
    pub values: Vec<T>,
    pub frames: FrameIndices,
}

/// The frames a track's keys sit on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameIndices {
    /// Key `i` is on frame `i`.
    Dense,
    Sparse(Vec<u16>),
}

/// A bone's rotation track. Half rotations turn about z only.
#[derive(Debug, Clone)]
pub enum Rotation {
    /// The clip leaves the bone's rotation alone.
    Default,
    HalfKeyed(Keyed<Quat>),
    FullKeyed(Keyed<Quat>),
    HalfConstant(Quat),
    FullConstant(Quat),
}

/// A bone's translation track.
#[derive(Debug, Clone)]
pub enum Translation {
    /// The clip leaves the bone where it is.
    Default,
    /// Keys stored as bytes.
    SmallKeyed(Keyed<Vec3>),
    /// Keys stored as u16.
    FullKeyed(Keyed<Vec3>),
    Constant(Vec3),
}

/// One bone the clip drives.
#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub rotation: Rotation,
    pub translation: Translation,
}

/// An XAnimParts ready to play.
#[derive(Debug, Clone)]
pub struct AnimClip {
    pub name: String,
    pub framerate: f32,
    pub numframes: u16,
    pub looping: bool,
    pub tracks: Vec<Track>,
    pub notifies: Vec<ClipNotify>,
    pub delta_translation: Translation,
}

/// A notetrack: its name and where it sits, 0 at the start of the clip and 1 at the end.
#[derive(Debug, Clone)]
pub struct ClipNotify {
    pub name: String,
    pub time: f32,
}

/// One track at one time. `None` where the track leaves the bone alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SampledTrack {
    pub rotation: Option<Quat>,
    pub translation: Option<Vec3>,
}

impl<T: Copy> Keyed<T> {
    fn new(values: Vec<T>, frames: Vec<u16>, clip: &str) -> Result<Self, ClipError> {
        if frames.windows(2).any(|pair| pair[1] < pair[0]) {
            return Err(ClipError::NonMonotonicIndices {
                name: clip.to_owned(),
            });
        }
        let dense = frames.iter().enumerate().all(|(i, &f)| usize::from(f) == i);
        Ok(Self {
            values,
            frames: if dense {
                FrameIndices::Dense
            } else {
                FrameIndices::Sparse(frames)
            },
        })
    }

    /// The track at frame position `f`, between the two keys around it.
    fn sample(&self, f: f32, lerp: fn(T, T, f32) -> T) -> Option<T> {
        let (a, b, t) = match &self.frames {
            FrameIndices::Dense => time_index_dense(self.values.len(), f),
            FrameIndices::Sparse(keys) => time_index(keys, f),
        };
        Some(lerp(*self.values.get(a)?, *self.values.get(b)?, t))
    }
}

impl Rotation {
    fn sample(&self, f: f32) -> Option<Quat> {
        match self {
            Self::Default => None,
            Self::HalfConstant(q) | Self::FullConstant(q) => Some(*q),
            Self::HalfKeyed(keys) | Self::FullKeyed(keys) => keys.sample(f, lerp4),
        }
    }
}

impl Translation {
    fn sample(&self, f: f32) -> Option<Vec3> {
        match self {
            Self::Default => None,
            Self::Constant(v) => Some(*v),
            Self::SmallKeyed(keys) | Self::FullKeyed(keys) => keys.sample(f, lerp3),
        }
    }
}

impl AnimClip {
    /// Decodes `parts`. Rotations are stored in bone order, grouped by kind; each
    /// translation names its bone. Every pool must be read to its end.
    pub fn from_parts(parts: &RawXAnimParts) -> Result<Self, ClipError> {
        let counts = parts.bone_count.map(usize::from);
        let quats: usize = counts[..5].iter().sum();
        let trans: usize = counts[5..9].iter().sum();
        let names = parts.names.len();
        if names != quats || names != trans {
            return Err(ClipError::InvalidBoneCounts {
                name: parts.name.clone(),
                names,
                quats,
                trans,
            });
        }

        // Short clips store key frames as bytes.
        let byte_frames = parts.numframes < 256;
        let mut pools = Pools::new(parts);

        let mut rotations = Vec::with_capacity(names);
        rotations.extend((0..counts[0]).map(|_| Rotation::Default));
        for _ in 0..counts[1] {
            let size = pools.short()?;
            rotations.push(Rotation::HalfKeyed(pools.keys(size, byte_frames, |p| {
                Ok(quat2([p.random_short()?, p.random_short()?]))
            })?));
        }
        for _ in 0..counts[2] {
            let size = pools.short()?;
            rotations.push(Rotation::FullKeyed(pools.keys(size, byte_frames, |p| {
                Ok(quat4([
                    p.random_short()?,
                    p.random_short()?,
                    p.random_short()?,
                    p.random_short()?,
                ]))
            })?));
        }
        for _ in 0..counts[3] {
            let zw = [pools.short()? as i16, pools.short()? as i16];
            rotations.push(Rotation::HalfConstant(quat2(zw)));
        }
        for _ in 0..counts[4] {
            let q = [
                pools.short()? as i16,
                pools.short()? as i16,
                pools.short()? as i16,
                pools.short()? as i16,
            ];
            rotations.push(Rotation::FullConstant(quat4(q)));
        }

        let mut tracks: Vec<Track> = parts
            .names
            .iter()
            .zip(rotations)
            .map(|(name, rotation)| Track {
                name: name.clone(),
                rotation,
                translation: Translation::Default,
            })
            .collect();

        let mut placed = vec![false; tracks.len()];
        for kind in 5..9 {
            for _ in 0..counts[kind] {
                let bone = usize::from(pools.byte()?);
                if bone >= tracks.len() {
                    return Err(ClipError::InvalidBoneIndex {
                        name: parts.name.clone(),
                        bone,
                        tracks: tracks.len(),
                    });
                }
                if core::mem::replace(&mut placed[bone], true) {
                    return Err(ClipError::DuplicateTranslation {
                        name: parts.name.clone(),
                        bone,
                    });
                }
                tracks[bone].translation = match kind {
                    5 | 6 => {
                        let size = pools.short()?;
                        let mins = pools.float3()?;
                        let step = pools.float3()?;
                        if kind == 5 {
                            Translation::SmallKeyed(pools.keys(size, byte_frames, |p| {
                                let key = [p.random_byte()?, p.random_byte()?, p.random_byte()?];
                                Ok(unpack_trans(mins, step, key))
                            })?)
                        } else {
                            Translation::FullKeyed(pools.keys(size, byte_frames, |p| {
                                let key = [
                                    p.random_short()? as u16,
                                    p.random_short()? as u16,
                                    p.random_short()? as u16,
                                ];
                                Ok(unpack_trans(mins, step, key))
                            })?)
                        }
                    }
                    7 => Translation::Constant(pools.float3()?),
                    _ => Translation::Default,
                };
            }
        }
        if let Some(bone) = placed.iter().position(|&p| !p) {
            return Err(ClipError::MissingTranslation {
                name: parts.name.clone(),
                bone,
            });
        }
        pools.finish()?;

        Ok(Self {
            name: parts.name.clone(),
            framerate: parts.framerate,
            numframes: parts.numframes,
            looping: parts.flags & FLAG_LOOP != 0,
            tracks,
            notifies: parts.notifies.clone(),
            delta_translation: delta_translation(parts),
        })
    }

    /// Seconds from the first frame to the last.
    pub fn duration(&self) -> f32 {
        if self.framerate > 0.0 {
            f32::from(self.numframes) / self.framerate
        } else {
            0.0
        }
    }

    /// Plays of the whole clip per second.
    pub fn frequency(&self) -> f32 {
        let duration = self.duration();
        if duration > 0.0 { 1.0 / duration } else { 0.0 }
    }

    /// `time` seconds in as a 0..1 clip time: wrapped when the clip loops, else held at
    /// the ends.
    fn time_fraction(&self, time: f32) -> f32 {
        let t = time * self.frequency();
        if self.looping {
            t - t.floor()
        } else {
            t.clamp(0.0, 1.0)
        }
    }

    fn time_to_frame(&self, time: f32) -> f32 {
        frame_position(self.time_fraction(time), self.numframes)
    }

    /// Track `track` at `time` seconds. A frame with a "jumpcut" notetrack is held, not
    /// blended into the next.
    pub fn sample_track(&self, track: usize, time: f32) -> Option<SampledTrack> {
        let track = self.tracks.get(track)?;
        let cuts: Vec<i32> = self
            .notifies
            .iter()
            .filter(|n| n.name.eq_ignore_ascii_case("jumpcut"))
            .map(|n| jumpcut_frame(n.time, self.numframes))
            .collect();
        let f = snap_jumpcut(self.time_to_frame(time), &cuts);
        Some(SampledTrack {
            rotation: track.rotation.sample(f),
            translation: track.translation.sample(f),
        })
    }

    /// Where the root has moved to at clip time `frac` (0..1); nowhere without a delta.
    #[must_use]
    pub fn abs_delta_trans(&self, frac: f32) -> [f32; 3] {
        let f = frame_position(frac.clamp(0.0, 1.0), self.numframes);
        self.delta_translation.sample(f).unwrap_or([0.0; 3])
    }

    #[must_use]
    pub fn length_msec(&self) -> i32 {
        (self.duration() * 1000.0) as i32
    }

    /// The clip moves its root.
    #[must_use]
    pub fn has_delta(&self) -> bool {
        !matches!(self.delta_translation, Translation::Default)
    }

    /// The notetracks passed going from `old_time` to `new_time` seconds, in the order they
    /// fire (across the loop seam when the clip loops). Nameless notes and "end" are left
    /// out.
    pub fn crossed_notifies(&self, old_time: f32, new_time: f32) -> Vec<String> {
        let duration = self.duration();
        if duration <= f32::EPSILON || self.notifies.is_empty() {
            return Vec::new();
        }
        let old = (old_time / duration).clamp(0.0, 1.0);
        let new = (new_time / duration).clamp(0.0, 1.0);
        if old == new {
            return Vec::new();
        }
        let times: Vec<f32> = self
            .notifies
            .iter()
            .map(|n| n.time.clamp(0.0, 1.0))
            .collect();
        fired_notes(&times, old, new, self.looping, false)
            .notes
            .into_iter()
            .map(|i| &self.notifies[i].name)
            .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("end"))
            .cloned()
            .collect()
    }
}

/// The root-motion track. A delta that is flagged off, missing or malformed is none.
fn delta_translation(parts: &RawXAnimParts) -> Translation {
    let Some(delta) = parts.delta_trans.as_ref() else {
        return Translation::Default;
    };
    if parts.flags & FLAG_DELTA == 0 {
        return Translation::Default;
    }
    if delta.size == 0 {
        return Translation::Constant(delta.mins);
    }
    let keys = usize::from(delta.size) + 1;
    if delta.indices.len() != keys {
        return Translation::Default;
    }
    let (mins, step) = (delta.mins, delta.step);
    let values: Option<Vec<Vec3>> = if delta.small {
        delta.packed.get(..keys * 3).map(|bytes| {
            bytes
                .chunks_exact(3)
                .map(|k| unpack_trans(mins, step, [k[0], k[1], k[2]]))
                .collect()
        })
    } else {
        delta.packed.get(..keys * 6).map(|bytes| {
            bytes
                .chunks_exact(6)
                .map(|k| {
                    let axis = |i: usize| u16::from_le_bytes([k[2 * i], k[2 * i + 1]]);
                    unpack_trans(mins, step, [axis(0), axis(1), axis(2)])
                })
                .collect()
        })
    };
    let Some(values) = values else {
        return Translation::Default;
    };
    match Keyed::new(values, delta.indices.clone(), &parts.name) {
        Ok(keys) if delta.small => Translation::SmallKeyed(keys),
        Ok(keys) => Translation::FullKeyed(keys),
        Err(_) => Translation::Default,
    }
}

/// How far each pool of an XAnimParts has been read.
struct Pools<'a> {
    parts: &'a RawXAnimParts,
    byte: usize,
    short: usize,
    int: usize,
    random_byte: usize,
    random_short: usize,
    index: usize,
}

/// The next `count` entries of `pool`, from `at`.
fn take<'p, T>(
    pool: &'p [T],
    at: &mut usize,
    count: usize,
    label: &'static str,
) -> Result<&'p [T], ClipError> {
    let remaining = pool.len().saturating_sub(*at);
    if count > remaining {
        return Err(ClipError::Exhausted {
            pool: label,
            needed: count,
            remaining,
        });
    }
    let run = &pool[*at..*at + count];
    *at += count;
    Ok(run)
}

impl<'a> Pools<'a> {
    fn new(parts: &'a RawXAnimParts) -> Self {
        Self {
            parts,
            byte: 0,
            short: 0,
            int: 0,
            random_byte: 0,
            random_short: 0,
            index: 0,
        }
    }

    fn byte(&mut self) -> Result<u8, ClipError> {
        Ok(take(&self.parts.data_byte, &mut self.byte, 1, "dataByte")?[0])
    }

    fn short(&mut self) -> Result<u16, ClipError> {
        Ok(take(&self.parts.data_short, &mut self.short, 1, "dataShort")?[0])
    }

    fn float3(&mut self) -> Result<Vec3, ClipError> {
        let bits = take(&self.parts.data_int, &mut self.int, 3, "dataInt")?;
        Ok([0, 1, 2].map(|i| f32::from_bits(bits[i])))
    }

    fn random_byte(&mut self) -> Result<u8, ClipError> {
        let pool = &self.parts.random_data_byte;
        Ok(take(pool, &mut self.random_byte, 1, "randomDataByte")?[0])
    }

    fn random_short(&mut self) -> Result<i16, ClipError> {
        let pool = &self.parts.random_data_short;
        Ok(take(pool, &mut self.random_short, 1, "randomDataShort")?[0] as i16)
    }

    /// The key frames of a track whose last key is `size`. Long lists live in `indices`,
    /// with (keys - 2) / 256 + 2 lookup entries for them in dataShort, which are skipped.
    fn frames(&mut self, size: u16, bytes: bool) -> Result<Vec<u16>, ClipError> {
        let keys = usize::from(size) + 1;
        if bytes {
            return (0..keys).map(|_| self.byte().map(u16::from)).collect();
        }
        if size >= 64 {
            let frames = take(&self.parts.indices, &mut self.index, keys, "indices")?.to_vec();
            take(
                &self.parts.data_short,
                &mut self.short,
                (keys - 2) / 256 + 2,
                "dataShort",
            )?;
            return Ok(frames);
        }
        (0..keys).map(|_| self.short()).collect()
    }

    /// A keyed track whose last key is `size`: its frames, then a value per key.
    fn keys<T: Copy>(
        &mut self,
        size: u16,
        byte_frames: bool,
        mut value: impl FnMut(&mut Self) -> Result<T, ClipError>,
    ) -> Result<Keyed<T>, ClipError> {
        let frames = self.frames(size, byte_frames)?;
        let values = (0..=size)
            .map(|_| value(self))
            .collect::<Result<Vec<T>, ClipError>>()?;
        Keyed::new(values, frames, &self.parts.name)
    }

    /// Every pool read to its end, checked in storage order.
    fn finish(&self) -> Result<(), ClipError> {
        let p = self.parts;
        let pools = [
            ("dataByte", p.data_byte.len(), self.byte),
            ("dataShort", p.data_short.len(), self.short),
            ("dataInt", p.data_int.len(), self.int),
            ("randomDataByte", p.random_data_byte.len(), self.random_byte),
            (
                "randomDataShort",
                p.random_data_short.len(),
                self.random_short,
            ),
            ("indices", p.indices.len(), self.index),
        ];
        match pools.into_iter().find(|&(_, len, read)| read != len) {
            Some((pool, len, read)) => Err(ClipError::Trailing {
                name: p.name.clone(),
                pool,
                remaining: len.saturating_sub(read),
            }),
            None => Ok(()),
        }
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
    fn two_bones() -> RawXAnimParts {
        RawXAnimParts {
            name: "test_anim".into(),
            numframes: 2,
            framerate: 2.0,
            flags: FLAG_LOOP,
            names: vec!["root".into(), "arm".into()],
            bone_count: [1, 0, 1, 0, 0, 1, 0, 1, 0, 0],
            // arm's rotation frames, arm's translation bone and frames, root's bone.
            data_byte: vec![0, 1, 2, 1, 0, 2, 0],
            // arm's last rotation key, arm's last translation key.
            data_short: vec![2, 1],
            // arm's translation mins and step, root's constant.
            data_int: floats(&[0.0, 0.0, 0.0, 1.0, 2.0, 0.5, 4.0, 5.0, 6.0]),
            random_data_short: vec![0, 0, 0, 32767, 0, 0, 32767, 0, 0, 0, 0, 32767],
            random_data_byte: vec![0, 0, 0, 10, 10, 10],
            ..Default::default()
        }
    }

    #[test]
    fn a_two_bone_clip_decodes() {
        let clip = AnimClip::from_parts(&two_bones()).unwrap();
        assert!(clip.looping);
        assert_eq!(clip.duration(), 1.0);
        assert_eq!(clip.frequency(), 1.0);
        assert_eq!(clip.length_msec(), 1000);
        assert!(!clip.has_delta());

        let root = &clip.tracks[0];
        assert_eq!(root.name, "root");
        assert!(matches!(root.rotation, Rotation::Default));
        assert!(matches!(root.translation, Translation::Constant(v) if v == [4.0, 5.0, 6.0]));

        let arm = &clip.tracks[1];
        let Rotation::FullKeyed(turns) = &arm.rotation else {
            panic!("{:?}", arm.rotation);
        };
        assert_eq!(turns.frames, FrameIndices::Dense);
        assert_eq!(turns.values[1], quat4([0, 0, 32767, 0]));
        let Translation::SmallKeyed(moves) = &arm.translation else {
            panic!("{:?}", arm.translation);
        };
        assert_eq!(moves.frames, FrameIndices::Sparse(vec![0, 2]));
        assert_eq!(moves.values, vec![[0.0; 3], [10.0, 20.0, 5.0]]);
    }

    #[test]
    fn sampling_blends_between_keys() {
        let clip = AnimClip::from_parts(&two_bones()).unwrap();
        // 0.25 s is frame 0.5: halfway between the first two rotation keys, a quarter of
        // the way along the translation's one span.
        let arm = clip.sample_track(1, 0.25).unwrap();
        let half = lerp4(quat4([0, 0, 0, 32767]), quat4([0, 0, 32767, 0]), 0.5);
        assert_eq!(arm.rotation, Some(half));
        assert_eq!(arm.translation, Some([2.5, 5.0, 1.25]));
        let root = clip.sample_track(0, 0.25).unwrap();
        assert_eq!(
            (root.rotation, root.translation),
            (None, Some([4.0, 5.0, 6.0]))
        );
        assert!(clip.sample_track(2, 0.0).is_none());
        // Looping wraps 1.25 s back to 0.25 s.
        assert_eq!(
            clip.sample_track(1, 1.25).unwrap().translation,
            arm.translation
        );
    }

    #[test]
    fn a_jumpcut_holds_its_frame() {
        let mut clip = AnimClip::from_parts(&two_bones()).unwrap();
        clip.notifies.push(ClipNotify {
            name: "JumpCut".into(),
            time: 0.5,
        });
        // The cut is after frame 0, so frame 0.5 is held at frame 0.
        let arm = clip.sample_track(1, 0.25).unwrap();
        assert_eq!(arm.translation, Some([0.0; 3]));
    }

    #[test]
    fn counts_must_cover_each_bone_once() {
        let mut parts = two_bones();
        parts.bone_count[0] = 2;
        assert_eq!(
            AnimClip::from_parts(&parts).unwrap_err(),
            ClipError::InvalidBoneCounts {
                name: "test_anim".into(),
                names: 2,
                quats: 3,
                trans: 2,
            }
        );
    }

    #[test]
    fn translations_name_each_bone_once() {
        let mut parts = two_bones();
        parts.data_byte[6] = 1;
        assert_eq!(
            AnimClip::from_parts(&parts).unwrap_err(),
            ClipError::DuplicateTranslation {
                name: "test_anim".into(),
                bone: 1
            }
        );
        parts.data_byte[6] = 7;
        assert_eq!(
            AnimClip::from_parts(&parts).unwrap_err(),
            ClipError::InvalidBoneIndex {
                name: "test_anim".into(),
                bone: 7,
                tracks: 2
            }
        );
        // Root's constant becomes a default that names arm again.
        let mut parts = two_bones();
        parts.bone_count[7] = 0;
        parts.bone_count[8] = 1;
        parts.data_int.truncate(6);
        parts.data_byte[6] = 1;
        assert!(matches!(
            AnimClip::from_parts(&parts),
            Err(ClipError::DuplicateTranslation { bone: 1, .. })
        ));
    }

    #[test]
    fn short_and_leftover_pools() {
        let mut parts = two_bones();
        parts.random_data_short.truncate(11);
        assert_eq!(
            AnimClip::from_parts(&parts).unwrap_err(),
            ClipError::Exhausted {
                pool: "randomDataShort",
                needed: 1,
                remaining: 0
            }
        );
        let mut parts = two_bones();
        parts.data_int.push(0);
        parts.random_data_byte.push(0);
        assert_eq!(
            AnimClip::from_parts(&parts).unwrap_err(),
            ClipError::Trailing {
                name: "test_anim".into(),
                pool: "dataInt",
                remaining: 1
            }
        );
    }

    #[test]
    fn key_frames_must_rise() {
        let mut parts = two_bones();
        parts.data_byte[1..3].copy_from_slice(&[2, 1]);
        assert_eq!(
            AnimClip::from_parts(&parts).unwrap_err(),
            ClipError::NonMonotonicIndices {
                name: "test_anim".into()
            }
        );
    }

    #[test]
    fn long_clips_keep_long_key_lists_apart() {
        // 300 frames, one bone with a half rotation of 65 keys: its frames come from
        // `indices`, and dataShort holds the size and two lookup entries.
        let parts = RawXAnimParts {
            name: "long".into(),
            numframes: 300,
            framerate: 30.0,
            names: vec!["root".into()],
            bone_count: [0, 1, 0, 0, 0, 0, 0, 0, 1, 0],
            data_short: vec![64, 0, 0],
            indices: (0..65).map(|i| i * 4).collect(),
            random_data_short: vec![0; 130],
            data_byte: vec![0],
            ..Default::default()
        };
        let clip = AnimClip::from_parts(&parts).unwrap();
        let Rotation::HalfKeyed(keys) = &clip.tracks[0].rotation else {
            panic!();
        };
        assert_eq!(keys.values.len(), 65);
        assert!(matches!(&keys.frames, FrameIndices::Sparse(f) if f[64] == 256));
        let mut short = parts.clone();
        short.data_short.pop();
        assert_eq!(
            AnimClip::from_parts(&short).unwrap_err(),
            ClipError::Exhausted {
                pool: "dataShort",
                needed: 2,
                remaining: 1
            }
        );
    }

    fn with_delta(small: bool, packed: Vec<u8>) -> AnimClip {
        let mut parts = two_bones();
        parts.flags |= FLAG_DELTA;
        parts.delta_trans = Some(RawDeltaTrans {
            size: 1,
            small,
            mins: [1.0, 0.0, 0.0],
            step: [1.0, 1.0, 1.0],
            indices: vec![0, 2],
            packed,
        });
        AnimClip::from_parts(&parts).unwrap()
    }

    #[test]
    fn the_root_motion_track() {
        let small = with_delta(true, vec![0, 0, 0, 4, 8, 0]);
        assert!(small.has_delta());
        assert_eq!(small.abs_delta_trans(0.0), [1.0, 0.0, 0.0]);
        assert_eq!(small.abs_delta_trans(0.5), [3.0, 4.0, 0.0]);
        assert_eq!(small.abs_delta_trans(9.0), [5.0, 8.0, 0.0]);
        let full = with_delta(false, vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0]);
        assert!(matches!(full.delta_translation, Translation::FullKeyed(_)));
        assert_eq!(full.abs_delta_trans(1.0), [257.0, 0.0, 2.0]);
        // Too few packed bytes, or the flag off: no root motion.
        assert!(!with_delta(true, vec![0; 5]).has_delta());
        let mut parts = two_bones();
        parts.delta_trans = one_key_delta();
        assert!(!AnimClip::from_parts(&parts).unwrap().has_delta());
        assert_eq!(
            AnimClip::from_parts(&parts).unwrap().abs_delta_trans(0.5),
            [0.0; 3]
        );
    }

    fn one_key_delta() -> Option<RawDeltaTrans> {
        Some(RawDeltaTrans {
            size: 0,
            mins: [1.0, 2.0, 3.0],
            ..Default::default()
        })
    }

    #[test]
    fn a_one_key_delta_is_constant() {
        let mut parts = two_bones();
        parts.flags |= FLAG_DELTA;
        parts.delta_trans = one_key_delta();
        let clip = AnimClip::from_parts(&parts).unwrap();
        assert_eq!(clip.abs_delta_trans(0.7), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn notetracks_crossed() {
        let mut clip = AnimClip::from_parts(&two_bones()).unwrap();
        clip.notifies = [("fire", 0.25), ("", 0.5), ("END", 1.0), ("reload", 0.75)]
            .map(|(name, time)| ClipNotify {
                name: name.into(),
                time,
            })
            .into();
        assert_eq!(clip.crossed_notifies(0.1, 0.8), ["fire", "reload"]);
        assert!(clip.crossed_notifies(0.3, 0.3).is_empty());
        // Looping past the end wraps to the start.
        assert_eq!(clip.crossed_notifies(0.8, 0.3), ["fire"]);
        clip.framerate = 0.0;
        assert!(clip.crossed_notifies(0.1, 0.8).is_empty());
    }
}
