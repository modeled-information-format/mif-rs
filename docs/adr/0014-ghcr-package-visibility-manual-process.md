---
title: "GHCR Package Visibility: Manual, Not Automated"
description: "Set GHCR container package visibility manually via the GitHub web UI after two same-day automated approaches were reverted, having each hit a real, undocumented-until-now GitHub Packages API constraint discovered the hard way."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: operations
tags:
  - adr
  - ghcr
  - ci
  - operations
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0013-chainguard-glibc-dynamic-container-base.md
---

# ADR-0014: GHCR Package Visibility: Manual, Not Automated

## Status

Accepted

## Context

### Background and Problem Statement

On first push to GHCR, the container packages for `mif-cli` and `mif-mcp` came
up **private** despite the `mif-rs` repository itself being public. This
blocked the `reusable-trivy.yml` image-scan CI job, which pulls the pushed
image to scan it — a private package meant that pull failed with a `401`.

Two automated fixes were attempted and reverted the same day (2026-07-02),
each hitting a real platform constraint that had not been documented anywhere
in this workspace until now:

1. **`admin-set-package-visibility.yml`**: a workflow minting a `release` app
   installation token and calling the GitHub Packages API to `PATCH` package
   visibility to public. Reverted roughly 3 minutes later, once it was
   confirmed that GitHub's REST API has **no endpoint for changing package
   visibility at all** — the Packages API only supports `GET`, `DELETE`, and
   `POST` (restore), for any authentication type, not just App tokens.
   Visibility can only be changed through the GitHub web UI (package
   settings). This is a permanent platform limitation, not a temporary gap in
   API coverage.
2. **`admin-delete-orphaned-packages.yml`**: a second workflow, built on a new
   hypothesis — that the packages remained tied to an archived fork's
   repository ID after `mif-rs` was detached into a fresh, non-fork
   repository, causing pushes to `403` against a package path GHCR still
   internally attributed to the old repo. This workflow deleted the orphaned
   packages via a `release` app token so the next push would recreate them
   fresh. Reverted about 22 minutes later, once GitHub's own OpenAPI
   specification was found to mark **every single org-packages endpoint** as
   `enabledForGitHubApps: false` — meaning a GitHub App installation token can
   never call any org-packages operation at all, regardless of the actual
   package state. The earlier `404`s this workflow saw were the platform
   rejecting the token type outright, not signaling anything about package
   state.

A third, unrelated same-day attempt — disabling BuildKit's default
provenance-attestation manifest on `docker push`, on the hypothesis that it
caused the GHCR `403` directly — was tried and reverted about 15 minutes
later, without ever being tested against a real push. The actual fix that
resolved the original `403` was the orphaned-package deletion described
above, which had already run before this third attempt's revert. The current
Dockerfile and release workflow carry no provenance override at all;
BuildKit's default provenance behavior is unmodified, and this attempt was
confirmed never to have been the real cause.

### Current Limitations

1. **No documented visibility-change path**: before this ADR, nothing in this
   repository recorded that GHCR package visibility cannot be changed via API
   at all, for any token type.
2. **No documented App-token exclusion**: before this ADR, nothing recorded
   that GitHub App installation tokens are categorically excluded from every
   org-packages endpoint, independent of package state or the specific
   operation attempted.
3. **A false lead left untested**: the BuildKit-provenance-disable attempt was
   reverted without ever being verified against a real push, and without
   anything recording that it was not, in fact, the cause of the original
   failure.

## Decision Drivers

### Primary Decision Drivers

1. **Automation must work within GitHub's actual, documented API surface**:
   package-visibility and package-lifecycle automation shall be built against
   what the Packages API and GitHub Apps platform actually support, not a
   plausible-sounding but unverified hypothesis about what they support.
2. **A diagnosis must be confirmed before being adopted as a fix**: a
   proposed root cause shall be tested against a real failure before being
   treated as resolved — not reverted before it was ever exercised.

### Secondary Decision Drivers

1. **Minimize recurring operational burden**: whatever is chosen should not
   trade a solved problem for a new maintenance obligation — a one-time step
   is preferable to a workflow that needs upkeep.
2. **Preserve engineering time**: two same-day reverts already consumed
   engineering effort chasing approaches the platform does not support; the
   chosen path should not risk a third.
3. **Leave a durable record for future maintainers**: whatever platform
   constraints were discovered the hard way should be written down once, so
   no future repository in this organization rediscovers them from scratch.

## Considered Options

### Option 1: Keep iterating on GitHub-App-token automation for package visibility or deletion

**Description**: Continue building workflows that mint a GitHub App
installation token (e.g., the `release` app) to change package visibility or
manage package lifecycle via the GitHub Packages API.

**Advantages**: If it worked, it would fully close the loop opened by the
first-push-comes-up-private symptom without requiring a human to touch the
GitHub web UI at all, and it would reuse the existing `release` App
installation token already minted for other release-time operations rather
than introducing a new credential type.

**Disadvantages**: Both concrete attempts at this hit hard, documented
platform constraints that no further App-token engineering effort can route
around: no visibility-`PATCH` endpoint exists for any token type, and GitHub
Apps are categorically excluded from every org-packages endpoint. These are
platform-level exclusions, not configuration mistakes to be debugged around.

**Disqualifying Factor**: continuing to invest in App-token automation for
these operations is engineering effort spent against a wall the platform has
built on purpose — it cannot be routed around by any workflow change.

**Risk Assessment**:

- **Technical Risk**: High. Every further attempt would hit the same
  categorical exclusions already confirmed twice.
- **Schedule Risk**: High. Repeated reverts consume engineering time for no
  forward progress.
- **Ecosystem Risk**: Low. No lasting damage to the repository; the cost is
  wasted effort, not breakage.

### Option 2: Accept GHCR package visibility as a manual, one-time, web-UI-only operation (chosen)

**Description**: Accept that GHCR package visibility is set once per new
repository through the GitHub web UI (package settings), not automated.
Visibility persists once set — per the original `admin-set-package-visibility.yml`
workflow's own noted assumption. Accept that any future package lifecycle
operation requiring the Packages API (such as the orphaned-package deletion
case above) needs a real personal-access token or OAuth token carrying the
`packages` scope, not a GitHub App installation token, since GitHub Apps are
excluded from these endpoints entirely.

**Advantages**:

- Works within the platform's actual, confirmed API surface instead of
  against it.
- A one-time manual step, performed once per repository, not a recurring
  operational burden.
- Documents the two real constraints that caused both prior attempts to fail,
  so a future maintainer does not rediscover the same two dead ends.

**Disadvantages**: Relies on a human remembering to perform the step on each
new repository, since nothing enforces it programmatically; as accepted in
the Consequences section below, this ADR itself was adopted before the
corresponding runbook step was written, so the manual step is not yet
discoverable from `docs/runbooks/`.

**Risk Assessment**:

- **Technical Risk**: Low. No automation to maintain or break.
- **Schedule Risk**: Low. A single manual step, once per repository.
- **Ecosystem Risk**: Low. No workflow depends on an operation the platform
  does not support.

### Option 3: Migrate to a different container registry with a documented visibility API

**Description**: Move container image publishing off GHCR to a registry whose
API does support programmatic visibility changes.

**Advantages**: Would eliminate the visibility-API gap entirely and permit
fully automated package lifecycle management, including the visibility and
deletion operations both reverted workflows attempted.

**Disadvantages**: Disproportionate to the actual problem — this is a
one-time, per-repository setup step encountered once, not a recurring
operational burden significant enough to justify a registry migration.

**Disqualifying Factor**: the cost of a registry migration far exceeds the
cost of a single manual click in a web UI performed once per repository.

**Risk Assessment**:

- **Technical Risk**: Medium. A registry migration touches every workflow
  that pushes or pulls container images.
- **Schedule Risk**: High. Disproportionate effort for a one-time setup step.
- **Ecosystem Risk**: Medium. Changes the published image location for every
  consumer of `mif-rs` container images.

## Decision

We accept **GHCR package visibility as a manual, one-time, web-UI-only
operation**, performed once per new repository. No workflow in this
repository automates GHCR package visibility or package deletion.

This ADR records that manual step for any future repository in this
organization that hits the same first-push-comes-up-private symptom, so a
future maintainer does not have to rediscover the same two dead ends:

- There is no GitHub Packages API endpoint for changing package visibility,
  for any token type. It is a web-UI-only operation, and it persists once
  set.
- GitHub App installation tokens are excluded from every org-packages
  endpoint by design (`enabledForGitHubApps: false` on all of them in
  GitHub's own OpenAPI specification). Any future package-lifecycle operation
  requiring the Packages API needs a real personal-access token or OAuth
  token carrying the `packages` scope instead.

## Consequences

### Positive

1. **No further wasted engineering effort**: no future attempt will chase an
   automation path that cannot work — no visibility-`PATCH` endpoint exists at
   all, and GitHub Apps are excluded from every org-packages endpoint by
   design.
2. **Real platform constraints now documented**: these constraints previously
   lived only in reverted commit messages; they now live in a durable,
   discoverable record.

### Negative

1. **A genuine documentation gap remains as of this ADR**: no runbook step in
   `docs/runbooks/` currently instructs a new-repository setup to manually
   flip package visibility to public in the GitHub web UI. `docs/runbooks/RELEASING.md`
   and `docs/runbooks/SECURITY-RESPONSE.md` both reference GHCR (image pushes,
   pull commands, immutability by tag) but neither mentions package
   visibility. This is flagged as an open follow-up in the Decision Outcome
   below, not something already closed.

### Neutral

1. The BuildKit-provenance-disable attempt is recorded here as a documented
   false lead: plausible on its face, never actually tested against a real
   push before being reverted, and confirmed not to have been the real cause
   — the orphaned-package deletion was. The current Dockerfile and release
   workflow carry no provenance override; BuildKit's default behavior is
   unmodified.

## Decision Outcome

The decision's primary objective — stop wasting engineering effort on a
fundamentally impossible automation path — is met: no workflow automating
GHCR package visibility or package deletion exists in this repository today.

The decision is only **partially** complete in one respect. The manual
web-UI step it prescribes is not yet written down as an actual runbook step
anywhere in `docs/runbooks/`. A check of `docs/runbooks/RELEASING.md` and
`docs/runbooks/SECURITY-RESPONSE.md` (the two files that mention `ghcr.io`)
found references to image pushes, pull commands, and tag immutability, but no
step instructing a new-repository setup to manually set package visibility to
public. This is an open follow-up this ADR surfaces, not something already
closed: a future contributor should add this step to the relevant
onboarding/release runbook.

## Related Decisions

- [ADR-0013: Chainguard glibc-dynamic as the Container Runtime Base, Superseding distroless/cc-debian12](0013-chainguard-glibc-dynamic-container-base.md)

## Links

- [Configuring a package's access control and visibility](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility) - GitHub's own documentation confirming visibility changes are a web-UI operation
- [GitHub Packages REST API](https://docs.github.com/en/rest/packages/packages) - The endpoint surface (`GET`/`DELETE`/`POST` restore only) that has no visibility-`PATCH` operation
- [GitHub Apps and OAuth apps API scopes](https://docs.github.com/en/rest/overview/permissions-required-for-github-apps) - Documents which endpoints are excluded from GitHub App installation tokens
- [GitHub REST API OpenAPI description](https://github.com/github/rest-api-description) - Source of the `enabledForGitHubApps: false` markers found on every org-packages endpoint

## More Information

- **Date**: 2026-07-03 (retroactively documents decisions made 2026-07-02)
- **Source**: commits `b0aeb53`, `6295c7a`, `fa6e87b`, `77993ab`, `79b2e98`,
  `0708e2a` (all 2026-07-02, direct pushes to `main`, each carrying its own
  explanatory commit body except the final revert).

## Audit

### 2026-07-03

**Status:** Partial

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| No workflow in this repository automates GHCR package visibility or package deletion | (repository-wide) | - | accepted |
| No runbook step exists instructing manual GHCR package-visibility setup | docs/runbooks/RELEASING.md, docs/runbooks/SECURITY-RESPONSE.md | - | gap |

**Summary:** The decision's primary objective — no further automation effort
against an unsupported API surface — is verified in place. The documentation
follow-up it depends on is not yet written: neither runbook file that
references GHCR mentions package visibility.

**Action Required:** Add a manual GHCR package-visibility step to the
relevant onboarding/release runbook in `docs/runbooks/`.
