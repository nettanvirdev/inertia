import { describe, expect, it } from "vitest";
import { allowRemote, mayFetch, remoteHost } from "./remote-image.js";

describe("remote pictures", () => {
  it("names the host a picture would be fetched from", () => {
    expect(remoteHost("https://example.com/a.png")).toBe("example.com");
    expect(remoteHost("http://cdn.example.com:8080/a.png?x=1")).toBe("cdn.example.com:8080");
    expect(remoteHost("//cdn.example.com/a.png")).toBe("cdn.example.com");
    // Credentials in the address are not the host, and must not pass for it.
    expect(remoteHost("https://example.com@evil.example/a.png")).toBe("evil.example");
  });

  it("does not treat a picture on this machine as remote", () => {
    for (const local of ["data:image/png;base64,aGk=", "blob:abc", "C:/work/project/a.png", "file:///C:/work/a.png", "files/a.png", ""]) {
      expect(remoteHost(local)).toBeNull();
      expect(mayFetch(local)).toBe(true);
    }
  });

  it("fetches a remote picture only once the person has asked for it", () => {
    const src = "https://example.com/report.png?q=1";
    expect(mayFetch(src)).toBe(false);
    allowRemote(src);
    expect(mayFetch(src)).toBe(true);
    expect(mayFetch("https://example.com/other.png")).toBe(false);
  });
});
