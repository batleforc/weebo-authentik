import type { FieldSchema } from "./types";

// The only "oneOf" shape our CRDs use: an object with an enum-typed
// `kind` property plus sibling properties named after each enum value
// (kube-core can't emit a real discriminated union — see
// docs/scripts/gen-crd-docs.mjs and .prompt/plan.md, decision 2). A
// variant with no sibling property (e.g. `saml`/`ldap` on
// AuthentikApplication.provider) is a schema-only stub: accepted here,
// rejected by the reconciler.
export function isDiscriminatedUnion(schema: FieldSchema): boolean {
  return schema.type === "object" && Array.isArray(schema.properties?.kind?.enum);
}

export function discriminatedVariants(schema: FieldSchema): string[] {
  return schema.properties?.kind?.enum ?? [];
}

export function variantSchema(schema: FieldSchema, variant: string): FieldSchema | undefined {
  return schema.properties?.[variant];
}

export function defaultForSchema(schema: FieldSchema): unknown {
  if (schema.default !== undefined) return structuredClone(schema.default);

  if (isDiscriminatedUnion(schema)) {
    const [firstVariant] = discriminatedVariants(schema);
    if (firstVariant === undefined) return {};
    const vSchema = variantSchema(schema, firstVariant);
    return vSchema ? { kind: firstVariant, [firstVariant]: defaultForSchema(vSchema) } : { kind: firstVariant };
  }

  switch (schema.type) {
    case "object":
      if (schema.additionalProperties) return {};
      if (schema.properties) {
        return Object.fromEntries(
          Object.entries(schema.properties).map(([key, childSchema]) => [
            key,
            defaultForSchema(childSchema),
          ]),
        );
      }
      return {};
    case "array":
      return [];
    case "boolean":
      return false;
    case "integer":
    case "number":
      return 0;
    default:
      return "";
  }
}

function isEmptyValue(value: unknown, schema: FieldSchema): boolean {
  if (value === undefined || value === null) return true;
  switch (schema.type) {
    case "string":
      return value === "";
    case "array":
      return Array.isArray(value) && value.length === 0;
    case "object":
      return typeof value === "object" && Object.keys(value as object).length === 0;
    case "boolean":
      return value === (schema.default ?? false);
    case "integer":
    case "number":
      return value === (schema.default ?? 0);
    default:
      return false;
  }
}

// Strips fields the user left at their empty/default value so the
// generated YAML only shows what was actually filled in, while always
// keeping required fields (even empty) so a missing one is visible in
// the preview rather than silently dropped.
export function pruneValue(schema: FieldSchema, value: unknown): unknown {
  if (isDiscriminatedUnion(schema)) {
    const kind = (value as Record<string, unknown> | undefined)?.kind as string | undefined;
    if (!kind) return undefined;
    const out: Record<string, unknown> = { kind };
    const vSchema = variantSchema(schema, kind);
    if (vSchema) {
      const prunedVariant = pruneValue(vSchema, (value as Record<string, unknown>)[kind]);
      if (prunedVariant !== undefined && !isEmptyValue(prunedVariant, vSchema)) {
        out[kind] = prunedVariant;
      } else if (Object.keys(vSchema.properties ?? {}).some((k) => (vSchema.required ?? []).includes(k))) {
        out[kind] = prunedVariant ?? {};
      }
    }
    return out;
  }

  if (schema.type === "object" && schema.additionalProperties) {
    const entries = Object.entries((value as Record<string, string>) ?? {}).filter(
      ([key, val]) => key !== "" && val !== "",
    );
    return entries.length > 0 ? Object.fromEntries(entries) : undefined;
  }

  if (schema.type === "object" && schema.properties) {
    const required = new Set(schema.required ?? []);
    const source = (value as Record<string, unknown>) ?? {};
    const out: Record<string, unknown> = {};
    for (const [key, childSchema] of Object.entries(schema.properties)) {
      const pruned = pruneValue(childSchema, source[key]);
      const empty = pruned === undefined || isEmptyValue(pruned, childSchema);
      if (required.has(key)) {
        out[key] = pruned ?? defaultForSchema(childSchema);
      } else if (!empty) {
        out[key] = pruned;
      }
    }
    return out;
  }

  if (schema.type === "array") {
    const items = ((value as unknown[]) ?? []).map((item) => pruneValue(schema.items ?? {}, item));
    return items;
  }

  return value;
}

// Dotted paths (relative to `spec`) of required fields still at their
// empty value — surfaced in the UI as a checklist, not a hard block:
// the apiserver remains the source of truth for validation.
export function findMissingRequired(schema: FieldSchema, value: unknown, path: string[] = []): string[] {
  const missing: string[] = [];

  if (isDiscriminatedUnion(schema)) {
    const kind = (value as Record<string, unknown> | undefined)?.kind as string | undefined;
    if (!kind) {
      missing.push([...path, "kind"].join("."));
      return missing;
    }
    const vSchema = variantSchema(schema, kind);
    if (vSchema) {
      missing.push(
        ...findMissingRequired(vSchema, (value as Record<string, unknown>)[kind], [...path, kind]),
      );
    }
    return missing;
  }

  if (schema.type === "object" && schema.properties) {
    const required = new Set(schema.required ?? []);
    const source = (value as Record<string, unknown>) ?? {};
    for (const [key, childSchema] of Object.entries(schema.properties)) {
      const childValue = source[key];
      if (required.has(key) && isEmptyValue(childValue, childSchema)) {
        missing.push([...path, key].join("."));
        continue;
      }
      missing.push(...findMissingRequired(childSchema, childValue, [...path, key]));
    }
    return missing;
  }

  if (schema.type === "array") {
    const items = (value as unknown[]) ?? [];
    items.forEach((item, index) => {
      missing.push(...findMissingRequired(schema.items ?? {}, item, [...path, String(index)]));
    });
  }

  return missing;
}
