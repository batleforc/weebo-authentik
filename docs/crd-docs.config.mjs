// Input of `weebo-crd-docs` (@weebo/operator-docs), run by `task recu` /
// `npm run gen:crds`: what the CRD reference pages say beyond the CRDs
// themselves. Paths default to the expected tree (../deploy/crd →
// content/docs/crds + public/crd-schemas).
import { defineConfig } from "@weebo/operator-docs/crd-docs";

export default defineConfig({
  // `AuthentikApplication` → example named `my-application`.
  kindPrefix: "Authentik",
  // Sidebar order follows the order you'd create things in, not the alphabet.
  order: [
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
  ],
  // The guide that explains each kind, linked from its reference page.
  guides: {
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
  },
  // A minimal, realistic `spec` per kind, checked against the schema (every
  // required field present, every key a real field): a renamed field fails
  // generation instead of shipping a stale example.
  examples: {
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
  },
  // The operator reference pages (content/docs/reference/), generated from
  // the Rust sources: reason codes (crates/domain/src/error.rs), Events,
  // metrics & health, resource budget.
  reference: {
    metricsPrefix: "weebo_authentik",
    // `task perf`'s report (target/perf/weebo-authentik.json), copied here.
    perfReport: "perf/weebo-authentik.json",
    perfNote: "kit v0.6.0, 2026-10-11",
  },
});
