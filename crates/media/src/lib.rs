//! Audio files for a project: finds the files its audio clips refer to and decodes them into
//! memory with symphonia (MPL-2.0), so the engine can play them without touching the disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use loopwerk_engine::model::{LaneData, Project};
pub use loopwerk_engine::{AudioData, AudioPool};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Longest file we decode, so one huge or hostile file cannot exhaust memory (about 2.6 GiB of
/// stereo f32 at 48 kHz).
const MAX_SECONDS: f64 = 60.0 * 60.0 * 2.0;

/// Loads every audio file the project uses. `project_dir` is the folder the project file is in;
/// a region's relative path is resolved from there first, then its absolute path is tried.
/// Files that cannot be found or decoded are reported in the returned warnings and left out.
pub fn load_project_audio(project: &Project, project_dir: &Path) -> (AudioPool, Vec<String>) {
    let mut pool = AudioPool::new();
    let mut warnings = Vec::new();
    for content in &project.contents {
        for lane in &content.lanes {
            let LaneData::Audio(region) = &lane.data else { continue };
            if pool.contains_key(&region.file) {
                continue;
            }
            let mut candidates = vec![project_dir.join(&region.file)];
            if let Some(absolute) = &region.absolute_file {
                candidates.push(PathBuf::from(absolute));
            }
            let Some(path) = candidates.iter().find(|p| p.is_file()) else {
                warnings.push(format!("audio file {} not found", region.file));
                continue;
            };
            match decode_file(path) {
                Ok(data) => {
                    pool.insert(region.file.clone(), Arc::new(data));
                }
                Err(e) => warnings.push(format!("audio file {}: {e}", path.display())),
            }
        }
    }
    (pool, warnings)
}

/// Decodes a whole audio file (WAV, AIFF, FLAC, MP3, Ogg Vorbis) to interleaved f32.
pub fn decode_file(path: &Path) -> Result<AudioData, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, stream, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("unsupported or unreadable format: {e}"))?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio track")?;
    let track_id = track.id;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("no audio codec parameters")?.clone();
    let sample_rate = params.sample_rate.filter(|r| (1..=1_000_000).contains(r)).ok_or("unknown sample rate")?;
    let channels =
        params.channels.as_ref().map(|c| c.count()).filter(|c| (1..=64).contains(c)).ok_or("unknown channel count")?;
    let channels = u16::try_from(channels).map_err(|_| "too many channels")?;
    let max_samples = (MAX_SECONDS * f64::from(sample_rate)) as usize * usize::from(channels);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;
    let mut samples = Vec::new();
    let mut packet_samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) | Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("read error: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                decoded.copy_to_vec_interleaved(&mut packet_samples);
                samples.extend_from_slice(&packet_samples);
                if samples.len() > max_samples {
                    return Err(format!("longer than {} hours", MAX_SECONDS / 3600.0));
                }
            }
            // A damaged packet is skipped, as players do.
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => continue,
            Err(e) => return Err(format!("decode error: {e}")),
        }
    }
    if samples.is_empty() {
        return Err("no audio decoded".into());
    }
    Ok(AudioData { sample_rate, channels, samples })
}

#[cfg(test)]
mod tests {
    use super::*;
    use loopwerk_engine::model::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("loopwerk-media-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("set")).unwrap();
        dir
    }

    fn write_wav(path: &Path, channels: u16, frames: usize) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames * usize::from(channels) {
            w.write_sample(if i % 2 == 0 { 16_384i16 } else { -16_384 }).unwrap();
        }
        w.finalize().unwrap();
    }

    fn project_with_audio(file: &str, absolute: Option<String>) -> Project {
        Project {
            contents: vec![ClipContent {
                id: ContentId(1),
                name: "a".into(),
                length: 4.0,
                lanes: vec![ContentLane {
                    target: None,
                    data: LaneData::Audio(AudioRegion {
                        file: file.into(),
                        absolute_file: absolute,
                        sample_rate: None,
                        channels: None,
                        duration_seconds: None,
                        warped: false,
                        warp_markers: Vec::new(),
                        gain: 1.0,
                        transpose: 0.0,
                    }),
                }],
                extensions: Extensions::default(),
            }],
            ..Project::default()
        }
    }

    #[test]
    fn decodes_wav_to_interleaved_f32() {
        let dir = temp_dir("decode");
        let path = dir.join("tone.wav");
        write_wav(&path, 2, 1000);
        let data = decode_file(&path).unwrap();
        assert_eq!((data.sample_rate, data.channels, data.frames()), (44_100, 2, 1000));
        assert!((data.samples[0] - 0.5).abs() < 1e-3 && (data.samples[1] + 0.5).abs() < 1e-3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_relative_paths_from_the_project_folder_then_absolute() {
        let dir = temp_dir("resolve");
        write_wav(&dir.join("shared.wav"), 1, 100);
        // Relative to the set's folder, the way Live writes "../file.wav".
        let (pool, warnings) = load_project_audio(&project_with_audio("../shared.wav", None), &dir.join("set"));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(pool["../shared.wav"].channels, 1);

        // Relative path broken, absolute path still good.
        let absolute = dir.join("shared.wav").display().to_string();
        let (pool, warnings) =
            load_project_audio(&project_with_audio("moved/away.wav", Some(absolute)), &dir.join("set"));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(pool.contains_key("moved/away.wav"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_broken_files_are_warnings() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join("set").join("noise.wav"), b"definitely not audio").unwrap();
        let (pool, warnings) = load_project_audio(&project_with_audio("gone.wav", None), &dir.join("set"));
        assert!(pool.is_empty() && warnings[0].contains("not found"));
        let (pool, warnings) = load_project_audio(&project_with_audio("noise.wav", None), &dir.join("set"));
        assert!(pool.is_empty() && warnings.len() == 1, "{warnings:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
