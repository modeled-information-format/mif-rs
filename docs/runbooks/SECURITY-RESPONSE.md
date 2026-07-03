---
id: how-to-respond-to-security-vulnerability
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/security
title: How to Respond to a Security Vulnerability Report in mif-rs
tags:
  - how-to
  - security
  - incident-response
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
relationships:
  - type: relates-to
    target: SECURITY.md
  - type: relates-to
    target: docs/runbooks/RELEASING.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Respond to a Security Vulnerability Report in mif-rs
  entity_type: how-to-guide
---

# How to Respond to a Security Vulnerability Report in mif-rs

Triage, fix, and coordinate disclosure of a security vulnerability reported
against `mif-rs`, from acknowledgment through a published advisory. Based on
the project's [Security Policy](../../SECURITY.md).

## Prerequisites

- Maintainer access to `modeled-information-format/mif-rs`, including
  [GitHub Security Advisories](https://github.com/modeled-information-format/mif-rs/security/advisories).
- `gh` CLI authenticated with permissions to manage advisories and releases.

## Step 1 — Acknowledge the report within 48 hours

Vulnerability reports arrive through GitHub Security Advisories, not public
issues. If someone reports one publicly, ask them to re-submit privately and
treat the issue as already disclosed when setting timelines.

Reply to the advisory draft with:

- Confirmation the report was received.
- An estimated timeline for assessment.
- Any immediate clarifying questions.

Per SECURITY.md, reporters are asked for a description, reproduction steps,
potential impact, and a suggested fix (if any).

## Step 2 — Assess severity and impact

Use a simplified severity scale to set the response deadline:

| Severity | Criteria | Fix target |
|---|---|---|
| Critical | RCE, data exfiltration, supply-chain compromise | 48 hours |
| High | Privilege escalation, DoS, significant data exposure | 1 week |
| Medium | Limited impact, needs uncommon config or local access | 30 days |
| Low | Minimal impact, theoretical or defense-in-depth | 90 days |

Determine scope:

- [ ] Is the vulnerability in this workspace's own code (`mif-core`,
      `mif-schema`, `mif-ontology`, `mif-problem`, `mif-frontmatter`,
      `mif-embed`, `mif-store`, `mif-cli`, `mif-mcp`), or in a dependency?
- [ ] Which published versions are affected?
- [ ] What's the attack vector — network, local, physical?
- [ ] Any evidence of exploitation in the wild?
- [ ] Does it affect a crates.io crate, a release binary, the container
      image, or all of them?

Record the assessment (CVSS score if applicable, affected versions/
components, exploitation prerequisites, mitigating factors) in the advisory
draft.

## Step 3 — Develop the fix privately

Use the advisory's **"Start a temporary private fork"** feature so the fix
isn't visible before disclosure:

```bash
# GitHub provides the private fork URL from the advisory draft
git clone <private-fork-url>
cd mif-rs
git checkout -b security/fix-<advisory-id>

# Apply the fix, then run the full local check suite:
just check
```

`just check` runs `fmt-check`, `lint`, `test`, `doc-build`, and `deny` — the
same gates as `ci-checks.yml`'s `fmt`, `clippy`, `test`, `doc`, and `deny`
jobs. Also run the advisory scan directly:

```bash
cargo audit --deny warnings
```

At least one other maintainer should review the fix in the private fork.
Confirm it addresses the root cause (not just the symptom), add a regression
test that doesn't reveal exploit details in its name or comments, and confirm
no new issues were introduced.

## Step 4 — Prepare release materials

While the fix is in review:

- [ ] Determine the new version (typically a PATCH bump — see
      [RELEASING.md](RELEASING.md) for SemVer policy).
- [ ] Draft release notes describing the fix without revealing exploit
      details before coordinated disclosure.
- [ ] Request a CVE ID if severity warrants it.
- [ ] Agree disclosure timing with the reporter.

## Step 5 — Merge and ship the fix

```bash
# Merge the private-fork fix into main (GitHub provides a merge button
# in the advisory UI), then bump the version:
git pull origin main
# Update version = "X.Y.(Z+1)" in Cargo.toml
cargo check   # regenerates Cargo.lock — never hand-edit it
git add Cargo.toml Cargo.lock
git commit -m "fix: address security vulnerability (GHSA-XXXX-XXXX-XXXX)"
git push origin main

git tag -a vX.Y.(Z+1) -m "Security release vX.Y.(Z+1)"
git push origin vX.Y.(Z+1)
```

The tag push triggers the standard release pipeline — see
[RELEASING.md](RELEASING.md) for the full workflow chain and verification
steps.

Verify deployment before notifying anyone:

- [ ] GitHub Release created with binaries and attestations.
- [ ] Container image pushed to `ghcr.io/modeled-information-format/mif-rs`.
- [ ] Affected crates updated on crates.io.
- [ ] Binaries pass a smoke test on at least one platform.

If the vulnerable version was already published, yank it:

```bash
cargo yank --version X.Y.Z -p <affected-crate>
```

## Step 6 — Publish the advisory and notify users

1. Go to the advisory draft at
   https://github.com/modeled-information-format/mif-rs/security/advisories.
2. Fill in affected products (`modeled-information-format/mif-rs`), affected
   version range, patched version, severity, and CWE.
3. Click **"Publish advisory"**.

Publishing makes the advisory public, notifies watchers, adds it to the
GitHub Advisory Database, and triggers Dependabot alerts for downstream
consumers.

Recommended disclosure timeline: fix published day 0, reporter notified
within 0-3 days, advisory published 14+ days later to give users time to
update, CVE published alongside if requested.

## Step 7 — Run a post-incident review

- [ ] **Root cause** — what introduced the vulnerability?
- [ ] **Detection gap** — why didn't existing tooling catch it? Should a new
      clippy lint, a new `deny.toml` ban, or a new CI gate be added?
- [ ] **Documentation** — update SECURITY.md if the process itself needs to
      change.
- [ ] **Timeline** — were the response deadlines from Step 2 met?

The vulnerability is now patched, disclosed, and the process gap (if any) is
tracked as a follow-up.

## Reference: automated security tooling already in place

These run continuously regardless of an active incident — useful context when
assessing "why didn't this get caught earlier."

| Tool | Workflow | Trigger | What it checks |
|---|---|---|---|
| cargo-deny | `ci-checks.yml` (`deny` job) | Every push/PR | Advisories, licenses, banned crates (`openssl`, `atty`), sources |
| cargo-audit | `security-audit.yml` | Daily 00:00 UTC; push touching `Cargo.toml`/`Cargo.lock`; manual | RustSec advisory database |
| Gitleaks + TruffleHog | `secrets-scan.yml` (calls the org's `reusable-secrets.yml`) | Every push/PR; manual | Committed secrets, API keys, tokens |
| CodeQL (SAST) | `quality-gates.yml` (`sast` job, calls `reusable-sast-codeql.yml`) | Push to `main`, every PR, weekly Monday 06:00 UTC | Static analysis, Rust code patterns |
| OSV-Scanner (SCA) | `quality-gates.yml` (`sca` job, calls `reusable-sca-osv.yml`) | Push to `main`, every PR, weekly | Known vulnerabilities against `Cargo.lock`, `fail-on-severity: high` |
| Trivy (IaC + license) | `quality-gates.yml` (`trivy` job, `scan-iac: true`) | Push to `main`, every PR, weekly | Dockerfile/manifest misconfig, license issues |
| Trivy (container image) | `pipeline.yml` (`gate-image` job) | Push to `main`/tags, once the container chain is armed (`publish != false`) | Container image vulnerabilities, bound to the image digest via attestation |
| OpenSSF Scorecard | `quality-gates.yml` (`posture` job, calls `reusable-scorecard.yml`) | Push to `main`; weekly | Supply-chain posture score |
| pin-check | `pipeline.yml` (`pin-check` job) | Every push/PR | Every `uses:` is pinned to a full commit SHA |

All SAST/SCA/IaC-license/container-scan verdicts land in the repo's code
scanning tab and are additionally signed as attestations bound to the release
artifact digest — see SECURITY.md § Verifying Release Artifacts for the
`gh attestation verify` commands.

## Quick Reference

| Action | Command / Location |
|---|---|
| View security advisories | https://github.com/modeled-information-format/mif-rs/security/advisories |
| Create new advisory | https://github.com/modeled-information-format/mif-rs/security/advisories/new |
| Run cargo-audit locally | `cargo audit --deny warnings` |
| Run cargo-deny locally | `cargo deny check` |
| Run the full local gate suite | `just check` |
| View Dependabot alerts | https://github.com/modeled-information-format/mif-rs/security/dependabot |
| View code scanning alerts | https://github.com/modeled-information-format/mif-rs/security/code-scanning |
| Yank a published crate version | `cargo yank --version X.Y.Z -p <crate>` |
