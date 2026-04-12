# 51Folds API — driver updates leave the model in a state that can't be reverted

**Reported by:** the team building *The Hedgehog*, a Rust desktop app that integrates the 51Folds public API via the `fiftyone-folds` Rust SDK.
**Audience:** the 51Folds API team.
**Status:** open. We have shipped a workaround in our app that is known to be wrong, and we are documenting that limitation to our users by telling them to perform reverts inside the native 51Folds web app.

## In one paragraph

When we update a model's drivers via your public API and then update them again to put the drivers back the way they started, the drivers come back exactly — but the **outcome probabilities don't**. The model never returns to its original inference result. The only way we have found to get a clean revert is to use the native 51Folds web application, which calls a private platform endpoint that the public API does not expose. Until you fix this, our app cannot offer a true "revert to original" feature, and our users have to switch over to the native UI to do it.

## What we expected

Bayesian inference is a pure function of network structure plus driver states. If we send the same driver states twice, we should get the same outcome probabilities twice. That's the contract we built our integration against.

## What actually happens

We built a model. It produced a specific set of driver states and a specific set of outcome probabilities. We changed one driver. We waited for re-inference. Then we sent **all 15 drivers back at their original values** — first via PATCH (partial update), then via PUT (atomic replace-all), to rule out merge-vs-replace as the cause.

In both cases:

- The driver states on the server after the second update were **bit-identical** to the original.
- The outcome probabilities were **not** identical to the original. They drifted by roughly one to two percentage points per outcome and stayed there. Sending the same PUT again did not converge. The drift is permanent for the lifetime of that model on the server.

This tells us your inference engine is retaining state across driver updates that isn't cleared when we resend the original inputs. We don't know whether it's a cached posterior, a warm-started solver, or something else, but from the API consumer's point of view it means **identical inputs do not produce identical outputs**, which breaks the most basic guarantee a forecasting API can make.

## Reproduction

Model `Je` (numeric id `1218`), Advanced tier, status `Successed`. Authenticated with our `at_sk_...` service token.

**Pristine state at build time** (fresh GET):

```
Drivers (15):
  BDAC=Medium  CBGP=High   COPT=High   FLMP=High   FRPS=High
  GEIOT=High   GRP=Medium  IE=High     MLC=High    PGD=Medium
  RIR=High     SPD=High    TRL=High    UDIS=Medium VMV=Medium

Outcomes:
  #1 Stays below $4800:  54.8800%
  #2 Breaks above $4800: 12.2900%
  #3 Drops below $4300:  32.8300%
```

**Step 1 — flip one driver via PATCH:**

```
PATCH https://api.51folds.ai/api/v1/models/Je/drivers
Body: {"drivers":[{"code":"VMV","state":"Low"}]}
```

Wait for `status` to flip from `Running` back to `Successed`, then GET:

```
VMV = Low
#1: 57.8300%
#2: 11.1600%
#3: 31.0100%
```

So far, so expected — different inputs, different outputs.

**Step 2 — put all 15 drivers back to the pristine values via PUT:**

```
PUT https://api.51folds.ai/api/v1/models/Je/drivers
Body: {"drivers":[
  {"code":"BDAC","state":"Medium"}, {"code":"CBGP","state":"High"},
  {"code":"COPT","state":"High"},   {"code":"FLMP","state":"High"},
  {"code":"FRPS","state":"High"},   {"code":"GEIOT","state":"High"},
  {"code":"GRP","state":"Medium"},  {"code":"IE","state":"High"},
  {"code":"MLC","state":"High"},    {"code":"PGD","state":"Medium"},
  {"code":"RIR","state":"High"},    {"code":"SPD","state":"High"},
  {"code":"TRL","state":"High"},    {"code":"UDIS","state":"Medium"},
  {"code":"VMV","state":"Medium"}
]}
```

Wait for re-inference, then GET:

```
Drivers on server:
  BDAC=Medium  CBGP=High   COPT=High   FLMP=High   FRPS=High   ← exact pristine
  GEIOT=High   GRP=Medium  IE=High     MLC=High    PGD=Medium
  RIR=High     SPD=High    TRL=High    UDIS=Medium VMV=Medium

Outcomes:
  #1: 56.3900%   ← expected 54.8800%, drift +1.5100%
  #2: 11.4700%   ← expected 12.2900%, drift -0.8200%
  #3: 32.1400%   ← expected 32.8300%, drift -0.6900%
```

Drivers correct. Outcomes wrong. Repeating the PUT does not change them.

We've ruled out:

- **A race condition.** Our SDK calls `wait_until_complete`, which polls `GET /models/{id}` every 2 seconds and only returns once `status` flips back to `Successed`. The GETs above happen strictly after the server reports re-inference is finished.
- **PATCH-vs-PUT semantics.** PUT documented as atomic replace-all gives the same drift as PATCH. This isn't a partial-merge artifact.
- **A floating-point rounding issue.** The drift is ~1–2 percentage points per outcome, far above any rounding noise.
- **Client-side caching on our end.** We confirmed via raw `curl` against the public API.

## We know the server can do this correctly

The native 51Folds web application at `app.51folds.ai` exposes a **private platform endpoint** that produces a true revert:

```
PUT https://app.51folds.ai/api/platform/v1/inference/revert-to-origin
Authorization: Bearer <OIDC JWT, audience "platform">
Content-Type: application/json
Body: {"modelId": 1218}
```

Response: `{"success":true,"errorCode":null,"error":null,"stackTrace":null,"validates":null}` HTTP 200.

After calling this, the public API GET returns:

```
#1: 54.8800%   ← exact pristine
#2: 12.2900%   ← exact pristine
#3: 32.8300%   ← exact pristine
```

Four-decimal exact match. So the underlying engine **can** restore a model to its original inference result — that capability already exists in your platform layer. It's just not reachable from the public API that third-party integrators are supposed to build against.

## Why we can't just consume the platform endpoint

Three blockers, in order of severity:

1. **Different auth.** The platform endpoint rejects our `at_sk_...` service token with HTTP 401. It only accepts a short-lived OIDC JWT (audience `platform`, issuer `identity.51folds.ai`, ~15 minute expiry) obtained via your browser login flow.
2. **Different identifier.** The body needs the **numeric** model id (`1218`) rather than the **short string** id (`"Je"`) the public API returns. There is no documented public-API endpoint to look up the numeric id from the string id.
3. **No SDK support.** Our Rust SDK and your other public SDKs don't expose anything in `/api/platform/...`.

We could in principle implement the OIDC refresh-token flow client-side and ask each user to paste a refresh token into our settings once. That's roughly 200 lines of Rust plus user-facing plumbing, which we are deliberately not building until we know whether you'll fix the public API instead.

## What we need from you, in order of preference

1. **Fix the inference engine** so that PATCH and PUT driver updates produce outcome probabilities that are a pure function of the current driver states, regardless of what the previous driver state was. Identical inputs, identical outputs. This is the right fix because it makes the public API obey the contract any consumer would reasonably expect, and it removes the need for any dedicated revert endpoint at all.
2. **Or expose the `revert-to-origin` capability on the public API** at `api.51folds.ai`, accepting `at_sk_...` tokens and using the **short string** `modelId` the public API already returns. We don't need access to your platform OIDC flow — we just need the revert capability reachable from the same auth surface as everything else we already use.

Either fix unblocks us. The first is better for your whole API ecosystem, not just us.

## What we're doing in the meantime

**Inside our app:**

- The "Revert to original" button is wired up to the public PUT path. It correctly restores the drivers, but the outcome probabilities are wrong by the drift amounts shown above. We've documented that limitation in the confirmation dialog so users aren't surprised.
- We keep an immutable copy of each model's pristine state in our local database (the full original `ModelResponse` JSON). Loading a model from our reports list always shows the user the pristine state, not whatever the server currently has, so they have a stable point of reference for what "original" meant.

**For our users:**

- We're telling them, in our user-facing documentation and in the revert confirmation dialog, that **a true revert to the exact original outcome probabilities is not currently possible via The Hedgehog**, and that if they need to put a model back to its pristine state with confidence, they should do it inside the native 51Folds web app at `app.51folds.ai`, which uses your platform endpoint and produces a clean revert.

This is a workaround we are not happy about. It forces our users to leave our app and go to yours to perform an operation that should be a one-click action in the integration. We'd very much like to retire it as soon as you fix one of the two paths above.

## Lower-priority observations from the same investigation

While we were tracing this we noticed two smaller things in the public API that we'd flag while we have your attention:

1. **`stateDescriptors[0].name` is emitted as `"Negligent"`** in some responses, where the canonical term used elsewhere in the same response (in `current.drivers[].state`) is `"Negligible"`. "Negligent" means careless; "Negligible" means tiny. They're different English words. We work around this in our client by normalising on ordinal position, but it's a systematic string mismatch that would be cheaper to fix server-side.
2. **PATCH responses are minimal stubs.** A PATCH against `/models/{id}/drivers` returns just `{"data":{"modelId":"Je","status":"Running"}}` rather than a full `ModelResponse`. The async behaviour is fine — we poll `wait_until_complete` and that works — but if the PATCH response carried the full re-inferred model on completion, we'd save a network round-trip per re-eval. Not critical, just noting it.

## Contact

Happy to share the exact `curl` invocations, the `wait_until_complete` polling logic from our Rust SDK wrapper, full request and response captures, or to pair on debugging this from your end. Reach us via the project owner.
