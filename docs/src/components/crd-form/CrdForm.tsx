"use client";

import { useEffect, useMemo, useState } from "react";
import { DynamicCodeBlock } from "fumadocs-ui/components/dynamic-codeblock";
import { ChoicePicker } from "./ChoicePicker";
import { ObjectFields } from "./fields";
import { defaultForSchema } from "@/lib/crd-form/schema-utils";
import { buildManifest } from "@/lib/crd-form/manifest";
import { yamlPreviewTheme } from "@/lib/crd-form/yaml-theme";
import {
  applySelection,
  CHOICES,
  chosenFields,
  defaultSelection,
  hiddenKeys,
  withChoiceRequirements,
  withPreset,
  type ChoiceGroup,
} from "@/lib/crd-form/choices";
import type { CrdSchema } from "@/lib/crd-form/types";
import { basePath } from "@/lib/shared";

const NO_CHOICES: ChoiceGroup[] = [];

const inputClass =
  "w-full border border-(--rule-strong) bg-(--ground-sunk) px-3 py-2 text-[15px] text-fd-foreground outline-none placeholder:text-fd-muted-foreground focus:border-(--accent)";

const labelClass = "text-[12px] font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground";

// Fetched client-side from the static file the same `docs/scripts/gen-crd-docs.mjs`
// run already writes to `docs/public/crd-schemas/` — no separate data source to keep in sync.
export function CrdForm({ kind }: { kind: string }) {
  const [schema, setSchema] = useState<CrdSchema | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [namespace, setNamespace] = useState("default");
  const [specValue, setSpecValue] = useState<unknown>(null);
  const [copied, setCopied] = useState(false);
  const groups = CHOICES[kind] ?? NO_CHOICES;
  const [selection, setSelection] = useState(() => defaultSelection(groups));

  useEffect(() => {
    let cancelled = false;
    fetch(`${basePath}/crd-schemas/${kind.toLowerCase()}.schema.json`)
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

  // What the manifest carries: the choices applied (other options' fields
  // dropped, implied values set) and the shared name folded in.
  const effectiveSpecValue = useMemo(() => {
    if (specValue === null) return specValue;
    const spec = applySelection(groups, selection, specValue as Record<string, unknown>);
    return hasSpecName ? { ...spec, name } : spec;
  }, [specValue, hasSpecName, name, groups, selection]);

  // The chosen options' must-have fields count as required, for the
  // asterisks and for the "still to fill in" list alike.
  const formSchema = useMemo(
    () => (schema ? withChoiceRequirements(groups, selection, schema) : null),
    [schema, groups, selection],
  );

  const built = useMemo(() => {
    if (!formSchema || effectiveSpecValue === null) return null;
    return buildManifest(formSchema, name, namespace, effectiveSpecValue);
  }, [formSchema, effectiveSpecValue, name, namespace]);

  const choose = (groupId: string, optionId: string) => {
    setSelection({ ...selection, [groupId]: optionId });
    const preset = groups.find((g) => g.id === groupId)?.options.find((o) => o.id === optionId)?.preset;
    if (preset) setSpecValue((prev: unknown) => withPreset(prev, preset));
  };

  if (error) {
    return (
      <p className="border border-(--rule-strong) px-6 py-10 text-[13px] text-fd-muted-foreground">
        Could not load the schema for <code>{kind}</code>: {error}
      </p>
    );
  }

  if (!schema || specValue === null) {
    return (
      <p className="border border-(--rule-strong) px-6 py-10 text-[13px] text-fd-muted-foreground">
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
    <div className="not-prose grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="flex min-w-0 flex-col gap-4">
        {groups.length > 0 ? (
          <ChoicePicker groups={groups} selection={selection} onChange={choose} />
        ) : null}
        <div className="border border-(--rule-soft) p-3">
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
                <p className="text-[12px] text-fd-muted-foreground">
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
          schema={formSchema ?? schema}
          value={specValue}
          onChange={setSpecValue}
          omitKeys={[...(hasSpecName ? ["name"] : []), ...hiddenKeys(groups, selection)]}
          collapseOptional
          priorityKeys={chosenFields(groups, selection)}
        />
      </div>

      <div className="flex min-w-0 flex-col gap-2 lg:sticky lg:top-20 lg:self-start">
        <div className="flex items-center justify-between">
          <span className={labelClass}>Generated manifest</span>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={copyYaml}
              className="ctl"
            >
              {copied ? "copied" : "copy"}
            </button>
            <button
              type="button"
              onClick={downloadYaml}
              className="ctl"
            >
              download
            </button>
          </div>
        </div>
        {built && built.missing.length > 0 ? (
          <div className="border border-(--accent) p-3 text-[13px] text-fd-foreground">
            <p className="mb-1 text-[12px] font-semibold uppercase tracking-[0.1em] text-fd-primary">
              Still to fill in
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
        <div className="yaml-preview max-h-[70vh] overflow-auto border border-(--rule-soft) text-[13px] [&_pre]:my-0">
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
