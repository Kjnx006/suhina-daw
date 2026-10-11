//! Agent-facing automation: an MCP server (the official `rmcp` SDK) over a Loopwerk session,
//! file access limited to granted roots, and the offline WAV render shared with the CLI.

pub mod render;
pub mod server;
pub mod workspace;

pub use render::{RenderSummary, render_wav, scene_as_arrangement};
pub use server::LoopwerkMcp;
pub use workspace::{Workspace, display_path};
