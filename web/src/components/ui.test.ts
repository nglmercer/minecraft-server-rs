import { describe, expect, it } from "vitest";
import { formatBytes, formatUptime } from "./ui";

describe("formatBytes", () => {
  it("keeps small sizes in bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("steps up a unit at a time", () => {
    expect(formatBytes(1024)).toBe("1.0 KiB");
    expect(formatBytes(1024 * 1024)).toBe("1.0 MiB");
    expect(formatBytes(1024 ** 3)).toBe("1.0 GiB");
    expect(formatBytes(1024 ** 4)).toBe("1.0 TiB");
  });

  it("drops the decimal once the number is wide enough to read", () => {
    expect(formatBytes(1024 * 9.5)).toBe("9.5 KiB");
    expect(formatBytes(1024 * 42)).toBe("42 KiB");
  });

  it("does not run past its largest unit", () => {
    // A petabyte world is not realistic, but silently rendering "NaN undefined"
    // if one appeared would be worse than saying a large number of TiB.
    expect(formatBytes(1024 ** 6)).toMatch(/TiB$/);
  });
});

describe("formatUptime", () => {
  it("shows a dash when there is no process", () => {
    expect(formatUptime(null)).toBe("—");
  });

  it("uses the largest useful unit", () => {
    expect(formatUptime(45)).toBe("45s");
    expect(formatUptime(90)).toBe("1m 30s");
    expect(formatUptime(3600)).toBe("1h 00m");
    expect(formatUptime(3661)).toBe("1h 01m");
  });

  it("treats a negative reading as absent rather than rendering nonsense", () => {
    expect(formatUptime(-5)).toBe("—");
  });
});
