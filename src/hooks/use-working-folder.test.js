import { describe, expect, it } from "vitest";
import { basename } from "./use-working-folder";

/**
 * The label on the working-folder chip.
 *
 * This split on the forward slash alone, which on the platform this app
 * actually ships to means it never split at all: `D:\projects\inertia` came
 * back whole, so a control whose entire job is to show a short name was
 * showing the full path.
 */
describe("naming a folder", () => {
  it("takes the last segment of a Windows path", () => {
    expect(basename("D:\\projects\\inertia")).toBe("inertia");
  });

  it("takes the last segment of a POSIX path", () => {
    expect(basename("/home/me/code")).toBe("code");
  });

  it("ignores a trailing separator of either kind", () => {
    expect(basename("D:\\projects\\inertia\\")).toBe("inertia");
    expect(basename("/home/me/code/")).toBe("code");
  });

  it("names a drive root by its drive", () => {
    // The trailing separator is stripped first, so this is "C:" rather than
    // "C:\\". Either reads as the root of a disk, which is what matters.
    expect(basename("C:\\")).toBe("C:");
  });

  it("passes through a bare name that has no separators", () => {
    expect(basename("foo")).toBe("foo");
  });

  it("survives nothing at all", () => {
    expect(basename("")).toBe("");
    expect(basename(null)).toBe("");
    expect(basename(undefined)).toBe("");
  });
});
