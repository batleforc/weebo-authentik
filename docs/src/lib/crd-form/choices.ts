// Guided choices for the manifest builder, in the spirit of batlehub's config
// generator: several kinds accept mutually exclusive ways of doing one thing
// (an instance's credentials go to a Kubernetes Secret or to Vault). The
// schema can only say "these are all optional"; this file says which go
// together, so the form asks "how?" first and shows only the matching fields.
//
// Field names are `spec` keys. A kind not listed here gets the plain form.

import type { FieldSchema } from "./types";

export interface ChoiceOption {
  id: string;
  label: string;
  /** One line under the segmented control, shown while this option is chosen. */
  hint: string;
  /** Spec fields shown (and kept in the manifest) only for this option. */
  fields: string[];
  /** Of those, the ones this option can't do without (marked required).
   * A dotted path (`tls.caSecretRef`) marks a nested field required too. */
  require?: string[];
  /** Spec values this option implies; their fields are never shown. */
  set?: Record<string, unknown>;
  /** Form values written in when this option is picked; unlike `set`, the
   * fields stay visible and editable (e.g. Vault preselects `backend`). */
  preset?: Record<string, unknown>;
}

export interface ChoiceGroup {
  id: string;
  label: string;
  options: ChoiceOption[];
}

export const CHOICES: Record<string, ChoiceGroup[]> = {
  AuthentikInstance: [
    {
      id: "store",
      label: "Credentials stored in",
      options: [
        {
          id: "kubernetes",
          label: "Kubernetes",
          hint: "The default: each oauth2 application's credentials go to a Secret named after it.",
          fields: [],
        },
        {
          id: "vault",
          label: "Vault",
          hint: "Written to Vault KV v2, authenticating with the operator's Kubernetes service account.",
          fields: ["secretStore"],
          require: ["secretStore", "secretStore.vault"],
          preset: { secretStore: { backend: "vault" } },
        },
      ],
    },
    {
      id: "tls",
      label: "TLS",
      options: [
        { id: "public", label: "Public CA", hint: "Verified against the platform trust store.", fields: [] },
        {
          id: "private",
          label: "Private CA",
          hint: "Also trust a CA bundle read from a Secret (tls.caSecretRef).",
          fields: ["tls"],
          require: ["tls", "tls.caSecretRef"],
        },
        {
          id: "skip",
          label: "Skip verify",
          hint: "No certificate verification at all: an escape hatch, not for production.",
          fields: [],
          set: { tls: { insecureSkipVerify: true } },
        },
      ],
    },
  ],
  AuthentikApplication: [
    {
      id: "targets",
      label: "Credentials go to",
      options: [
        {
          id: "instance",
          label: "Instance default",
          hint: "One destination, set by the AuthentikInstance's secretStore.",
          fields: [],
        },
        {
          id: "fanout",
          label: "Listed targets",
          hint: "Fanned out to every listed Kubernetes Secret or Vault path (oauth2 only).",
          fields: ["secretTargets"],
          require: ["secretTargets"],
        },
      ],
    },
  ],
  AuthentikGroup: [
    {
      id: "hierarchy",
      label: "Place",
      options: [
        { id: "root", label: "Top level", hint: "A group with no parent.", fields: [] },
        {
          id: "child",
          label: "Under a parent",
          hint: "Nested under another group, by its Authentik name (spec.name).",
          fields: ["parentRef"],
          require: ["parentRef"],
        },
      ],
    },
  ],
};

export type Selection = Record<string, string>;

export function defaultSelection(groups: ChoiceGroup[]): Selection {
  return Object.fromEntries(groups.map((g) => [g.id, g.options[0].id]));
}

function chosen(groups: ChoiceGroup[], selection: Selection): ChoiceOption[] {
  return groups.map((g) => g.options.find((o) => o.id === selection[g.id]) ?? g.options[0]);
}

/** Spec keys the form must not render: other options' fields, and implied values. */
export function hiddenKeys(groups: ChoiceGroup[], selection: Selection): string[] {
  const picked = chosen(groups, selection);
  const shown = new Set(picked.flatMap((o) => o.fields));
  const hidden = new Set<string>();
  for (const group of groups) {
    for (const option of group.options) {
      for (const key of option.fields) if (!shown.has(key)) hidden.add(key);
      for (const key of Object.keys(option.set ?? {})) hidden.add(key);
    }
  }
  return [...hidden];
}

/** The spec as the manifest should carry it: hidden fields dropped, implied values set. */
export function applySelection(
  groups: ChoiceGroup[],
  selection: Selection,
  spec: Record<string, unknown>,
): Record<string, unknown> {
  const out = { ...spec };
  for (const key of hiddenKeys(groups, selection)) delete out[key];
  for (const option of chosen(groups, selection)) Object.assign(out, option.set ?? {});
  return out;
}

/** The schema with the chosen options' must-have fields marked required. */
export function withChoiceRequirements<S extends FieldSchema>(
  groups: ChoiceGroup[],
  selection: Selection,
  schema: S,
): S {
  const extra = chosen(groups, selection).flatMap((o) => o.require ?? []);
  return extra.reduce((acc, path) => requirePath(acc, path.split(".")), schema);
}

// Copy-on-write down `path`, adding its last segment to its parent's `required`.
function requirePath<S extends FieldSchema>(schema: S, path: string[]): S {
  const [head, ...rest] = path;
  if (rest.length === 0) {
    return { ...schema, required: [...new Set([...(schema.required ?? []), head])] };
  }
  const child = schema.properties?.[head];
  if (!child) return schema;
  return { ...schema, properties: { ...schema.properties, [head]: requirePath(child, rest) } };
}

/** Deep-merge `preset` into a form value: objects merge, anything else replaces. */
export function withPreset(value: unknown, preset: Record<string, unknown>): Record<string, unknown> {
  const base = value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
  const out = { ...base };
  for (const [key, v] of Object.entries(preset)) {
    out[key] =
      v && typeof v === "object" && !Array.isArray(v)
        ? withPreset(base[key], v as Record<string, unknown>)
        : v;
  }
  return out;
}

/** The spec fields the chosen options are about, shown before the rest. */
export function chosenFields(groups: ChoiceGroup[], selection: Selection): string[] {
  return chosen(groups, selection).flatMap((o) => o.fields);
}
