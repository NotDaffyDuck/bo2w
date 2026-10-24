//! Decoded sound data and the source Bevy pulls samples from.

#[path = "t5_stream.rs"]
pub(crate) mod t5_stream;

use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bevy::{
    audio::{ChannelCount, Decodable, PlaybackSettings, SampleRate, Source, Volume},
    prelude::*,
};

/// Output rate reported when a clip somehow carries a rate of zero.
const FALLBACK_RATE: u32 = 22_050;

/// Left and right gains that can be changed while a sound is playing; the
/// audio thread reads them for every stereo pair it produces.
#[derive(Clone, Debug)]
pub struct LivePan {
    left: Arc<AtomicU32>,
    right: Arc<AtomicU32>,
}

impl LivePan {
    pub fn new(left: f32, right: f32) -> Self {
        let cell = |gain: f32| Arc::new(AtomicU32::new(gain.to_bits()));
        Self {
            left: cell(left),
            right: cell(right),
        }
    }

    pub fn unity() -> Self {
        Self::new(1.0, 1.0)
    }

    pub fn set(&self, left: f32, right: f32) {
        self.left.store(left.to_bits(), Ordering::Relaxed);
        self.right.store(right.to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> (f32, f32) {
        let read = |cell: &AtomicU32| f32::from_bits(cell.load(Ordering::Relaxed));
        (read(&self.left), read(&self.right))
    }
}

/// A fully decoded clip: interleaved samples at one rate.
#[derive(Asset, TypePath, Clone)]
pub struct PcmAudio {
    samples: Arc<[f32]>,
    channels: u16,
    sample_rate: u32,
    live_pan: Option<LivePan>,
}

/// A clip that starts over when it reaches its end.
#[derive(Asset, TypePath, Clone)]
pub struct LoopingPcmAudio(PcmAudio);

#[derive(Bundle)]
pub struct LoopingPcmPlayback {
    player: AudioPlayer<LoopingPcmAudio>,
    settings: PlaybackSettings,
}

impl LoopingPcmPlayback {
    pub fn new(handle: Handle<LoopingPcmAudio>, volume: Volume) -> Self {
        Self {
            player: AudioPlayer(handle),
            settings: PlaybackSettings::ONCE.with_volume(volume),
        }
    }
}

impl PcmAudio {
    pub(crate) fn from_prepared(
        samples: Arc<[f32]>,
        channels: u16,
        sample_rate: u32,
    ) -> Option<Self> {
        let usable = !samples.is_empty() && channels != 0 && sample_rate != 0;
        usable.then(|| Self {
            samples,
            channels,
            sample_rate,
            live_pan: None,
        })
    }

    pub(crate) fn samples(&self) -> &Arc<[f32]> {
        &self.samples
    }

    pub(crate) fn channel_count(&self) -> u16 {
        self.channels
    }

    pub(crate) fn rate(&self) -> u32 {
        self.sample_rate
    }

    /// The same samples with a pan that can be adjusted while playing.
    pub fn with_live_pan(&self) -> Self {
        Self {
            live_pan: Some(LivePan::unity()),
            ..self.clone()
        }
    }

    pub fn live_pan(&self) -> Option<&LivePan> {
        self.live_pan.as_ref()
    }

    pub fn into_looping(self) -> LoopingPcmAudio {
        LoopingPcmAudio(self)
    }

    fn open(&self, looping: bool) -> PcmDecoder {
        // A live pan always produces stereo.
        let out_channels = if self.live_pan.is_some() { 2 } else { self.channels };
        PcmDecoder {
            samples: Arc::clone(&self.samples),
            pos: 0,
            src_channels: self.channels,
            sample_rate: self.sample_rate,
            live: self.live_pan.clone(),
            pending_right: None,
            looping,
            eq: crate::master::VoiceEq::new(self.sample_rate, out_channels),
        }
    }
}

impl LoopingPcmAudio {
    pub fn live_pan(&self) -> Option<&LivePan> {
        self.0.live_pan()
    }
}

/// Walks a clip sample by sample, applying the live pan if there is one,
/// then the master EQ.
pub struct PcmDecoder {
    samples: Arc<[f32]>,
    pos: usize,
    src_channels: u16,
    sample_rate: u32,
    live: Option<LivePan>,
    /// The right half of a pan pair whose left half was just handed out.
    pending_right: Option<f32>,
    looping: bool,
    /// Black Ops II's master EQ (the audio preset), one filter per output channel.
    eq: crate::master::VoiceEq,
}

impl PcmDecoder {
    fn source_stride(&self) -> usize {
        self.src_channels.max(1) as usize
    }

    /// Samples still to come out, counting the held-back right half.
    fn remaining_out(&self) -> usize {
        let left_in_source = self.samples.len().saturating_sub(self.pos);
        if self.live.is_none() {
            return left_in_source;
        }
        let held = usize::from(self.pending_right.is_some());
        held + (left_in_source / self.source_stride()) * 2
    }

    /// Makes sure `pos` points at a sample, wrapping if this is a loop.
    fn ready(&mut self) -> bool {
        if self.pos < self.samples.len() {
            return true;
        }
        if self.looping && !self.samples.is_empty() {
            self.pos = 0;
            return true;
        }
        false
    }

    fn next_raw(&mut self) -> Option<f32> {
        if let Some(right) = self.pending_right.take() {
            return Some(right);
        }
        if !self.ready() {
            return None;
        }

        let Some(pan) = &self.live else {
            let sample = self.samples[self.pos];
            self.pos += 1;
            return Some(sample);
        };

        let (gain_left, gain_right) = pan.get();
        let left = *self.samples.get(self.pos)?;
        let right = self.samples.get(self.pos + 1).copied().unwrap_or(left);
        self.pos += self.source_stride();
        self.pending_right = Some(right * gain_right);
        Some(left * gain_left)
    }
}

impl Iterator for PcmDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let raw = self.next_raw()?;
        let shaped = self.eq.process(raw);
        // With BO2MP_SOUND=off every voice still runs its full course (sinks,
        // timing, loops and stops behave as usual, so logs can prove them),
        // but only silence reaches the device.
        Some(if crate::AudioSilent::active() { 0.0 } else { shaped })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.looping {
            (0, None)
        } else {
            let left = self.remaining_out();
            (left, Some(left))
        }
    }
}

impl Source for PcmDecoder {
    fn current_span_len(&self) -> Option<usize> {
        (!self.looping).then(|| self.remaining_out())
    }

    fn channels(&self) -> ChannelCount {
        let count = if self.live.is_some() { 2 } else { self.src_channels.max(1) };
        NonZero::new(count).unwrap_or(NonZero::<u16>::MIN)
    }

    fn sample_rate(&self) -> SampleRate {
        NonZero::new(self.sample_rate).unwrap_or(NonZero::new(FALLBACK_RATE).unwrap())
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.looping {
            return None;
        }
        let frames = self.samples.len() / self.source_stride();
        let seconds = frames as f64 / f64::from(self.sample_rate.max(1));
        Some(Duration::from_secs_f64(seconds))
    }
}

impl Decodable for PcmAudio {
    type Decoder = PcmDecoder;

    fn decoder(&self) -> Self::Decoder {
        self.open(false)
    }
}

impl Decodable for LoopingPcmAudio {
    type Decoder = PcmDecoder;

    fn decoder(&self) -> Self::Decoder {
        self.0.open(true)
    }
}

/// A clip that plays through a Black Ops II pan's fixed stereo gains; the
/// clip itself when there is no pan or the pan leaves both sides whole.
pub(crate) fn with_stereo_pan(pcm: PcmAudio, pan: Option<asset_audio::StereoPan>) -> PcmAudio {
    let gains = pan
        .map(|pan| pan.gains(pcm.channel_count()))
        .filter(|&gains| gains != (1.0, 1.0));
    let Some((left, right)) = gains else {
        return pcm;
    };
    let panned = pcm.with_live_pan();
    if let Some(live) = panned.live_pan() {
        live.set(left, right);
    }
    panned
}

/// Decodes any container Symphonia can probe into interleaved f32 samples.
/// Packets that fail to decode are skipped; nothing decodable gives `None`.
pub fn decode_audio_bytes(bytes: &[u8]) -> Option<PcmAudio> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let source = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    let mut format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?
        .format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)?;
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;

    let mut samples: Vec<f32> = Vec::new();
    let mut channels = 0_u16;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::ResetRequired) => continue,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        let Ok(decoded) = decoder.decode(&packet) else {
            continue;
        };
        let spec = *decoded.spec();
        channels = channels.max(spec.channels.count() as u16);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }

    if samples.is_empty() || channels == 0 {
        return None;
    }
    Some(PcmAudio {
        samples: samples.into(),
        channels,
        sample_rate,
        live_pan: None,
    })
}
