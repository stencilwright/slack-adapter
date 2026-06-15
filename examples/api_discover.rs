//! Discovery tool for the browser-automation search path (no-PII workspace only).
//!
//! Navigates to the workspace, then — entirely in-page — reads the active
//! team's `xoxc` token from `localStorage.localConfig_v2` and replays a real
//! `search.modules.messages` call, reporting the endpoint, HTTP status, and the
//! *shape* of the response (top keys + one sample row). The token never leaves
//! the page (only a 5-char prefix is returned, to confirm it's `xoxc-`).
//!
//! ```text
//! cargo run --example api_discover
//! ```
//!
//! Goal: validate that the adapter can query Slack's own search endpoint
//! directly instead of reading the virtualized DOM. If `api.ok == true`, the
//! refactor in `src/search.rs` is unblocked.

use std::time::Duration;

use apiwright::{AdapterSession, RuntimeConfig};

const WORKSPACE: &str = "https://app.slack.com/client/T0000000000";

/// Runs in the page. Returns a redacted diagnostic bundle (no token).
const JS: &str = r#"
(async () => {
  const out = { href: location.href, origin: location.origin };

  // --- token + api base for the active team (no exfiltration) ---------------
  let token = null, apiBase = null, activeId = null;
  try {
    const cfg = JSON.parse(localStorage.localConfig_v2 || '{}');
    const teams = cfg.teams || {};
    activeId = cfg.lastActiveTeamId || Object.keys(teams)[0];
    const t = teams[activeId] || {};
    token = t.token || null;
    apiBase = t.url || null;
    out.team = { id: activeId, name: t.name, url: t.url,
                 hasToken: !!token, tokenPrefix: (token || '').slice(0, 5) };
  } catch (e) { out.cfgErr = String(e); }
  if (!token) return out;

  const teamBase = (apiBase || location.origin).replace(/\/$/, '');

  // Harvest the routing query string (?_x_id=…&slack_route=…) from any of
  // Slack's own /api/ calls — the `_x_*` edge-routing params are global, and
  // boot fires many, so this is far more reliable than waiting on search.*.
  const apiUrls = performance.getEntriesByType('resource').map(e => e.name)
    .filter(n => /\/api\/[a-zA-Z]/.test(n));
  let fullQS = '';
  try { fullQS = new URL(apiUrls[apiUrls.length - 1] || '').search; } catch (e) {}
  out.sawApiCalls = apiUrls.length;
  out.sampleApi = apiUrls.length ? apiUrls[apiUrls.length - 1].split('?')[0] : null;
  // Expose the routing-param vocabulary (values are non-secret edge routing).
  try {
    const o = {};
    for (const [k, v] of new URLSearchParams(fullQS)) o[k] = (k === 'token') ? '<redacted>' : v;
    out.routeParams = o;
  } catch (e) {}

  // Self identity — `from:@me` is a UI-only token; the API may need it expanded
  // to from:<@Uxxxx>. Find where the authenticated user id lives.
  try {
    const cfg = JSON.parse(localStorage.localConfig_v2 || '{}');
    const t = (cfg.teams || {})[activeId] || {};
    out.self = {
      user_id: t.user_id || t.self_id || null,
      teamKeys: Object.keys(t).filter(k => k !== 'token'),
    };
  } catch (e) {}

  function body(query, count) {
    const f = new URLSearchParams();
    f.set('token', token);
    f.set('module', 'messages');
    f.set('query', query);
    f.set('count', String(count)); f.set('page', '1');
    f.set('sort', 'timestamp'); f.set('sort_dir', 'desc');
    f.set('highlight', '0'); f.set('extracts', '0');
    f.set('extra_message_data', '1'); f.set('no_user_profile', '1');
    return f.toString();
  }
  // Each item groups by channel; the real message(s) are in item.messages[].
  function itemSample(item) {
    const msgs = item.messages || [];
    const m = msgs.find(x => x && x.is_match) || msgs[0] || {};
    return {
      channel: item.channel && { id: item.channel.id, name: item.channel.name },
      messageCount: msgs.length,
      msgKeys: Object.keys(m),
      ts: m.ts, user: m.user, username: m.username, type: m.type, subtype: m.subtype,
      is_match: m.is_match, permalink: m.permalink, text: (m.text || '').slice(0, 160),
    };
  }
  async function call(query, count) {
    try {
      const res = await fetch(teamBase + '/api/search.modules.messages' + fullQS, {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: body(query, count), credentials: 'include', mode: 'cors',
      });
      const j = await res.json();
      const items = j.items || [];
      return {
        status: res.status, ok: j.ok, error: j.error,
        total: j.pagination && j.pagination.total_count,
        itemCount: items.length,
        samples: items.slice(0, 3).map(itemSample),
      };
    } catch (e) { return { fetchErr: String(e) }; }
  }

  // Confirm the `@me` → `<@Uxxxx>` expansion resolves to the real user, and
  // that human messages carry non-empty `text`.
  const me = (out.self && out.self.user_id) || '';
  out.fromMeId = await call('from:<@' + me + '>', 5);
  out.toMeId   = await call('to:<@' + me + '>', 5);
  return out;
})()
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if apiwright::run_if_daemon().await? {
        return Ok(());
    }
    let session = AdapterSession::open(RuntimeConfig::new("acme")).await?;
    session.goto(WORKSPACE).await?;
    // Let the SPA boot so localConfig_v2 (and the token) are populated.
    let _ = session
        .wait_for("[data-qa='top_nav_search']", Duration::from_secs(30))
        .await;
    // Open the search box so Slack fires `search.autocomplete` — that populates
    // the Performance resource we harvest the edge routing params from.
    let _ = session.type_text("[data-qa='top_nav_search']", " ").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let v = session.evaluate(JS).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}
