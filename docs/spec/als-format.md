# Live Set (`.als`) format

What we have learned about the Live Set format, from Live Sets we authored ourselves (see
[`als-corpus.md`](als-corpus.md)). Written in our own words; no Ableton code or documentation text.

- **Observed with:** Live 12.4.6 (`Creator="Ableton Live 12.4.6"`, `MinorVersion="12.0_12402"`,
  `MajorVersion="5"`), Windows.
- **Corpus:** series 1, files `00`–`20` (MIDI clips, scenes, mixer); series 2, files `30`–`36` (audio clips); series 3, files `40`–`44` (devices, plug-ins).
- Every statement marked **(hypothesis)** still needs a corpus file that proves it.

## Container

A `.als` file is gzip-compressed UTF-8 XML. The root element is `<Ableton>` with the version
attributes above; its only relevant child is `<LiveSet>`.

Most scalar values are stored as an element with a `Value` attribute, for example
`<Name Value="Test" />`. Automatable parameters are an element with a `<Manual Value="…"/>` child
(the current value when there is no automation) plus automation and MIDI-mapping data.

Many elements carry an `Id` attribute. Ids are unique only among siblings of the same kind and
are **not stable**: Live renumbers some of them on every save (see *Noise*), and moving an
arrangement clip gives it a new id. Never use them to identify an object across saves.

## Time

All musical times are in **beats** (quarter notes), as decimal numbers. Note `Time="1"` is beat 2
of bar 1; `Duration="1"` is a quarter note. Bars are not stored; at 4/4, bar 3 starts at beat 8.

Default automation values use the time `-63072000` as a "before everything" marker.

## Transport

| What | Where (under `LiveSet/MainTrack/DeviceChain/Mixer`) | Encoding |
|---|---|---|
| Tempo | `Tempo/Manual@Value` | BPM as a decimal number |
| Time signature | `TimeSignature/Manual@Value` | an integer code; `201` = 4/4. The mapping for other signatures is unknown **(needs corpus)**. |

The tempo is also the value of the tempo automation's default event
(`MainTrack/AutomationEnvelopes/…/FloatEvent[@Time="-63072000"]`); both change together.

## Tracks

`LiveSet/Tracks` lists the tracks in display order. Element names give the kind: `MidiTrack`,
`AudioTrack`, `ReturnTrack`, and `GroupTrack` **(not in corpus yet)**. The main output is the
separate `LiveSet/MainTrack`.

Per track:

| What | Where | Notes |
|---|---|---|
| Name | `Name/EffectiveName@Value` | Default names look like `1-MIDI`, `3-Audio`, `A-Reverb`. |
| Colour | `Color@Value` | Index into Live's colour palette. |
| Group | `TrackGroupId@Value` | `-1` when not in a group; otherwise the group track's `Id` **(hypothesis)**. |
| Mixer | `DeviceChain/Mixer` | see below |
| Output | `DeviceChain/AudioOutputRouting/Target@Value` | `AudioOut/Main` routes to the main output. |
| Session clips | `DeviceChain/MainSequencer/ClipSlotList/ClipSlot` | One slot per scene, in scene order. Return tracks have no slots. |
| Arrangement clips | `DeviceChain/MainSequencer/ClipTimeable/ArrangerAutomation/Events` | `MidiClip` (and `AudioClip`) elements. |

### Mixer

Under `DeviceChain/Mixer`, values in `…/Manual@Value`:

| What | Element | Encoding |
|---|---|---|
| Volume | `Volume` | Linear gain. `1` = 0 dB; `0.5011872053` = −6 dB. |
| Pan | `Pan` | `-1` … `1`; Live's "50L" is `-1` (full left). |
| Track on/off ("mute") | `Speaker` | `true` = playing, `false` = muted. |
| Sends | `Sends/TrackSendHolder[n]/Send` | Linear gain. One holder per return track, in return-track order. "−∞" in the UI is stored as `0.0003162277571` (−70 dB). |

## Scenes

`LiveSet/Scenes/Scene`, in display order. A new scene inserted in the middle gets the next free
`Id` but takes its place in the list, so **order comes from the list, not the ids**.

Per scene: `Name@Value`, `Tempo@Value` with `IsTempoEnabled@Value` (the tempo applies only when
enabled), and the same pair for time signature.

## MIDI clips

The same `MidiClip` element is used in session slots
(`ClipSlot/ClipSlot/Value/MidiClip`) and on the arrangement.

| What | Where | Notes |
|---|---|---|
| Arrangement position | `@Time` | Beats. Session clips have `Time="0"`. |
| Extent | `CurrentStart@Value`, `CurrentEnd@Value` | Arrangement clips: positions on the arrangement (`8` … `12` for a one-bar clip at bar 3). Session clips: positions in the clip's content (`0` … `4`). |
| Loop on/off | `Loop/LoopOn@Value` | |
| Loop region | `Loop/LoopStart@Value`, `Loop/LoopEnd@Value` | In content beats. Setting the clip length to 2 bars changed `LoopEnd` (and `CurrentEnd`) from 4 to 8. |
| Start marker | `Loop/StartRelative@Value` | Relative to `LoopStart` **(hypothesis)**. |
| End marker (loop off) | `Loop/OutMarker@Value` | **(hypothesis)** |
| Name, colour | `Name@Value`, `Color@Value` | |
| Muted clip | `Disabled@Value` | **(hypothesis: not toggled in corpus)** |

### Notes

Notes are grouped by key: `Notes/KeyTracks/KeyTrack`, each with `MidiKey@Value` (MIDI note
number) and its notes in `Notes/MidiNoteEvent`:

| Attribute | Meaning |
|---|---|
| `Time` | Start in content beats |
| `Duration` | Length in beats |
| `Velocity` | 0–127, **a decimal**: dragging gave `64.251297` |
| `OffVelocity` | Release velocity, 0–127 |
| `NoteId` | Per-clip note id |

Live names MIDI note 60 "C3" (so 64 is E3, 67 is G3). Notes drawn without changing anything got
velocity 100 in one clip and 98 in another.

## Audio clips

Corpus: series 2, files `30`–`35` (one 4-second, 48 kHz WAV rendered by Loopwerk).

`AudioClip` shares `Time`, `CurrentStart`/`CurrentEnd`, `Loop`, `Name`, `Color` and `Disabled`
with `MidiClip`. Session audio clips sit in clip slots like MIDI clips, but **arrangement audio
clips are under `DeviceChain/MainSequencer/Sample/ArrangerAutomation/Events`**, not under
`ClipTimeable` as on MIDI tracks.

| What | Where | Notes |
|---|---|---|
| File | `SampleRef/FileRef/RelativePath@Value` | Relative to the Live Set's folder (`../loopwerk-demo.wav`). |
| File, absolute | `SampleRef/FileRef/Path@Value` | The full path on the machine that saved the set. **Personal data**: strip before sharing a corpus. |
| File size, checksum | `FileRef/OriginalFileSize`, `FileRef/OriginalCrc` | |
| Length | `SampleRef/DefaultDuration@Value` | In sample frames (`192000`). |
| Sample rate | `SampleRef/DefaultSampleRate@Value` | `48000` |
| Warp on/off | `IsWarped@Value` | |
| Warp mode | `WarpMode@Value` | `4` for this file by default; meaning of the codes unknown **(needs corpus)**. |
| Warp markers | `WarpMarkers/WarpMarker@SecTime`, `@BeatTime` | Pairs of (seconds in the file, beats in the clip). |
| Clip gain | `SampleVolume@Value` | Linear, `1` = 0 dB; −6 dB is `0.5011872053` (file `36`). |
| Transpose | `PitchCoarse@Value`, `PitchFine@Value` | Semitones, and cents **(fine: hypothesis)**. |

**Warp markers.** A newly added file got markers at (0 s, beat 0) and (4 s, beat 8): Live assumed
120 bpm. Each list ends with one more marker 1/32 beat after the last real one
((4.015625 s, beat 8.03125)); it sets the playback rate after the last marker. Adding a marker on
beat 2 and dragging it right replaced the list with (0, 0), (0.500146 s, beat 1.125) and its
1/32-beat follower.

**Unwarped clips.** Turning warp off changed `LoopEnd` from `8` to `4` while the file is 4 s long:
with warp off, the loop region is in **seconds** **(hypothesis: confirm with a moved loop)**.
`OutMarker` and `CurrentEnd` did not change.

Adding the first clip to an empty track renamed the track after the clip (`4-Audio` →
`4-loopwerk-demo`), stored in `Name/EffectiveName` and `Name/MemorizedFirstClipName`.

## Devices

Corpus: series 3, files `40`–`44` (Simpler, EQ Eight, and the commercial plug-ins ShaperBox 3
and Sylenth1 as VST2 and VST3; the plug-in files stay local, see below).

A track's device chain is `DeviceChain/DeviceChain/Devices`, devices in chain order. The main
track has the same structure. Adding an instrument to a track named after its first clip renamed
it after the device (`1-MIDI` → `1-Simpler`, later `1-Sylenth1`).

Every device has `On/Manual@Value` (the device on/off switch; turning EQ Eight off changed it
from `true` to `false`) and `UserName@Value` (empty unless renamed).

**Live's own devices** are elements named after the device type: `OriginalSimpler`, `Eq8`,
`Reverb`, `Delay`. Their parameters are nested elements with the usual `Manual` values; band 3
gain of EQ Eight is `Bands.2/ParameterA/Gain/Manual` (+3 dB stored as `3`). `LastPresetRef`
points at the default preset the device was loaded from.

**Plug-ins** are `PluginDevice` elements whose `PluginDesc` holds one of:

| | `VstPluginInfo` (VST2) | `Vst3PluginInfo` (VST3) |
|---|---|---|
| Name | `PlugName@Value` | `Name@Value` |
| Identity | `UniqueId@Value`: the 32-bit plug-in id as a decimal | `Uid/Fields.0`…`Fields.3`: the 16-byte class id as four signed 32-bit decimals |
| State | `Preset/VstPreset/Buffer`: hex text | `Preset/Vst3Preset/ProcessorState` and `ControllerState`: hex text |
| Location | `Path@Value`: absolute path of the `.dll` on the saving machine (**personal data**) | none |
| Other | `Version`, `NumberOfParameters`, `Category`, window position | `DeviceType` (`1` for the instrument, `2` for the effect **(hypothesis)**), window position |

Plug-in state is the plug-in vendor's own format. Loopwerk keeps it verbatim and hands it back
to the plug-in. **Never commit a corpus file that contains a commercial plug-in's state.**

## Noise: ignore when importing or diffing

These change without any musical edit:

- `LiveSet/NoteAlgorithms`: state of the MIDI tools; renumbered on every save.
- Everything named `ViewState*`, `SequencerNavigator`, `ScrollerTimePreserver`, `TimeSelection`,
  `ClientSize`, `ScrollerPos`, `CurrentZoom`, `HighlightedTrackIndex`, `Selected*`,
  `IsContentSelectedInDocument`, `LastSelected*`: view and selection state.
- `ColorSequenceIndex`, `AutoColorPicker*/NextColorIndex`: automatic colour assignment.
- `TakeId` on clips.
- `Transport/CurrentTime`: the play position.

## Open questions (next corpus files)

- Time-signature codes other than 4/4.
- Start marker, end marker and loop start moved away from 0, with loop on and off.
- Group tracks, return tracks added or removed, track output to another track.
- Disabled clips and disabled notes; note probability and velocity deviation.
- Warp modes, a moved loop on an unwarped clip.
- CLAP plug-ins, Audio Units, racks, Max for Live devices.
