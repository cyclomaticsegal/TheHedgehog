# ADR 0016: Splash Screen, Revert-to-Original Architecture, and 51Folds PATCH/PUT Drift

**Date:** 2026-04-11
**Status:** **In limbo — partially accepted.** Splash, mascot, version, and the hybrid DB-baseline + in-memory state architecture are all landed and working. **Revert-to-original is currently shipping a known-imperfect implementation** pending an upstream fix in 51Folds (see "Known Issue" and the Handoff Note at the bottom of this document). We will return to this ADR after the 51Folds team responds.
**Extends:** ADR 0013 (SDK integration), ADR 0015 (dark theme + UI polish)

## Context

This ADR captures the second pass of work on the 51Folds model explorer and the introduction of the app's branding/startup affordances. It also documents a significant **server-side limitation in the 51Folds public API** that we discovered via direct curl reproduction, which forced several client-side architectural choices.

The work touched five broad areas:

1. **Branding and startup experience** — first impression of the app when it launches.
2. **Version change** — the app moves from `0.2.0` to `0.1.0-preview` to reflect that it is not yet generally available.
3. **Help and README integration** — mascot, tagline, and "Why the hedgehog?" Isaiah Berlin framing.
4. **51Folds model state management** — a significant architectural rework after a session in which we repeatedly broke and then repaired model `Je`'s local data, culminating in the "immutable DB baseline + in-memory live state" hybrid.
5. **Revert-to-original** — attempted via PATCH, then PUT, both of which drift; discovery of a private platform endpoint that works correctly; decision to ship a known-imperfect revert pending upstream fix.

## Decisions

### 1. Startup splash screen

**Added** a modal startup splash rendered in egui's own frame rather than via a separate viewport, with the following characteristics:

- **Classic centred card** (`540×640` default, growing to `540×780` when the loading-activity feed is visible), rather than a full-screen takeover. Matches the Microsoft Office / Blender / Figma convention.
- **Hedgehog mascot** at the top of the card, sized to `42%` of the card height (clamped to `[220, 360]` px), aspect ratio preserved.
- **Title, tagline, and version chip** stacked below the mascot. Title at 32 px bold white. Tagline: *"Causal, probabilistic modelling of capital-markets regimes"*. Version chip renders `PREVIEW 0.1` in an accent-blue rounded pill.
- **Isaiah Berlin epigraph**: the Archilochus quote "*The fox knows many things, but the hedgehog knows one big thing.*" with the attribution "Archilochus, via Isaiah Berlin". Included as a deliberate editorial choice — see "The Hedgehog and the Fox" framing in the README and help.
- **Fade timeline**: 260 ms fade-in → 5600 ms hold → 420 ms fade-out → dismissed. Plus 2 additional seconds of hold if the app is still doing its startup auto-refresh (see next point).
- **Dynamic loading feed**: when the app kicks off its auto-refresh at startup, the last 5 entries from the activity log are rendered inside the splash card as a compressed two-column table (`Instrument · status`). The splash does not dismiss until the refresh completes. This addresses Nielsen heuristic #1 (visibility of system status) on first launch so the user can see what's happening before the dashboard appears.
- **User skip**: any click or key press dismisses the splash early, unless loading is still in progress (in which case input is ignored so the user can't race the startup fetch).

### 2. Mascot asset handling

`artwork/hedgehog-mascot-transparent.png` ships in the repository but has the editor's **grey-and-white transparency checker pattern baked in as opaque pixels**. (That is, the PNG is nominally RGBA but the "transparent" cells are rendered as solid colours. We can't recut the source asset.)

We strip the checker at decode time with a **two-pass algorithm**:

1. **Flood-fill from the image border** — any pixel that is both light (`avg(r,g,b) ≥ 170`) and near-neutral (`max - min < 30`) and reachable from any edge pixel via 4-connected neighbours is set to `alpha = 0`. The mascot's dark navy outline is the natural stopping boundary.
2. **Edge feather pass** — any still-opaque pixel adjacent to a newly-transparent pixel has its alpha reduced if it's lightish (the anti-aliased band at the outline). This eliminates a visible halo where the outline used to blend with the checker.

This runs once on the first frame that loads the mascot texture (~10–20 ms on the 474×600 source). The texture is then cached on `DashboardApp::mascot_texture` and reused by the splash and Help window headers.

### 3. Version: `0.2.0` → `0.1.0-preview`

`Cargo.toml`, `main.rs::USER_AGENT`, and `help.rs` footer all updated. The app is not generally available; the preview version label reflects that and appears in both the splash card and the Help window header.

### 4. README and Help header redesign

**README** now opens with the mascot (centred, 260 px wide, HTML `<p align="center">`), the Isaiah Berlin epigraph as a blockquote, and a "Why the hedgehog?" paragraph that frames the app's "one big thing" as **causal, probabilistic modelling of capital-markets regimes**, marrying generative AI with 51Folds Bayesian causal networks and real-time market data.

**Help window** gains an Apple-iOS-style info header: mascot at 96 px on the left, title / tagline / version pill stacked on the right, a thin separator, then the markdown body. The markdown itself opens with the same "Why the hedgehog?" framing as the README.

### 5. 51Folds model state management — hybrid architecture

This is the most consequential architectural decision in this ADR. The previous design (see ADR 0013 and the subsequent patches) had a cycle of bugs around what the DB stored, what the in-memory state held, and when/whether the DB was updated on re-eval. We ended up with this clean separation:

| Concern | Where it lives | Write rules |
|---|---|---|
| **Original baseline** (what the model looked like when it was first built) | `folds_models.response_json` column, as a JSON blob of the full `ModelResponse` | Written **once** by `create_and_poll` when the initial build completes. **Never overwritten** by any subsequent operation. This is our archive of the pristine original — the server does not keep a separate copy, so we have to. |
| **Live model view** (what the user sees in the pills and the Outcome tab right now) | `DashboardApp::folds_task::model` (in-memory, `Option<Box<ModelResponse>>`) | Replaced by `poll_main` on every successful re-eval, by `poll_refresh` on every successful manual refresh, and by `load_from_json` when the user opens the model from the Reports list. |
| **Pending driver edits** | `DashboardApp::folds_task::draft_drivers` | Edited via pill clicks in the Drivers tab; sent as a PATCH payload when the user clicks Re-evaluate. |
| **Server's current state** | 51Folds server — the only canonical copy of "what is the live model *right now*" | Mutated by every successful PATCH/PUT we send. There is no way to roll back except by sending another PATCH/PUT. |

**Consequences**:

- Loading a model from the Reports list reads the DB baseline. **The user always sees the pristine original on first load**, regardless of what the server currently has. This matches the user's mental model established in earlier (pre-ADR-0013) versions of the app.
- Re-evaluating does not touch the DB. The in-memory `model` gets updated via `patch_drivers`, but `response_json` stays frozen at the original.
- Refresh from server **does** touch the DB — it's the explicit recovery path for rows that have been corrupted by older buggy code paths. Clicking Refresh calls `GET /api/v1/models/{id}` and overwrites `response_json` with the returned payload. Post-refresh, the DB reflects whatever the server had at that moment.
- Revert-to-original reads the DB baseline, extracts its 15 driver states, and pushes them back to the server via PUT. **This is where the server-side drift problem bites** — see "Known Issue" below.

### 6. Async re-inference polling (PATCH race fix)

The 51Folds PATCH endpoint's response body is a **minimal acknowledgment** of the form `{"data":{"modelId":"Je","status":"Running"}}` — not a full `ModelResponse`. Re-inference happens asynchronously in the background after the PATCH returns.

Previously, our `patch_drivers` fired PATCH and then immediately fired GET. This is a race: if the GET won, it returned stale outcomes from **before** the re-inference finished, and the user's Outcome tab silently showed unchanged probabilities. Symptoms: "I clicked Re-evaluate but nothing changed."

**Fix** (`src/folds.rs::patch_drivers`): after the PATCH acknowledgment, call `client.models().wait_until_complete(&model_id, Some(PollConfig { interval: 2s, timeout: 60s }))`. This polls `GET /models/{id}` every 2 seconds until `status` flips from `"Running"` back to `"Successed"`, then returns the final re-inferred model. Same mechanism `create_and_poll` uses for initial builds, just with a tighter interval and shorter ceiling since post-PATCH re-inference takes seconds rather than the 25–30 minutes of an Advanced-tier initial build.

Also applied to `src/folds.rs::put_drivers` (the Revert path).

### 7. Refresh Model button — in both sidebar and central panel

Originally placed only in the right-hand AI panel summary. Moved to **also** appear inline on the right of the `Created … · Last updated …` row at the top of the central 51Folds model view, so it's visible and discoverable from any sub-view (Outcome, Drivers, Driver detail, Driver section). Label flips to "Refreshing…" while in flight; shows a spinner; red error line below on failure.

### 8. Session history feature — built then removed

A full client-side "session history" feature was built: a `SessionSnapshot` struct capturing (timestamp, trigger, drivers, outcomes); a `session_history: Vec<SessionSnapshot>` field on `FoldsTask` populated on initial load, successful re-evaluation, and refresh; a "History" tab in the 51Folds sub-toolbar; plain-English diff rendering between adjacent snapshots; an "Apply this snapshot" action with a confirmation modal showing the diff; server-revisions integration that was later ripped out when we confirmed the server's `/revisions` endpoint was not producing useful data for our workflow.

**Decision: removed.** After live-testing the history feature, the user concluded it was not carrying its weight. What the user actually wanted was:
- The original preserved so Revert always works
- The `(Previously: X% ↑/↓)` delta annotation already on the Outcome tab after each re-eval (driven by `previous_outcomes`, captured one-deep)
- No session history tab

The History tab, the `SessionSnapshot` struct, `push_session_snapshot`, `render_central_history_tab`, `start_folds_apply_snapshot`, the apply-snapshot confirmation dialog, and `snapshot_diff_summary` were all deleted. A simpler `diff_model_states(from, to) -> Vec<String>` helper remains, used only by the revert-to-original confirmation modal to show the user what will change when they click Apply.

### 9. Revert-to-original button

Always-enabled (whenever a model is loaded and no re-eval / refresh is in flight), distinct from the existing Reset button which only undoes unsaved pill edits. Click path:

1. Opens a confirmation modal that computes `diff_model_states(current_in_memory, db_baseline)` and renders the bulleted list of driver/outcome changes.
2. On Apply, `start_folds_revert_to_original()` reads the baseline from the DB, extracts all 15 driver states, and sends them via a **PUT** (not PATCH) through a new wrapper `folds::put_drivers` that calls `client.models().update_drivers()` followed by `wait_until_complete`.
3. PUT was chosen deliberately over PATCH because the API Kit documentation describes PUT as "replaces all driver states" (atomic) versus PATCH as "partially updates driver states" (merge). We wanted atomic replace semantics for revert.

**This revert is known to be imperfect. See Known Issue below.**

### 10. Various smaller fixes bundled into this work

- Fixed a BLOB-vs-TEXT column type bug: an earlier manual repair script wrote `response_json` via sqlite `readfile()`, which stores the file as a BLOB; rusqlite panics on `row.get::<_, String>()` against a BLOB column, and the error was silently swallowed by an outer `if let Ok(Some(json)) = …` pattern, causing model loads to fail invisibly. Repair script now uses `CAST(readfile(...) AS TEXT)`.
- Em-dash sweep on hardcoded UI labels (egui's default font renders U+2014 as tofu in several places).
- Stub detection in `load_from_json` — if a stored model has empty `current.drivers[]` or empty `current.outcomes[]`, leave `model = None` but keep `model_id` so the UI can render a recovery screen with a "Reload from 51Folds" button rather than silently failing.
- Recovery screen in the central panel empty state for corrupted DB rows.
- `start_folds_reevaluate` logs a one-line reeval trace to stderr (`[folds] reeval: model_id=… changed_drivers=…`) rather than the earlier 15-line per-driver dump.

## Known Issue: 51Folds PATCH and PUT drift

**This is a significant server-side limitation that we should not own as a client-side bug. It should be reported to the 51Folds API team.**

### The symptom

Reverting a model to its original driver states via the public API produces outcome probabilities that **do not match the pristine build outcomes**, even though the driver states after revert are bit-identical to the original driver states.

### Reproduction (via curl against production)

Model `Je`, pristine driver states known and recorded:

```
BDAC=Medium  CBGP=High  COPT=High  FLMP=High  FRPS=High  GEIOT=High
GRP=Medium   IE=High    MLC=High   PGD=Medium RIR=High   SPD=High
TRL=High     UDIS=Medium VMV=Medium
```

Pristine outcomes from the first successful build:

```
#1 Stays below $4800:   54.8800%
#2 Breaks above $4800:  12.2900%
#3 Drops below $4300:   32.8300%
```

**Step 1** — PATCH a single driver:

```bash
curl -X PATCH https://api.51folds.ai/api/v1/models/Je/drivers \
  -H "Authorization: Bearer at_sk_…" \
  -H "Content-Type: application/json" \
  -d '{"drivers":[{"code":"VMV","state":"Low"}]}'
```

Response: `{"data":{"modelId":"Je","status":"Running"}}`. Wait for the async re-inference, then GET:

```
VMV = Low
#1: 57.8300%
#2: 11.1600%
#3: 31.0100%
```

**Step 2** — PUT all 15 drivers back at their pristine values (atomic revert):

```bash
curl -X PUT https://api.51folds.ai/api/v1/models/Je/drivers \
  -H "Authorization: Bearer at_sk_…" \
  -H "Content-Type: application/json" \
  -d '{"drivers":[{"code":"BDAC","state":"Medium"}, …, {"code":"VMV","state":"Medium"}]}'
```

Wait for re-inference, then GET:

```
VMV = Medium          ← drivers correctly restored
#1: 56.3900%          ← expected 54.8800%, drift +1.5100%
#2: 11.4700%          ← expected 12.2900%, drift -0.8200%
#3: 32.1400%          ← expected 32.8300%, drift -0.6900%
```

**Every driver state on the server is bit-identical to the pristine set, but the outcome probabilities land somewhere between the pristine inference result and the post-PATCH inference result.** The server's Bayesian solver is retaining some form of inference state that is not cleared by subsequent driver updates, regardless of whether they come via PATCH (partial) or PUT (atomic "replace all").

### What actually does produce a clean revert

The 51Folds native web application (`app.51folds.ai`) exposes a **private platform endpoint** at:

```
PUT https://app.51folds.ai/api/platform/v1/inference/revert-to-origin
Body: {"modelId": <numeric>}
```

where `modelId` is a **numeric id** different from the short string id returned by the public API (e.g. `1218` rather than `"Je"`). Verified via curl during this session:

```
=== Fire the revert-to-origin PUT ===
{"success":true,"errorCode":null,"error":null,"stackTrace":null,"validates":null}
HTTP 200

=== Server state after revert (via public GET) ===
VMV = Medium
#1: 54.8800%          ← exact match to pristine
#2: 12.2900%          ← exact match to pristine
#3: 32.8300%          ← exact match to pristine
```

This endpoint **produces the exact original outcome probabilities, to four decimal places**, confirming that the server *can* perform a clean revert — it's just not exposed via the public API.

### Constraints on using the platform endpoint from our app

1. **Different auth.** The platform endpoint rejects our `at_sk_...` service token with `HTTP 401 Unauthorized`. It requires a short-lived JWT (audience `platform`, issuer `identity.51folds.ai`) obtained via the browser OIDC login flow, expiring ~15 minutes after issue.
2. **Different identifier.** The body uses a numeric `modelId` (e.g. `1218`), not the short string id (`"Je"`) the public API returns. There is no documented endpoint to look up the numeric id from the string id.
3. **Refresh token available.** The JWT has `offline_access` in its scope, meaning a refresh token exists somewhere in the browser session. In principle we could capture it once and use it to mint fresh JWTs without requiring a re-login on every revert — but this is a non-trivial implementation and we've elected to defer it pending the upstream fix.

### Decisions

1. **Ship the imperfect PUT-based revert** via `folds::put_drivers`. It restores driver states atomically (correct) but leaves outcome probabilities with known drift (incorrect). The confirmation dialog shows the user what *should* change based on a diff against the DB baseline; they will see that the actual result differs slightly.
2. **Do not implement the platform endpoint client-side yet.** The auth and ID-lookup work to consume it cleanly is non-trivial and would be obviated if 51Folds fix the public API.
3. **Track this as an upstream bug**. Action for the project owner: raise with the 51Folds development team and ask them to either:
   - **Fix the Bayesian solver** so that PATCH/PUT driver updates produce exact-original outcome probabilities when all drivers are set to their original values. This is the ideal fix — it makes the public API match the platform API's behaviour and respects the user expectation that identical inputs should produce identical outputs.
   - **Or expose the `revert-to-origin` endpoint on the public API** (`api.51folds.ai`) with `at_sk_...` auth and using the short string `modelId`. Less desirable but easier to ship client-side from multiple languages (Rust, Python, TypeScript) without OIDC plumbing.
4. **Document the limitation in the app**. A tooltip on the Revert-to-original confirmation dialog explains that driver states will be restored exactly but outcome probabilities may drift slightly due to a known 51Folds inference-state limitation.

## Files changed

| File | Summary |
|---|---|
| `Cargo.toml` | Version `0.2.0 → 0.1.0-preview`; added `image` dependency (PNG feature only) for the mascot decode |
| `src/main.rs` | `USER_AGENT` updated to `the-hedgehog/0.1.0-preview` |
| `src/app.rs` | Splash screen (`SplashState`, `render_splash`, mascot texture loading with checker-strip); FoldsTask hybrid state (remove session_history, add refresh_rx, etc.); Revert-to-original flow; Refresh-Model button (sidebar + central); recovery screen; poll split into `poll_main` / `poll_refresh`; confirmation dialog for revert; `diff_model_states` helper; stub detection in `load_from_json`; numerous dead-code removals from session history / apply-snapshot path |
| `src/folds.rs` | New wrappers: `refresh_model` (GET + persist), `put_drivers` (PUT + wait_until_complete, used by revert). `patch_drivers` rewritten: PATCH → `wait_until_complete` → return. No DB writes. Stub-response guards on all paths. Trimmed verbose per-driver debug logging |
| `src/help.rs` | "Why the hedgehog?" section at top; Isaiah Berlin framing; preview version label |
| `README.md` | Centred mascot at top; "Why the hedgehog?" block; Preview 0.1 badge; ADR index updated to include 0014 / 0015 / (this) |
| `artwork/hedgehog-mascot-transparent.png` | Source asset (new) |
| `artwork/hedgehog-mascot-white-bground.png` | Source asset used in the README for light/dark theme neutrality (new) |
| `docs/adr/README.md` | Added this ADR row |
| `docs/adr/0016-…md` | This document |
| `docs/GH-Actions-Plan.md` | GitHub Actions release plan (new, earlier in the session) — unimplemented pending sign-off |

## Consequences

- First-launch experience now has an identity: mascot, tagline, Berlin epigraph, version chip, live loading feed if auto-refresh is running. Not jarring, not slow.
- The 51Folds model explorer has a coherent state model: **DB is immutable original, memory is the live view**. Load from Reports always shows the pristine state. Re-evals update in-memory only. Revert reads from DB. No more cycles of my own bad persistence logic corrupting the local copy.
- Post-PATCH re-inference is no longer a race. Re-evals that previously failed to update visibly now correctly wait for the server.
- **The revert-to-original flow is known-imperfect** and this is documented both here and in the app. Driver states are restored exactly; outcome probabilities drift due to the 51Folds server-side Bayesian inference state-retention bug. Users who need exact revert must use the native 51Folds web app.
- The session history feature was built, tested, and removed — reflecting the discovery that it was not actually what the user wanted once the DB-baseline + revert-to-original flow was in place. The work was not wasted; it informed the cleaner final architecture.
- One upstream bug to pursue with 51Folds: PATCH/PUT inference drift. Raising with their team is a follow-up action outside this repo.

---

## Addendum: In limbo — handoff note for the 51Folds API team

**This ADR is not closed.** The splash screen, version change, DB-baseline architecture, PATCH race fix, and Refresh flow are all landed and working. But the **revert-to-original** feature is currently shipping an implementation that is known to produce wrong outcome probabilities, because the public 51Folds API does not provide a correct revert mechanism. We are pausing this ADR until the 51Folds team responds to the bug report below, at which point we'll either remove the workaround (if they fix the public API) or implement the platform endpoint properly (if they expose it to `at_sk_...` tokens).

### What the user sees today

When the user clicks **Revert to original** on a model they've been editing, the app:
- Reads the pristine driver states from the local DB baseline (correct)
- PUTs those 15 driver states atomically to `api.51folds.ai/api/v1/models/{id}/drivers` via the SDK's `update_drivers` method (correct)
- Polls `wait_until_complete` until re-inference finishes (correct)
- The server's response shows all 15 drivers correctly restored to their pristine values (correct)
- **But the outcome probabilities returned by the re-inference do not match the original build's outcomes** — they drift by roughly 1–2 percentage points per outcome (wrong)

So the button does its job on the driver side but fails on the outcome side. The user is left with a model whose driver configuration is the original but whose probability distribution is not. This defeats the purpose of "revert".

---

### Handoff note — can be copy-pasted to the 51Folds team

**Subject: PATCH/PUT driver updates produce drift vs original inference; public API lacks a working revert-to-origin**

We are building a Rust desktop integration against the 51Folds public API (`api.51folds.ai`, using `at_sk_...` auth, via our Rust SDK `fiftyone-folds`). The integration builds a model, lets the user re-evaluate drivers, and offers a "revert to original" action that we expected to implement via a driver update back to the pristine values. We've hit a reproducible server-side problem where driver updates leave the Bayesian solver in a state that does not match the original build's inference result, even when all drivers are restored to identical values.

**What we expected**

Given a model built with a specific set of driver states and resulting in a specific set of outcome probabilities, sending those same driver states back via PATCH or PUT should produce the same outcome probabilities, because the Bayesian network is fully specified by its structure + driver states and the inference should be a pure function of those inputs.

**What actually happens**

Outcome probabilities drift. The drivers on the server after the update are bit-identical to the pristine values, but the outcomes are not. The drift is not a rounding issue — it's ~1–2 percentage points per outcome.

**Reproduction**

Model ID: `Je` (numeric: `1218`). Built with the Advanced tier, status `Successed`. Fresh GET against the public API at build time produces (to four decimal places):

```
Driver states (15 drivers):
BDAC=Medium  CBGP=High   COPT=High   FLMP=High   FRPS=High
GEIOT=High   GRP=Medium  IE=High     MLC=High    PGD=Medium
RIR=High     SPD=High    TRL=High    UDIS=Medium VMV=Medium

Outcome probabilities:
#1 Stays below $4800:  54.8800%
#2 Breaks above $4800: 12.2900%
#3 Drops below $4300:  32.8300%
```

Step 1 — PATCH a single driver:

```bash
curl -X PATCH "https://api.51folds.ai/api/v1/models/Je/drivers" \
  -H "Authorization: Bearer at_sk_…" \
  -H "Content-Type: application/json" \
  -d '{"drivers":[{"code":"VMV","state":"Low"}]}'
```

Wait for re-inference (`status` flips from `Running` to `Successed`), then GET:

```
VMV = Low
#1: 57.8300%
#2: 11.1600%
#3: 31.0100%
```

Step 2 — PUT all 15 drivers back to their pristine values (atomic replace):

```bash
curl -X PUT "https://api.51folds.ai/api/v1/models/Je/drivers" \
  -H "Authorization: Bearer at_sk_…" \
  -H "Content-Type: application/json" \
  -d '{"drivers":[
        {"code":"BDAC","state":"Medium"},
        {"code":"CBGP","state":"High"},
        {"code":"COPT","state":"High"},
        {"code":"FLMP","state":"High"},
        {"code":"FRPS","state":"High"},
        {"code":"GEIOT","state":"High"},
        {"code":"GRP","state":"Medium"},
        {"code":"IE","state":"High"},
        {"code":"MLC","state":"High"},
        {"code":"PGD","state":"Medium"},
        {"code":"RIR","state":"High"},
        {"code":"SPD","state":"High"},
        {"code":"TRL","state":"High"},
        {"code":"UDIS","state":"Medium"},
        {"code":"VMV","state":"Medium"}
      ]}'
```

Wait for re-inference, then GET:

```
Driver states on server after PUT:
BDAC=Medium  CBGP=High   COPT=High   FLMP=High   FRPS=High   ← identical to pristine
GEIOT=High   GRP=Medium  IE=High     MLC=High    PGD=Medium
RIR=High     SPD=High    TRL=High    UDIS=Medium VMV=Medium

Outcome probabilities:
#1: 56.3900%   ← expected 54.8800%, drift +1.5100%
#2: 11.4700%   ← expected 12.2900%, drift -0.8200%
#3: 32.1400%   ← expected 32.8300%, drift -0.6900%
```

**The drivers are correct. The outcomes are not.** Repeating PUT with the same payload does not converge; the outcomes stay drifted. PATCH produces the same behaviour. This is reproducible on every attempt and is not a race condition — we poll `wait_until_complete` so the GET happens strictly after `status` flips back to `Successed`.

**What we know works**

The 51Folds native web application has a private platform endpoint that performs a true revert and produces outcomes that match the pristine build to four decimal places:

```
PUT https://app.51folds.ai/api/platform/v1/inference/revert-to-origin
Authorization: Bearer <OIDC JWT, audience "platform", short-lived ~15 min>
Content-Type: application/json
Body: {"modelId": 1218}
```

Response: `{"success":true,"errorCode":null,"error":null,"stackTrace":null,"validates":null}` HTTP 200.

After calling this endpoint and GETting the model via the public API, outcomes are:

```
#1: 54.8800%   ← exact match to pristine
#2: 12.2900%   ← exact match to pristine
#3: 32.8300%   ← exact match to pristine
```

So the server **can** do a clean revert — the capability exists in the platform layer, it's just not exposed to public API consumers.

**Ideal fix (in order of preference)**

1. **Fix PATCH/PUT driver updates so that identical driver states produce identical outcome probabilities**, regardless of the server's prior update history. Same inputs, same outputs. This matches what any consumer of the Bayesian API would reasonably expect and avoids the need for any dedicated revert endpoint.
2. **Or expose `revert-to-origin` on the public API** (`api.51folds.ai`), accepting `at_sk_...` tokens and using the short string `modelId` the public API already returns. We don't need access to the platform-API JWT flow; we just need the revert capability accessible with our existing service token.

**Lower-priority related observations**

- **`stateDescriptors[0].name` is emitted as `"Negligent"`** (a different English word meaning "careless") whereas the canonical Bayesian schema and `current.drivers[].state` use `"Negligible"` (meaning "tiny"). Both appear in the same response for the same model. We work around this client-side with ordinal fallback normalisation, but it's a systematic string mismatch that would be cheaper to fix upstream.
- **PATCH response body is a minimal `{modelId, status: "Running"}` stub** rather than a full `ModelResponse`. The asynchronous behaviour is fine (we poll); we just can't use the PATCH response body as a source of truth for outcomes and have to GET afterwards. A response body containing the full model would save a network round-trip per re-eval. Not critical but worth noting.

---

### Where we pick up tomorrow

1. Send the handoff note above to the 51Folds API team.
2. Until we hear back, ship the imperfect PUT-based revert as-is — it gets driver states right, and the drift is documented in the UI so users aren't surprised.
3. If the 51Folds team fixes the public API: remove the documentation caveat and confirm via the reproduction curl.
4. If they expose `revert-to-origin` on the public API with `at_sk_...` auth: swap `folds::put_drivers` to call the new endpoint instead.
5. If they don't fix either: decide whether to invest in implementing the OIDC refresh-token flow client-side so we can hit the platform endpoint directly. This is ~200 lines of Rust plus user-friendly "paste your refresh token once" settings plumbing. Not worth doing unless we know there's no upstream fix coming.
6. **Rewrite `ai.rs::assemble_system_prompt` to remove its commodity bias and handle the nuance properly.** Found tonight: when the user selects only Soybeans and runs "Analyse current view", the LLM produces a hypothesis about *crude oil* because the template example in the prompt uses crude oil as its subject. LLMs latch onto concrete examples in the prompt, especially when the target commodity has weaker training priors than the example. The fix has three parts:
   - **Explicit primary instruction**: *"The hypothesis MUST be about the instruments listed in 'Instruments in view' in the user message. If only one instrument is in view, the hypothesis is about that one. Do not substitute a different commodity regardless of which one the template example happens to use."*
   - **Generic example placeholder** instead of a hardcoded "Crude oil will remain above $78…" — use `"[Instrument] will [direction] $[level] through [horizon] as [mechanism]"` or rotate through several examples so there's no single-subject bias. Same change needed for the `**Hypothesis Outcomes**` example and the "unselected instruments" paragraph that currently uses crude oil as its "for example" case.
   - **Three-tier framing** for how the LLM should weigh its inputs, made explicit in the system prompt rather than left implicit:
     1. **Primary**: the instruments in view. The hypothesis is about them. Every price reference comes from the "Latest closes" block.
     2. **Secondary**: the knowledge base chunks already appended to the system prompt — these describe the *historical* volatility-regime behaviour of each commodity (2008 GFC, 2020 COVID, 2022 Ukraine, etc.). The LLM should consult them for how the primary instruments *typically* behave during the current regime, and use that as the mechanism-naming source for the causal transmission channels required by the Hypothesis Context section.
     3. **Tertiary**: other commodities' *current* behaviour (the "unselected instruments" block in the user message). Only relevant as contextual background — "copper is falling sharply, which corroborates the demand-shock reading" — never as the subject. If they aren't corroborating or contradicting anything meaningful, omit them.
   - The current prompt dumps the knowledge chunks at the bottom with no framing about when to use them, and conflates "other commodities in the data" with "should be part of the analysis" via vague language. Tomorrow's rewrite should spell out the three tiers as a numbered list and anchor the template example to a generic placeholder.

At that point, this ADR closes and the follow-up is either "no further action" or a fresh ADR documenting the OIDC integration (and possibly a small ADR on the prompt-engineering fix if the rewrite turns out to be substantive).
