//! Reads Live Sets (`.als`) into the Loopwerk project model.
//!
//! The format is described in `docs/spec/als-format.md`, which was written from Live Sets we
//! authored ourselves. This reader covers what that spec covers: tempo, tracks, mixer, sends,
//! scenes and MIDI clips. Anything else (audio clips, devices, automation) is skipped and
//! reported in [`Import::warnings`], so a project still opens with everything else intact.

use std::fmt;
use std::io::Read;
use std::path::Path;

use loopwerk_model::{
    Arrangement, ArrangementLane, Channel, ChannelId, ChannelRole, Clip, ClipContent, ContentId, ContentKinds,
    ContentLane, Extensions, LaneData, Launcher, LauncherColumn, LoopRange, Note, Project, Scene, Send, Slot,
    TimeSignature, Track, TrackId, Transport, ValidationError,
};
use roxmltree::Node;

/// Upper bound for the decompressed XML, so a malicious file cannot exhaust memory.
const MAX_XML_BYTES: u64 = 512 * 1024 * 1024;
/// Live stores a send turned all the way down ("-inf" in the UI) as -70 dB.
const SEND_OFF: f64 = 0.000_316_3;

/// A project read from a Live Set, with notes about everything that was not imported.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub project: Project,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum AlsError {
    Io(std::io::Error),
    TooLarge,
    NotUtf8,
    Xml(roxmltree::Error),
    NotALiveSet,
    Invalid(Vec<ValidationError>),
}

impl fmt::Display for AlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "could not read the Live Set: {e}"),
            Self::TooLarge => write!(f, "the Live Set is larger than {} MiB uncompressed", MAX_XML_BYTES / 1024 / 1024),
            Self::NotUtf8 => write!(f, "the Live Set is not valid UTF-8"),
            Self::Xml(e) => write!(f, "the Live Set is not valid XML: {e}"),
            Self::NotALiveSet => write!(f, "the file is not a Live Set"),
            Self::Invalid(errors) => {
                write!(f, "the imported project is inconsistent:")?;
                for e in errors {
                    write!(f, " {e};")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for AlsError {}

impl From<std::io::Error> for AlsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Reads a `.als` file from disk.
pub fn read_file(path: &Path) -> Result<Import, AlsError> {
    let file = std::fs::File::open(path)?;
    read_gzip(file)
}

/// Reads a gzip-compressed Live Set.
pub fn read_gzip(reader: impl Read) -> Result<Import, AlsError> {
    let mut xml = Vec::new();
    flate2::read::GzDecoder::new(reader).take(MAX_XML_BYTES + 1).read_to_end(&mut xml)?;
    if xml.len() as u64 > MAX_XML_BYTES {
        return Err(AlsError::TooLarge);
    }
    let xml = String::from_utf8(xml).map_err(|_| AlsError::NotUtf8)?;
    read_xml(&xml)
}

/// Reads an already decompressed Live Set.
pub fn read_xml(xml: &str) -> Result<Import, AlsError> {
    let doc = roxmltree::Document::parse(xml).map_err(AlsError::Xml)?;
    let root = doc.root_element();
    if !root.has_tag_name("Ableton") {
        return Err(AlsError::NotALiveSet);
    }
    let live_set = child(root, "LiveSet").ok_or(AlsError::NotALiveSet)?;
    let mut reader = Reader::default();
    let project = reader.live_set(live_set);
    project.validate().map_err(AlsError::Invalid)?;
    Ok(Import { project, warnings: reader.warnings })
}

/// A track as listed in the Live Set, with the ids it gets in the model.
struct TrackEntry<'a, 'input> {
    node: Node<'a, 'input>,
    kind: TrackKind,
    track: TrackId,
    channel: ChannelId,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackKind {
    Midi,
    Audio,
    Return,
    Group,
}

#[derive(Default)]
struct Reader {
    warnings: Vec<String>,
    contents: Vec<ClipContent>,
}

impl Reader {
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    fn live_set(&mut self, live_set: Node) -> Project {
        let transport = self.transport(live_set);

        // The main output gets channel 1; tracks are numbered from 1 and their channels from 2.
        let main_channel = ChannelId(1);
        let mut entries = Vec::new();
        if let Some(tracks) = child(live_set, "Tracks") {
            for node in tracks.children().filter(Node::is_element) {
                let kind = match node.tag_name().name() {
                    "MidiTrack" => TrackKind::Midi,
                    "AudioTrack" => TrackKind::Audio,
                    "ReturnTrack" => TrackKind::Return,
                    "GroupTrack" => TrackKind::Group,
                    other => {
                        self.warn(format!("skipped unknown track type {other}"));
                        continue;
                    }
                };
                let n = u32::try_from(entries.len()).unwrap_or(u32::MAX).saturating_add(1);
                let name = value(node, &["Name", "EffectiveName"]).unwrap_or_default().to_string();
                entries.push(TrackEntry {
                    node,
                    kind,
                    track: TrackId(n),
                    channel: ChannelId(n.saturating_add(1)),
                    name,
                });
            }
        }
        let returns: Vec<ChannelId> =
            entries.iter().filter(|e| e.kind == TrackKind::Return).map(|e| e.channel).collect();

        let mut channels = vec![self.main_channel(live_set, main_channel)];
        let mut tracks = Vec::new();
        let mut lanes = Vec::new();
        let mut columns = Vec::new();
        let scenes = self.scenes(live_set);

        for entry in &entries {
            channels.push(self.channel(entry, &returns));
            tracks.push(Track {
                id: entry.track,
                name: entry.name.clone(),
                color: None,
                content: ContentKinds { notes: entry.kind == TrackKind::Midi, audio: entry.kind == TrackKind::Audio },
                channel: Some(entry.channel),
                parent: self.parent(entry, &entries),
                extensions: Extensions::default(),
            });
            self.devices(entry);
            if entry.kind != TrackKind::Return {
                lanes.push(ArrangementLane {
                    name: entry.name.clone(),
                    track: Some(entry.track),
                    clips: self.arrangement_clips(entry),
                });
            }
            if matches!(entry.kind, TrackKind::Midi | TrackKind::Audio) {
                columns.push(LauncherColumn { track: entry.track, slots: self.slots(entry, scenes.len()) });
            }
        }

        Project {
            transport,
            tracks,
            channels,
            contents: std::mem::take(&mut self.contents),
            arrangement: Arrangement { lanes },
            launcher: Launcher { scenes, columns },
            extensions: Extensions::default(),
        }
    }

    fn transport(&mut self, live_set: Node) -> Transport {
        let mut transport = Transport::default();
        let mixer = path(live_set, &["MainTrack", "DeviceChain", "Mixer"])
            .or_else(|| path(live_set, &["MasterTrack", "DeviceChain", "Mixer"]));
        let Some(mixer) = mixer else {
            self.warn("no main track found; using 120 bpm in 4/4");
            return transport;
        };
        match number(mixer, &["Tempo", "Manual"]) {
            Some(bpm) if (1.0..=999.0).contains(&bpm) => transport.tempo_bpm = bpm,
            _ => self.warn("tempo missing or out of range; using 120 bpm"),
        }
        match value(mixer, &["TimeSignature", "Manual"]) {
            Some("201") | None => {}
            Some(code) => self.warn(format!("time signature code {code} is not understood yet; using 4/4")),
        }
        transport.time_signature = TimeSignature { numerator: 4, denominator: 4 };
        transport
    }

    fn main_channel(&mut self, live_set: Node, id: ChannelId) -> Channel {
        let mixer = path(live_set, &["MainTrack", "DeviceChain", "Mixer"])
            .or_else(|| path(live_set, &["MasterTrack", "DeviceChain", "Mixer"]));
        let mut channel = empty_channel(id, "Main", ChannelRole::Master);
        if let Some(mixer) = mixer {
            self.mixer(mixer, &mut channel, "Main");
        }
        channel
    }

    fn channel(&mut self, entry: &TrackEntry, returns: &[ChannelId]) -> Channel {
        let role = match entry.kind {
            TrackKind::Return => ChannelRole::Effect,
            TrackKind::Group => ChannelRole::Submix,
            TrackKind::Midi | TrackKind::Audio => ChannelRole::Regular,
        };
        let mut channel = empty_channel(entry.channel, &entry.name, role);
        let Some(mixer) = path(entry.node, &["DeviceChain", "Mixer"]) else {
            self.warn(format!("track {}: no mixer found", entry.name));
            return channel;
        };
        self.mixer(mixer, &mut channel, &entry.name);

        if let Some(sends) = child(mixer, "Sends") {
            for (holder, target) in sends.children().filter(|n| n.has_tag_name("TrackSendHolder")).zip(returns) {
                let level = number(holder, &["Send", "Manual"]).unwrap_or(0.0);
                if level > SEND_OFF {
                    channel.sends.push(Send { target: *target, level, pre_fader: false });
                }
            }
        }

        match value(entry.node, &["DeviceChain", "AudioOutputRouting", "Target"]) {
            Some("AudioOut/Main") | None => {}
            Some(other) => {
                self.warn(format!("track {}: output {other} is not supported yet; using the main output", entry.name))
            }
        }
        channel
    }

    fn mixer(&mut self, mixer: Node, channel: &mut Channel, name: &str) {
        match number(mixer, &["Volume", "Manual"]) {
            Some(v) if v >= 0.0 => channel.volume = v,
            Some(_) => self.warn(format!("{name}: negative volume ignored")),
            None => {}
        }
        if let Some(pan) = number(mixer, &["Pan", "Manual"]) {
            channel.pan = pan.clamp(-1.0, 1.0);
        }
        if let Some(on) = boolean(mixer, &["Speaker", "Manual"]) {
            channel.mute = !on;
        }
    }

    fn parent(&mut self, entry: &TrackEntry, entries: &[TrackEntry]) -> Option<TrackId> {
        let group = value(entry.node, &["TrackGroupId"])?;
        if group == "-1" {
            return None;
        }
        let found = entries.iter().find(|e| e.kind == TrackKind::Group && e.node.attribute("Id") == Some(group));
        if found.is_none() {
            self.warn(format!("track {}: group {group} not found", entry.name));
        }
        found.map(|e| e.track)
    }

    fn devices(&mut self, entry: &TrackEntry) {
        let Some(devices) = path(entry.node, &["DeviceChain", "DeviceChain", "Devices"]) else { return };
        for device in devices.children().filter(Node::is_element) {
            self.warn(format!("track {}: device {} is not imported yet", entry.name, device.tag_name().name()));
        }
    }

    fn scenes(&mut self, live_set: Node) -> Vec<Scene> {
        let Some(scenes) = child(live_set, "Scenes") else { return Vec::new() };
        scenes
            .children()
            .filter(|n| n.has_tag_name("Scene"))
            .map(|scene| Scene {
                name: value(scene, &["Name"]).unwrap_or_default().to_string(),
                tempo_bpm: if boolean(scene, &["IsTempoEnabled"]) == Some(true) {
                    number(scene, &["Tempo"]).filter(|bpm| (1.0..=999.0).contains(bpm))
                } else {
                    None
                },
            })
            .collect()
    }

    fn arrangement_clips(&mut self, entry: &TrackEntry) -> Vec<Clip> {
        let Some(events) =
            path(entry.node, &["DeviceChain", "MainSequencer", "ClipTimeable", "ArrangerAutomation", "Events"])
        else {
            return Vec::new();
        };
        let mut clips = Vec::new();
        for node in events.children().filter(Node::is_element) {
            if let Some(clip) = self.clip(node, true, &entry.name) {
                clips.push(clip);
            }
        }
        clips
    }

    fn slots(&mut self, entry: &TrackEntry, scenes: usize) -> Vec<Slot> {
        let mut slots = Vec::new();
        if let Some(list) = path(entry.node, &["DeviceChain", "MainSequencer", "ClipSlotList"]) {
            for slot in list.children().filter(|n| n.has_tag_name("ClipSlot")) {
                let clip = path(slot, &["ClipSlot", "Value"])
                    .and_then(|v| v.children().find(Node::is_element))
                    .and_then(|node| self.clip(node, false, &entry.name));
                let has_stop = boolean(slot, &["HasStop"]).unwrap_or(true);
                slots.push(Slot { clip, has_stop });
            }
        }
        if slots.len() > scenes {
            self.warn(format!(
                "track {}: {} clip slots but {scenes} scenes; extra slots dropped",
                entry.name,
                slots.len()
            ));
            slots.truncate(scenes);
        }
        slots.resize_with(scenes, || Slot { clip: None, has_stop: true });
        slots
    }

    /// Reads one clip into the content pool and returns its placement.
    fn clip(&mut self, node: Node, arrangement: bool, track: &str) -> Option<Clip> {
        if !node.has_tag_name("MidiClip") {
            self.warn(format!("track {track}: {} is not imported yet", node.tag_name().name()));
            return None;
        }
        let name = value(node, &["Name"]).unwrap_or_default().to_string();
        let loop_on = boolean(node, &["Loop", "LoopOn"]).unwrap_or(false);
        let loop_start = number(node, &["Loop", "LoopStart"]).unwrap_or(0.0);
        let loop_end = number(node, &["Loop", "LoopEnd"]).unwrap_or(loop_start);
        let start_relative = number(node, &["Loop", "StartRelative"]).unwrap_or(0.0);
        let out_marker = number(node, &["Loop", "OutMarker"]).unwrap_or(loop_end);

        let notes = self.notes(node, &name, track);
        let id = ContentId(u32::try_from(self.contents.len()).unwrap_or(u32::MAX).saturating_add(1));
        self.contents.push(ClipContent {
            id,
            name: name.clone(),
            length: loop_end.max(out_marker).max(0.0),
            lanes: vec![ContentLane { target: None, data: LaneData::Notes(notes) }],
            extensions: Extensions::default(),
        });

        let (offset, loop_range) = if loop_on && loop_end > loop_start {
            (loop_start + start_relative, Some(LoopRange { start: loop_start, end: loop_end }))
        } else {
            (loop_start, None)
        };
        let (start, length) = if arrangement {
            let start = node.attribute("Time").and_then(parse_number).unwrap_or(0.0);
            let current_start = number(node, &["CurrentStart"]).unwrap_or(start);
            let current_end = number(node, &["CurrentEnd"]).unwrap_or(current_start);
            (start, current_end - current_start)
        } else if let Some(range) = loop_range {
            (0.0, range.end - range.start)
        } else {
            (0.0, out_marker - offset)
        };
        if !(length.is_finite() && length > 0.0) {
            self.warn(format!("track {track}: clip {name:?} has no length; skipped"));
            return None;
        }

        Some(Clip {
            content: id,
            start,
            length,
            offset,
            loop_range,
            enabled: boolean(node, &["Disabled"]) != Some(true),
            extensions: Extensions::default(),
        })
    }

    fn notes(&mut self, clip: Node, clip_name: &str, track: &str) -> Vec<Note> {
        let mut notes = Vec::new();
        let mut skipped = 0usize;
        let Some(key_tracks) = path(clip, &["Notes", "KeyTracks"]) else { return notes };
        for key_track in key_tracks.children().filter(|n| n.has_tag_name("KeyTrack")) {
            let Some(key) = value(key_track, &["MidiKey"]).and_then(|k| k.parse::<u8>().ok()).filter(|k| *k <= 127)
            else {
                skipped = skipped.saturating_add(1);
                continue;
            };
            let Some(events) = path(key_track, &["Notes"]) else { continue };
            for event in events.children().filter(|n| n.has_tag_name("MidiNoteEvent")) {
                if event.attribute("IsEnabled") == Some("false") {
                    continue;
                }
                let time = event.attribute("Time").and_then(parse_number);
                let duration = event.attribute("Duration").and_then(parse_number);
                let (Some(time), Some(duration)) = (time, duration) else {
                    skipped = skipped.saturating_add(1);
                    continue;
                };
                let velocity = event.attribute("Velocity").and_then(parse_number).unwrap_or(100.0);
                let release = event.attribute("OffVelocity").and_then(parse_number).unwrap_or(64.0);
                notes.push(Note {
                    time,
                    duration: duration.max(0.0),
                    key,
                    velocity: (velocity / 127.0).clamp(0.0, 1.0) as f32,
                    release_velocity: (release / 127.0).clamp(0.0, 1.0) as f32,
                    channel: 0,
                });
            }
        }
        if skipped > 0 {
            self.warn(format!("track {track}: clip {clip_name:?}: {skipped} malformed notes skipped"));
        }
        notes.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.key.cmp(&b.key)));
        notes
    }
}

fn empty_channel(id: ChannelId, name: &str, role: ChannelRole) -> Channel {
    Channel {
        id,
        name: name.to_string(),
        role,
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        output: None,
        sends: Vec::new(),
        devices: Vec::new(),
        extensions: Extensions::default(),
    }
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|n| n.has_tag_name(name))
}

fn path<'a, 'input>(node: Node<'a, 'input>, names: &[&str]) -> Option<Node<'a, 'input>> {
    names.iter().try_fold(node, |n, name| child(n, name))
}

/// The `Value` attribute of the element at `names` below `node`.
fn value<'a>(node: Node<'a, '_>, names: &[&str]) -> Option<&'a str> {
    path(node, names)?.attribute("Value")
}

fn number(node: Node, names: &[&str]) -> Option<f64> {
    value(node, names).and_then(parse_number)
}

fn boolean(node: Node, names: &[&str]) -> Option<bool> {
    match value(node, names)? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}
