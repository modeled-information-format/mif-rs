import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightLlmsTxt from "starlight-llms-txt";
import astroMermaid from "astro-mermaid";

// mif-rs documentation site — Astro + Starlight, modeled on the org's
// doc-site (same llms.txt + Mermaid + mif-brand wiring). Deployed to project
// Pages at /mif-rs. The error-reference catalog (docs/references/errors/),
// the ADR log (docs/adr/), docs/DEPLOYMENT.md, docs/runbooks/, and
// docs/security/ are sourced via src/content/docs symlinks (see
// src/content.config.ts) — the error catalog's markdown is also
// mif_problem::ERROR_TYPE_BASE_URI's dereference target, not just site content.
const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const githubRepo = "https://github.com/modeled-information-format/mif-rs";

// Driven by `cargo metadata`, not a hand-written list — mirrors this repo's
// own publish.yml/release.yml convention of resolving workspace members
// dynamically so a crate add/remove/rename needs no manual sidebar edit.
// Only crates with a README.md get a sidebar entry, so a future crate
// without one is silently skipped rather than linking to a 404.
function crateSidebarItems() {
  let raw;
  try {
    raw = execFileSync(
      "cargo",
      ["metadata", "--no-deps", "--format-version", "1"],
      { cwd: repoRoot, encoding: "utf-8" },
    );
  } catch (cause) {
    throw new Error(
      "Failed to run `cargo metadata` while building the Crates sidebar — " +
        "is a Rust toolchain installed and on PATH?",
      { cause },
    );
  }
  const { packages } = JSON.parse(raw);
  return packages
    .map((pkg) => pkg.name)
    .filter((name) => existsSync(join(repoRoot, "crates", name, "README.md")))
    .sort()
    .map((name) => ({
      label: name,
      link: `${githubRepo}/blob/main/crates/${name}/README.md`,
    }));
}

export default defineConfig({
  site: "https://modeled-information-format.github.io",
  base: "/mif-rs",
  // The references/, adr/, and deployment.md paths are symlinked into
  // src/content/docs. Without preserveSymlinks, Vite resolves a symlinked
  // file to its real repo-root path, where the site's own node_modules can't
  // be found during dev/build.
  vite: { resolve: { preserveSymlinks: true } },
  integrations: [
    astroMermaid(),
    starlight({
      plugins: [starlightLlmsTxt()],
      title: "mif-rs",
      customCss: ["./src/styles/mif-brand.css"],
      logo: {
        light: "./src/assets/logo-light.svg",
        dark: "./src/assets/logo-dark.svg",
        replacesTitle: true,
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: githubRepo,
        },
      ],
      sidebar: [
        {
          label: "Reference",
          items: [
            { label: "Error catalog", link: "/references/errors/" },
            { label: "Rust API docs (rustdoc)", link: "/rustdoc/mif_core/" },
          ],
        },
        {
          label: "Architecture Decision Records",
          items: [{ autogenerate: { directory: "adr" } }],
        },
        {
          label: "Runbooks",
          items: [
            { label: "Deploying a release", link: "/deployment/" },
            { label: "Releasing mif-rs", link: "/runbooks/releasing/" },
            { label: "Troubleshooting a failing CI run", link: "/runbooks/ci-troubleshooting/" },
            { label: "Managing dependency updates", link: "/runbooks/dependency-updates/" },
            { label: "Responding to a security report", link: "/runbooks/security-response/" },
          ],
        },
        {
          label: "Security",
          items: [
            { label: "Signed releases & SLSA provenance", link: "/security/signed-releases/" },
            { label: "Attested delivery, end to end", link: "/security/attested-delivery/" },
          ],
        },
        {
          // Rendered as external GitHub pages (not symlinked into the
          // content collection like the ADRs/deployment guide) so these
          // stay the single crates.io-facing README, with no duplicated or
          // drifting copy inside the site.
          label: "Crates",
          items: crateSidebarItems(),
        },
        {
          label: "MIF ecosystem",
          items: [
            { label: "MIF home", link: "https://modeled-information-format.github.io/" },
            { label: "Ecosystem docs", link: "https://modeled-information-format.github.io/docs/" },
            { label: "Research harness", link: "https://modeled-information-format.github.io/research-harness-template/" },
            { label: "Ontology corpus", link: "https://modeled-information-format.github.io/ontologies/" },
            { label: "mif-docs plugin", link: "https://modeled-information-format.github.io/mif-docs-plugin/" },
            { label: "Plugin marketplace", link: "https://modeled-information-format.github.io/claude-code-plugins/" },
            { label: "Structured MADR", link: "https://smadr.dev/" },
            { label: "Specification (mif-spec.dev)", link: "https://mif-spec.dev/" },
          ],
        },
      ],
    }),
  ],
});
