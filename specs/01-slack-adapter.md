# 01 — slack-adapter

Status: **implemented (v1)**. The adapter logs in via the embedded site map and
runs search through Slack's **own browser-automation-backed API** (`search.modules.messages`),
called from the authenticated page — see **§6.4**. The DOM-driving + scroll-collect
design (§6.3, §7.1–7.2) is **superseded** by that path but kept for context and as
a fallback if Slack changes the browser-automation-backed API.

Companions:
[`apiwright/specs/01-apiwright.md`](https://github.com/stencilwright/stencilwright/blob/main/specs/02-apiwright.md)
(the runtime) and
[`stencilwright/specs/01-stencil.md`](https://github.com/stencilwright/stencilwright/blob/main/specs/01-stencil.md)
(the mapping harness). Read both first; this spec assumes their vocabulary
(*place*, *signature*, *element*, *map*, *raw feature*, *surface*).

## 1. Goal & motivation

Provide a small, ergonomic, **local** API over Slack's web app:

- log in as the user (SSO / password / 2FA), in a real browser;
- run date-ranged searches with the modifiers that matter for reconciling
  billable work — `from:@me`, `to:@me`, `in:#channel`, free text;
- extract each result as a structured row: timestamp, channel, author, text,
  **permalink**;
- present results as Rust values, JSON, or CSV.

### Why drive the web app instead of the Slack API?

This adapter exists *because* the official API is the wrong tool for this job:

- `search.messages` (the only method that does `from:`/`to:`/date search) is a
  **legacy/deprecated** method.
- Its sanctioned replacement (the Real-time Search API,
  `assistant.search.context`) is restricted to **directory-published or internal
  apps**, requires **admin install**, and — for semantic search — a **paid Slack
  AI plan**.
- On a **client's** workspace (the common case for a contractor) you are a
  member or guest, not an admin, and app installs may need admin approval.

Driving the web client as yourself needs **no token, app, or admin approval**,
and a user-scoped search returns exactly what that user can already see. The
remaining trade-off is Slack's ToS posture on automating the client (§12) —
manageable for personal, your-own-access, low-volume use. (The original
DOM-fragility trade-off is largely retired: as implemented the adapter calls
Slack's *own* browser-automation-backed API from the authenticated page rather than
scraping the results DOM — §6.4.)

### Non-goals (v1)

- Posting, reacting, editing, or any write action. **Read-only search.**
- Thread reconstruction. We capture the matching message + its permalink; the
  user opens the thread to read context (§7.5).
- Bulk export of other people's content. Results are bounded to the user's own
  searches over their own access.
- Multi-workspace fan-out in one call. One adapter instance = one mapped
  workspace; the caller loops if they have several.

## 2. Architecture

```
stencilwright  ──maps (masked, once)──▶  maps/acme/{places,elements,mask}.toml
                                               │  (embedded via include_str!)
                                               ▼  (loaded, raw)
slack-adapter  ───────────────────────▶  apiwright::AdapterSession
   SearchQuery → Slack modifiers             │  goto workspace → boot (login if needed)
   parse JSON   ← search.modules.messages ◀──┘  evaluate(): call Slack's browser-automation-backed API
                  (in-page fetch, authed)       surface on login/captcha
```

- **`slack-adapter`** (this repo) holds the only Slack-specific logic: query →
  modifier rendering (§6.2) and the in-page API search (§6.4) — `from:@me`
  expansion, paging, and JSON→row mapping. The map travels **embedded** in the
  binary (`maps/acme/…`, `include_str!`), so the adapter is standalone.
- **`apiwright`** provides the runtime: the raw-DOM browser session, place
  recognition/navigation, extraction primitives, and the visibility/consent
  model. slack-adapter depends on `apiwright` alone (it re-exports the stencil
  crates).
- **`stencilwright`** is not a runtime dependency. It is the *tool you use to
  build and maintain the map* this adapter consumes (§3).

`site` throughout is the stencilwright map name for a workspace (e.g.
`acme`), so a user with several client Slacks keeps one map each.

## 3. Building & maintaining the adapter with stencilwright

This is the operational heart of the spec: **the adapter's selectors and
navigation are not hand-guessed — they are produced by a stencilwright mapping
session and stored as a map.** Maintenance = re-mapping when Slack's UI shifts.

### 3.1 One-time map creation

```sh
stencilwright init <site>                 # scaffold ~/.stencilwright/<site>/
stencilwright <site> value add slack_email    "secret://1password/<vault>/<item>/username"
stencilwright <site> value add slack_password "secret://1password/<vault>/<item>/password"
stencilwright <site> value add slack_totp     "secret://1password/<vault>/<item>/otp?"
```

Then drive the live site and register places (masked throughout — your
colleagues' and client's messages never enter the agent transcript):

```sh
stencilwright <site> page goto "https://app.slack.com/client/<TEAM_ID>"
stencilwright <site> place add login_email     # capture the auth wall
stencilwright <site> place add search_results  # after running a search by hand once
stencilwright <site> place <name> element add <selector> --as <name> [--reason …]
```

The mapping is **collaborative**: an LLM reads only the masked DOM + selectors
and proposes the place signatures and element selectors; the user solves
captchas, completes SSO/2FA, and clicks **Approve** in the native dialog for any
element whose text should be unmasked (for Slack, almost nothing needs
unmasking to *map* — structure is enough; the real text is read later by the
adapter via apiwright's raw path, which the user runs).

### 3.2 Places to map

| place | kind | signature anchors | purpose |
|---|---|---|---|
| `login_email` | interactive | the email/SSO form | start of auth |
| `login_password` | interactive | password field (same-URL, `visible_selector`) | password step |
| `login_totp` | interactive | 2FA/OTP field | 2FA step (auto-fill `slack_totp`, prompt submit) |
| `sso_redirect` | interactive | IdP host in `signature.url` | Google/Okta — surface + let the user finish |
| `captcha` | interactive (surface) | challenge container | human-solved |
| `workspace` | nav target | the message client shell, no auth wall | logged-in home |
| `search_results` | nav target + interactive | the results pane container | run + read searches |

Login is "just an interactive place" (per the stencil model): auto-fill what we
can from `values.toml`, submit, then **surface and halt** for anything human
(SSO bounce, push/SMS 2FA, captcha).

### 3.3 Elements to map at `search_results`

- the **search input** (to type the query) and its submit affordance;
- the **results scroll container** (for the scroll-collect loop, §7);
- per result row: **author**, **channel**, **timestamp/permalink link**,
  **message text**, and the row container itself;
- the **"no results"** marker and any **"end of results"** sentinel;
- (optional) the messages/files results **tab toggle** — we want Messages.

### 3.4 Maintenance loop

Slack reships its web UI often; selectors and signatures drift. Maintenance is
cheap *because* it's a re-map, not a code change:

1. The adapter's parse step fails a self-check (e.g. zero rows where the "no
   results" marker is absent, or a missing required element) and errors loudly
   with "map for `<site>` looks stale at place `search_results`".
2. Re-run a `stencilwright <site> place search_results goto`, read the masked
   dump, update the drifted selectors via `element add` / TOML edit.
3. No adapter rebuild needed unless the *shape* of the flow changed.

The adapter must therefore **fail fast and legibly** on drift rather than
silently returning empty/garbage (§7.4).

## 4. Runtime & visibility (consent)

slack-adapter inherits apiwright's model and sets sensible Slack defaults:

- **Headed by default.** `Slack::open(site)` opens a visible window.
- **Off-screen opt-in.** `Slack::open_offscreen(site)` runs off-screen with the
  default [`SurfacePolicy`] (surface on login, captcha, unrecognized, consent;
  `Requested` always surfaces). Right for a weekly batch the user kicks off and
  glances at only if it needs them.
- **Never headless.** Slack *will* sometimes throw a login refresh or a
  challenge; the window must always be surfaceable so the user can intervene.

The principle, concretely: a `slack-search` run that needs a 2FA code snaps the
real Slack window to the foreground, the user types the code, and the run
continues. Nothing happens in the user's name that the user can't see happen.

## 5. Login

`Slack::open` ensures an authenticated session:

1. `AdapterSession::open(RuntimeConfig::new(site))` attaches the raw daemon and
   loads the map. The persistent Chrome profile under
   `~/.stencilwright/<site>/profile/` usually means the user is *already* logged
   in — most runs skip auth entirely.
2. If recognition lands on an auth place, the runner auto-fills
   `slack_email`/`slack_password` from `values.toml`, submits, and for
   `login_totp` resolves the trailing-`?` OTP just in time.
3. For SSO bounces, push/SMS 2FA, or captcha, it **surfaces and halts** until
   recognition leaves the interactive place (the user finishes in the window).

No credential ever passes through slack-adapter; secrets are resolved
daemon-side by the stencil secret provider.

## 6. Search

### 6.1 Query model

[`SearchQuery`] is a small builder (inclusive dates):

```rust
SearchQuery::new()
    .mine()                     // from:@me
    .mentions()                 // to:@me
    .in_channel("proj-acme")
    .text("invoice")
    .between(from, to)          // inclusive
```

### 6.2 Rendering to Slack modifiers

`to_slack_queries()` builds one search string **per scope** (so `from:@me` and
`to:@me` are separate searches, unioned in §7.3):

- `from:@me` / `to:@me` from scopes;
- `in:#<channel>` when set;
- the free text appended verbatim;
- **dates: inclusive → exclusive.** Slack's `after:`/`before:` are *exclusive*
  of the named day, so an inclusive window `[from, to]` renders as
  `after:<from − 1 day> before:<to + 1 day>`. (Example: `2026-05-25..=2026-05-31`
  → `after:2026-05-24 before:2026-06-01`.) This off-by-one is the single most
  common date bug; it lives in one tested function.

Example rendered query: `from:@me in:#proj-acme after:2026-05-24 before:2026-06-01 invoice`

### 6.3 Driving the search (SUPERSEDED — see §6.4)

> **Superseded by §6.4.** This UI-driving design worked but was fragile (typeahead
> timing, virtualized results, two-step Enter). The shipped adapter calls Slack's
> browser-automation-backed API instead and does **not** drive the search UI. Kept for context.

The `app.slack.com` client is an SPA; the search query is **not reliably URL
-addressable**, and the date-picker UI is fiddly. So the adapter:

1. recognizes/navigates to `search_results` (or focuses search from
   `workspace`);
2. **types the rendered query string into the mapped search input** (modifiers
   and `after:`/`before:` included — no calendar clicking) and submits;
3. ensures the **Messages** results tab is active;
4. hands off to the extraction loop (§7).

### 6.4 Direct API path (as implemented)

Slack's web client backs message search with a **browser-automation-backed JSON endpoint**,
`search.modules.messages`. The adapter calls it directly from the authenticated
page (`apiwright`'s `evaluate` → in-page `fetch`), reusing the browser's session
`d` cookie and the page's own `xoxc` client token. No Slack app, token grant, or
admin approval — the request is indistinguishable from the web client's own, and
it returns **full text, all pages, correct `from:@me`**, with none of the DOM
fragility (virtualization, lazy headers, typeahead). Discovered by analysis;
reproduce with `examples/api_discover.rs`.

**Flow** (`src/search.rs`):

1. `goto` the mapped `workspace` URL and wait for the client to boot (so the
   page holds a fresh token + cookie). If unauthenticated, Slack shows login in
   the headed window; the call then reports `not_authenticated` until completed.
2. For each per-scope query string from §6.2, run the in-page search and map the
   rows; union, dedup by permalink, sort by `ts` ascending (§7.3).

**Request.** `POST https://<team>.slack.com/api/search.modules.messages?<routing>`

- **Cross-origin.** The page is on `app.slack.com` but the API is on
  `<team>.slack.com`; the **credentialed** (`credentials: include`) call only
  succeeds with Slack's edge-routing query string. The adapter harvests it from a
  live `/api/` resource (via the Performance API), else rebuilds a minimal
  accepted set from local config: `slack_route=<team>`, `_x_version_ts=<build>`
  (from `localConfig_v2.teams[].versionDataTs`), `_x_frontend_build_type=current`,
  `_x_desktop_ia=4`, `_x_gantry=true`. (`slack_route` **alone** is rejected.)
- **Body** (`application/x-www-form-urlencoded`): `token` (the team's `xoxc-…`
  from `localConfig_v2`), `module=messages`, `query`, `count=100`, `page`,
  `sort=timestamp`, `sort_dir=asc`, `extra_message_data=1`, `no_user_profile=1`.
- **`@me` expansion.** `from:@me`/`to:@me` are **UI-only** tokens the API does
  *not* resolve (they return zero results). The adapter rewrites them to the
  encoded mention `from:<@Uxxxx>` using `localConfig_v2.teams[].user_id`.

**Response.** `{ ok, pagination:{ total_count, page_count, … }, items:[ … ] }`,
where each `item` groups by channel: `item.channel = {id, name, …}` and the
messages are nested in **`item.messages[]`** (each `{ ts, user, username, text,
permalink, … }`). The adapter flattens `items × messages` into rows. Paging runs
to `page_count`, capped per scope (a stderr notice fires if the cap truncates).
Note: bot/attachment-only messages can have empty `text` (the permalink still
resolves); DM `channel.name` is the other party's user id, not a `#name`.

## 7. Result extraction

> **§7.1–7.2 are SUPERSEDED by the API path (§6.4).** The JSON API returns full
> results with full text and server-side paging, so there is no virtualized DOM
> to scroll-collect. These remain as the documented fallback if the browser-automation-backed API
> changes. §7.3 (merge scopes) and §7.5 (fields) still apply as written.

### 7.1 The virtualization problem (DOM fallback only)

Slack's results pane is a **virtualized, lazily-loaded** list: only a window of
rows is in the DOM at once; scrolling materializes more and recycles old nodes.
A naive "read all rows" gets only the first screenful. (Confirmed live: the
block-kit message body `[data-qa=message-text]` is absent from the raw DOM until
scrolled — the original reason DOM extraction returned empty `text`.)

### 7.2 Scroll-collect loop (DOM fallback only — via apiwright's list helper)

```
seen = {}                       # dedup set, keyed by permalink (stable per msg)
loop:
    rows = extract rows currently in the results container
    for row in rows: if key(row) not in seen: seen.add; emit
    if "no results" marker present and seen empty: return []
    if end-of-results sentinel present: return collected
    scroll the results container by ~one viewport
    if no new keys appeared after N consecutive scrolls: return collected   # safety stop
```

- **Dedup key = permalink** (the message `ts`+channel encoded in the timestamp
  link). Stable across re-virtualization; survives recycled DOM nodes.
- **Termination**: explicit end sentinel preferred; the "N idle scrolls" rule is
  the backstop. The backstop's N and scroll delay are config with sane defaults.

### 7.3 Merging scopes

`from:@me` and `to:@me` run as separate searches; their result lists are
concatenated and **deduped by permalink** (a message you sent in a thread you're
also in shouldn't appear twice). Final order: by timestamp ascending.

### 7.4 Fail-fast on drift

Silent emptiness reads as "nothing billable that week" — a dangerous false
negative for invoicing — so the adapter errors loudly instead. Under the API
path (§6.4) the drift signals are explicit: a non-`ok` response surfaces Slack's
`error` (e.g. `not_authenticated` → "complete login"; any other → "token expired
or the browser-automation-backed API changed"), and a JSON shape that no longer deserializes fails
with a parse error naming the response. (DOM fallback: zero rows with no
"no results" marker, or a missing required element, errors naming the suspect
place/selectors per §3.4.)

### 7.5 Fields & threads

Each [`SearchResult`]: `ts` (ISO 8601 from the Slack `ts`), `channel`, `author`,
`text` (raw), `permalink`. Thread context is intentionally out of scope (§1);
the permalink is the click-through. A future `--with-thread` could expand the
thread via a mapped `thread` place.

## 8. Public API (Rust)

```rust
pub enum Scope { FromMe, ToMe }              // from:@me / to:@me

pub struct SearchQuery { /* text, scopes, channel, from, to (inclusive) */ }
impl SearchQuery {
    pub fn new() -> Self;
    pub fn mine(self) -> Self;               // + from:@me
    pub fn mentions(self) -> Self;           // + to:@me
    pub fn in_channel(self, c: impl Into<String>) -> Self;
    pub fn text(self, t: impl Into<String>) -> Self;
    pub fn between(self, from: NaiveDate, to: NaiveDate) -> Self;  // inclusive
    pub fn to_slack_queries(&self) -> Vec<String>;                // exclusive bounds
}

#[derive(Serialize)]
pub struct SearchResult { pub ts: String, pub channel: String,
                          pub author: String, pub text: String, pub permalink: String }

pub struct Slack { /* … */ }
impl Slack {
    pub async fn open(site: &str) -> anyhow::Result<Self>;            // headed
    pub async fn open_offscreen(site: &str) -> anyhow::Result<Self>;  // surfaceable
    pub async fn search(&self, q: &SearchQuery) -> anyhow::Result<Vec<SearchResult>>;
}
```

Design intent: the common case is two lines (`open`, `search`); everything hard
(login, virtualization, dedup, surfacing) is hidden.

## 9. CLI (`slack-search`)

```text
slack-search --site <name> --from <YYYY-MM-DD> --to <YYYY-MM-DD>
             [--mine] [--mentions] [--channel <name>] [--text <terms>]
             [--offscreen] [--format json|csv]
```

`--from/--to` inclusive; `--mine`/`--mentions` set scopes (at least one
required — default to both if neither given); `--offscreen` for batch;
`--format` selects JSON (default) or CSV. CSV columns:
`ts,channel,author,permalink,text`.

The intended weekly flow: run `slack-search` for the billing week, eyeball the
rows against the Toggl entries, and bill anything the time tracker missed.

## 10. Mapping artifact sketch (`places.toml`)

Illustrative; exact selectors come from a real mapping session (§3).

```toml
target = "slack"
description = "Slack web app: login, workspace, search_results"

[[place]]
name = "login_password"
interactive = true
submit.click = "button[data-qa='login-submit']"
signature.url = "https://*.slack.com/**"
signature.visible_selector = "input[data-qa='login_password']"
  [[place.element]]
  name = "password_field"
  selector = "input[data-qa='login_password']"
  auto_fill = "{slack_password}"

[[place]]
name = "search_results"
url = "https://app.slack.com/client/{team_id}"
signature.selector = "[data-qa='search_results'], .c-search__results"
signature.absent_selector = "input[data-qa='login_password']"
  [[place.element]]
  name = "search_input"
  selector = "[data-qa='search_input'], input.c-search_autocomplete__input"
  [[place.element]]
  name = "results_container"
  selector = "[data-qa='search_results']"
  [[place.element]]
  name = "result_row"
  selector = "[data-qa='search_message']"
  [[place.element]]
  name = "result_permalink"
  selector = "[data-qa='message_timestamp_label'] a, a.c-timestamp"
  [[place.element]]
  name = "result_author"
  selector = "[data-qa='message_sender_name']"
  [[place.element]]
  name = "result_text"
  selector = "[data-qa='message_content'] .c-message__body"
  [[place.element]]
  name = "no_results"
  selector = "[data-qa='search_no_results']"
```

## 11. Milestone — first green run

All must hold against a real workspace map:

1. `cargo build` produces `slack-search`; `lib` + `bin` compile.
2. From a fresh profile, `Slack::open(site)` reaches `workspace` after the user
   completes SSO/2FA in the **surfaced** window; a second run reuses the profile
   and skips auth.
3. `to_slack_queries()` unit tests pin the **exclusive-boundary** date math and
   modifier composition (no live browser needed).
4. A search with a known answer (e.g. `from:@me` over a day you posted) returns
   the right rows with correct `permalink`s, scroll-collected past the first
   viewport, deduped.
5. A search with no matches returns `[]` **only** when the `no_results` marker is
   present; otherwise it errors as stale (§7.4).
6. `--offscreen` runs without presenting a window until a login/captcha forces a
   surface.

## 12. Open questions / Slack-specific gotchas

1. **ToS posture.** Automating the web client is against the letter of Slack's
   ToS. Mitigations: your own access only, your own/authorized workspaces,
   low request volume, no redistribution. Document; don't paper over.
2. **Search indexing lag.** Very recent messages may not be searchable
   immediately; for "this week" billing this is usually fine, but note it.
3. **`to:@me` semantics.** Slack's `to:@me` covers mentions + DMs to you; verify
   it captures the cases the user means by "mentions of me," and consider adding
   a raw `@displayname` text search as a complement.
4. **Free vs paid retention.** On free workspaces, history/search is limited to a
   retention window; results outside it simply won't exist.
5. **Team-id discovery.** Map `{team_id}` per workspace (a `value`), or detect
   from the post-login URL.
6. **Enterprise Grid org search.** Cross-workspace search differs; v1 targets a
   single workspace.
7. **Rate / pacing.** Human-like scroll pacing both helps virtualization settle
   and keeps behavior unsurprising; expose the delay as config.
8. **Native "Open Slack.app?" dialog on `/ssb/redirect`.** After magic-link
   login, Slack bounces through `<workspace>.slack.com/ssb/redirect`, which
   invokes the `slack://` protocol and pops Chrome's **native** "Open Slack.app?"
   dialog. That dialog is browser chrome, not page DOM: apiwright's `page` ops
   can't click it, and Playwright's `page.on('dialog')` only catches JS dialogs
   (alert/confirm/prompt/beforeunload), not the external-protocol prompt.
   **Countermeasure (in use):** drive the web client by navigating directly to
   `https://app.slack.com/client[/<TEAM_ID>]` (a plain https URL) instead of
   following any `/ssb/*` deep link — this sidesteps the protocol handler
   entirely, so the mapped `workspace`/`search_results` places use web-client
   URLs. Belt-and-suspenders to investigate: a persistent-profile Chrome
   pref / launch arg to auto-deny external-protocol launches, so a stray `/ssb/`
   navigation can't strand an unattended (off-screen) run on an unclickable
   dialog. Observed live while mapping `acme` (2026-06-14).
9. **Driving search = type-into-button + double-Enter, not URL.** *(Superseded
   by §6.4 — the adapter no longer drives search via the UI. Retained as the
   DOM-fallback mechanic.)* In the IA4
   client, `?q=…` on `/search` does NOT execute a query (it loads an empty
   search view). The working drive: type into `[data-qa="top_nav_search"]`,
   which opens a typeahead over the real input (`[data-qa="texty_input"]`); the
   query then needs **two** `Enter` presses — the first commits the typeahead
   entry, the second executes the full-text search. Results render as
   `[data-qa="search_result"]` rows. The adapter drives this via apiwright's
   `type` + page-level `key("Enter")` (added to `stencil-browser` for exactly
   this — `page press`/`type`/`key` + `click --force`). The **permalink** lives
   in each row's `a.c-timestamp` **`href`** (`/archives/<chan>/p<ts>`), not its
   text, so the extractor must read the attribute, not `extract_text`. Masking
   note: Slack puts display names in `data-stringify-text` / `aria-label`
   attributes, which the masker does not redact — fine for low-stakes Slack, but
   a known attribute-leak class to harden before any financial-site mapping.
   Observed live mapping `acme` (2026-06-14).
10. **Private search API — `@me` and cross-origin routing.** *(The path in use;
    §6.4 has the full request.)* Two non-obvious live findings: (a) `from:@me`/
    `to:@me` are **UI-only** — the API returns **zero** results for them, so they
    must be expanded to `from:<@Uxxxx>` using the user id from
    `localConfig_v2`; (b) the credentialed cross-origin call from
    `app.slack.com` to `<team>.slack.com` is **rejected without Slack's edge
    routing params** — `slack_route` alone is insufficient; the minimal accepted
    set adds `_x_version_ts`/`_x_frontend_build_type`/`_x_desktop_ia`/`_x_gantry`.
    Validated live on `acme` (2026-06-14): `from:@me` → 253 of the user's
    own messages with full text (vs. the DOM path's bots-and-blanks).
11. **Daemon `stop` doesn't reap its Chrome (stencilwright).** `stencilwright
    session stop <site>` SIGTERMs the daemon, but the Playwright-launched Chrome
    holding the `~/.stencilwright/<site>/profile` can survive, keeping the
    profile `SingletonLock`. The next daemon then launches into the locked
    profile, Chrome reports *"Opening in existing browser session"*, and attach
    fails with `TargetClosedError` / "daemon failed to come up". Workaround:
    `pkill -f "stencilwright/<site>/profile"` then remove `profile/Singleton*`.
    Better: don't `stop` between adapter runs (the daemon is meant to persist;
    the deterministic routing-QS fallback in §6.4 removes the only reason we
    were forcing fresh boots). A stencilwright fix — have the daemon reap its
    browser on shutdown — is filed in that repo's notes. Observed 2026-06-14.
