# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| latest  | Yes                |
| < latest | No                |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please report them via [GitHub Security Advisories](https://github.com/attested-delivery/rust-template/security/advisories/new).

### What to Include

- A description of the vulnerability
- Steps to reproduce the issue
- Potential impact
- Suggested fix (if any)

### Response Timeline

- **Acknowledgment**: Within 48 hours of the report
- **Initial assessment**: Within 1 week
- **Fix and disclosure**: Coordinated with the reporter, typically within 90 days

### Disclosure Policy

We follow responsible disclosure practices:

1. The reporter privately notifies us of the vulnerability.
2. We work together to understand and fix the issue.
3. We release a patched version.
4. The vulnerability is publicly disclosed after users have had time to update.

### Scope

This policy applies to the rust_template crate and its published artifacts. Third-party dependencies
are managed via `cargo-deny` and audited regularly through our CI pipeline.

## Security Measures

This project employs several security practices:

- **cargo-deny**: Audits dependencies for known vulnerabilities, license compliance, and banned crates
- **cargo-audit**: Checks for known security advisories in dependencies
- **Dependabot**: Automated dependency updates for security patches
- **No unsafe code**: The crate forbids `unsafe` unless explicitly justified
- **Minimal dependencies**: Only essential dependencies are included
- **SHA-pinned actions**: Every GitHub Actions `uses:` reference is pinned to a full commit SHA, enforced by a `pin-check` CI gate
- **Attested releases**: Container images are signed and attested (SLSA provenance, signature, SBOM, vulnerability report) by a centralized signer workflow and verified fail-closed before anything publishes

## Verifying Release Artifacts

Container images are signed and attested by the centralized signer workflow
`attested-delivery/.github/.github/workflows/sign-and-attest.yml` (SLSA Build L3:
the signing identity is the central workflow, not this repository).
Prerequisites: `gh` CLI authenticated, `cosign` installed.

### Resolve the digest for a tag

```bash
DIGEST=$(gh api 'users/attested-delivery/packages/container/rust-template/versions?per_page=100' \
  --jq '[.[] | select((.metadata.container.tags // []) | index("<tag>"))][0].name')
```

### SLSA provenance

`--repo` asserts where the build ran; `--signer-workflow` asserts the
signing identity. Both are required — `--repo` alone fails by design.

```bash
gh attestation verify "oci://ghcr.io/attested-delivery/rust-template@${DIGEST}" \
  --repo attested-delivery/rust-template \
  --signer-workflow attested-delivery/.github/.github/workflows/sign-and-attest.yml \
  --predicate-type https://slsa.dev/provenance/v1
```

### Keyless signature

```bash
cosign verify "ghcr.io/attested-delivery/rust-template@${DIGEST}" \
  --certificate-identity-regexp '^https://github.com/attested-delivery/\.github/\.github/workflows/sign-and-attest\.yml@.*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

### SBOM and vulnerability report attestations

```bash
cosign verify-attestation "ghcr.io/attested-delivery/rust-template@${DIGEST}" \
  --type cyclonedx \
  --certificate-identity-regexp '^https://github.com/attested-delivery/\.github/\.github/workflows/sign-and-attest\.yml@.*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
# Vulnerability report: same command with
#   --type "https://in-toto.io/attestation/vulns/v0.1"
```

### Release binaries and SBOM

Binaries attached to a GitHub Release carry SLSA build provenance and a
CycloneDX SBOM attestation, both attested by this repository's own
release workflow (no `--signer-workflow` needed). Artifact names embed
the version: `rust_template-<version>-<platform>`.

```bash
gh release download v<X.Y.Z> --repo attested-delivery/rust-template
gh attestation verify rust_template-<X.Y.Z>-linux-amd64 \
  --repo attested-delivery/rust-template
gh attestation verify rust_template-<X.Y.Z>-linux-amd64 \
  --repo attested-delivery/rust-template \
  --predicate-type https://cyclonedx.org/bom
shasum -a 256 -c rust_template-<X.Y.Z>-checksums.txt
```

A passing `gh attestation verify` is the contract: it confirms the artifact's
exact bytes are covered by a valid, keyless, digest-bound attestation from this
repo's release workflow. A single digest may legitimately carry **more than one**
attestation. Release builds are byte-reproducible, so any run that built the
same bytes yields the identical digest and therefore an additional valid
attestation. For releases tagged before dry-runs were made non-attesting, an
earlier `workflow_dispatch` dry-run from a feature branch did exactly this — its
subject is named for that branch's `Cargo.toml` version plus `-dev` (which is
**independent of the release tag**: e.g. v0.1.0's reproducible binaries also
carry a `0.4.0-dev` subject from a pre-release dry-run). That is evidence of
reproducibility, not a discrepancy — and tooling that inspects only the first
returned attestation may surface the `-dev` subject. To pin the release-specific
one, filter by the tag ref:

```bash
gh attestation verify rust_template-<X.Y.Z>-linux-amd64 \
  --repo attested-delivery/rust-template --format json \
| jq -r '.[] | select(.verificationResult.signature.certificate.buildSignerURI
        | endswith("release.yml@refs/tags/v<X.Y.Z>"))
        | .verificationResult.statement.subject[0].name'
```

### Gate-verdict attestations (seam-signed)

The SAST (CodeQL), SCA (OSV), and IaC/license (Trivy) verdicts are signed over
the published source snapshot, and the container-scan (Trivy image) verdict over
the image digest, by the central attestation seam `reusable-attest-scan.yml`.
Under SLSA Build L3 the signer identity is that central workflow — so
`--signer-workflow` is **required** and `--owner`/`--repo` alone is
insufficient. Verify one signer/predicate per command:

```bash
# SAST (CodeQL), SCA (OSV), IaC/license (Trivy) verdicts over the source snapshot.
# gh release download v<X.Y.Z> --repo attested-delivery/rust-template first.
SUBJECT=rust_template-<X.Y.Z>-source.tar.gz
for PT in sast sca iac-license; do
  gh attestation verify "$SUBJECT" --owner attested-delivery \
    --signer-workflow attested-delivery/.github/.github/workflows/reusable-attest-scan.yml \
    --predicate-type "https://attested-delivery.github.io/attestations/${PT}/v1"
done

# Container-scan (Trivy image) verdict over the image digest.
gh attestation verify "oci://ghcr.io/attested-delivery/rust-template@${DIGEST}" \
  --owner attested-delivery \
  --signer-workflow attested-delivery/.github/.github/workflows/reusable-attest-scan.yml \
  --predicate-type https://attested-delivery.github.io/attestations/container-scan/v1
```

A passing verification proves the gate **ran and recorded a verdict** bound to
the subject digest; read the predicate body for the verdict itself (signed ≠
passed).

> **Coverage by release.** The SAST (`sast/v1`) and OpenVEX verdicts were added
> to the release pipeline after the initial release. Releases tagged before that
> (notably **v0.1.0**) carry only the `sca` and `iac-license` source-snapshot
> verdicts; `gh attestation verify … --predicate-type …/sast/v1` (and the VEX
> verify) returns HTTP 404 for them — the attestation was never minted, not
> dropped. Verify only the predicates a given release actually produced.

### Published crate

The `.crate` archive served by crates.io is downloaded back from the
registry after publish, byte-compared against the locally packaged
archive, and attested — the attestation covers the bytes the registry
actually serves:

```bash
curl -fsSL -A 'release-check' \
  -O https://static.crates.io/crates/rust_template/rust_template-<X.Y.Z>.crate
gh attestation verify rust_template-<X.Y.Z>.crate --repo attested-delivery/rust-template
```
