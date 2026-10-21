// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Launched stretched clips: a golden render of launches, a legato take-over and a relaunch, and
//! the voices' preparation off the audio thread (ADR 0003 › Launched clips).

use std::sync::Arc;

use suhina_model::*;

use super::super::tests::{clip, project, with};
use crate::fade_tests::{assert_golden, left};
use crate::{AudioData, AudioPool, LaunchCommand, render_launches};

/// A low rate keeps the golden file small.
const SR: u32 = 8_000;

/// A sine of `hz` lasting two seconds, under `file`.
fn sine(pool: &mut AudioPool, file: &str, hz: f64) {
    let samples = (0..2 * SR as usize).map(|i| (std::f64::consts::TAU * hz * i as f64 / f64::from(SR)).sin() as f32 * 0.5);
    pool.files.insert(file.into(), Arc::new(AudioData { sample_rate: SR, channels: 1, samples: samples.collect() }));
}

/// Content `id`: `file` warped so that two beats play `seconds` of it.
fn warped(id: u32, file: &str, seconds: f64) -> ClipContent {
    let region = AudioRegion {
        sample_rate: Some(SR),
        channels: Some(1),
        duration_seconds: Some(2.0),
        warped: true,
        stretch: StretchMode::Smooth,
        warp_markers: vec![WarpMarker { time: 0.0, audio_time: 0.0 }, WarpMarker { time: 2.0, audio_time: seconds }],
        ..AudioRegion::new(file)
    };
    ClipContent {
        id: ContentId(id),
        name: file.into(),
        length: 2.0,
        lanes: vec![ContentLane { target: None, data: LaneData::Audio(region) }],
        extensions: Extensions::default(),
        scale: None,
    }
}

/// One column at 240 bpm (a bar is a second): slot 1 loops a 440 Hz sine stretched to 0.9 times
/// its speed, slot 2 a legato 330 Hz sine at 1.25 times.
pub(super) fn two_stretched() -> (Project, AudioPool) {
    let mut pool = AudioPool::new();
    sine(&mut pool, "a.wav", 440.0);
    sine(&mut pool, "b.wav", 330.0);
    let legato = Clip { launch: LaunchSettings { legato: true, ..LaunchSettings::default() }, ..clip(2, 2.0, true) };
    let mut p = project(vec![warped(1, "a.wav", 0.45), warped(2, "b.wav", 0.625)], vec![vec![
        with(clip(1, 2.0, true)),
        with(legato),
    ]]);
    p.transport.tempo_bpm = 240.0;
    (p, pool)
}

/// Slot 1 at once, slot 2 at the next bar (legato, so from where slot 1 was), slot 1 again
/// from its start at the bar after.
pub(super) const LAUNCHES: [(f64, LaunchCommand); 3] = [
    (0.0, LaunchCommand::LaunchClip { column: 0, slot: 0 }),
    (0.3, LaunchCommand::LaunchClip { column: 0, slot: 1 }),
    (1.3, LaunchCommand::LaunchClip { column: 0, slot: 0 }),
];

/// Seconds rendered.
pub(super) const SECONDS: f64 = 2.75;

/// Upward zero crossings per second in `left[from..to]` (seconds).
fn hz(left: &[f32], from: f64, to: f64) -> f64 {
    let (a, b) = ((from * f64::from(SR)) as usize, (to * f64::from(SR)) as usize);
    let crossings = left[a..b].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    crossings as f64 / (to - from)
}

#[test]
fn launched_stretched_clips_render_like_their_golden_file() {
    let (p, pool) = two_stretched();
    let out = left(&render_launches(&p, &pool, SR, &LAUNCHES, SECONDS).unwrap());
    // Each clip keeps its pitch: stretched, not re-pitched (396 Hz and 412.5 Hz).
    assert!((hz(&out, 0.2, 0.9) - 440.0).abs() < 8.0, "{} Hz", hz(&out, 0.2, 0.9));
    assert!((hz(&out, 1.2, 1.9) - 330.0).abs() < 8.0, "{} Hz", hz(&out, 1.2, 1.9));
    assert!((hz(&out, 2.2, 2.7) - 440.0).abs() < 8.0, "{} Hz", hz(&out, 2.2, 2.7));
    assert_golden("launched-stretched", &out);
}
