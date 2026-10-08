#!/usr/bin/env node
// Generates docs/content/docs/crds/*.mdx (facts, example, field reference
// tables), the section's index.mdx and meta.json, and
// docs/public/crd-schemas/*.schema.json from deploy/crd/*.yaml — the
// same CRD YAML `crdgen` already produces via `CustomResourceExt::crd()`.
// Never hand-edit the generated files; re-run via `task recu`.
// See .prompt/plan.md, "Documentation" / "Pipeline de generation".
//
// Field `description`s trace back to the Rust `#[schemars(description =
// ...)]`/doc-comments on each `*Spec` struct — a CR field with no
// description here means the Rust source is missing one, which is a
// useful signal in review, not a bug in this script.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as yaml from "js-yaml";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "../..");
const crdDir = path.join(repoRoot, "deploy/crd");
const mdxOutDir = path.join(__dirname, "../content/docs/crds");
const schemaOutDir = path.join(__dirname, "../public/crd-schemas");

fs.mkdirSync(mdxOutDir, { recursive: true });
fs.mkdirSync(schemaOutDir, { recursive: true });

// A Kubernetes structural schema isn't plain JSON Schema — these
// extensions have no meaning for a generic client-side form/validator
// and would only confuse it. Stripped here (this reference page's own
// consumption), not from `deploy/crd/*.yaml` itself. See
// .prompt/plan.md, "Risque a anticiper, pas a ignorer" — a fuller
// JSON-Schema-compatibility pass belongs to the interactive form
// (deferred), not this generator.
const K8S_EXTENSION_KEYS = new Set([
  "x-kubernetes-preserve-unknown-fields",
  "x-kubernetes-int-or-string",
]);

function stripK8sExtensions(node) {
  if (Array.isArray(node)) {
    return node.map(stripK8sExtensions);
  }
  if (node && typeof node === "object") {
    const out = {};
    for (const [key, value] of Object.entries(node)) {
      if (K8S_EXTENSION_KEYS.has(key)) continue;
      out[key] = stripK8sExtensions(value);
    }
    return out;
  }
  return node;
}

function fieldType(schema) {
  if (!schema) return "object";
  if (schema.type === "array") {
    return `${fieldType(schema.items)}[]`;
  }
  if (schema.type === "object" && !schema.properties) {
    return "object";
  }
  return schema.type ?? "object";
}

function escapeCell(text) {
  return text.replace(/\|/g, "\\|").replace(/\r?\n/g, " ");
}

function renderFieldsTable(properties, required) {
  const requiredSet = new Set(required ?? []);
  const rows = Object.entries(properties ?? {}).map(([name, schema]) => {
    const type = fieldType(schema);
    const isRequired = requiredSet.has(name) ? "yes" : "no";
    const def =
      schema.default !== undefined ? `\`${JSON.stringify(schema.default)}\`` : "";
    const description = escapeCell((schema.description ?? "").trim());
    return `| \`${name}\` | \`${type}\` | ${isRequired} | ${def} | ${description} |`;
  });
  return [
    "| Field | Type | Required | Default | Description |",
    "| --- | --- | --- | --- | --- |",
    ...rows,
  ].join("\n");
}

// The guide that explains each kind, linked from its reference page.
const GUIDES = {
  AuthentikInstance: ["Connect an AuthentikInstance", "/docs/guides/connect-instance"],
  AuthentikNamespacePolicy: ["The allow-list", "/docs/guides/allow-list"],
  AuthentikGroup: ["First application walkthrough", "/docs/guides/first-application#1-create-a-group"],
  AuthentikApplication: [
    "First application walkthrough",
    "/docs/guides/first-application#2-create-the-application-oauth2-provider",
  ],
  AuthentikAccessPolicy: [
    "First application walkthrough",
    "/docs/guides/first-application#3-bind-the-group-to-the-application",
  ],
};

// A minimal, realistic `spec` per kind. Checked against the schema below
// (every required field present, every key a real field), so a renamed
// field fails generation instead of shipping a stale example.
const EXAMPLES = {
  AuthentikInstance: {
    url: "https://login.example.com",
    tokenSecretRef: { name: "authentik-api-token", namespace: "weebo-authentik", key: "token" },
  },
  AuthentikNamespacePolicy: {
    rules: [
      {
        namespaces: ["team-a"],
        allowedKinds: ["AuthentikApplication", "AuthentikAccessPolicy"],
        effect: "Allow",
      },
    ],
  },
  AuthentikGroup: { name: "weebo_user", parentRef: "weebo_base" },
  AuthentikUser: {
    username: "alice",
    name: "Alice Example",
    email: "alice@example.com",
    groupRefs: ["weebo_user"],
  },
  AuthentikApplication: {
    instanceRef: "main",
    name: "Harbor",
    slug: "harbor",
    provider: {
      kind: "oauth2",
      oauth2: {
        authorizationFlow: "default-provider-authorization-implicit-consent",
        invalidationFlow: "default-provider-invalidation-flow",
        allowedRedirectUris: [
          { matchingMode: "strict", url: "https://harbor.example.com/c/oidc/callback" },
        ],
      },
    },
  },
  AuthentikAccessPolicy: { applicationRef: "harbor", groupRef: "weebo-user" },
  AuthentikScopeMapping: {
    name: "weebo groups",
    scopeName: "groups",
    expression: 'return {"groups": [g.name for g in request.user.ak_groups.all()]}\n',
  },
  AuthentikBrand: {
    domain: "login.example.com",
    brandingTitle: "Weebo",
    default: true,
    flowAuthentication: "default-authentication-flow",
  },
  AuthentikFlow: {
    name: "Device code",
    slug: "device-code",
    title: "Sign in to your device",
    designation: "stage_configuration",
  },
  AuthentikOutpost: { name: "proxy-outpost", type: "proxy" },
};

function checkExample(kind, example, specSchema) {
  if (!example) throw new Error(`no example spec for ${kind} in gen-crd-docs.mjs`);
  const props = specSchema.properties ?? {};
  for (const key of Object.keys(example)) {
    if (!(key in props)) throw new Error(`${kind} example: unknown field spec.${key}`);
  }
  for (const key of specSchema.required ?? []) {
    if (!(key in example)) throw new Error(`${kind} example: missing required spec.${key}`);
  }
}

function exampleManifest(kind, apiVersion, scope, example) {
  const metadata = { name: `my-${kind.replace(/^Authentik/, "").toLowerCase()}` };
  if (scope === "Namespaced") metadata.namespace = "team-a";
  return yaml
    .dump({ apiVersion, kind, metadata, spec: example }, { lineWidth: 100 })
    .trim();
}

const files = fs.readdirSync(crdDir).filter((f) => f.endsWith(".yaml"));

// Collected while emitting each CRD, then written as crd-schemas/index.json —
// the kind picker on the standalone "Build a manifest" page reads this to list
// every CRD without a second data source to keep in sync.
const index = [];

for (const file of files) {
  const crd = yaml.load(fs.readFileSync(path.join(crdDir, file), "utf8"));
  const kind = crd.spec.names.kind;
  const kindLower = crd.spec.names.singular;
  const version = crd.spec.versions[0];
  const specSchema = version.schema.openAPIV3Schema.properties.spec;
  const apiVersion = `${crd.spec.group}/${version.name}`;
  const scope = crd.spec.scope;

  const description = (specSchema.description ?? `${kind} spec.`).trim();
  // The source doc-comment is hand-wrapped at ~80 columns in Rust, not
  // at sentence boundaries — join it into one paragraph first, then
  // take the first sentence, so the frontmatter description doesn't
  // arbitrarily cut off mid-sentence at wherever the Rust source
  // happened to wrap.
  // Only the first paragraph counts, so a section that follows it never
  // bleeds into the summary; a lead-in colon becomes a period.
  const paragraph = description.split(/\n\s*\n/)[0].replace(/\s*\n\s*/g, " ");
  // "i.e." / "e.g." are not sentence ends.
  const firstSentenceEnd = paragraph.search(/(?<!\b(?:i\.e|e\.g))\.\s/);
  const shortDescription = (
    firstSentenceEnd === -1 ? paragraph : paragraph.slice(0, firstSentenceEnd + 1)
  ).replace(/:$/, ".");
  const table = renderFieldsTable(specSchema.properties, specSchema.required);

  const shortNames = crd.spec.names.shortNames ?? [];
  const [guideTitle, guideHref] = GUIDES[kind] ?? [];
  checkExample(kind, EXAMPLES[kind], specSchema);
  const example = exampleManifest(kind, apiVersion, scope, EXAMPLES[kind]);

  const facts = [
    `- **API version:** \`${apiVersion}\``,
    `- **Scope:** ${scope === "Namespaced" ? "Namespaced" : "Cluster"}`,
    `- **Short names:** ${shortNames.map((n) => `\`${n}\``).join(", ") || "none"}`,
    ...(guideTitle ? [`- **Guide:** [${guideTitle}](${guideHref})`] : []),
  ].join("\n");

  const frontmatter = yaml
    // `full`: the page drops its TOC and widens to fit the manifest builder's
    // form and preview side by side; prose keeps its 68ch measure regardless.
    // Backticks are stripped: the description renders as plain text, not MDX.
    .dump({ title: kind, description: shortDescription.replace(/`|\*\*/g, ""), full: true })
    .trim();
  const mdx = `---\n${frontmatter}\n---\n\n${facts}\n\n${description}\n\n## Example\n\n\`\`\`yaml\n${example}\n\`\`\`\n\n## Spec fields\n\n${table}\n\n## Build a manifest\n\nAnswer the questions and fill in the fields: the YAML preview updates as you type, ready to copy or download and \`kubectl apply\`.\n\n<CrdForm kind="${kind}" />\n`;
  fs.writeFileSync(path.join(mdxOutDir, `${kindLower}.mdx`), mdx);

  // apiVersion/kind/scope aren't JSON Schema keywords — added here purely
  // for the client-side form (CrdForm) to assemble a full manifest
  // (apiVersion + kind + metadata.namespace-or-not) without a second
  // fetch. Consumers that want strict JSON Schema should ignore them.
  const cleanSchema = { apiVersion, kind, scope, ...stripK8sExtensions(specSchema) };
  fs.writeFileSync(
    path.join(schemaOutDir, `${kindLower}.schema.json`),
    `${JSON.stringify(cleanSchema, null, 2)}\n`,
  );

  index.push({ kind, singular: kindLower, scope, shortNames, shortDescription });
}

// Sidebar order follows the order you'd create things in, not the alphabet.
const ORDER = [
  "AuthentikInstance",
  "AuthentikNamespacePolicy",
  "AuthentikGroup",
  "AuthentikUser",
  "AuthentikApplication",
  "AuthentikAccessPolicy",
  "AuthentikScopeMapping",
  "AuthentikBrand",
  "AuthentikFlow",
  "AuthentikOutpost",
];
const rank = (kind) => (ORDER.includes(kind) ? ORDER.indexOf(kind) : ORDER.length);
index.sort((a, b) => rank(a.kind) - rank(b.kind) || a.kind.localeCompare(b.kind));

const rows = index.map(
  (k) =>
    `| [\`${k.kind}\`](/docs/crds/${k.singular}) | ${k.scope === "Namespaced" ? "Namespaced" : "Cluster"} | ${escapeCell(k.shortDescription)} |`,
);
const indexMdx = `---
title: CRD reference
description: Every kind the operator adds, its fields, and an interactive manifest builder.
---

Each page lists a kind's \`spec\` fields, shows a minimal example, and has a
form that builds a ready-to-apply manifest. These pages are generated from
the CRDs themselves, so they always match the installed version.

| Kind | Scope | Description |
| --- | --- | --- |
${rows.join("\n")}
`;
fs.writeFileSync(path.join(mdxOutDir, "index.mdx"), indexMdx);
fs.writeFileSync(
  path.join(mdxOutDir, "meta.json"),
  `${JSON.stringify({ title: "CRD reference", pages: ["index", ...index.map((k) => k.singular)] }, null, 2)}\n`,
);

fs.writeFileSync(
  path.join(schemaOutDir, "index.json"),
  `${JSON.stringify(index.map(({ kind, singular, scope }) => ({ kind, singular, scope })), null, 2)}\n`,
);

console.log(`Generated CRD docs + schemas for ${files.length} CRDs.`);
