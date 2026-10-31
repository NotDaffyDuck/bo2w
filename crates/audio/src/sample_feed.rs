//! Decoded sound data and the source Bevy pulls samples from.

#[path = "adpcm_t5.rs"]
pub(crate) mod adpcm_t5;

use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use bevy::audio::{ChannelCount, Decodable, PlaybackSettings, SampleRate, Source, Volume};
use bevy::prelude::*;

/// Output rate reported when a clip somehow carries a rate of zero.
const FALLBACK_RATE: u32 = 22_050;

/// Sources the mixer holds right now (opened, not yet dropped).
pub(crate) static LIVE_SOURCES: AtomicI64 = AtomicI64::new(0);
/// Samples all sources have handed the mixer so far.
pub(crate) static SAMPLES_MIXED: AtomicU64 = AtomicU64::new(0);
/// Of those sources, the ones ringing a room echo.
pub(crate) static LIVE_ECHOES: AtomicI64 = AtomicI64::new(0);
/// Most voices that ring a room echo at once; later ones play dry. Every
/// echo runs on the one sound thread: measured 10-29, 180 echoing voices took
/// 816 ms per second of sound (32: 68 ms), so a 12-bot firefight starved the
/// device: static, and the sound sped up and slowed down.
const ECHO_CAP: i64 = 24;

/// Left and right gains that can be changed while a sound is playing; the
/// audio thread reads them for every stereo pair it produces.
#[derive(Clone, Debug)]
pub struct PanHandle {
    /// Left then right, as float bits.
    gains: Arc<[AtomicU32; 2]>,
}

impl PanHandle {
    pub fn with_gains(left: f32, pan_r: f32) -> Self {
        Self {
            gains: Arc::new([left, pan_r].map(|gain| AtomicU32::new(gain.to_bits()))),
        }
    }

    pub fn centred() -> Self {
        Self::with_gains(1.0, 1.0)
    }

    pub fn store(&self, left: f32, r_gain: f32) {
        for (cell, gain) in self.gains.iter().zip([left, r_gain]) {
            cell.store(gain.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn load(&self) -> (f32, f32) {
        let [left, right] = self
            .gains
            .each_ref()
            .map(|cell| f32::from_bits(cell.load(Ordering::Relaxed)));
        (left, right)
    }
}

/// A fully decoded clip: interleaved samples at one rate.
#[derive(Asset, TypePath, Clone)]
pub struct ClipPcm {
    pan: Option<PanHandle>,
    hz: u32,
    lane_n: u16,
    data: Arc<[f32]>,
    /// bo2zm: the room echo this play sends into, and how much.
    echo: Option<(Arc<crate::room_echo::EchoParams>, f32)>,
}

/// A clip that starts over when it reaches its end.
#[derive(Asset, TypePath, Clone)]
pub struct LoopedPcm(ClipPcm);

#[derive(Bundle)]
pub struct LoopedPcmPlayer {
    mix: PlaybackSettings,
    voice: AudioPlayer<LoopedPcm>,
}

impl LoopedPcmPlayer {
    pub fn new(handle: Handle<LoopedPcm>, loudness: Volume) -> Self {
        let mix = PlaybackSettings::ONCE.with_volume(loudness);
        Self {
            mix,
            voice: AudioPlayer(handle),
        }
    }
}

impl ClipPcm {
    pub(crate) fn from_parts(data: Arc<[f32]>, lanes: u16, hz: u32) -> Option<Self> {
        let usable = !data.is_empty() && lanes != 0 && hz != 0;
        usable.then_some(Self {
            pan: None,
            hz,
            lane_n: lanes,
            data,
            echo: None,
        })
    }

    pub(crate) fn data(&self) -> &Arc<[f32]> {
        &self.data
    }

    pub(crate) fn lane_count(&self) -> u16 {
        self.lane_n
    }

    pub(crate) fn hz(&self) -> u32 {
        self.hz
    }

    /// The same samples with a pan that can be adjusted while playing.
    pub fn with_pan_handle(&self) -> Self {
        let mut clip = self.clone();
        clip.pan = Some(PanHandle::centred());
        clip.echo = None;
        clip
    }

    /// bo2zm: this play also sends `send` (0..1) into the room's echo (it plays as stereo,
    /// then rings on for the echo's tail).
    pub fn with_echo(mut self, params: Arc<crate::room_echo::EchoParams>, send: f32) -> Self {
        if self.pan.is_none() {
            self.pan = Some(PanHandle::centred());
        }
        self.echo = Some((params, send));
        self
    }

    pub fn pan_handle(&self) -> Option<&PanHandle> {
        self.pan.as_ref()
    }

    pub fn looped(self) -> LoopedPcm {
        LoopedPcm(self)
    }

    fn open(&self, repeat: bool) -> ClipDecoder {
        LIVE_SOURCES.fetch_add(1, Ordering::Relaxed);
        // A live pan always produces stereo.
        let lanes_out = if self.pan.is_some() { 2 } else { self.lane_n };
        ClipDecoder {
            eq: crate::master::VoiceEq::new(self.hz, lanes_out),
            data: Arc::clone(&self.data),
            lanes_in: self.lane_n,
            hz: self.hz,
            pan: self.pan.clone(),
            repeat,
            read_at: 0,
            held_right: None,
            echo: None,
            tail_left: 0,
        }
        .with_echo(self.echo.as_ref().filter(|_| !repeat))
    }
}

impl LoopedPcm {
    pub fn pan_handle(&self) -> Option<&PanHandle> {
        self.0.pan_handle()
    }
}

/// Walks a clip sample by sample, applying the live pan if there is one,
/// then the master EQ.
pub struct ClipDecoder {
    data: Arc<[f32]>,
    lanes_in: u16,
    hz: u32,
    pan: Option<PanHandle>,
    repeat: bool,
    read_at: usize,
    /// The right half of a pan pair whose left half was just handed out.
    held_right: Option<f32>,
    /// Black Ops II's master EQ (the audio preset), one filter per output channel.
    eq: crate::master::VoiceEq,
    /// bo2zm: the room echo this play rings into.
    echo: Option<crate::room_echo::Echo>,
    /// Echo frames still to ring after the clip.
    tail_left: usize,
}

impl ClipDecoder {
    fn with_echo(mut self, echo: Option<&(Arc<crate::room_echo::EchoParams>, f32)>) -> Self {
        if let Some((params, send)) = echo
            && *send > 0.0
            && self.pan.is_some()
            && LIVE_ECHOES
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < ECHO_CAP).then_some(n + 1))
                .is_ok()
        {
            let ring = crate::room_echo::Echo::new(params, self.hz, *send);
            self.tail_left = ring.tail_frames();
            self.echo = Some(ring);
        }
        self
    }

    fn source_stride(&self) -> usize {
        usize::from(self.lanes_in.max(1))
    }

    /// Samples still to come out, counting the held-back right half.
    fn samples_left(&self) -> usize {
        let unread = self.data.len().saturating_sub(self.read_at);
        if self.pan.is_none() {
            return unread;
        }
        // Panned output is always two lanes, plus the right half held back.
        (unread / self.source_stride() + self.tail_left) * 2 + usize::from(self.held_right.is_some())
    }

    /// Where the next sample is read from, sending a loop back to its start at the end.
    fn cursor(&mut self) -> Option<usize> {
        if self.read_at < self.data.len() {
            return Some(self.read_at);
        }
        if !self.repeat || self.data.is_empty() {
            return None;
        }
        self.read_at = 0;
        Some(0)
    }

    fn next_raw(&mut self) -> Option<f32> {
        let pending = self.held_right.take();
        if pending.is_some() {
            return pending;
        }
        let Some(at) = self.cursor() else {
            return self.ring_tail();
        };
        let left = self.data[at];
        match self.pan.as_ref().map(PanHandle::load) {
            None => {
                self.read_at = at + 1;
                Some(left)
            }
            Some((to_left, to_right)) => {
                let right = self.data.get(at + 1).copied().unwrap_or(left);
                self.read_at = at + self.source_stride();
                if let Some(echo) = &mut self.echo {
                    let g = (to_left + to_right) * 0.5;
                    let (wet_left, wet_right) = echo.echo_frame((left + right) * 0.5);
                    self.held_right = Some(right * to_right + wet_right * g);
                    return Some(left * to_left + wet_left * g);
                }
                self.held_right = Some(right * to_right);
                Some(left * to_left)
            }
        }
    }

    /// bo2zm: the clip is over; its echo rings on.
    fn ring_tail(&mut self) -> Option<f32> {
        let (Some(echo), Some(pan)) = (&mut self.echo, &self.pan) else {
            return None;
        };
        if self.tail_left == 0 {
            return None;
        }
        self.tail_left -= 1;
        let (to_left, to_right) = pan.load();
        let g = (to_left + to_right) * 0.5;
        let (wet_left, wet_right) = echo.echo_frame(0.0);
        self.held_right = Some(wet_right * g);
        Some(wet_left * g)
    }
}

impl Iterator for ClipDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let raw = self.next_raw()?;
        SAMPLES_MIXED.fetch_add(1, Ordering::Relaxed);
        let shaped = self.eq.process(raw);
        // With BO2MP_SOUND=off every voice still runs its full course (sinks,
        // timing, loops and stops behave as usual, so logs can prove them),
        // but only silence reaches the device: the master bus mutes it after
        // its meter, or, should that not be set, the voice itself.
        Some(if crate::AudioSilent::active() && !bevy::audio::master_bus::muted() {
            0.0
        } else {
            shaped
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.repeat {
            return (0, None);
        }
        let left = self.samples_left();
        (left, Some(left))
    }
}

impl Source for ClipDecoder {
    fn current_span_len(&self) -> Option<usize> {
        (!self.repeat).then(|| self.samples_left())
    }

    fn channels(&self) -> ChannelCount {
        let count = if self.pan.is_some() { 2 } else { self.lanes_in };
        NonZero::new(count).unwrap_or(NonZero::<u16>::MIN)
    }

    fn sample_rate(&self) -> SampleRate {
        let rate = if self.hz == 0 { FALLBACK_RATE } else { self.hz };
        NonZero::new(rate).unwrap_or(NonZero::<u32>::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.repeat {
            return None;
        }
        let tail = self.echo.as_ref().map_or(0, |echo| echo.tail_frames());
        let frames = (self.data.len() / self.source_stride() + tail) as f64;
        Some(Duration::from_secs_f64(frames / f64::from(self.hz.max(1))))
    }
}

impl Drop for ClipDecoder {
    fn drop(&mut self) {
        LIVE_SOURCES.fetch_sub(1, Ordering::Relaxed);
        if self.echo.is_some() {
            LIVE_ECHOES.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Decodable for ClipPcm {
    type Decoder = ClipDecoder;

    fn decoder(&self) -> Self::Decoder {
        self.open(false)
    }
}

impl Decodable for LoopedPcm {
    type Decoder = ClipDecoder;

    fn decoder(&self) -> Self::Decoder {
        self.0.open(true)
    }
}

/// A clip that plays through a Black Ops II pan's fixed stereo gains; the
/// clip itself when there is no pan or the pan leaves both sides whole.
pub(crate) fn with_stereo_pan(pcm: ClipPcm, pan: Option<asset_audio::StereoPan>) -> ClipPcm {
    match pan.map(|pan| pan.gains(pcm.lane_count())) {
        Some((left, right)) if (left, right) != (1.0, 1.0) => {
            let panned = pcm.with_pan_handle();
            panned.pan_handle().inspect(|live| live.store(left, right));
            panned
        }
        _ => pcm,
    }
}

/// A probed container with the decoder for its first real audio track.
struct OpenedStream {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    track_hz: u32,
}

impl OpenedStream {
    fn probe(bytes: &[u8]) -> Option<Self> {
        use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let source = MediaSourceStream::new(
            Box::new(std::io::Cursor::new(bytes.to_vec())),
            Default::default(),
        );
        let format = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                source,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .ok()?
            .format;
        let (track_id, params) = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .map(|track| (track.id, track.codec_params.clone()))?;
        Some(Self {
            track_hz: params.sample_rate?,
            decoder: symphonia::default::get_codecs()
                .make(&params, &DecoderOptions::default())
                .ok()?,
            format,
            track_id,
        })
    }

    /// The next packet of the chosen track; `None` once the container runs dry.
    fn next_track_packet(&mut self) -> Option<symphonia::core::formats::Packet> {
        use symphonia::core::errors::Error;
        loop {
            match self.format.next_packet() {
                Ok(packet) if packet.track_id() == self.track_id => return Some(packet),
                Ok(_) | Err(Error::ResetRequired) => {}
                Err(_) => return None,
            }
        }
    }

    /// Every sample of the track, interleaved, and its channel count. Packets that fail
    /// to decode are skipped.
    fn drain(mut self) -> (Vec<f32>, u16) {
        use symphonia::core::audio::SampleBuffer;

        let mut samples = Vec::new();
        let mut channels = 0_u16;
        while let Some(packet) = self.next_track_packet() {
            let Ok(decoded) = self.decoder.decode(&packet) else {
                continue;
            };
            let spec = *decoded.spec();
            channels = channels.max(spec.channels.count() as u16);
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            buffer.copy_interleaved_ref(decoded);
            samples.extend_from_slice(buffer.samples());
        }
        (samples, channels)
    }
}

/// Decodes any container Symphonia can probe into interleaved f32 samples.
/// Packets that fail to decode are skipped; nothing decodable gives `None`.
pub fn decode_clip_bytes(bytes: &[u8]) -> Option<ClipPcm> {
    let stream = OpenedStream::probe(bytes)?;
    let sample_rate = stream.track_hz;
    let (samples, channels) = stream.drain();
    ClipPcm::from_parts(samples.into(), channels, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reel(samples: &[f32], chan_count: u16, rate: u32) -> ClipPcm {
        ClipPcm::from_parts(samples.into(), chan_count, rate).unwrap()
    }

    fn raw(mut decoder: ClipDecoder, upto: usize) -> Vec<f32> {
        std::iter::from_fn(|| decoder.next_raw())
            .take(upto)
            .collect()
    }

    /// Past the cap, new voices play dry; once voices end, echo comes back.
    #[test]
    fn room_echo_is_capped() {
        let room = Arc::new(crate::room_echo::EchoParams {
            early_ms: 30.0,
            early_gain: 0.5,
            decay_s: 0.8,
            late_gain: 1.0,
            wet: 1.0,
            input_lpf: 8000.0,
            damp_lpf: 5000.0,
            size: 1.0,
            diffusion: 0.5,
        });
        let clip = reel(&[0.1; 64], 1, 48_000).with_pan_handle().with_echo(room, 0.5);
        let held: Vec<ClipDecoder> = (0..ECHO_CAP + 6).map(|_| clip.decoder()).collect();
        let echoing = held.iter().filter(|d| d.echo.is_some()).count();
        assert!(echoing as i64 <= ECHO_CAP, "{echoing} echoing voices");
        assert!(held.iter().any(|d| d.echo.is_none()));
        drop(held);
        assert!(clip.decoder().echo.is_some());
    }

    /// What one second of output costs for many panned voices, with and
    /// without the room echo (run with `--ignored --nocapture`, optimised).
    #[test]
    #[ignore]
    fn voice_cost_per_second() {
        let room = Arc::new(crate::room_echo::EchoParams {
            early_ms: 30.0,
            early_gain: 0.5,
            decay_s: 0.8,
            late_gain: 1.0,
            wet: 1.0,
            input_lpf: 8000.0,
            damp_lpf: 5000.0,
            size: 1.0,
            diffusion: 0.5,
        });
        let noise: Vec<f32> = (0..48_000).map(|i| ((i * 7919 % 1000) as f32 / 1000.0) - 0.5).collect();
        for echo in [false, true] {
            for voices in [32usize, 64, 128, 180] {
                let mut decoders: Vec<ClipDecoder> = (0..voices)
                    .map(|_| {
                        let clip = reel(&noise, 1, 48_000).with_pan_handle();
                        let clip = if echo { clip.with_echo(Arc::clone(&room), 0.5) } else { clip };
                        clip.decoder()
                    })
                    .collect();
                let began = std::time::Instant::now();
                let mut sink = 0.0f32;
                for _ in 0..96_000 {
                    for d in &mut decoders {
                        sink += d.next().unwrap_or(0.0);
                    }
                }
                println!("voice cost: echo={echo} {voices} voices, 1 s took {:?} ({sink})", began.elapsed());
            }
        }
    }

    /// A 16-bit PCM WAV holding `samples` for `channels` interleaved channels.
    fn wav(samples: &[i16], n_chan: u16, rate: u32) -> Vec<u8> {
        let data = samples.len() as u32 * 2;
        let mut out = b"RIFF".to_vec();
        out.extend((36 + data).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16_u32.to_le_bytes());
        out.extend(1_u16.to_le_bytes());
        out.extend(n_chan.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(n_chan) * 2).to_le_bytes());
        out.extend((n_chan * 2).to_le_bytes());
        out.extend(16_u16.to_le_bytes());
        out.extend(b"data");
        out.extend(data.to_le_bytes());
        out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        out
    }

    #[test]
    fn unusable_clips_are_refused() {
        let some: Arc<[f32]> = vec![0.5].into();
        assert!(ClipPcm::from_parts(Arc::from(Vec::new()), 1, 8000).is_none());
        assert!(ClipPcm::from_parts(some.clone(), 0, 8000).is_none());
        assert!(ClipPcm::from_parts(some, 1, 0).is_none());
    }

    #[test]
    fn plain_clip_plays_once_in_order() {
        let pcm = reel(&[0.1, 0.2, 0.3], 1, 8000);
        assert_eq!(raw(pcm.open(false), 10), vec![0.1, 0.2, 0.3]);
        let decoder = pcm.open(false);
        assert_eq!(decoder.size_hint(), (3, Some(3)));
        assert_eq!(decoder.current_span_len(), Some(3));
    }

    #[test]
    fn looping_clip_wraps_and_has_no_length() {
        let pcm = reel(&[1.0, 2.0], 1, 8000);
        let decoder = pcm.open(true);
        assert_eq!(decoder.size_hint(), (0, None));
        assert_eq!(decoder.current_span_len(), None);
        assert_eq!(decoder.total_duration(), None);
        assert_eq!(raw(pcm.open(true), 5), vec![1.0, 2.0, 1.0, 2.0, 1.0]);
    }

    #[test]
    fn live_pan_splits_each_frame_into_a_pair() {
        let pcm = reel(&[1.0, 3.0, 5.0], 1, 8000).with_pan_handle();
        pcm.pan_handle().unwrap().store(0.5, 0.25);
        let decoder = pcm.open(false);
        assert_eq!(decoder.size_hint(), (6, Some(6)));
        // On one channel the "right" half is the next sample along.
        assert_eq!(raw(decoder, 10), vec![0.5, 0.75, 1.5, 1.25, 2.5, 1.25]);
    }

    #[test]
    fn live_pan_on_stereo_uses_each_side() {
        let pcm = reel(&[1.0, 2.0, 3.0, 4.0], 2, 8000).with_pan_handle();
        pcm.pan_handle().unwrap().store(2.0, 0.5);
        let mut decoder = pcm.open(false);
        assert_eq!(decoder.next_raw(), Some(2.0));
        assert_eq!(decoder.samples_left(), 3);
        assert_eq!(decoder.next_raw(), Some(1.0));
        assert_eq!(decoder.samples_left(), 2);
        assert_eq!(raw(decoder, 10), vec![6.0, 2.0]);
    }

    #[test]
    fn source_facts() {
        let mono = reel(&[0.0; 16], 1, 8000).open(false);
        assert_eq!(mono.channels().get(), 1);
        assert_eq!(mono.sample_rate().get(), 8000);
        assert_eq!(
            mono.total_duration(),
            Some(Duration::from_secs_f64(16.0 / 8000.0))
        );
        let panned = reel(&[0.0; 16], 1, 8000).with_pan_handle().open(false);
        assert_eq!(panned.channels().get(), 2);
        let stereo = reel(&[0.0; 16], 2, 4000).open(false);
        assert_eq!(
            stereo.total_duration(),
            Some(Duration::from_secs_f64(8.0 / 4000.0))
        );
    }

    #[test]
    fn stereo_pan_is_skipped_when_both_sides_whole() {
        let pcm = reel(&[1.0, 1.0], 2, 8000);
        assert!(with_stereo_pan(pcm.clone(), None).pan_handle().is_none());
    }

    #[test]
    fn wav_bytes_decode_to_interleaved_samples() {
        let bytes = wav(&[0, 16384, -16384, 32767], 2, 22050);
        let pcm = decode_clip_bytes(&bytes).unwrap();
        assert_eq!((pcm.lane_count(), pcm.hz()), (2, 22050));
        let got = pcm.data();
        assert_eq!(got.len(), 4);
        assert!((got[1] - 0.5).abs() < 1e-3 && (got[2] + 0.5).abs() < 1e-3);
        assert!(decode_clip_bytes(b"not audio at all").is_none());
    }
}
