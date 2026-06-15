//! Embedded workspace maps.
//!
//! The structural map (`places` / `elements` / `mask`) is compiled into the
//! binary so the adapter is **standalone** — it never reads `~/.stencilwright`
//! at runtime, which would be empty for anyone but the person who mapped it.
//! Per-user `values` (secret references) are supplied at runtime, not bundled.
//!
//! Re-mapping a workspace (when Slack's UI drifts) means re-running
//! `stencilwright`, copying the refreshed TOML in `maps/<site>/`, and rebuilding.

use anyhow::{Result, bail};
use apiwright::stencil_places::PlaceGraph;

/// Load the embedded map for `site`. All bundled maps are compiled in; the
/// match selects which one. Errors if `site` isn't bundled.
pub fn load(site: &str) -> Result<PlaceGraph> {
    let (places, elements, mask) = match site {
        "acme" => (
            include_str!("../maps/acme/places.toml"),
            include_str!("../maps/acme/elements.toml"),
            include_str!("../maps/acme/mask.toml"),
        ),
        other => bail!("no embedded map for site '{other}' (this build bundles: acme)"),
    };
    PlaceGraph::from_toml_strs(Some(places), Some(elements), Some(mask), None)
}
