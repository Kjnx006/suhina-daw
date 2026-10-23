// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `marker.*` commands, and what runs them here.

use super::*;

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "marker.add",
        label: "Add a named marker on the arrangement's timeline",
        params: &[
            param("beat", ParamType::Number, "where, in beats from the beginning, 0 … 120000"),
            optional("name", ParamType::Text, "the marker's name (default \"Marker n\")"),
        ],
        kind: CommandKind::Edit,
        run: add_marker,
    },
    CommandSpec {
        id: "marker.move",
        label: "Move a marker to another beat; markers stay in beat order, so its index can change",
        params: &[MARKER, param("beat", ParamType::Number, "the new beat, 0 … 120000")],
        kind: CommandKind::Edit,
        run: move_marker,
    },
    CommandSpec {
        id: "marker.rename",
        label: "Rename a marker",
        params: &[MARKER, param("name", ParamType::Text, "the new name, not empty")],
        kind: CommandKind::Edit,
        run: rename_marker,
    },
    CommandSpec {
        id: "marker.delete",
        label: "Delete a marker",
        params: &[MARKER],
        kind: CommandKind::Edit,
        run: delete_marker,
    },
    CommandSpec {
        id: "marker.jump",
        label: "Move the play start marker to a marker; while playing, playback jumps there at once",
        params: &[MARKER],
        kind: CommandKind::Transport,
        run: jump_to_marker,
    },
];

/// `params.beat`, checked to be a beat a marker can be at.
fn marker_beat(params: &Value) -> Result<f64, CommandError> {
    let beat = number(params, "beat")?;
    if !(0.0..=MAX_START_BEAT).contains(&beat) {
        return Err(format!("beat must be between 0 and {MAX_START_BEAT}, got {beat}").into());
    }
    Ok(beat)
}

/// `params.index`, checked to name a marker.
fn marker_index(p: &Project, params: &Value) -> Result<usize, CommandError> {
    let index =
        params.get("index").and_then(Value::as_u64).ok_or("index must be a whole number from project.markers")?;
    let count = p.arrangement.markers.len();
    usize::try_from(index)
        .ok()
        .filter(|i| *i < count)
        .ok_or_else(|| format!("marker {index} does not exist; the arrangement has {count} markers").into())
}

fn add_marker(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let beat = marker_beat(params)?;
    let markers = &mut session.project.arrangement;
    let name = match params.get("name") {
        None | Some(Value::Null) => {
            // The first "Marker n" not in use, counting from how many there are.
            let mut n = markers.markers.len() + 1;
            while markers.markers.iter().any(|m| m.name == format!("Marker {n}")) {
                n += 1;
            }
            format!("Marker {n}")
        }
        Some(v) => {
            v.as_str().map(str::trim).filter(|n| !n.is_empty()).ok_or("name must be a text, not empty")?.to_string()
        }
    };
    let index = markers.insert_marker(Marker::new(beat, name.clone()));
    Ok(json!({ "index": index, "beat": beat, "name": name }))
}

fn move_marker(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let index = marker_index(&session.project, params)?;
    let beat = marker_beat(params)?;
    let arrangement = &mut session.project.arrangement;
    if index >= arrangement.markers.len() {
        return Err("the marker does not exist".into());
    }
    let mut marker = arrangement.markers.remove(index);
    marker.beat = beat;
    let name = marker.name.clone();
    let index = arrangement.insert_marker(marker);
    Ok(json!({ "index": index, "beat": beat, "name": name }))
}

fn rename_marker(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let name = params.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
    let name = name.ok_or("name must be a text, not empty")?.to_string();
    let index = marker_index(&session.project, params)?;
    let marker = session.project.arrangement.markers.get_mut(index).ok_or("the marker does not exist")?;
    marker.name.clone_from(&name);
    Ok(json!({ "index": index, "name": name }))
}

fn delete_marker(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let index = marker_index(&session.project, params)?;
    let markers = &mut session.project.arrangement.markers;
    if index >= markers.len() {
        return Err("the marker does not exist".into());
    }
    let marker = markers.remove(index);
    Ok(json!({ "deleted": index, "name": marker.name }))
}

fn jump_to_marker(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    // Without an engine there is nothing to jump; say so before looking at the marker.
    session.player_mut()?;
    let index = marker_index(session.project(), params)?;
    let beat = session.project().arrangement.markers.get(index).map_or(0.0, |m| m.beat).clamp(0.0, MAX_START_BEAT);
    send(session, LaunchCommand::SetPosition { beat })?;
    Ok(json!({ "index": index, "start_beat": beat }))
}
