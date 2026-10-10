//! Headless Loopwerk CLI.

use std::path::PathBuf;
use std::process::ExitCode;

use loopwerk_engine::model::*;
use loopwerk_engine::render_offline;

const USAGE: &str = "\
usage: loopwerk-cli render --demo -o <out.wav> [--sample-rate <hz>]

commands:
  render    render a project's arrangement offline to a 32-bit float stereo WAV file

options:
  --demo             render the built-in demo project (project files are not supported yet)
  -o, --output       output WAV path
  --sample-rate      sample rate in Hz (default 48000)";

struct RenderArgs {
    output: PathBuf,
    sample_rate: u32,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("render") => match parse_render(args.get(1..).unwrap_or_default()).and_then(|a| render(&a)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
        Some("--version" | "-V") => {
            println!("loopwerk-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn parse_render(args: &[String]) -> Result<RenderArgs, String> {
    let mut output = None;
    let mut sample_rate = 48_000;
    let mut demo = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--demo" => demo = true,
            "-o" | "--output" => output = Some(PathBuf::from(it.next().ok_or("--output needs a path")?)),
            "--sample-rate" => {
                let value = it.next().ok_or("--sample-rate needs a value")?;
                sample_rate = value.parse().map_err(|_| format!("invalid sample rate: {value}"))?;
            }
            other => return Err(format!("unknown argument: {other}\n\n{USAGE}")),
        }
    }
    if !demo {
        return Err(format!("only --demo is supported until project import lands\n\n{USAGE}"));
    }
    Ok(RenderArgs { output: output.ok_or(format!("missing --output\n\n{USAGE}"))?, sample_rate })
}

fn render(args: &RenderArgs) -> Result<(), String> {
    let project = demo_project();
    let samples = render_offline(&project, args.sample_rate, 512).map_err(|e| e.to_string())?;

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: args.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let path = &args.output;
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| format!("{}: {e}", path.display()))?;
    for sample in &samples {
        writer.write_sample(*sample).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    writer.finalize().map_err(|e| format!("{}: {e}", path.display()))?;

    let seconds = samples.len() as f64 / 2.0 / f64::from(args.sample_rate);
    println!("wrote {} ({seconds:.2} s, {} Hz)", path.display(), args.sample_rate);
    Ok(())
}

/// Two bars at 120 bpm: a bass line and a chord pattern on separate tracks, each placed twice.
fn demo_project() -> Project {
    let note = |time: f64, duration: f64, key: u8, velocity: f32| Note {
        time,
        duration,
        key,
        velocity,
        release_velocity: 0.5,
        channel: 0,
    };
    let track = |id: u32, name: &str| Track {
        id: TrackId(id),
        name: name.into(),
        color: None,
        content: ContentKinds { notes: true, audio: false },
        channel: Some(ChannelId(id)),
        parent: None,
        extensions: Extensions::default(),
    };
    let channel = |id: u32, name: &str, volume: f64, pan: f64| Channel {
        id: ChannelId(id),
        name: name.into(),
        role: ChannelRole::Regular,
        volume,
        pan,
        mute: false,
        solo: false,
        output: None,
        sends: Vec::new(),
        devices: Vec::new(),
        extensions: Extensions::default(),
    };
    let content = |id: u32, name: &str, notes: Vec<Note>| ClipContent {
        id: ContentId(id),
        name: name.into(),
        length: 4.0,
        lanes: vec![ContentLane { target: None, data: LaneData::Notes(notes) }],
        extensions: Extensions::default(),
    };
    let clip = |content: u32, start: f64| Clip {
        content: ContentId(content),
        start,
        length: 4.0,
        offset: 0.0,
        loop_range: None,
        enabled: true,
        extensions: Extensions::default(),
    };

    let bass = (0..8).map(|i| note(f64::from(i) * 0.5, 0.4, if i % 4 == 3 { 43 } else { 36 }, 0.9)).collect();
    let chords = [0.0, 1.5, 2.5].into_iter().flat_map(|t| [60, 64, 67].map(|k| note(t, 0.75, k, 0.5))).collect();

    Project {
        tracks: vec![track(1, "Bass"), track(2, "Chords")],
        channels: vec![channel(1, "Bass", 0.9, 0.0), channel(2, "Chords", 0.6, 0.3)],
        contents: vec![content(1, "Bass", bass), content(2, "Chords", chords)],
        arrangement: Arrangement {
            lanes: vec![
                ArrangementLane {
                    name: "Bass".into(),
                    track: Some(TrackId(1)),
                    clips: vec![clip(1, 0.0), clip(1, 4.0)],
                },
                ArrangementLane {
                    name: "Chords".into(),
                    track: Some(TrackId(2)),
                    clips: vec![clip(2, 0.0), clip(2, 4.0)],
                },
            ],
        },
        ..Project::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_project_is_valid_and_renders_two_bars() {
        let project = demo_project();
        assert_eq!(project.validate(), Ok(()));
        let samples = render_offline(&project, 48_000, 512).unwrap();
        let seconds = samples.len() as f64 / 2.0 / 48_000.0;
        assert!((4.0..4.2).contains(&seconds), "{seconds} s");
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.1 && peak <= 1.0, "peak {peak}");
    }

    #[test]
    fn render_args_require_demo_and_output() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_render(&args(&["-o", "x.wav"])).is_err());
        assert!(parse_render(&args(&["--demo"])).is_err());
        assert!(parse_render(&args(&["--demo", "-o"])).is_err());
        assert!(parse_render(&args(&["--demo", "-o", "x.wav", "--sample-rate", "abc"])).is_err());
        let ok = parse_render(&args(&["--demo", "-o", "x.wav", "--sample-rate", "44100"])).unwrap();
        assert_eq!(ok.sample_rate, 44_100);
    }
}
