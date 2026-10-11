//! Offline rendering to WAV files, shared by the CLI and the MCP server.

use std::path::Path;

use loopwerk_engine::model::{Arrangement, ArrangementLane, Project};
use loopwerk_engine::render_offline;

/// What a render produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSummary {
    pub seconds: f64,
    pub sample_rate: u32,
    /// Largest absolute sample value; above 1.0 means the file clips.
    pub peak: f32,
}

/// Renders the project's arrangement to a 32-bit float stereo WAV file at `path`.
pub fn render_wav(project: &Project, path: &Path, sample_rate: u32) -> Result<RenderSummary, String> {
    let samples = render_offline(project, sample_rate, 512).map_err(|e| e.to_string())?;
    let spec =
        hound::WavSpec { channels: 2, sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| format!("{}: {e}", path.display()))?;
    for sample in &samples {
        writer.write_sample(*sample).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    writer.finalize().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(RenderSummary {
        seconds: samples.len() as f64 / 2.0 / f64::from(sample_rate),
        sample_rate,
        peak: samples.iter().fold(0.0f32, |m, s| m.max(s.abs())),
    })
}

/// Replaces the arrangement with the clips of one launcher scene (0-based), each played once
/// from the start, as if the scene were launched at time zero.
pub fn scene_as_arrangement(project: &Project, scene: usize) -> Result<Project, String> {
    let count = project.launcher.scenes.len();
    if scene >= count {
        return Err(format!("scene {} does not exist; the project has {count} scenes", scene.saturating_add(1)));
    }
    let lanes = project
        .launcher
        .columns
        .iter()
        .map(|column| ArrangementLane {
            name: String::new(),
            track: Some(column.track),
            clips: column.slots.get(scene).and_then(|s| s.clip.clone()).into_iter().collect(),
        })
        .collect();
    let mut project = project.clone();
    project.arrangement = Arrangement { lanes };
    Ok(project)
}
