//! Running Slack's search through its internal JSON API.
//!
//! Slack's web client backs message search with a browser-automation-backed JSON endpoint,
//! `search.modules.messages`. Instead of typing into the UI and reading the
//! (virtualized) result DOM, the adapter calls that endpoint directly from the
//! authenticated page — reusing the browser's session `d` cookie and the
//! page's own `xoxc` client token — so no Slack app, token grant, or admin
//! approval is needed. The request is indistinguishable from the web client's.
//!
//! This is the *only* Slack-specific runtime logic; everything else (login,
//! navigation, the raw-DOM bridge) is generic apiwright. It is fully
//! deterministic and sidesteps every DOM fragility the UI path hit — virtualized
//! message bodies, lazily-rendered headers, and the two-step typeahead.
//!
//! Two wrinkles, both discovered by analysis (see `examples/api_discover.rs`):
//!
//! - **`@me` is UI-only.** The API does *not* resolve `from:@me`/`to:@me`
//!   (they return zero results), so they're expanded to the encoded
//!   `from:<@Uxxxx>` mention using the authenticated user's id from local config.
//! - **Cross-origin routing.** The page runs on `app.slack.com` but the API
//!   lives on `<team>.slack.com`; the credentialed call only succeeds with
//!   Slack's edge-routing query params (`slack_route`, `_x_version_ts`, …),
//!   harvested from a live `/api/` call or rebuilt from local config.
//!
//! Request:  `POST https://<team>.slack.com/api/search.modules.messages?<routing>`
//!           `credentials: include`; body = `token` + `query` + paging.
//! Response: `{ ok, pagination:{ total_count, page_count, … },
//!             items:[ { channel:{id,name}, messages:[ {ts,user,username,text,permalink} ] } ] }`.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use apiwright::AdapterSession;
use serde::Deserialize;

use crate::{SearchQuery, SearchResult};

/// Page size for the API (its documented max behaviour is ~100/page).
const PER_PAGE: usize = 100;

/// Safety cap on messages collected per individual scope query. A date-ranged
/// reconciliation query is far below this; the cap only bounds an accidentally
/// broad query (e.g. dates with no other modifier). When hit, [`run_query`]
/// warns on stderr so truncation is never silent.
const MAX_PER_QUERY: usize = 1000;

/// Run every per-scope query, union the results, dedup by permalink, and sort
/// by timestamp ascending.
pub(crate) async fn run(session: &AdapterSession, query: &SearchQuery) -> Result<Vec<SearchResult>> {
    ensure_ready(session).await?;
    let mut seen = HashSet::new();
    let mut out: Vec<SearchResult> = Vec::new();
    for q in query.to_slack_queries() {
        for r in run_query(session, &q).await? {
            if seen.insert(r.permalink.clone()) {
                out.push(r);
            }
        }
    }
    out.sort_by(|a, b| a.ts.cmp(&b.ts));
    Ok(out)
}

/// Navigate to the workspace and wait for the client to boot, so the page holds
/// a fresh `xoxc` token + session cookie before the API call. If the persistent
/// profile isn't authenticated, Slack shows its login UI in the headed window
/// for the user to complete (magic-link / SSO); the subsequent API call then
/// reports `not_authenticated` until they do.
async fn ensure_ready(session: &AdapterSession) -> Result<()> {
    let workspace_url = session
        .place_url("workspace")
        .context("map missing 'workspace' place url — re-map the site")?;
    session.goto(&workspace_url).await?;
    // The top-nav search control is the "client booted" signal; its selector
    // lives in the map, so Slack-specific selectors stay out of the code.
    if let Some(boot_signal) = session.element_selector("search_results", "search_input") {
        let _ = session.wait_for(&boot_signal, Duration::from_secs(30)).await;
    }
    Ok(())
}

/// One scope query: drives the in-page API search (with pagination) and maps
/// the flattened rows into [`SearchResult`]s.
async fn run_query(session: &AdapterSession, query: &str) -> Result<Vec<SearchResult>> {
    let js = SEARCH_JS
        .replace("__QUERY__", &serde_json::to_string(query)?)
        .replace("__PER_PAGE__", &PER_PAGE.to_string())
        .replace("__MAX__", &MAX_PER_QUERY.to_string());
    let v = session
        .evaluate(&js)
        .await
        .context("calling Slack's search.modules.messages API")?;
    let resp: ApiResponse =
        serde_json::from_value(v).context("parsing the Slack search response")?;

    if let Some(err) = resp.error {
        if err == "not_authenticated" {
            bail!(
                "not signed in to Slack — complete login in the browser window, then re-run"
            );
        }
        if err == "no_user_id" {
            bail!(
                "query scopes to `@me` but the active workspace has no user_id in local \
                 config — reopen the workspace so the web client rehydrates, or remove the \
                 @me scope"
            );
        }
        bail!(
            "Slack's search endpoint returned error '{err}' for query '{query}' \
             (the session token may have expired, or the browser-automation-backed API changed)"
        );
    }
    if resp.results.len() >= MAX_PER_QUERY && resp.total > resp.results.len() {
        eprintln!(
            "slack-adapter: query '{query}' matched {} messages; returning the first {} \
             (narrow the date range or add a channel/text filter for the rest)",
            resp.total, resp.results.len(),
        );
    }

    Ok(resp
        .results
        .into_iter()
        .map(|r| SearchResult {
            ts: iso8601(&r.ts),
            channel: r.channel,
            author: r.author,
            text: r.text,
            permalink: r.permalink,
        })
        .collect())
}

/// The shape returned by [`SEARCH_JS`]: either `results` (+ `total` match count
/// for truncation reporting) or an `error` string.
#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    results: Vec<RawRow>,
    #[serde(default)]
    total: usize,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct RawRow {
    ts: String,
    channel: String,
    author: String,
    text: String,
    permalink: String,
}

/// The in-page search, evaluated against the authenticated Slack page. It reads
/// the active team's `xoxc` token + base URL from `localConfig_v2`, expands
/// `@me`, then pages through `search.modules.messages` and returns flattened
/// rows. `__QUERY__` (a JSON string), `__PER_PAGE__`, and `__MAX__` are injected
/// by [`run_query`].
const SEARCH_JS: &str = r#"
(async () => {
  const QUERY = __QUERY__;
  const PER_PAGE = __PER_PAGE__;
  const MAX = __MAX__;

  const cfg = JSON.parse(localStorage.localConfig_v2 || '{}');
  const teams = cfg.teams || {};
  const activeId = cfg.lastActiveTeamId || Object.keys(teams)[0];
  const t = teams[activeId] || {};
  const token = t.token;
  if (!token) return { error: 'not_authenticated' };
  const teamBase = (t.url || location.origin).replace(/\/$/, '');
  // The authenticated user id lives under `user_id` in current client builds
  // and `self_id` in older ones (see examples/api_discover.rs) — read both so
  // the @me guard below only fires when neither is present.
  const userId = t.user_id || t.self_id || '';

  // Edge-routing query string. Prefer harvesting it from a live /api/ call (the
  // most faithful, picks up the current build params); otherwise rebuild a
  // deterministic minimal set from local config that is known to be accepted.
  let qs = '';
  try {
    const apiUrls = performance.getEntriesByType('resource').map(e => e.name)
      .filter(n => /\/api\/[a-zA-Z]/.test(n));
    qs = new URL(apiUrls[apiUrls.length - 1] || '').search;
  } catch (e) {}
  if (!/[?&]slack_route=/.test(qs)) {
    const p = new URLSearchParams();
    p.set('slack_route', activeId);
    if (t.versionDataTs) p.set('_x_version_ts', String(t.versionDataTs));
    p.set('_x_frontend_build_type', 'current');
    p.set('_x_desktop_ia', '4');
    p.set('_x_gantry', 'true');
    qs = '?' + p.toString();
  }

  // `@me` is a UI-only token the API doesn't resolve — expand to the encoded
  // user mention so `from:`/`to:` scope to the authenticated user. If the query
  // asks for `@me` but local config carries no user_id, fail loudly: replaying
  // the bare token makes the API return zero rows, which reads as "no matches"
  // rather than the misconfiguration it is.
  const wantsMe = /(?:from|to):@me\b/.test(QUERY);
  if (wantsMe && !userId) return { error: 'no_user_id' };
  const query = QUERY.replace(/from:@me\b/g, 'from:<@' + userId + '>')
                     .replace(/to:@me\b/g, 'to:<@' + userId + '>');

  async function fetchPage(page) {
    const f = new URLSearchParams();
    f.set('token', token);
    f.set('module', 'messages');
    f.set('query', query);
    f.set('count', String(PER_PAGE));
    f.set('page', String(page));
    f.set('sort', 'timestamp');
    f.set('sort_dir', 'asc');
    f.set('highlight', '0');
    f.set('extracts', '0');
    f.set('extra_message_data', '1');
    f.set('no_user_profile', '1');
    const res = await fetch(teamBase + '/api/search.modules.messages' + qs, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: f.toString(),
      credentials: 'include',
      mode: 'cors',
    });
    return res.json();
  }

  const results = [];
  let total = 0;
  let page = 1, pageCount = 1;
  do {
    let j;
    try { j = await fetchPage(page); }
    catch (e) { return { error: 'fetch_failed: ' + String(e) }; }
    if (!j || !j.ok) return { error: (j && j.error) || 'api_error' };
    if (page === 1) total = (j.pagination && j.pagination.total_count) || 0;
    pageCount = (j.pagination && j.pagination.page_count) || 1;
    for (const item of (j.items || [])) {
      const ch = item.channel || {};
      for (const m of (item.messages || [])) {
        results.push({
          ts: m.ts || '',
          channel: ch.name || ch.id || '',
          author: m.username || m.user || '',
          text: m.text || '',
          permalink: m.permalink || '',
        });
        if (results.length >= MAX) return { results, total };
      }
    }
    page++;
  } while (page <= pageCount);
  return { results, total };
})()
"#;

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
