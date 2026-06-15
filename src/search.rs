//! Driving Slack's search and extracting result rows.
//!
//! This is the *only* Slack-specific runtime logic — everything generic (login,
//! navigation, raw extraction) is apiwright interpreting the embedded map. The
//! non-obvious bits, discovered while mapping `acme`:
//!
//! - the IA4 search box opens on **typing** into the top-nav button;
//! - a query needs **two** Enter presses — the first commits the typeahead
//!   entry, the second executes the full-text search;
//! - each result row exposes author/channel/text as `data-qa` text, but the
//!   **permalink is the row timestamp link's `href`** (and `data-ts` carries the
//!   Slack `ts`), so those come from attributes, not text.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result};
use apiwright::AdapterSession;

use crate::{SearchQuery, SearchResult};

const PLACE: &str = "search_results";

/// Run every per-scope query, union the results, dedup by permalink, sort by
/// timestamp ascending.
pub(crate) async fn run(
    session: &AdapterSession,
    query: &SearchQuery,
) -> Result<Vec<SearchResult>> {
    let sels = Selectors::resolve(session)?;
    let mut seen = HashSet::new();
    let mut out: Vec<SearchResult> = Vec::new();
    for q in query.to_slack_queries() {
        for r in run_one(session, &q, &sels).await? {
            if seen.insert(r.permalink.clone()) {
                out.push(r);
            }
        }
    }
    out.sort_by(|a, b| a.ts.cmp(&b.ts));
    Ok(out)
}

async fn run_one(
    session: &AdapterSession,
    query: &str,
    sels: &Selectors,
) -> Result<Vec<SearchResult>> {
    // Slack search isn't URL-addressable, so the query must be entered through
    // the UI. From a channel view (where the top-nav search accepts typing),
    // open the box, replace any existing content, type the query, then click the
    // "Show results for …" typeahead entry to execute. Extraction below reads the
    // DOM directly — the click is the one unavoidable UI step.
    session.goto(&sels.workspace_url).await?;
    session.type_text(&sels.search_input, " ").await?; // open the search box
    session.press(&sels.search_box, "Meta+a").await?; // select any existing content
    session
        .type_text(&sels.search_box, query)
        .await
        .context("typing the search query")?;
    session
        .wait_for(&sels.confirm, Duration::from_secs(10))
        .await
        .context("the 'Show results for' search suggestion never appeared")?;
    session
        .click(&sels.confirm)
        .await
        .context("clicking the search suggestion to execute")?;

    // Poll for results to render. Clicking the suggestion triggers a
    // navigation, so a single `wait_for` can trip on a torn-down execution
    // context and error out; retrying *extraction* (treating an error or empty
    // result as "not ready yet") is robust to that. We poll the author *field*,
    // not just the row container, since fields render slightly later. Nothing
    // within the window ⇒ no matches.
    // Poll until the result *headers* render. Author / channel / timestamp /
    // permalink all live in the row header and appear together for the whole
    // screenful; the block-kit message *body* is virtualized (it renders lazily
    // on scroll), so it's handled best-effort below. Extraction errors (the
    // post-click navigation tears down the execution context) count as "not
    // ready yet". Nothing within the window ⇒ no matches.
    let author_sel = sels.scoped(&sels.author);
    let channel_sel = sels.scoped(&sels.channel);
    let perma_sel = sels.scoped(&sels.permalink);
    let mut authors = Vec::new();
    for _ in 0..25 {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        authors = session.extract_text(&author_sel).await.unwrap_or_default();
        let channels = session.extract_text(&channel_sel).await.unwrap_or_default();
        let perms = session.extract_attr(&perma_sel, "href").await.unwrap_or_default();
        if !authors.is_empty() && authors.len() == channels.len() && authors.len() == perms.len() {
            break;
        }
    }
    if authors.is_empty() {
        return Ok(Vec::new());
    }

    // Header columns — one entry per row, reliably present.
    let channels = session.extract_text(&channel_sel).await?;
    let permalinks = session.extract_attr(&perma_sel, "href").await?;
    let ts_raw = session.extract_attr(&perma_sel, "data-ts").await?;
    // Message body — virtualized, so only the visible rows' text is in the DOM.
    // Used only when it lines up one-per-row; otherwise the permalink is the
    // click-through to read the message. Full inline text needs scroll-collect
    // (specs/01-slack-adapter.md §7.2) — a follow-up.
    let texts = session
        .extract_text(&sels.scoped(&sels.text))
        .await
        .unwrap_or_default();

    let n = authors.len();
    if [channels.len(), permalinks.len(), ts_raw.len()]
        .iter()
        .any(|&l| l != n)
    {
        anyhow::bail!(
            "map looks stale at `{PLACE}`: header columns misaligned \
             (authors={n}, channels={}, permalinks={}, ts={}) — re-map the site",
            channels.len(),
            permalinks.len(),
            ts_raw.len(),
        );
    }
    let texts_aligned = texts.len() == n;

    Ok((0..n)
        .map(|i| SearchResult {
            ts: iso8601(&ts_raw[i]),
            channel: channels[i].trim().to_string(),
            author: authors[i].trim().to_string(),
            text: if texts_aligned {
                texts[i].trim().to_string()
            } else {
                String::new()
            },
            permalink: permalinks[i].clone(),
        })
        .collect())
}

/// Slack `ts` (`"1718200000.632709"`) → RFC 3339 UTC; falls back to the raw
/// string if it doesn't parse.
fn iso8601(ts: &str) -> String {
    ts.split('.')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ts.to_string())
}

/// Extraction selectors resolved from the embedded map *by name*. A missing
/// name fails fast (a stale/incomplete map) rather than silently extracting
/// nothing — the §7.4 drift guard.
struct Selectors {
    workspace_url: String,
    search_input: String,
    search_box: String,
    confirm: String,
    row: String,
    author: String,
    channel: String,
    permalink: String,
    text: String,
}

impl Selectors {
    fn resolve(session: &AdapterSession) -> Result<Self> {
        let get = |name: &str| -> Result<String> {
            session.element_selector(PLACE, name).with_context(|| {
                format!("map missing element '{name}' at `{PLACE}` — re-map the site")
            })
        };
        Ok(Self {
            workspace_url: session
                .place_url("workspace")
                .context("map missing 'workspace' place url — re-map the site")?,
            search_input: get("search_input")?,
            search_box: get("search_box")?,
            confirm: get("search_confirm")?,
            row: get("result_row")?,
            author: get("result_author")?,
            channel: get("result_channel")?,
            permalink: get("result_permalink")?,
            text: get("result_text")?,
        })
    }

    /// `field` scoped within the result-row container.
    fn scoped(&self, field: &str) -> String {
        format!("{} {field}", self.row)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn slack_ts_to_iso8601() {
        // 1700000000 = 2023-11-14T22:13:20Z
        assert_eq!(super::iso8601("1700000000.632709"), "2023-11-14T22:13:20+00:00");
        // unparseable → returned as-is
        assert_eq!(super::iso8601("not-a-ts"), "not-a-ts");
    }
}
