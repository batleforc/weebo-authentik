// Mirrors the subset of a Kubernetes structural schema that
// docs/scripts/gen-crd-docs.mjs actually emits into
// docs/public/crd-schemas/*.schema.json (k8s-only extensions already
// stripped there). Not general JSON Schema — see .prompt/plan.md,
// "Risque a anticiper, pas a ignorer".

export type FieldType = "string" | "boolean" | "integer" | "number" | "object" | "array";

export interface FieldSchema {
  type?: FieldType;
  description?: string;
  nullable?: boolean;
  default?: unknown;
  enum?: string[];
  format?: string;
  items?: FieldSchema;
  properties?: Record<string, FieldSchema>;
  required?: string[];
  additionalProperties?: FieldSchema | boolean;
}

export interface CrdSchema extends FieldSchema {
  apiVersion: string;
  kind: string;
  scope: "Namespaced" | "Cluster";
}
