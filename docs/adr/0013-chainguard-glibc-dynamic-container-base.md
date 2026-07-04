---
title: "Chainguard glibc-dynamic as the Container Runtime Base, Superseding distroless/cc-debian12"
description: "Migrate mif-rs's container runtime base image from gcr.io/distroless/cc-debian12 to cgr.dev/chainguard/glibc-dynamic after finding 14 permanently-unfixed CVEs in the Debian-derived base, replacing a permanent .trivyignore suppression with an empirically-verified, 0-vulnerability image."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: build
tags:
  - adr
  - docker
  - security
  - container-base
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0012-cargo-chef-docker-layer-caching.md
---

# ADR-0013: Chainguard glibc-dynamic as the Container Runtime Base, Superseding distroless/cc-debian12

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs`'s container images were originally built on `gcr.io/distroless/cc-debian12`
as the runtime stage base — chosen at initial bootstrap specifically because it
bundles a CA certificate bundle and glibc, unlike `scratch` or
`distroless/static`, which have neither.

On 2026-07-02, a Trivy scan found 14 CVEs in `distroless/cc-debian12` that
Debian had classified `<no-dsa>` or that upstream had disputed for the
`bookworm` branch. `<no-dsa>` means Debian has no fix planned — these were not
findings a future digest bump would ever resolve. A `.trivyignore` file was
suppressing all 14 findings, which was judged a permanent workaround, not a
fix, on an attested, signed release pipeline where that isn't acceptable: this
repository's release pipeline produces SLSA-attested, signed container images,
and a permanently-suppressed CVE list is not a defensible security posture for
that pipeline to stand behind.

### Current Limitations

1. **Unfixable CVEs with no resolution path**: 14 findings in the Debian base
   were marked `<no-dsa>` or disputed — no Debian security advisory would ever
   land, so no routine digest bump could close them.
2. **A suppression file standing in for a fix**: `.trivyignore` documented the
   gap without closing it, on a pipeline whose entire value proposition is an
   attested, verifiable security posture.

## Decision Drivers

### Primary Decision Drivers

1. **An attested, signed pipeline cannot rely on permanent suppression**: a
   suppression file is documentation of a gap, not a closure of it; this
   repository's release images are SLSA-attested and signed, and their actual
   security posture has to be the scan result, not a list of CVEs excused from
   the scan.
2. **No functional regression**: any replacement base image must still provide
   a CA certificate bundle, glibc, and a working non-root user, matching what
   `distroless/cc-debian12` already provided, so the migration itself
   introduces no functional regression.

### Secondary Decision Drivers

1. **No open-ended suppression policy going forward**: whatever replaces the
   permanent `.trivyignore` list must still allow a genuinely time-bounded
   suppression (a stated recheck-by date) for a newly-discovered CVE, rather
   than requiring this decision to be re-litigated every time a single new
   finding appears in the chosen base image.
2. **Verifiability over vendor claims**: the replacement's clean security
   posture should be checked empirically against the actual built image
   (scan results, UID/user presence, cert presence, multi-arch manifest),
   not accepted on the base image vendor's own marketing.

## Considered Options

### Option 1: Keep distroless/cc-debian12, continue suppressing the 14 CVEs

**Description**: Retain `gcr.io/distroless/cc-debian12` as the runtime base
and continue carrying the 14 `<no-dsa>`/disputed CVEs in `.trivyignore`
indefinitely.

**Advantages**: Zero migration effort — the Dockerfile, the existing
`nonroot` user, and the already-verified CA certificate bundle all stay
exactly as they are, with no risk of introducing a new base-image
incompatibility.

**Disadvantages**: A permanently-growing suppression list is the opposite of
what an attested pipeline is meant to demonstrate — it substitutes a
documented exception for an actual fix, indefinitely.

**Disqualifying Factor**: this is a permanent workaround, not a fix,
inappropriate for a pipeline whose entire value proposition is attested,
verifiable security posture.

**Risk Assessment**:

- **Technical Risk**: Low. No code changes required.
- **Schedule Risk**: None.
- **Ecosystem Risk**: High. The suppression list only grows as Debian's
  `<no-dsa>` backlog accumulates, undermining the pipeline's attested-security
  claim.

### Option 2: scratch or distroless/static

**Description**: Move to a minimal base with no OS layer at all.

**Advantages**: The smallest possible attack surface and image size — no OS
package layer at all means no Debian (or any distro's) CVE backlog to track
in the first place.

**Disadvantages**: Neither `scratch` nor `distroless/static` provides a CA
certificate bundle or glibc. Adopting either would require statically linking
against a different libc or manually vendoring a certificate bundle into the
image — a larger and riskier change than swapping to an equivalent-featured
base image.

**Risk Assessment**:

- **Technical Risk**: Medium. Requires either a libc migration or manual cert
  vendoring, neither of which this decision's drivers call for.
- **Schedule Risk**: Medium.
- **Ecosystem Risk**: Low.

### Option 3: cgr.dev/chainguard/glibc-dynamic (chosen)

**Description**: Migrate the runtime stage to Chainguard's `glibc-dynamic`
image, built on Wolfi and continuously rebuilt from source rather than
tracking Debian stable's frozen CVE backlog.

**Advantages**:

- Verified empirically before merging, not assumed from vendor marketing: a
  Trivy scan of the bare base image showed 0 vulnerabilities, and a Trivy scan
  of the fully built application image also showed 0 vulnerabilities.
- The `nonroot:x:65532:65532` user was already present, so the Dockerfile's
  existing `USER nonroot:nonroot` directive needed no change.
- CA certificates were present with `SSL_CERT_FILE` set correctly.
- The multi-arch manifest was confirmed.
- Both `mif-cli` and `mif-mcp` were actually built, run, and used to validate
  real MIF documents against the new base before the change was merged.

**Disadvantages**: A less universally-familiar base image family than
Debian-derived distroless images — Wolfi/Chainguard tooling and conventions
represent a small ramp-up cost for maintainers who have only worked with
Debian-based images before.

**Risk Assessment**:

- **Technical Risk**: Low. Empirically verified functionally equivalent to the
  prior base before merging.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low. Wolfi's continuously-rebuilt-from-source model
  avoids the frozen-CVE-backlog problem that motivated this migration.

## Decision

We migrate the runtime stage base image to
**`cgr.dev/chainguard/glibc-dynamic`**, superseding
`gcr.io/distroless/cc-debian12`.

`.trivyignore` was deleted at the time of this migration as no longer needed.
It was later recreated for one unrelated, newly-discovered CVE in the
Chainguard base itself, following the same "suppress with a stated expiry
date, recheck when the fix ships" pattern this repository already used once
before for the original distroless CVEs. That pattern — a suppression entry
carrying a stated recheck-by date, not an open-ended exclusion — is the
recurring policy going forward, not a re-litigation of this decision.

## Consequences

### Positive

1. **A genuinely clean base image**: a 0-vulnerability, empirically-verified
   base image, appropriate for an attested/signed release pipeline, rather
   than a base image with a permanently-suppressed CVE list.
2. **Future CVEs expected to resolve via routine digest bump**: Wolfi's
   continuously-rebuilt-from-source model means future CVEs are expected to be
   resolved by a routine digest bump rather than requiring another full
   base-image migration.

### Negative

1. **Less familiar base image family**: Chainguard/Wolfi is a less
   universally-familiar base image family than Debian-derived distroless
   images, representing a small ramp-up cost for future maintainers
   unfamiliar with it.

### Neutral

1. Every verification claim behind this decision — vulnerability scan counts,
   UID/user presence, cert presence, multi-arch manifest, functional
   document-validation test — was checked empirically against the real built
   image before merging, not assumed from the base image vendor's own claims
   about it.

## Decision Outcome

The decision achieves its primary objective — a runtime base image with no
permanently-suppressed CVE list — measured by: the current Dockerfile's
runtime stage `FROM` line references
`cgr.dev/chainguard/glibc-dynamic@sha256:ea9eab0adc5716fb9937ab60155a31bce9cbc8b56e6f2e21fb9af9218be195b7`,
and `.trivyignore` currently contains at most one time-bounded entry with a
stated recheck-by date, not a growing permanent list.

## Related Decisions

- [ADR-0012: Cargo Chef Docker Layer Caching](0012-cargo-chef-docker-layer-caching.md)

## Links

- [Chainguard Images documentation](https://edu.chainguard.dev/chainguard/chainguard-images/) - overview of the Wolfi-based, continuously-rebuilt image catalog
- [`cgr.dev/chainguard/glibc-dynamic`](https://images.chainguard.dev/directory/image/glibc-dynamic/versions) - the specific image reference adopted by this decision
- [Debian Security Tracker: `<no-dsa>` status](https://security-tracker.debian.org/tracker/status/no-dsa) - explains why the 14 findings in `distroless/cc-debian12` had no resolution path
- [Trivy scanner documentation](https://trivy.dev/) - the tool used to empirically verify 0 vulnerabilities in the new base

## More Information

- **Date**: 2026-07-03 (retroactively documents a decision made 2026-07-02)
- **Source**: commits 8767a18/2614a1b ("fix(docker): migrate runtime base to
  chainguard/glibc-dynamic"), plus the current `Dockerfile` and `.trivyignore`.

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Runtime stage `FROM` references `cgr.dev/chainguard/glibc-dynamic@sha256:ea9eab0adc5716fb9937ab60155a31bce9cbc8b56e6f2e21fb9af9218be195b7`; `.trivyignore` contains a single time-bounded entry (`CVE-2026-6791 exp:2027-01-02`) | Dockerfile, .trivyignore | 41, 10 | accepted |

**Summary:** Verified against the current repository state: the runtime base
migration is in place and the CVE suppression list is a single, time-bounded
exception rather than a permanent, growing list.

**Action Required:** None — this ADR documents current, already-adopted
practice.
