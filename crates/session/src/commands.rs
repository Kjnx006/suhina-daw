//! The command registry. Each command is a [`CommandSpec`]; new commands are added to
//! [`COMMANDS`] with tests below.

use loopwerk_model::{Channel, Clip, LaneData, Project, Track, TrackId};
use serde_json::{Value, json};

use crate::{CommandError, Session};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// Reads the project; never changes it.
    Read,
    /// Changes the project and records one undo step.
    Edit,
    /// Moves through the undo history.
    History,
}

type Run = fn(&mut Session, &Value) -> Result<Value, CommandError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Number,
    Integer,
    Bool,
    /// A track id (integer) or a track name (string).
    Track,
    /// A number, or the string "-inf".
    Decibels,
}

#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub ty: ParamType,
    pub required: bool,
    pub doc: &'static str,
}

const fn param(name: &'static str, ty: ParamType, doc: &'static str) -> Param {
    Param { name, ty, required: true, doc }
}

const fn optional(name: &'static str, ty: ParamType, doc: &'static str) -> Param {
    Param { name, ty, required: false, doc }
}

const TRACK: Param = param("track", ParamType::Track, "track id or name, from project.tracks");

pub struct CommandSpec {
    /// Stable id in `area.verb` form; part of the public interface for agents.
    pub id: &'static str,
    /// One line saying what the command does.
    pub label: &'static str,
    pub params: &'static [Param],
    pub kind: CommandKind,
    pub run: Run,
}

impl CommandSpec {
    /// The parameters as a JSON Schema object, for MCP tool listings.
    pub fn input_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        for p in self.params {
            let ty = match p.ty {
                ParamType::Number => json!("number"),
                ParamType::Integer => json!("integer"),
                ParamType::Bool => json!("boolean"),
                ParamType::Track => json!(["integer", "string"]),
                ParamType::Decibels => json!(["number", "string"]),
            };
            properties.insert(p.name.to_string(), json!({ "type": ty, "description": p.doc }));
        }
        let required: Vec<&str> = self.params.iter().filter(|p| p.required).map(|p| p.name).collect();
        json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
    }
}

/// Every command, in the order `commands.list` shows them.
pub fn commands() -> &'static [CommandSpec] {
    COMMANDS
}

static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "commands.list",
        label: "List every command with its parameters",
        params: &[],
        kind: CommandKind::Read,
        run: list,
    },
    CommandSpec {
        id: "project.info",
        label: "Summarise the open project: tempo, time signature, counts of tracks, scenes and clips, undo state",
        params: &[],
        kind: CommandKind::Read,
        run: info,
    },
    CommandSpec {
        id: "project.tracks",
        label: "List tracks with id, name, volume (dB), pan, mute and devices",
        params: &[],
        kind: CommandKind::Read,
        run: tracks,
    },
    CommandSpec {
        id: "project.clips",
        label: "List arrangement clips with track, index, start and length in beats, and note count",
        params: &[optional("track", ParamType::Track, "only clips on this track (id or name)")],
        kind: CommandKind::Read,
        run: clips,
    },
    CommandSpec {
        id: "transport.setTempo",
        label: "Set the project tempo",
        params: &[param("bpm", ParamType::Number, "beats per minute, 20 to 999")],
        kind: CommandKind::Edit,
        run: set_tempo,
    },
    CommandSpec {
        id: "track.setVolume",
        label: "Set a track's volume",
        params: &[
            TRACK,
            param("db", ParamType::Decibels, "level in dB, at most +6; \"-inf\" or below -150 is silence"),
        ],
        kind: CommandKind::Edit,
        run: set_volume,
    },
    CommandSpec {
        id: "track.setPan",
        label: "Set a track's pan",
        params: &[TRACK, param("pan", ParamType::Number, "-1 is full left, 0 centre, 1 full right")],
        kind: CommandKind::Edit,
        run: set_pan,
    },
    CommandSpec {
        id: "track.setMute",
        label: "Mute or unmute a track",
        params: &[TRACK, param("mute", ParamType::Bool, "true mutes the track")],
        kind: CommandKind::Edit,
        run: set_mute,
    },
    CommandSpec {
        id: "clip.move",
        label: "Move an arrangement clip to a new start position",
        params: &[
            TRACK,
            param("index", ParamType::Integer, "clip index on the track, from project.clips"),
            param("start", ParamType::Number, "new start in beats from the beginning, at least 0"),
        ],
        kind: CommandKind::Edit,
        run: move_clip,
    },
    CommandSpec { id: "edit.undo", label: "Undo the last edit", params: &[], kind: CommandKind::History, run: undo },
    CommandSpec {
        id: "edit.redo",
        label: "Redo the last undone edit",
        params: &[],
        kind: CommandKind::History,
        run: redo,
    },
];

fn list(_: &mut Session, _: &Value) -> Result<Value, CommandError> {
    Ok(Value::Array(
        COMMANDS
            .iter()
            .map(|c| {
                let kind = match c.kind {
                    CommandKind::Read => "read",
                    CommandKind::Edit => "edit",
                    CommandKind::History => "history",
                };
                json!({ "id": c.id, "label": c.label, "params": c.input_schema()["properties"], "kind": kind })
            })
            .collect(),
    ))
}

fn info(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    let p = session.project();
    let arrangement_clips: usize = p.arrangement.lanes.iter().map(|l| l.clips.len()).sum();
    let launcher_clips = p.launcher.columns.iter().flat_map(|c| &c.slots).filter(|s| s.clip.is_some()).count();
    Ok(json!({
        "source": session.source().map(|s| s.display().to_string()),
        "tempo_bpm": p.transport.tempo_bpm,
        "time_signature": format!("{}/{}", p.transport.time_signature.numerator, p.transport.time_signature.denominator),
        "tracks": p.tracks.len(),
        "scenes": p.launcher.scenes.len(),
        "arrangement_clips": arrangement_clips,
        "launcher_clips": launcher_clips,
        "can_undo": session.can_undo(),
        "can_redo": session.can_redo(),
    }))
}

fn tracks(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    let p = session.project();
    Ok(Value::Array(
        p.tracks
            .iter()
            .map(|t| {
                let channel = channel_of(p, t);
                let clips =
                    p.arrangement.lanes.iter().filter(|l| l.track == Some(t.id)).map(|l| l.clips.len()).sum::<usize>();
                json!({
                    "id": t.id.0,
                    "name": t.name,
                    "plays": if t.content.notes { "notes" } else if t.content.audio { "audio" } else { "none" },
                    "volume_db": channel.map(|c| to_db(c.volume)),
                    "pan": channel.map(|c| c.pan),
                    "mute": channel.map(|c| c.mute),
                    "devices": channel.map(|c| c.devices.iter().map(|d| d.name.clone()).collect::<Vec<_>>()),
                    "arrangement_clips": clips,
                })
            })
            .collect(),
    ))
}

fn clips(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p = session.project();
    let only = match params.get("track") {
        Some(_) => Some(track_id(p, params)?),
        None => None,
    };
    let mut out = Vec::new();
    for lane in &p.arrangement.lanes {
        let Some(track) = lane.track else { continue };
        if only.is_some_and(|t| t != track) {
            continue;
        }
        let name = p.tracks.iter().find(|t| t.id == track).map(|t| t.name.as_str()).unwrap_or_default();
        for (index, clip) in lane.clips.iter().enumerate() {
            out.push(clip_json(p, name, track, index, clip));
        }
    }
    Ok(Value::Array(out))
}

fn clip_json(p: &Project, track_name: &str, track: TrackId, index: usize, clip: &Clip) -> Value {
    let content = p.content(clip.content);
    let (notes, audio) = content.map_or((0, None), |c| {
        c.lanes.iter().fold((0, None), |(n, a), lane| match &lane.data {
            LaneData::Notes(notes) => (n + notes.len(), a),
            LaneData::Audio(region) => (n, Some(region.file.clone())),
        })
    });
    json!({
        "track": track.0,
        "track_name": track_name,
        "index": index,
        "name": content.map(|c| c.name.as_str()),
        "start": clip.start,
        "length": clip.length,
        "enabled": clip.enabled,
        "notes": notes,
        "audio_file": audio,
    })
}

fn set_tempo(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let bpm = number(params, "bpm")?;
    if !(20.0..=999.0).contains(&bpm) {
        return Err(format!("bpm must be between 20 and 999, got {bpm}").into());
    }
    session.project.transport.tempo_bpm = bpm;
    Ok(json!({ "tempo_bpm": bpm }))
}

fn set_volume(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let db = number_or_neg_infinity(params, "db")?;
    if db > 6.0 {
        return Err(format!("db must be at most +6, got {db}").into());
    }
    let gain = if db < -150.0 { 0.0 } else { 10f64.powf(db / 20.0) };
    let channel = channel_param(session, params)?;
    channel.volume = gain;
    Ok(json!({ "volume_db": to_db(gain) }))
}

fn set_pan(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let pan = number(params, "pan")?;
    if !(-1.0..=1.0).contains(&pan) {
        return Err(format!("pan must be between -1 and 1, got {pan}").into());
    }
    channel_param(session, params)?.pan = pan;
    Ok(json!({ "pan": pan }))
}

fn set_mute(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let mute = params.get("mute").and_then(Value::as_bool).ok_or("mute must be true or false")?;
    channel_param(session, params)?.mute = mute;
    Ok(json!({ "mute": mute }))
}

fn move_clip(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let track = track_id(&session.project, params)?;
    let index = params.get("index").and_then(Value::as_u64).ok_or("index must be a whole number from project.clips")?;
    let start = number(params, "start")?;
    if start < 0.0 {
        return Err(format!("start must be at least 0, got {start}").into());
    }
    let lane = session
        .project
        .arrangement
        .lanes
        .iter_mut()
        .find(|l| l.track == Some(track))
        .ok_or("the track has no arrangement lane")?;
    let count = lane.clips.len();
    let clip = usize::try_from(index)
        .ok()
        .and_then(|i| lane.clips.get_mut(i))
        .ok_or_else(|| format!("clip index {index} does not exist; the track has {count} arrangement clips"))?;
    clip.start = start;
    lane.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(json!({ "start": start }))
}

fn undo(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    if session.undo() { Ok(json!({ "undone": true })) } else { Err("nothing to undo".into()) }
}

fn redo(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    if session.redo() { Ok(json!({ "redone": true })) } else { Err("nothing to redo".into()) }
}

/// Resolves `params.track`: a track id (number) or a name. Names match exactly first, then
/// ignoring case, and must be unambiguous.
fn track_id(p: &Project, params: &Value) -> Result<TrackId, CommandError> {
    match params.get("track") {
        Some(Value::Number(n)) => {
            let id = n.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("track id must be a whole number")?;
            p.tracks
                .iter()
                .find(|t| t.id.0 == id)
                .map(|t| t.id)
                .ok_or_else(|| format!("track {id} does not exist").into())
        }
        Some(Value::String(name)) => {
            if let Some(t) = p.tracks.iter().find(|t| t.name == *name) {
                return Ok(t.id);
            }
            let matches: Vec<&Track> = p.tracks.iter().filter(|t| t.name.eq_ignore_ascii_case(name)).collect();
            match matches.as_slice() {
                [t] => Ok(t.id),
                [] => Err(format!("no track named {name:?}; project.tracks lists them").into()),
                _ => Err(format!("several tracks are named {name:?}; use the track id").into()),
            }
        }
        _ => Err("track must be a track id or name".into()),
    }
}

fn channel_param<'a>(session: &'a mut Session, params: &Value) -> Result<&'a mut Channel, CommandError> {
    let track = track_id(&session.project, params)?;
    let channel = session
        .project
        .tracks
        .iter()
        .find(|t| t.id == track)
        .and_then(|t| t.channel)
        .ok_or("the track has no mixer channel")?;
    session.project.channels.iter_mut().find(|c| c.id == channel).ok_or_else(|| "the track's channel is missing".into())
}

fn channel_of<'a>(p: &'a Project, track: &Track) -> Option<&'a Channel> {
    let id = track.channel?;
    p.channels.iter().find(|c| c.id == id)
}

fn number(params: &Value, name: &str) -> Result<f64, CommandError> {
    params
        .get(name)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("{name} must be a number").into())
}

/// Like [`number`], but also accepts the string "-inf".
fn number_or_neg_infinity(params: &Value, name: &str) -> Result<f64, CommandError> {
    match params.get(name) {
        Some(Value::String(s)) if s == "-inf" => Ok(f64::NEG_INFINITY),
        _ => number(params, name),
    }
}

/// Linear gain to decibels, rounded to 0.01 dB; `None` (JSON null) for silence.
fn to_db(gain: f64) -> Option<f64> {
    (gain > 0.0).then(|| (20.0 * gain.log10() * 100.0).round() / 100.0)
}
