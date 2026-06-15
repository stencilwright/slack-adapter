//! The embedded Slack map.
//!
//! The structural map (`places` / `elements` / `mask`) is compiled into the
//! binary so the adapter is **standalone** — it never reads `~/.stencilwright`
//! at runtime, which would be empty for anyone but the person who mapped it.
//!
//! The map is a **template**: it carries no workspace-specific identifiers. Your
//! workspace subdomain and team id (from [`SlackConfig`]) fill the
//! `{workspace}` / `{team_id}` placeholders at load, and per-user secret
//! references attach as runtime `values` — so anyone can use the published
//! crate against their own workspace without forking it.
//!
//! Re-mapping (when Slack's web UI drifts) means re-running `stencilwright`,
//! refreshing the TOML under `maps/slack/`, and rebuilding.

use anyhow::{Result, bail};
use apiwright::stencil_places::PlaceGraph;

use crate::SlackConfig;

const PLACES: &str = include_str!("../maps/slack/places.toml");
const ELEMENTS: &str = include_str!("../maps/slack/elements.toml");
const MASK: &str = include_str!("../maps/slack/mask.toml");

/// Build the embedded Slack [`PlaceGraph`] for `config`'s workspace: substitute
/// the `{workspace}` / `{team_id}` placeholders and attach the per-user
/// `values` (e.g. a `slack_email` secret reference for login auto-fill).
pub fn load(config: &SlackConfig) -> Result<PlaceGraph> {
    let workspace = config.workspace_subdomain();
    let team_id = config.team_id.trim();
    if workspace.is_empty() || team_id.is_empty() {
        bail!("SlackConfig needs a non-empty workspace subdomain and team_id");
    }

    let places = PLACES
        .replace("{workspace}", workspace)
        .replace("{team_id}", team_id);

    let mut graph = PlaceGraph::from_toml_strs(Some(&places), Some(ELEMENTS), Some(MASK), None)?;
    graph
        .values
        .entries
        .extend(config.values.iter().map(|(k, v)| (k.clone(), v.clone())));
    Ok(graph)
}
