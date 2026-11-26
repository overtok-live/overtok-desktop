import { describe, expect, it } from "vitest";
import { buildManifests } from "./manifest.mjs";

const files = [
  { name: "Overtok_0.2.0_x64-setup.exe", sig: "sig-win" },
  { name: "Overtok_0.2.0_x64-setup.exe.sig", sig: null },
  { name: "Overtok_universal.app.tar.gz", sig: "sig-mac" },
  { name: "Overtok_0.2.0_universal.dmg", sig: null },
  { name: "Overtok_0.2.0_amd64.AppImage", sig: "sig-linux" },
  { name: "Overtok_0.2.0_amd64.deb", sig: null },
];
const base = "https://downloads.overtok.live/desktop/0.2.0";

describe("buildManifests", () => {
  it("writes the updater's manifest and the website's download list", () => {
    const { latest, downloads } = buildManifests({
      version: "0.2.0",
      notes: "Faster start.",
      pubDate: "2026-11-24T12:00:00.000Z",
      minimum: null,
      base,
      files,
    });
    expect(latest).toEqual({
      version: "0.2.0",
      notes: "Faster start.",
      pub_date: "2026-11-24T12:00:00.000Z",
      platforms: {
        "windows-x86_64": { url: `${base}/Overtok_0.2.0_x64-setup.exe`, signature: "sig-win" },
        "darwin-aarch64": { url: `${base}/Overtok_universal.app.tar.gz`, signature: "sig-mac" },
        "darwin-x86_64": { url: `${base}/Overtok_universal.app.tar.gz`, signature: "sig-mac" },
        "linux-x86_64": { url: `${base}/Overtok_0.2.0_amd64.AppImage`, signature: "sig-linux" },
      },
    });
    expect(downloads).toEqual({
      version: "0.2.0",
      windows: `${base}/Overtok_0.2.0_x64-setup.exe`,
      macos: `${base}/Overtok_0.2.0_universal.dmg`,
      linuxAppImage: `${base}/Overtok_0.2.0_amd64.AppImage`,
      linuxDeb: `${base}/Overtok_0.2.0_amd64.deb`,
    });
  });

  it("marks a minimum version that forces the update", () => {
    const { latest } = buildManifests({
      version: "0.2.1",
      notes: "",
      pubDate: "2026-11-24T12:00:00.000Z",
      minimum: "0.2.1",
      base,
      files,
    });
    expect(latest.minimum).toBe("0.2.1");
  });

  it("refuses a release missing a system", () => {
    expect(() =>
      buildManifests({
        version: "0.2.0",
        notes: "",
        pubDate: "x",
        minimum: null,
        base,
        files: files.filter((file) => !file.name.endsWith(".exe")),
      }),
    ).toThrow(/windows/);
  });
});
