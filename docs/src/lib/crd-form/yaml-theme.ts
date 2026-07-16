import { createCssVariablesTheme } from "shiki";

// Renders through `--shiki-*` CSS variables instead of a bundled Shiki
// theme — the generated manifest is highlighted client-side from live
// form state, so it can't go through the static MDX build-time pipeline
// the rest of the site's code fences use.
//
// `variableDefaults` bakes the Monofolio colors in as `var(--shiki-x,
// <default>)` fallbacks, so highlighting still renders correctly even if
// the `.yaml-preview` wrapper class in global.css never reaches these
// spans (e.g. inherited through a portal, or the class name changes) —
// `.yaml-preview` there is now an optional override, not a hard
// dependency.
export const yamlPreviewTheme = createCssVariablesTheme({
  name: "monofolio-css-variables",
  variablePrefix: "--shiki-",
  fontStyle: true,
  variableDefaults: {
    foreground: "oklch(0.93 0.018 25)",
    background: "transparent",
    "token-keyword": "oklch(0.65 0.26 25)",
    "token-string": "oklch(0.72 0.14 52)",
    "token-string-expression": "oklch(0.72 0.14 52)",
    "token-constant": "oklch(0.72 0.14 52)",
    "token-comment": "oklch(0.58 0.04 30)",
    "token-punctuation": "oklch(0.58 0.04 30)",
    "token-link": "oklch(0.65 0.26 25)",
  },
});
