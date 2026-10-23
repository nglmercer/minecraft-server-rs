import { mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { DemoBrowser } from "./browser.js";
import {
  ChapterLog,
  createRecordingStagingPath,
  recordingDir,
  recordingPath,
  saveRecording,
} from "./recording.js";

describe("ChapterLog", () => {
  it("records a failed marker for failed raw footage", () => {
    const log = new ChapterLog(Date.now());

    log.mark("failed");

    expect(log.events).toEqual([{ name: "failed", at: expect.any(Number) }]);
    expect(log.at("failed")).not.toBeNull();
  });

  it("does not duplicate observed lifecycle markers", () => {
    const consoleSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    const log = new ChapterLog(Date.now());

    log.markOnce("starting");
    log.markOnce("starting");

    expect(log.events).toHaveLength(1);
    consoleSpy.mockRestore();
  });
});

function fakeHandle(stagingPath: string): DemoBrowser {
  return {
    page: {
      screencast: { stop: vi.fn().mockResolvedValue(undefined) },
    },
    recording: { stagingPath, active: true },
  } as unknown as DemoBrowser;
}

describe("screencast recording publication", () => {
  beforeAll(async () => {
    await mkdir(recordingDir(), { recursive: true });
  });
  it("generates a safe staging path inside the demo artifact directory", () => {
    const path = createRecordingStagingPath("first run/quality");
    expect(path.startsWith(recordingDir())).toBe(true);
    expect(path).toMatch(/\.webm$/);
    expect(path).not.toContain("first run");
    expect(path).not.toContain("quality/");
  });

  it("stops, validates, and publishes a non-empty screencast", async () => {
    const fileName = `recording-test-${process.pid}-${Date.now()}.webm`;
    const stagingPath = createRecordingStagingPath("recording-test");
    const target = recordingPath(fileName);
    await writeFile(stagingPath, Buffer.from("valid-webm-placeholder"));
    const handle = fakeHandle(stagingPath);

    try {
      await expect(saveRecording(handle, fileName)).resolves.toBe(target);
      expect(handle.recording.active).toBe(false);
      expect(await readFile(target, "utf8")).toBe("valid-webm-placeholder");
      expect(await stat(target)).toBeTruthy();
      expect((handle.page.screencast.stop as unknown as ReturnType<typeof vi.fn>)).toHaveBeenCalledOnce();
    } finally {
      await rm(target, { force: true });
      await rm(stagingPath, { force: true });
    }
  });

  it("rejects an empty screencast without replacing good footage", async () => {
    const fileName = `recording-empty-${process.pid}-${Date.now()}.webm`;
    const stagingPath = createRecordingStagingPath("recording-empty");
    const target = recordingPath(fileName);
    await writeFile(stagingPath, "");
    await writeFile(target, "previous-good-footage");

    try {
      await expect(saveRecording(fakeHandle(stagingPath), fileName)).rejects.toThrow(
        "Playwright produced an empty screencast.",
      );
      expect(await readFile(target, "utf8")).toBe("previous-good-footage");
    } finally {
      await rm(target, { force: true });
      await rm(stagingPath, { force: true });
    }
  });

  it("preserves existing footage when the replacement staging file is missing", async () => {
    const fileName = `recording-missing-${process.pid}-${Date.now()}.webm`;
    const stagingPath = createRecordingStagingPath("recording-missing");
    const target = recordingPath(fileName);
    await writeFile(target, "previous-good-footage");

    try {
      await expect(saveRecording(fakeHandle(stagingPath), fileName)).rejects.toThrow();
      expect(await readFile(target, "utf8")).toBe("previous-good-footage");
    } finally {
      await rm(target, { force: true });
      await rm(stagingPath, { force: true });
    }
  });
});
