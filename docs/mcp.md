# MCP server

`loopwerk-cli mcp` serves the [Model Context Protocol](https://modelcontextprotocol.io) on
stdin/stdout (the official `rmcp` SDK), so an agent can open, inspect, edit and render projects.
It follows the same conventions as PhotoCraft's MCP server.

```sh
loopwerk-cli mcp --read-root ./projects --write-root ./renders
```

- **Files.** Paths are relative to a root, or absolute paths beneath it. `..` and symlinks that
  lead outside are refused. Without `--read-root` the agent cannot open files; without
  `--write-root` it cannot write any. Existing files are replaced only with `overwrite: true`.
- **One session.** The server holds one open project with its undo history. `project_open`
  replaces it.

## Tools

| Tool | What it does |
|---|---|
| `project_open` | Open a `.als` Live Set below the read root; returns a summary and import warnings. |
| `project_inspect` | Summary, tracks (volume, pan, mute, devices) and arrangement clips as JSON. |
| `command_list` | Command ids with description, kind (`read`, `edit`, `history`) and parameter schema; optional `filter`. |
| `command_run` | Run one command: `{"id": "track.setPan", "params": {"track": "Bass", "pan": -1}}`. |
| `command_batch` | Run `steps` in order; each editing command is its own undo step; `stop_on_error` (default true). |
| `project_render` | Render the arrangement, or `scene` (1-based), to a WAV below the write root. |

Commands are defined once in `loopwerk-session` (ADR 0002); the MCP server, the CLI and later
the UI all run the same ones.

## Errors

- Unknown tool arguments return JSON-RPC error `-32602` naming the key, before anything runs.
- A command that fails returns a tool result with `isError: true` and leaves the project
  unchanged. Unknown command parameters are errors too.
- Every tool has a title and read-only/destructive hints.

## Using it from Claude Code

```sh
claude mcp add loopwerk -- /path/to/loopwerk-cli mcp --read-root /path/to/projects --write-root /path/to/renders
```
