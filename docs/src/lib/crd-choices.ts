// Guided choices of the manifest builder (`<CrdForm>`, @weebo/operator-docs):
// several kinds accept mutually exclusive ways of doing one thing
// (an instance's credentials go to a Kubernetes Secret or to Vault). The
// schema can only say "these are all optional"; this table says which go
// together, so the form asks "how?" first and shows only the matching fields.
//
// Field names are `spec` keys. A kind not listed here gets the plain form.
// Keep them in sync with the CRDs when renaming a field.

import type { CrdChoices } from "@weebo/operator-docs/crd-form";

export const CHOICES: CrdChoices = {
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
