import * as yaml from "js-yaml";
import { findMissingRequired, pruneValue } from "./schema-utils";
import type { CrdSchema } from "./types";

export interface BuiltManifest {
  yaml: string;
  missing: string[];
}

export function buildManifest(
  schema: CrdSchema,
  name: string,
  namespace: string,
  specValue: unknown,
): BuiltManifest {
  const missing = findMissingRequired(schema, specValue).map((path) => `spec.${path}`);

  const metadata: Record<string, string> = { name };
  if (schema.scope === "Namespaced") metadata.namespace = namespace;
  if (!name) missing.unshift("metadata.name");
  if (schema.scope === "Namespaced" && !namespace) missing.push("metadata.namespace");

  const manifest = {
    apiVersion: schema.apiVersion,
    kind: schema.kind,
    metadata,
    spec: pruneValue(schema, specValue),
  };

  return {
    yaml: yaml.dump(manifest, { noRefs: true, lineWidth: 100 }),
    missing,
  };
}
