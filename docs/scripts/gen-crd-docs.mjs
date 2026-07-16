#!/usr/bin/env node
// Generates docs/content/docs/crds/*.mdx (field reference tables) and
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

const files = fs.readdirSync(crdDir).filter((f) => f.endsWith(".yaml"));

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
  const paragraph = description.replace(/\s*\n\s*/g, " ");
  const firstSentenceEnd = paragraph.indexOf(". ");
  const shortDescription =
    firstSentenceEnd === -1 ? paragraph : paragraph.slice(0, firstSentenceEnd + 1);
  const table = renderFieldsTable(specSchema.properties, specSchema.required);

  const frontmatter = yaml.dump({ title: kind, description: shortDescription }).trim();
  const mdx = `---\n${frontmatter}\n---\n\n${description}\n\n## Spec fields\n\n${table}\n\n## Build a manifest\n\nFill in the fields below to generate a ready-to-apply \`${kind}\` YAML manifest.\n\n<CrdForm kind="${kind}" />\n`;
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
}

console.log(`Generated CRD docs + schemas for ${files.length} CRDs.`);
