"use client";

import type { ChoiceGroup, Selection } from "@/lib/crd-form/choices";

// A three-or-fewer-way choice drawn the BatleHub way: one 1px bounding box,
// hairline cell dividers, no gaps, the chosen cell reversed to an ink block,
// state on `aria-pressed` (DESIGN.md, "Preferences" / segmented groups).
export function ChoicePicker({
  groups,
  selection,
  onChange,
}: {
  groups: ChoiceGroup[];
  selection: Selection;
  onChange: (groupId: string, optionId: string) => void;
}) {
  return (
    <div className="flex flex-col gap-4 border border-(--rule-soft) p-3">
      {groups.map((group) => {
        const current = group.options.find((o) => o.id === selection[group.id]) ?? group.options[0];
        const labelId = `choice-${group.id}`;
        return (
          <div key={group.id} className="flex flex-col gap-2">
            <span id={labelId} className="text-[12px] font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground">
              {group.label}
            </span>
            <div role="group" aria-labelledby={labelId} className="flex border border-(--rule-strong)">
              {group.options.map((option) => {
                const pressed = option.id === current.id;
                return (
                  <button
                    key={option.id}
                    type="button"
                    aria-pressed={pressed}
                    onClick={() => onChange(group.id, option.id)}
                    className={`flex-1 px-1 py-2 text-[12px] uppercase tracking-[0.06em] not-first:border-l not-first:border-(--rule-strong) ${
                      pressed
                        ? "bg-(--ink) font-bold text-(--ground)"
                        : "text-fd-muted-foreground hover:text-fd-foreground"
                    }`}
                  >
                    {option.label}
                  </button>
                );
              })}
            </div>
            <p className="text-[13px] leading-[1.6] text-fd-muted-foreground">{current.hint}</p>
          </div>
        );
      })}
    </div>
  );
}
