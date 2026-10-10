# Live Set (`.als`) format

What we have learned about the Live Set format, from Live Sets we authored ourselves (see
[`als-corpus.md`](als-corpus.md)). Written in our own words; no Ableton code or documentation text.

- **Observed with:** Live 12.4.6 (`Creator="Ableton Live 12.4.6"`, `MinorVersion="12.0_12402"`,
  `MajorVersion="5"`), Windows.
- **Corpus:** series 1, files `00`–`20` (MIDI clips, scenes, mixer).
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
- Audio clips and warping (series 2), devices and plug-ins (series 3).
