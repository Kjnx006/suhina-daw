//! Real-time playback on the default audio output.
//!
//! The engine's [`Renderer`] is built here, on the main thread, and then moved into the audio
//! callback. The callback only calls `Renderer::process`, converts samples and sets atomics: no
//! allocation, locks or I/O on the audio thread (AGENTS.md rule 3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use loopwerk_engine::Renderer;
use loopwerk_engine::model::{LaneData, Project};

/// Frames rendered per engine call; output buffers larger than this are filled in several calls.
const SCRATCH_FRAMES: usize = 4096;
/// Extra time to wait after the last frame, so the device can drain its buffer.
const TAIL: Duration = Duration::from_millis(300);

/// State the audio callback shares with the main thread.
#[derive(Default)]
struct Progress {
    frames: AtomicU64,
    stream_error: AtomicBool,
}

/// Plays the project's arrangement once on the default output device and returns when it ends.
pub fn play(project: &Project) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device found")?;
    let supported = device.default_output_config().map_err(|e| format!("audio output {device}: {e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    if config.channels == 0 {
        return Err(format!("audio output {device} reports no channels"));
    }

    let renderer = Renderer::new(project, config.sample_rate).map_err(|e| e.to_string())?;
    let length = renderer.length();
    if length == 0 {
        println!("{}", nothing_to_play(project));
        return Ok(());
    }
    let progress = Arc::new(Progress::default());

    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, renderer, &progress),
        cpal::SampleFormat::I16 => build::<i16>(&device, config, renderer, &progress),
        cpal::SampleFormat::I32 => build::<i32>(&device, config, renderer, &progress),
        cpal::SampleFormat::U16 => build::<u16>(&device, config, renderer, &progress),
        other => return Err(format!("audio output {device} uses sample format {other}, which is not supported yet")),
    }?;
    stream.play().map_err(|e| format!("could not start audio output {device}: {e}"))?;

    let seconds = length as f64 / f64::from(config.sample_rate);
    println!(
        "playing {seconds:.2} s on {device} ({} Hz, {} channels, {format}); Ctrl+C stops",
        config.sample_rate, config.channels
    );

    // Wait for the callback to reach the end, with a wall-clock limit in case the device stalls.
    let deadline = Instant::now() + Duration::from_secs_f64(seconds) + Duration::from_secs(5);
    while progress.frames.load(Ordering::Relaxed) < length {
        if progress.stream_error.load(Ordering::Relaxed) {
            return Err(format!("audio output {device} reported an error during playback"));
        }
        if Instant::now() > deadline {
            return Err(format!("audio output {device} stopped advancing"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(TAIL);
    drop(stream);
    println!("done");
    Ok(())
}

/// Explains why a project produced no sound: no clips at all, or only clips the engine cannot
/// play yet.
fn nothing_to_play(project: &Project) -> String {
    let clips: Vec<_> = project.arrangement.lanes.iter().flat_map(|l| &l.clips).collect();
    let audio = clips
        .iter()
        .filter(|c| {
            project
                .content(c.content)
                .is_some_and(|content| content.lanes.iter().any(|l| matches!(l.data, LaneData::Audio(_))))
        })
        .count();
    match (clips.len(), audio) {
        (0, _) => "nothing to play: the arrangement has no clips (try --scene <n> for launcher clips)".to_string(),
        (_, 0) => "nothing to play: the arrangement's clips have no notes".to_string(),
        (total, audio) => format!(
            "nothing to play yet: {audio} of {total} arrangement clips are audio clips, and audio playback \
             is not implemented yet (ROADMAP M5); only MIDI notes play for now"
        ),
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut renderer: Renderer,
    progress: &Arc<Progress>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let mut scratch = vec![0.0f32; SCRATCH_FRAMES * 2];
    let on_data = Arc::clone(progress);
    let on_error = Arc::clone(progress);

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for block in data.chunks_mut(SCRATCH_FRAMES * channels) {
                    let frames = block.len() / channels;
                    let Some(stereo) = scratch.get_mut(..frames * 2) else { return };
                    renderer.process(stereo);
                    write_frames(block, channels, stereo);
                    on_data.frames.fetch_add(frames as u64, Ordering::Relaxed);
                }
            },
            move |_err| on_error.stream_error.store(true, Ordering::Relaxed),
            None,
        )
        .map_err(|e| format!("could not open audio output {device}: {e}"))
}

/// Writes interleaved stereo into a device buffer with any channel count: mono gets the average,
/// channels beyond the first two get silence.
fn write_frames<T: SizedSample + FromSample<f32>>(out: &mut [T], channels: usize, stereo: &[f32]) {
    for (frame, [left, right]) in out.chunks_mut(channels).zip(stereo.as_chunks::<2>().0) {
        let mut samples = frame.iter_mut();
        if channels == 1 {
            if let Some(mono) = samples.next() {
                *mono = T::from_sample((left + right) * 0.5);
            }
            continue;
        }
        if let Some(l) = samples.next() {
            *l = T::from_sample(*left);
        }
        if let Some(r) = samples.next() {
            *r = T::from_sample(*right);
        }
        for rest in samples {
            *rest = T::from_sample(0.0f32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_maps_onto_any_channel_count() {
        let stereo = [0.5f32, -0.5, 1.0, 0.0];

        let mut mono = [0.0f32; 2];
        write_frames(&mut mono, 1, &stereo);
        assert_eq!(mono, [0.0, 0.5]);

        let mut two = [0.0f32; 4];
        write_frames(&mut two, 2, &stereo);
        assert_eq!(two, stereo);

        let mut four = [9.0f32; 8];
        write_frames(&mut four, 4, &stereo);
        assert_eq!(four, [0.5, -0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);

        let mut ints = [0i16; 4];
        write_frames(&mut ints, 2, &stereo);
        assert_eq!(ints[2], i16::MAX);
    }
}
