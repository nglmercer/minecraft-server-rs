import { describe, expect, it } from "vitest";
import { en } from "./en";
import { es } from "./es";

/** Every dotted leaf key in a dictionary, e.g. `server.tabs.console`. */
function leaves(node: unknown, prefix = ""): string[] {
  if (typeof node === "string") return [prefix];
  if (typeof node !== "object" || node === null) return [];

  return Object.entries(node).flatMap(([key, value]) =>
    leaves(value, prefix ? `${prefix}.${key}` : key),
  );
}

describe("dictionaries", () => {
  it("cover exactly the same keys", () => {
    // TypeScript already enforces this, but only for keys it can see; this also
    // catches a key that exists in both and was silently nested differently.
    expect(leaves(es).sort()).toEqual(leaves(en).sort());
  });

  it("has no empty strings", () => {
    for (const dictionary of [en, es]) {
      const blank = leaves(dictionary).filter((key) => {
        const value = key
          .split(".")
          .reduce<any>((node, part) => node?.[part], dictionary);
        return typeof value === "string" && value.trim() === "";
      });
      expect(blank).toEqual([]);
    }
  });

  it("uses the same placeholders in every language", () => {
    const placeholders = (text: string) =>
      (text.match(/\{(\w+)\}/g) ?? []).sort();

    for (const key of leaves(en)) {
      const read = (dict: unknown) =>
        key.split(".").reduce<any>((node, part) => node?.[part], dict) as string;

      // A translation that drops {count} renders a sentence missing its number;
      // one that invents {total} renders the literal braces.
      expect(placeholders(read(es)), `mismatched placeholders in ${key}`).toEqual(
        placeholders(read(en)),
      );
    }
  });
});
