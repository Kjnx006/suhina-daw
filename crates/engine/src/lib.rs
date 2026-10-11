//! Loopwerk audio engine. Offline rendering and real-time playback share this code path:
//! [`Renderer::new`] does all allocation up front, and [`Renderer::process`] never allocates,
//! so the same `process` call can later run on the audio thread.

use std::fmt;

pub use loopwerk_model as model;
use loopwerk_model::{ChannelRole, Clip, ContentLane, LaneData, Project, TrackId};

/// Voices that can sound at once. A note that arrives when all are busy steals the oldest.
const MAX_VOICES: usize = 64;
/// Release time after note-off, in seconds.
const RELEASE_SECONDS: f64 = 0.05;
/// Attack time, in seconds; long enough to avoid a click at note-on.
const ATTACK_SECONDS: f64 = 0.002;
/// Upper bound for an offline render, so a malformed project cannot exhaust memory.
const MAX_RENDER_SECONDS: f64 = 60.0 * 60.0;

#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    InvalidProject(Vec<loopwerk_model::ValidationError>),
    InvalidSampleRate(u32),
    InvalidTempo(f64),
    TooLong { seconds: f64 },
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProject(errors) => {
                write!(f, "invalid project:")?;
                for e in errors {
                    write!(f, " {e};")?;
                }
                Ok(())
            }
            Self::InvalidSampleRate(sr) => write!(f, "sample rate {sr} is not supported"),
            Self::InvalidTempo(bpm) => write!(f, "tempo {bpm} bpm is not supported"),
            Self::TooLong { seconds } => {
                write!(f, "render would be {seconds:.0} s long; the limit is {MAX_RENDER_SECONDS:.0} s")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// A note with its position resolved to sample frames and its mix resolved from the channel.
#[derive(Debug, Clone, Copy, PartialEq)]
struct NoteEvent {
    start: u64,
    end: u64,
    frequency: f64,
    gain_left: f32,
    gain_right: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Voice {
    active: bool,
    started_at: u64,
    end: u64,
    phase: f64,
    phase_step: f64,
    gain_left: f32,
    gain_right: f32,
}

/// Plays a project's arrangement. Build it once, then call [`Renderer::process`] block by block.
pub struct Renderer {
    sample_rate: u32,
    events: Vec<NoteEvent>,
    next_event: usize,
    voices: [Voice; MAX_VOICES],
    position: u64,
    length: u64,
}

impl Renderer {
    pub fn new(project: &Project, sample_rate: u32) -> Result<Self, RenderError> {
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(RenderError::InvalidSampleRate(sample_rate));
        }
        let bpm = project.transport.tempo_bpm;
        if !bpm.is_finite() || !(1.0..=999.0).contains(&bpm) {
            return Err(RenderError::InvalidTempo(bpm));
        }
        project.validate().map_err(RenderError::InvalidProject)?;

        let frames_per_beat = 60.0 / bpm * f64::from(sample_rate);
        let mut events = Vec::new();
        for lane in &project.arrangement.lanes {
            for clip in lane.clips.iter().filter(|c| c.enabled) {
                let Some(content) = project.content(clip.content) else { continue };
                for content_lane in &content.lanes {
                    let Some(track) = content_lane.target.or(lane.track) else { continue };
                    let Some((gain_left, gain_right)) = track_gains(project, track) else { continue };
                    collect_notes(clip, content_lane, frames_per_beat, gain_left, gain_right, &mut events);
                }
            }
        }
        events.sort_by_key(|e| e.start);

        let release = (RELEASE_SECONDS * f64::from(sample_rate)) as u64;
        let length = events.iter().map(|e| e.end.saturating_add(release)).max().unwrap_or(0);
        let seconds = length as f64 / f64::from(sample_rate);
        if seconds > MAX_RENDER_SECONDS {
            return Err(RenderError::TooLong { seconds });
        }

        Ok(Self { sample_rate, events, next_event: 0, voices: [Voice::default(); MAX_VOICES], position: 0, length })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Length of the arrangement including release tails, in frames.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Renders the next `out.len() / 2` frames of interleaved stereo into `out`, overwriting it.
    /// Never allocates.
    pub fn process(&mut self, out: &mut [f32]) {
        let attack = (ATTACK_SECONDS * f64::from(self.sample_rate)).max(1.0);
        let release = (RELEASE_SECONDS * f64::from(self.sample_rate)).max(1.0);
        for [out_left, out_right] in out.as_chunks_mut::<2>().0 {
            while let Some(event) = self.events.get(self.next_event) {
                if event.start > self.position {
                    break;
                }
                let event = *event;
                self.next_event += 1;
                self.start_voice(&event);
            }

            let mut left = 0.0f32;
            let mut right = 0.0f32;
            for voice in self.voices.iter_mut().filter(|v| v.active) {
                let age = (self.position - voice.started_at) as f64;
                let envelope = if self.position < voice.end {
                    (age / attack).min(1.0)
                } else {
                    let released = (self.position - voice.end) as f64;
                    if released >= release {
                        voice.active = false;
                        continue;
                    }
                    ((voice.end - voice.started_at) as f64 / attack).min(1.0) * (1.0 - released / release)
                };
                let sample = (voice.phase * std::f64::consts::TAU).sin() * envelope;
                voice.phase = (voice.phase + voice.phase_step).fract();
                left += sample as f32 * voice.gain_left;
                right += sample as f32 * voice.gain_right;
            }
            *out_left = left;
            *out_right = right;
            self.position += 1;
        }
    }

    fn start_voice(&mut self, event: &NoteEvent) {
        let slot = match self.voices.iter().position(|v| !v.active) {
            Some(free) => free,
            None => self.voices.iter().enumerate().min_by_key(|(_, v)| v.started_at).map_or(0, |(i, _)| i),
        };
        if let Some(voice) = self.voices.get_mut(slot) {
            *voice = Voice {
                active: true,
                started_at: self.position,
                end: event.end.max(self.position + 1),
                phase: 0.0,
                phase_step: event.frequency / f64::from(self.sample_rate),
                gain_left: event.gain_left,
                gain_right: event.gain_right,
            };
        }
    }
}

/// Renders the whole arrangement offline into interleaved stereo, `block_frames` at a time.
pub fn render_offline(project: &Project, sample_rate: u32, block_frames: usize) -> Result<Vec<f32>, RenderError> {
    let mut renderer = Renderer::new(project, sample_rate)?;
    let total = usize::try_from(renderer.length())
        .ok()
        .and_then(|frames| frames.checked_mul(2))
        .ok_or(RenderError::TooLong { seconds: renderer.length() as f64 / f64::from(sample_rate) })?;
    let mut out = vec![0.0f32; total];
    for block in out.chunks_mut(block_frames.max(1).saturating_mul(2)) {
        renderer.process(block);
    }
    Ok(out)
}

/// Equal-power pan and channel volume for a track; `None` when the track is silent (muted, or
/// it has no channel). The volume and mute of every channel the track's output passes through
/// (groups, then the master) apply too; their pan does not yet.
fn track_gains(project: &Project, track: TrackId) -> Option<(f32, f32)> {
    let channel_id = project.tracks.iter().find(|t| t.id == track)?.channel?;
    let channel = project.channels.iter().find(|c| c.id == channel_id)?;
    let pan = if channel.pan.is_finite() { channel.pan.clamp(-1.0, 1.0) } else { 0.0 };
    let gain = |c: &loopwerk_model::Channel| if c.volume.is_finite() { c.volume.max(0.0) } else { 0.0 };

    let mut volume = 1.0;
    let mut visited = Vec::new();
    let mut next = Some(channel);
    // Bounded, so a routing cycle in a malformed project cannot loop forever.
    while let Some(c) = next.take() {
        if c.mute || visited.len() > 64 || visited.contains(&c.id) {
            return None;
        }
        visited.push(c.id);
        volume *= gain(c);
        next = c.output.and_then(|id| project.channels.iter().find(|o| o.id == id));
    }
    if let Some(master) = project.channels.iter().find(|c| c.role == ChannelRole::Master && !visited.contains(&c.id)) {
        if master.mute {
            return None;
        }
        volume *= gain(master);
    }
    let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
    // Scaled so a centred note has unity gain on each side, and quieter overall to leave headroom.
    let scale = volume * std::f64::consts::SQRT_2 * 0.25;
    Some(((angle.cos() * scale) as f32, (angle.sin() * scale) as f32))
}

/// Adds the notes of one content lane, as placed by `clip`, to `events`. Notes outside the clip's
/// window are skipped; with a loop range the loop repeats until the clip ends.
fn collect_notes(
    clip: &Clip,
    lane: &ContentLane,
    frames_per_beat: f64,
    gain_left: f32,
    gain_right: f32,
    events: &mut Vec<NoteEvent>,
) {
    let LaneData::Notes(notes) = &lane.data else { return };
    if !(clip.length.is_finite() && clip.length > 0.0 && clip.start.is_finite() && clip.offset.is_finite()) {
        return;
    }
    let to_frame = |beats: f64| (beats.max(0.0) * frames_per_beat).round() as u64;

    // Each pass maps the content range [from, to) to the clip position `at`.
    let mut push_range = |from: f64, to: f64, at: f64| {
        for note in notes {
            if !(note.time.is_finite() && note.duration.is_finite()) || note.time < from || note.time >= to {
                continue;
            }
            let start = clip.start + at + (note.time - from);
            let clip_end = clip.start + clip.length;
            if start >= clip_end {
                continue;
            }
            let end = (start + note.duration.max(0.0)).min(clip_end);
            events.push(NoteEvent {
                start: to_frame(start),
                end: to_frame(end),
                frequency: 440.0 * 2f64.powf((f64::from(note.key) - 69.0) / 12.0),
                gain_left: gain_left * note.velocity.clamp(0.0, 1.0),
                gain_right: gain_right * note.velocity.clamp(0.0, 1.0),
            });
        }
    };

    match clip.loop_range {
        Some(range) if range.end.is_finite() && range.start.is_finite() && range.end > range.start => {
            let loop_len = range.end - range.start;
            // First pass from the offset to the loop end, then whole loops until the clip ends.
            let first_from = clip.offset.clamp(range.start, range.end);
            push_range(first_from, range.end, 0.0);
            let mut at = range.end - first_from;
            let max_passes = 100_000;
            let mut passes = 0;
            while at < clip.length && passes < max_passes {
                push_range(range.start, range.end, at);
                at += loop_len;
                passes += 1;
            }
        }
        _ => push_range(clip.offset, f64::INFINITY, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loopwerk_model::*;

    const SR: u32 = 48_000;

    fn note(time: f64, duration: f64, key: u8) -> Note {
        Note { time, duration, key, velocity: 1.0, release_velocity: 0.5, channel: 0 }
    }

    fn project_with(lanes: Vec<ContentLane>, clips: Vec<Clip>, tracks: u32) -> Project {
        Project {
            tracks: (1..=tracks)
                .map(|i| Track {
                    id: TrackId(i),
                    name: format!("T{i}"),
                    color: None,
                    content: ContentKinds { notes: true, audio: false },
                    channel: Some(ChannelId(i)),
                    parent: None,
                    extensions: Extensions::default(),
                })
                .collect(),
            channels: (1..=tracks)
                .map(|i| Channel {
                    id: ChannelId(i),
                    name: format!("C{i}"),
                    role: ChannelRole::Regular,
                    volume: 1.0,
                    pan: 0.0,
                    mute: false,
                    solo: false,
                    output: None,
                    sends: Vec::new(),
                    devices: Vec::new(),
                    extensions: Extensions::default(),
                })
                .collect(),
            contents: vec![ClipContent {
                id: ContentId(1),
                name: "c".into(),
                length: 4.0,
                lanes,
                extensions: Extensions::default(),
            }],
            arrangement: Arrangement {
                lanes: vec![ArrangementLane { name: "l".into(), track: Some(TrackId(1)), clips }],
            },
            ..Project::default()
        }
    }

    fn clip(start: f64, length: f64) -> Clip {
        Clip {
            content: ContentId(1),
            start,
            length,
            offset: 0.0,
            loop_range: None,
            enabled: true,
            extensions: Extensions::default(),
        }
    }

    fn untargeted(notes: Vec<Note>) -> Vec<ContentLane> {
        vec![ContentLane { target: None, data: LaneData::Notes(notes) }]
    }

    /// Frames where sound starts after more than 100 silent frames.
    fn onsets(out: &[f32]) -> Vec<usize> {
        let mut result = Vec::new();
        let mut silent_run = usize::MAX;
        for (i, frame) in out.as_chunks::<2>().0.iter().enumerate() {
            let loud = frame.iter().any(|s| s.abs() > 1e-6);
            if loud && silent_run > 100 {
                result.push(i);
            }
            silent_run = if loud { 0 } else { silent_run.saturating_add(1) };
        }
        result
    }

    /// A sine starts at phase zero, so the first audible frame lands a frame or two after note-on.
    fn assert_onsets(out: &[f32], expected: &[usize]) {
        let actual = onsets(out);
        assert_eq!(actual.len(), expected.len(), "onsets {actual:?}, expected {expected:?}");
        for (a, e) in actual.iter().zip(expected) {
            assert!(a.abs_diff(*e) <= 2, "onsets {actual:?}, expected {expected:?}");
        }
    }

    #[test]
    fn empty_project_renders_nothing() {
        assert_eq!(render_offline(&Project::default(), SR, 256), Ok(Vec::new()));
    }

    #[test]
    fn note_on_beat_two_starts_half_a_second_in_at_120_bpm() {
        let project = project_with(untargeted(vec![note(1.0, 1.0, 69)]), vec![clip(0.0, 4.0)], 1);
        let out = render_offline(&project, SR, 256).unwrap();
        assert_onsets(&out, &[SR as usize / 2]);
    }

    #[test]
    fn a4_plays_at_440_hz() {
        let project = project_with(untargeted(vec![note(0.0, 2.0, 69)]), vec![clip(0.0, 4.0)], 1);
        let out = render_offline(&project, SR, 256).unwrap();
        // Count upward zero crossings over one second of the left channel.
        let left: Vec<f32> = out.as_chunks::<2>().0.iter().map(|f| f[0]).take(SR as usize).collect();
        let crossings = left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((439..=441).contains(&crossings), "{crossings} crossings");
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let project = project_with(
            untargeted(vec![note(0.0, 0.5, 60), note(0.25, 1.0, 64), note(1.5, 0.3, 67)]),
            vec![clip(0.0, 4.0)],
            1,
        );
        let reference = render_offline(&project, SR, 1).unwrap();
        for block in [7, 64, 256, 1024, 100_000] {
            assert_eq!(render_offline(&project, SR, block).unwrap(), reference, "block {block}");
        }
    }

    #[test]
    fn muted_channel_is_silent() {
        let mut project = project_with(untargeted(vec![note(0.0, 1.0, 60)]), vec![clip(0.0, 4.0)], 1);
        project.channels[0].mute = true;
        assert_eq!(render_offline(&project, SR, 256), Ok(Vec::new()));
    }

    #[test]
    fn hard_left_pan_leaves_right_silent() {
        let mut project = project_with(untargeted(vec![note(0.0, 1.0, 60)]), vec![clip(0.0, 4.0)], 1);
        project.channels[0].pan = -1.0;
        let out = render_offline(&project, SR, 256).unwrap();
        assert!(out.as_chunks::<2>().0.iter().all(|f| f[1].abs() < 1e-6));
        assert!(out.as_chunks::<2>().0.iter().any(|f| f[0].abs() > 0.1));
    }

    #[test]
    fn clip_cuts_notes_at_its_end_and_skips_notes_after_it() {
        let project = project_with(untargeted(vec![note(0.0, 4.0, 60), note(2.0, 1.0, 64)]), vec![clip(0.0, 1.0)], 1);
        let out = render_offline(&project, SR, 256).unwrap();
        // One beat at 120 bpm plus the release tail.
        let expected = SR as usize / 2 + (RELEASE_SECONDS * f64::from(SR)) as usize;
        assert_eq!(out.len() / 2, expected);
        assert_eq!(onsets(&out).len(), 1);
    }

    #[test]
    fn loop_range_repeats_until_the_clip_ends() {
        let mut looped = clip(0.0, 4.0);
        looped.loop_range = Some(LoopRange { start: 0.0, end: 1.0 });
        let project = project_with(untargeted(vec![note(0.0, 0.25, 60)]), vec![looped], 1);
        let out = render_offline(&project, SR, 256).unwrap();
        let beat = SR as usize / 2;
        assert_onsets(&out, &[0, beat, 2 * beat, 3 * beat]);
    }

    /// A shared pattern drives two tracks and is placed twice on a free lane.
    #[test]
    fn shared_pattern_plays_every_placement_on_its_target_tracks() {
        let lanes = vec![
            ContentLane { target: Some(TrackId(1)), data: LaneData::Notes(vec![note(0.0, 0.25, 36)]) },
            ContentLane { target: Some(TrackId(2)), data: LaneData::Notes(vec![note(0.5, 0.25, 38)]) },
        ];
        let mut project = project_with(lanes, vec![clip(0.0, 1.0), clip(2.0, 1.0)], 2);
        project.arrangement.lanes[0].track = None;
        let out = render_offline(&project, SR, 256).unwrap();
        let beat = SR as usize / 2;
        assert_onsets(&out, &[0, beat / 2, 2 * beat, 2 * beat + beat / 2]);

        // Muting track 2 removes only its notes.
        project.channels[1].mute = true;
        let out = render_offline(&project, SR, 256).unwrap();
        assert_onsets(&out, &[0, 2 * beat]);
    }

    #[test]
    fn group_and_master_volume_and_mute_apply_to_the_tracks_routed_through_them() {
        let mut project = project_with(untargeted(vec![note(0.0, 1.0, 69)]), vec![clip(0.0, 4.0)], 2);
        let peak = |p: &Project| render_offline(p, SR, 256).unwrap().iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let alone = peak(&project);

        // Track 1 plays into channel 2 (a group) at half volume.
        project.channels[0].output = Some(ChannelId(2));
        project.channels[1].volume = 0.5;
        assert!((peak(&project) - alone * 0.5).abs() < 1e-3);

        project.channels[1].mute = true;
        assert_eq!(render_offline(&project, SR, 256), Ok(Vec::new()));
        project.channels[1].mute = false;

        // A master channel scales everything that does not route to it explicitly.
        project.channels.push(Channel {
            id: ChannelId(9),
            role: ChannelRole::Master,
            volume: 0.5,
            ..project.channels[1].clone()
        });
        assert!((peak(&project) - alone * 0.25).abs() < 1e-3);

        // A routing cycle is silence, not a hang.
        project.channels[1].output = Some(ChannelId(1));
        assert_eq!(render_offline(&project, SR, 256), Ok(Vec::new()));
    }

    #[test]
    fn hostile_input_returns_errors_or_silence_instead_of_panicking() {
        assert_eq!(render_offline(&Project::default(), 0, 256).err(), Some(RenderError::InvalidSampleRate(0)));
        let mut project = Project::default();
        project.transport.tempo_bpm = f64::NAN;
        assert!(matches!(render_offline(&project, SR, 256), Err(RenderError::InvalidTempo(_))));

        let weird = vec![note(f64::NAN, 1.0, 60), note(0.0, f64::INFINITY, 127), note(-5.0, 1.0, 0)];
        let mut project = project_with(untargeted(weird), vec![clip(0.0, 1.0)], 1);
        project.channels[0].volume = f64::NAN;
        assert!(render_offline(&project, SR, 256).is_ok());

        let project = project_with(untargeted(vec![note(0.0, 1.0, 60)]), vec![clip(0.0, 1e12)], 1);
        let mut looped = clip(0.0, 1e12);
        looped.loop_range = Some(LoopRange { start: 0.0, end: 1e-9 });
        let looping = project_with(untargeted(vec![note(0.0, 1.0, 60)]), vec![looped], 1);
        assert!(render_offline(&project, SR, 256).is_ok());
        assert!(render_offline(&looping, SR, 256).is_ok());
    }
}
