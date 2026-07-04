---
title: "Publish rustdoc Alongside the Starlight Site in One Pages Deployment"
description: "Build cargo doc output and the Astro + Starlight documentation site in the same deploy.yml workflow and publish both under a single GitHub Pages artifact, rather than operating a second hosting surface for Rust API docs."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - documentation
  - rustdoc
  - github-pages
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0017-rfc9457-error-uri-hosted-on-pages.md
---

# ADR-0018: Publish rustdoc Alongside the Starlight Site in One Pages Deployment

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs` needed a public documentation site matching the Astro + Starlight
pattern already used by sibling repositories in the same GitHub organization
(`MIF`, `mif-docs-plugin`, `ontologies`, `research-harness-template`) — an
established, deliberately uniform deployment pattern across the org.
Separately, `mif-rs`, as a Rust workspace with 7 library crates, needed to
publish its generated Rust API documentation (`cargo doc` output).

GitHub Pages serves exactly one deployed artifact per repository — there is
no native way to run two independent Pages deployments from the same repo.

### Current Limitations

1. **One Pages deployment per repository**: GitHub Pages has no native
   mechanism for hosting two independently-built artifacts from the same
   repository under the same Pages site.
2. **Two documentation systems, one deployment slot**: the Astro + Starlight
   site's prose/guide content and `cargo doc`'s generated rustdoc HTML are
   built by entirely different toolchains, but both need to end up published
   somewhere.

## Decision Drivers

### Primary Decision Drivers

1. **Stay within GitHub's one-Pages-deployment-per-repository model**: the
   solution shall not require operating a second hosting surface.
2. **Preserve the organization's established deployment pattern**: the
   solution shall not diverge from the uniform Astro + Starlight pattern
   already used by sibling repositories, for this one repository alone.

### Secondary Decision Drivers

1. **GitHub-owned actions only**: the deploy workflow shall use only
   GitHub-owned actions. `withastro/action` is disqualified because it nests
   a `pnpm/action-setup` step not allow-listed by this organization's Actions
   policy — an existing, binding constraint already documented elsewhere in
   this workspace, not a new one invented for this decision.

## Considered Options

### Option 1: Publish only to docs.rs, skip a hosted rustdoc mirror entirely

**Description**: Rely on docs.rs for Rust API documentation and do not
publish rustdoc on this repository's own Pages site at all.

**Advantages**:

- No additional build step in `deploy.yml`; docs.rs handles the build and
  hosting independently once a crate is published.

**Disadvantages**:

- Requires every crate intended for rustdoc coverage to first be published
  to crates.io, and several of this workspace's 9 crates are internal
  implementation details not (yet) intended for standalone crates.io
  publication.
- docs.rs cannot host the prose/guide/error-reference content the Starlight
  site itself needs to serve, so this option would not actually replace the
  Starlight site — it would only fail to cover the rustdoc need.

**Risk Assessment**:

- **Technical Risk**: Low. No new infrastructure.
- **Schedule Risk**: None.
- **Ecosystem Risk**: High. Leaves internal crates with no published API
  documentation at all, and does not address the Starlight site's own
  hosting need.

### Option 2: Stand up a second GitHub Pages-like host for rustdoc specifically

**Description**: Publish rustdoc to a dedicated docs-only mirror repository,
or to a separate `gh-pages` branch deployed independently of the Astro site.

**Advantages**:

- Decouples the rustdoc build from the Astro build; each could iterate on
  its own schedule.

**Disadvantages**:

- Adds an entirely separate maintained deployment surface with its own CI
  job, secrets, and failure modes.
- Diverges from the organization's established one-repository-one-Pages-site
  convention for no clear benefit over combining the two builds.

**Risk Assessment**:

- **Technical Risk**: Medium. A second deployment surface is a second thing
  that can break.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Medium. Diverges from the org's uniform deployment
  pattern, increasing maintenance surface across the org rather than within
  a single workflow.

### Option 3: Build both in the same deploy.yml, combine into one artifact (chosen)

**Description**: Run `cargo doc --workspace --no-deps --all-features` first,
then build the Astro site (`npm ci && npm run build` in `site/`), then copy
the generated `target/doc/` output into the Astro build's own output
directory (`site/dist/rustdoc/`), then upload that single combined directory
as the one Pages artifact for this repository.

**Advantages**:

- Exactly one deployment, one URL space, one CI workflow to maintain.
- Uses only GitHub-owned actions already in use elsewhere in this repository
  (`actions/setup-node`, `dtolnay/rust-toolchain`), satisfying the
  `withastro/action` constraint without introducing a new action.
- No divergence from the organization's established Astro + Starlight
  deployment pattern.

**Disadvantages**:

- Couples the two builds in one workflow: a failure in either the `cargo
  doc` step or the Astro build step blocks the entire Pages deployment,
  including the half that succeeded.

**Risk Assessment**:

- **Technical Risk**: Low. Both build steps already exist independently
  (`cargo doc` and `npm run build`); combining them is a copy step, not new
  tooling.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low. Matches the pattern already proven by sibling
  repositories.

## Decision

We build **`cargo doc` and the Astro/Starlight site in the same
`deploy.yml` workflow**, and publish both under a single GitHub Pages
artifact.

The workflow:

- Runs `cargo doc --workspace --no-deps --all-features` first.
- Builds the Astro site (`npm ci && npm run build` in `site/`).
- Copies the generated `target/doc/` output into `site/dist/rustdoc/`.
- Uploads `site/dist` as the one Pages artifact via
  `actions/upload-pages-artifact`, deployed by a single downstream `deploy`
  job using `actions/deploy-pages`.

## Consequences

### Positive

1. **Exactly one deployment, one URL space, one CI workflow to maintain.**
2. **Cross-linking is direct**: the Starlight site's own sidebar can link
   directly to specific rustdoc pages (e.g. `/mif-rs/rustdoc/mif_core/`)
   since both live under the same deployed root.
3. **No second hosting surface**: no divergence from the organization's
   established Astro + Starlight deployment pattern.

### Negative

1. **Visually disjoint**: the two documentation systems are not visually
   unified — rustdoc's own generated HTML does not pick up the Starlight
   site's `mif-brand` CSS theming, since rustdoc has its own independent,
   non-Starlight HTML/CSS output the Astro build does not touch or restyle.

### Neutral

1. rustdoc's routes (under `/mif-rs/rustdoc/...`) occupy a separate route
   subtree from every route Starlight itself generates; there is no naming
   collision risk, since Starlight's own content-collection routing has no
   reason to ever generate a top-level route literally named `rustdoc`.

## Decision Outcome

The decision achieves its primary objective — a single Pages deployment
serving both documentation systems — measured by: a single
`.github/workflows/deploy.yml` run produces one Pages artifact containing
both the Starlight site's own pages and a `/rustdoc/` subtree with real,
browsable Rust API documentation for every one of this workspace's library
crates.

Verified by reading the actual current `deploy.yml`. The `build` job's steps
run in this exact sequence: checkout, install Rust toolchain, cache cargo
registry, **build rustdoc** (`cargo doc --workspace --no-deps --all-features`),
setup Node, install site dependencies (`npm ci`, `working-directory: site`),
**build site** (`npm run build`, `working-directory: site`), **copy rustdoc
into the site build output** (`mkdir -p site/dist/rustdoc && cp -r
target/doc/. site/dist/rustdoc/`), configure Pages, and a single **upload
Pages artifact** step (`actions/upload-pages-artifact`, `path: ./site/dist`).
A separate `deploy` job, gated on `needs: build` and
`if: github.ref == 'refs/heads/main'`, runs the single `actions/deploy-pages`
step. There is genuinely only one upload step and one deploy step — not two
independent deployments.

## Related Decisions

- [ADR-0017: RFC 9457 error URI hosted on Pages](https://modeled-information-format.github.io/mif-rs/adr/0017-rfc9457-error-uri-hosted-on-pages/) — another decision that depends on this repository's Pages deployment.

## Links

- [cargo doc](https://doc.rust-lang.org/cargo/commands/cargo-doc.html) - the command this decision runs first in `deploy.yml` to generate the workspace's rustdoc HTML output.
- [Starlight](https://starlight.astro.build/) - the Astro documentation framework producing the prose/guide site this decision publishes alongside rustdoc.
- [actions/upload-pages-artifact](https://github.com/actions/upload-pages-artifact) - the GitHub-owned action that uploads the single combined `site/dist` directory as the one Pages artifact.
- [actions/deploy-pages](https://github.com/actions/deploy-pages) - the GitHub-owned action the downstream `deploy` job uses to publish the uploaded artifact.

## More Information

- **Date**: 2026-07-03
- **Source**: this session's work, merged as pull request #11 to this
  repository; see `.github/workflows/deploy.yml` for the current, in-workflow
  implementation of this decision.

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Build sequence confirmed: checkout → install Rust toolchain → cache cargo registry → `cargo doc --workspace --no-deps --all-features` → setup Node → `npm ci` (site/) → `npm run build` (site/) → copy `target/doc/.` into `site/dist/rustdoc/` → configure Pages → single `actions/upload-pages-artifact` (`path: ./site/dist`); separate `deploy` job runs a single `actions/deploy-pages` step | .github/workflows/deploy.yml | 30-98 | accepted |

**Summary:** Decision matches the deploy workflow as merged in PR #11; exactly
one upload-pages-artifact step and one deploy-pages step exist, confirming a
single combined Pages deployment rather than two independent ones.

**Action Required:** None — this ADR documents current, already-implemented
practice.
