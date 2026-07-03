import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightLlmsTxt from "starlight-llms-txt";
import astroMermaid from "astro-mermaid";

// mif-rs documentation site — Astro + Starlight, modeled on the org's
// doc-site (same llms.txt + Mermaid + mif-brand wiring). Deployed to project
// Pages at /mif-rs. The error-reference catalog (docs/references/errors/) is
// sourced via the src/content/docs/references symlink (see src/content.config.ts) —
// its markdown is also mif_problem::ERROR_TYPE_BASE_URI's dereference target,
// not just site content.
export default defineConfig({
  site: "https://modeled-information-format.github.io",
  base: "/mif-rs",
  // The references/ subtree is symlinked into src/content/docs. Without
  // preserveSymlinks, Vite resolves a symlinked file to its real repo-root
  // path, where the site's own node_modules can't be found during dev/build.
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
          href: "https://github.com/modeled-information-format/mif-rs",
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
          label: "MIF ecosystem",
          items: [
            { label: "MIF home", link: "https://modeled-information-format.github.io/" },
            { label: "Ecosystem docs", link: "https://modeled-information-format.github.io/docs/" },
            { label: "Specification (mif-spec.dev)", link: "https://mif-spec.dev" },
          ],
        },
      ],
    }),
  ],
});
