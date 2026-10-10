# AGENTS.md: guide for AI agents and contributors

Loopwerk is an open-source, native, clip-launching DAW written in **Rust only**, comparable in
workflow to Ableton Live. The product name is always written **Loopwerk** in user-facing text;
machine names stay lowercase (`loopwerk-*` crates, binaries, file names). Read this file first,
then [`CONTRIBUTING.md`](CONTRIBUTING.md).

## 1. Golden rules

1. **Clean-room (outranks everything).** Follow [`CONTRIBUTING.md`](CONTRIBUTING.md) › Clean-room.
   Never disassemble or decompile proprietary DAW or plug-in binaries, never use leaked code, never
   commit content from commercial products. Knowledge comes from observation, self-authored files
   and public documentation, and is written down in `docs/spec/` before it is implemented.
2. **Never crash.** Non-test code never panics: no `unwrap()`, `expect()`, `panic!`,
   `unreachable!`, `todo!` or `unimplemented!`. Return the crate's error type. Input-derived
   numbers are hostile: use `get()` and `checked_*`, cap allocations sized by input. No `unsafe`
   (`unsafe_code = "forbid"` workspace-wide).
3. **Real-time safety.** Code reachable from the audio callback never allocates, frees, locks,
   blocks, logs or does I/O. Hand data to it through lock-free queues (`rtrb`) and swap whole
   graphs atomically.
4. **One render path.** Offline rendering and real-time playback run the same engine code, so
   offline renders are the test oracle for the engine.
5. **Everything is a command.** User-visible behaviour is a command in the engine with an id,
   params and tests. The UI, CLI and MCP all dispatch the same commands.
6. **Tests are the gate.** Every change comes with tests: round-trip and malformed-input tests for
   formats, golden-file renders for DSP and engine behaviour.
7. **GPL-compatible dependencies only.** Loopwerk is GPL-3.0-or-later. Dependencies must be
   compatible with GPLv3: MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, LGPL, GPL-3.0. Not GPL-2.0-only,
   not proprietary or "non-commercial" licences.

## 2. Workspace map

```text
crates/
  model    L0 DAW-neutral project as pure data (tracks, channels, clip contents, clips, launcher)
  engine   L1 audio graph and offline/real-time rendering
  formats/ (planned) one crate per file format (als, dawproject, flp), converting to and from model
apps/
  loopwerk-cli   headless CLI (render, later mcp)
docs/
  decisions/  architecture decision records; read 0001 before changing the model
  spec/       clean-room specs written from observation (formats, behaviour)
scripts/   research helpers (e.g. extracting and diffing .als files)
```

A crate may depend only on lower layers. Nothing below the UI layer may depend on a GUI toolkit.

## 3. Before you finish a task

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Keep the tree building at every step.
