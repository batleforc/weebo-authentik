"use client";

import type { ReactNode } from "react";
import { Callout } from "fumadocs-ui/components/callout";
import {
  defaultForSchema,
  discriminatedVariants,
  isDiscriminatedUnion,
  variantSchema,
} from "@/lib/crd-form/schema-utils";
import type { FieldSchema } from "@/lib/crd-form/types";

const inputClass =
  "w-full border border-(--rule-strong) bg-(--ground-sunk) px-3 py-2 text-[15px] text-fd-foreground outline-none placeholder:text-fd-muted-foreground focus:border-(--accent)";

const labelClass = "text-[12px] font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground";

const ghostButtonClass = "ctl";

function firstParagraph(description?: string): string {
  if (!description) return "";
  return description
    .split(/\n\s*\n/)[0]
    .replace(/\s*\n\s*/g, " ")
    .trim();
}

function FieldHeader({
  name,
  required,
  description,
}: {
  name: string;
  required?: boolean;
  description?: string;
}) {
  const summary = firstParagraph(description);
  return (
    <div className="flex flex-col gap-0.5">
      <span className={labelClass} title={description}>
        {name}
        {required ? <span className="text-fd-primary"> *</span> : null}
      </span>
      {summary ? (
        <p className="line-clamp-2 text-[12px] text-fd-muted-foreground" title={description}>
          {summary}
        </p>
      ) : null}
    </div>
  );
}

function FieldGroup({
  name,
  required,
  description,
  children,
}: {
  name: string;
  required?: boolean;
  description?: string;
  children: ReactNode;
}) {
  return (
    <fieldset className="flex flex-col gap-3 border border-(--rule-soft) p-3">
      <legend className="px-1">
        <FieldHeader name={name} required={required} description={description} />
      </legend>
      {children}
    </fieldset>
  );
}

export interface FieldProps {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: unknown;
  onChange: (value: unknown) => void;
}

export function Field({ name, schema, required, value, onChange }: FieldProps) {
  if (isDiscriminatedUnion(schema)) {
    return (
      <DiscriminatedUnionField
        name={name}
        schema={schema}
        required={required}
        value={value as Record<string, unknown>}
        onChange={onChange as (v: Record<string, unknown>) => void}
      />
    );
  }
  if (schema.type === "array") {
    return (
      <ArrayField
        name={name}
        schema={schema}
        required={required}
        value={(value as unknown[]) ?? []}
        onChange={onChange as (v: unknown[]) => void}
      />
    );
  }
  if (schema.type === "object" && schema.additionalProperties) {
    return (
      <MapField
        name={name}
        schema={schema}
        required={required}
        value={(value as Record<string, string>) ?? {}}
        onChange={onChange as (v: Record<string, string>) => void}
      />
    );
  }
  if (schema.type === "object" && schema.properties) {
    return (
      <FieldGroup name={name} required={required} description={schema.description}>
        <ObjectFields
          schema={schema}
          value={value}
          onChange={onChange as (v: Record<string, unknown>) => void}
        />
      </FieldGroup>
    );
  }
  if (schema.type === "object") {
    return <RawJsonField name={name} schema={schema} required={required} value={value} onChange={onChange} />;
  }
  if (schema.enum) {
    return (
      <SelectField
        name={name}
        schema={schema}
        required={required}
        value={(value as string) ?? ""}
        onChange={onChange as (v: string) => void}
        options={schema.enum}
      />
    );
  }
  if (schema.type === "boolean") {
    return (
      <BooleanField
        name={name}
        schema={schema}
        value={Boolean(value)}
        onChange={onChange as (v: boolean) => void}
      />
    );
  }
  if (schema.type === "integer" || schema.type === "number") {
    return (
      <NumberField
        name={name}
        schema={schema}
        required={required}
        value={Number(value ?? 0)}
        onChange={onChange as (v: number) => void}
      />
    );
  }
  return (
    <StringField
      name={name}
      schema={schema}
      required={required}
      value={(value as string) ?? ""}
      onChange={onChange as (v: string) => void}
    />
  );
}

export function ObjectFields({
  schema,
  value,
  onChange,
  omitKeys,
  collapseOptional,
  priorityKeys,
}: {
  schema: FieldSchema;
  value: unknown;
  onChange: (value: Record<string, unknown>) => void;
  // Lets the caller take a property out of the generic recursion to render
  // it itself (e.g. CrdForm merging `spec.name` into the metadata.name
  // input) — only meant for the root call, nested ObjectFields calls never
  // pass this.
  omitKeys?: string[];
  // Root call only: optional fields fold under "More settings", so what must
  // be filled in is what the reader sees first.
  collapseOptional?: boolean;
  // Optional fields to keep upfront anyway (the ones a guided choice is about).
  priorityKeys?: string[];
}) {
  const properties = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  const obj = (value as Record<string, unknown>) ?? {};
  const omit = new Set(omitKeys ?? []);
  const entries = Object.entries(properties).filter(([key]) => !omit.has(key));
  // Required fields, then priority ones, then the rest; each keeps schema order.
  const priority = new Set(priorityKeys ?? []);
  const mandatory = [
    ...entries.filter(([key]) => required.has(key)),
    ...entries.filter(([key]) => !required.has(key) && priority.has(key)),
  ];
  const optional = entries.filter(([key]) => !required.has(key) && !priority.has(key));
  const render = ([key, childSchema]: [string, FieldSchema]) => (
    <Field
      key={key}
      name={key}
      schema={childSchema}
      required={required.has(key)}
      value={obj[key]}
      onChange={(v) => onChange({ ...obj, [key]: v })}
    />
  );
  return (
    <div className="flex flex-col gap-4">
      {mandatory.map(render)}
      {collapseOptional && optional.length > 0 ? (
        <details className="group border border-(--rule-soft)">
          <summary className="cursor-pointer list-none px-3 py-2 text-[12px] font-semibold uppercase tracking-[0.1em] text-fd-muted-foreground hover:text-fd-foreground">
            <span className="group-open:hidden">+ </span>
            <span className="hidden group-open:inline">− </span>
            More settings ({optional.length})
          </summary>
          <div className="flex flex-col gap-4 border-t border-(--rule-soft) p-3">{optional.map(render)}</div>
        </details>
      ) : (
        optional.map(render)
      )}
    </div>
  );
}

function StringField({
  name,
  schema,
  required,
  value,
  onChange,
}: {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader name={name} required={required} description={schema.description} />
      <input
        type="text"
        className={inputClass}
        value={value}
        placeholder={schema.nullable ? "(optional)" : ""}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

function NumberField({
  name,
  schema,
  required,
  value,
  onChange,
}: {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader name={name} required={required} description={schema.description} />
      <input
        type="number"
        className={inputClass}
        value={Number.isFinite(value) ? value : 0}
        onChange={(e) => onChange(e.target.valueAsNumber || 0)}
      />
    </div>
  );
}

function BooleanField({
  name,
  schema,
  value,
  onChange,
}: {
  name: string;
  schema: FieldSchema;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2">
      <input
        type="checkbox"
        checked={value}
        onChange={(e) => onChange(e.target.checked)}
        className="size-4 border border-(--rule-strong) bg-(--ground-sunk) accent-(--accent)"
      />
      <FieldHeader name={name} description={schema.description} />
    </label>
  );
}

function SelectField({
  name,
  schema,
  required,
  value,
  onChange,
  options,
}: {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: string;
  onChange: (v: string) => void;
  options: string[];
}) {
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader name={name} required={required} description={schema.description} />
      <select className={inputClass} value={value} onChange={(e) => onChange(e.target.value)}>
        {!value ? <option value="">-- select --</option> : null}
        {options.map((opt) => (
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
    </div>
  );
}

function RawJsonField({
  name,
  schema,
  required,
  value,
  onChange,
}: {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const text = typeof value === "string" ? value : JSON.stringify(value ?? {}, null, 2);
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader
        name={name}
        required={required}
        description={schema.description ?? "Free-form JSON, passed through to Authentik as-is."}
      />
      <textarea
        className={`${inputClass} min-h-24 font-mono`}
        defaultValue={text}
        onBlur={(e) => {
          try {
            onChange(JSON.parse(e.target.value));
          } catch {
            // Keep the last valid value until the textarea holds parseable JSON again.
          }
        }}
      />
    </div>
  );
}

function ArrayField({
  name,
  schema,
  required,
  value,
  onChange,
}: {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: unknown[];
  onChange: (v: unknown[]) => void;
}) {
  const itemSchema = schema.items ?? {};
  const singular = name.endsWith("s") ? name.slice(0, -1) : name;
  return (
    <FieldGroup name={name} required={required} description={schema.description}>
      <div className="flex flex-col gap-3">
        {value.map((item, index) => (
          <div key={index} className="flex items-start gap-2 border border-dashed border-(--rule-soft) p-2">
            <div className="flex-1">
              <Field
                name={`[${index}]`}
                schema={itemSchema}
                value={item}
                onChange={(v) => onChange(value.map((it, i) => (i === index ? v : it)))}
              />
            </div>
            <button
              type="button"
              onClick={() => onChange(value.filter((_, i) => i !== index))}
              className={ghostButtonClass}
            >
              remove
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={() => onChange([...value, defaultForSchema(itemSchema)])}
          className={`${ghostButtonClass} self-start`}
        >
          + add {singular}
        </button>
      </div>
    </FieldGroup>
  );
}

function MapField({
  name,
  schema,
  required,
  value,
  onChange,
}: {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: Record<string, string>;
  onChange: (v: Record<string, string>) => void;
}) {
  const entries = Object.entries(value ?? {});

  const updateEntry = (index: number, key: string, val: string) => {
    const next = [...entries];
    next[index] = [key, val];
    onChange(Object.fromEntries(next.filter(([k]) => k !== "")));
  };

  return (
    <FieldGroup name={name} required={required} description={schema.description}>
      <div className="flex flex-col gap-2">
        {entries.map(([key, val], index) => (
          <div key={index} className="flex items-center gap-2">
            <input
              className={inputClass}
              placeholder="key"
              defaultValue={key}
              onBlur={(e) => updateEntry(index, e.target.value, val)}
            />
            <input
              className={inputClass}
              placeholder="value"
              defaultValue={val}
              onBlur={(e) => updateEntry(index, key, e.target.value)}
            />
            <button
              type="button"
              onClick={() => onChange(Object.fromEntries(entries.filter((_, i) => i !== index)))}
              className={ghostButtonClass}
            >
              remove
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={() => onChange({ ...value, [`key${entries.length}`]: "" })}
          className={`${ghostButtonClass} self-start`}
        >
          + add entry
        </button>
      </div>
    </FieldGroup>
  );
}

function DiscriminatedUnionField({
  name,
  schema,
  required,
  value,
  onChange,
}: {
  name: string;
  schema: FieldSchema;
  required?: boolean;
  value: Record<string, unknown>;
  onChange: (v: Record<string, unknown>) => void;
}) {
  const variants = discriminatedVariants(schema);
  const kind = (value?.kind as string) ?? variants[0];
  const vSchema = variantSchema(schema, kind);

  return (
    <FieldGroup name={name} required={required} description={schema.description}>
      <div className="flex flex-col gap-4">
        <SelectField
          name="kind"
          schema={schema.properties?.kind ?? {}}
          required
          value={kind ?? ""}
          onChange={(newKind) => {
            const newVariantSchema = variantSchema(schema, newKind);
            onChange({
              kind: newKind,
              ...(newVariantSchema ? { [newKind]: defaultForSchema(newVariantSchema) } : {}),
            });
          }}
          options={variants}
        />
        {vSchema ? (
          <ObjectFields
            schema={vSchema}
            value={value?.[kind]}
            onChange={(v) => onChange({ kind, [kind]: v })}
          />
        ) : kind ? (
          <Callout type="warn" title={`"${kind}" is a schema-only stub in v1`}>
            Accepted here for forward compatibility, but the reconciler rejects it with an explicit
            error rather than applying it silently.
          </Callout>
        ) : null}
      </div>
    </FieldGroup>
  );
}
