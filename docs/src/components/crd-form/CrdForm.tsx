"use client";

import { useEffect, useMemo, useState } from "react";
import { DynamicCodeBlock } from "fumadocs-ui/components/dynamic-codeblock";
import { ObjectFields } from "./fields";
import { defaultForSchema } from "@/lib/crd-form/schema-utils";
import { buildManifest } from "@/lib/crd-form/manifest";
import { yamlPreviewTheme } from "@/lib/crd-form/yaml-theme";
import type { CrdSchema } from "@/lib/crd-form/types";

const inputClass =
  "w-full rounded-sm border border-fd-border bg-fd-background px-2.5 py-1.5 text-sm text-fd-foreground outline-none transition-colors focus:border-fd-primary focus:ring-1 focus:ring-fd-ring";

const labelClass = "font-mono text-[0.7rem] font-semibold uppercase tracking-wide text-fd-muted-foreground";

// Fetched client-side from the static file the same `docs/scripts/gen-crd-docs.mjs`
// run already writes to `docs/public/crd-schemas/` — no separate data source to keep in sync.
export function CrdForm({ kind }: { kind: string }) {
  const [schema, setSchema] = useState<CrdSchema | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [namespace, setNamespace] = useState("default");
  const [specValue, setSpecValue] = useState<unknown>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    fetch(`/crd-schemas/${kind.toLowerCase()}.schema.json`)
      .then((res) => {
        if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
        return res.json() as Promise<CrdSchema>;
      })
      .then((data) => {
        if (cancelled) return;
        setSchema(data);
        setSpecValue(defaultForSchema(data));
      })
      .catch((err: Error) => {
        if (!cancelled) setError(err.message);
      });
    return () => {
      cancelled = true;
    };
  }, [kind]);

  // `spec.name` shows up on AuthentikApplication/Group/User/Outpost as the
  // Authentik-side display name — a different concept from metadata.name
  // (the k8s object identifier), but distinct enough in name only, not in
  // UX value: merged into the single "Name" input below rather than shown
  // twice.
  const hasSpecName = schema?.properties?.name?.type === "string" && !schema.properties.name.enum;

  const effectiveSpecValue = useMemo(() => {
    if (!hasSpecName || specValue === null) return specValue;
    return { ...(specValue as Record<string, unknown>), name };
  }, [specValue, hasSpecName, name]);

  const built = useMemo(() => {
    if (!schema || effectiveSpecValue === null) return null;
    return buildManifest(schema, name, namespace, effectiveSpecValue);
  }, [schema, effectiveSpecValue, name, namespace]);

  if (error) {
    return (
      <p className="rounded-sm border border-fd-border bg-fd-card p-4 text-sm text-fd-muted-foreground">
        Could not load the schema for <code>{kind}</code>: {error}
      </p>
    );
  }

  if (!schema || specValue === null) {
    return (
      <p className="rounded-sm border border-fd-border bg-fd-card p-4 text-sm text-fd-muted-foreground">
        Loading form...
      </p>
    );
  }

  const copyYaml = async () => {
    if (!built) return;
    await navigator.clipboard.writeText(built.yaml);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const downloadYaml = () => {
    if (!built) return;
    const blob = new Blob([built.yaml], { type: "text/yaml" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${name || kind.toLowerCase()}.yaml`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <div className="flex flex-col gap-4">
        <div className="rounded-sm border border-fd-border bg-fd-card/40 p-3">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="flex flex-col gap-1">
              <label className={labelClass}>
                {hasSpecName ? "name" : "metadata.name"} <span className="text-fd-primary">*</span>
              </label>
              <input
                className={inputClass}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="my-resource"
              />
              {hasSpecName ? (
                <p className="text-xs text-fd-muted-foreground/80">
                  Used as both <code>metadata.name</code> and <code>spec.name</code> — keep it a valid
                  Kubernetes name (lowercase, alphanumeric, hyphens).
                </p>
              ) : null}
            </div>
            {schema.scope === "Namespaced" ? (
              <div className="flex flex-col gap-1">
                <label className={labelClass}>
                  metadata.namespace <span className="text-fd-primary">*</span>
                </label>
                <input
                  className={inputClass}
                  value={namespace}
                  onChange={(e) => setNamespace(e.target.value)}
                  placeholder="default"
                />
              </div>
            ) : null}
          </div>
        </div>
        <ObjectFields
          schema={schema}
          value={specValue}
          onChange={setSpecValue}
          omitKeys={hasSpecName ? ["name"] : undefined}
        />
      </div>

      <div className="flex flex-col gap-2 lg:sticky lg:top-20 lg:self-start">
        <div className="flex items-center justify-between">
          <span className={labelClass}>Generated manifest</span>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={copyYaml}
              className="cyber-glow rounded-sm border border-fd-primary/50 bg-fd-primary px-3 py-1 font-mono text-xs font-semibold uppercase tracking-wide text-fd-primary-foreground transition-transform hover:-translate-y-px active:translate-y-0"
            >
              {copied ? "copied" : "copy"}
            </button>
            <button
              type="button"
              onClick={downloadYaml}
              className="rounded-sm border border-fd-border px-3 py-1 font-mono text-xs font-semibold uppercase tracking-wide text-fd-muted-foreground transition-colors hover:border-fd-primary hover:text-fd-primary"
            >
              download
            </button>
          </div>
        </div>
        {built && built.missing.length > 0 ? (
          <div className="rounded-sm border border-fd-primary/40 bg-fd-primary/10 p-3 text-xs text-fd-foreground">
            <p className="mb-1 font-mono font-semibold uppercase tracking-wide text-fd-primary">
              Missing required fields
            </p>
            <ul className="list-inside list-disc">
              {built.missing.map((path) => (
                <li key={path}>
                  <code>{path}</code>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        <div className="yaml-preview max-h-[70vh] overflow-auto rounded-sm border border-fd-border text-xs [&_pre]:my-0">
          <DynamicCodeBlock
            lang="yaml"
            code={built?.yaml ?? ""}
            options={{ theme: yamlPreviewTheme }}
          />
        </div>
      </div>
    </div>
  );
}
