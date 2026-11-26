// Builds a desktop release's two manifests from its files (ADR 0010):
// - latest.json: what installed apps read to update themselves (Tauri's updater format, plus
//   `minimum`: below it, the update is required);
// - downloads.json: the installers the website's Download page links to.
// CLI: node manifest.mjs <dir> <version> <base-url> [minimum]  (notes from RELEASE_NOTES.md)
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** The file whose name ends with one of `endings`; throws naming the system when none does. */
function pick(files, system, endings) {
  const file = files.find((one) => endings.some((ending) => one.name.endsWith(ending)));
  if (!file) throw new Error(`no ${system} build among the release's files`);
  return file;
}

export function buildManifests({ version, notes, pubDate, minimum, base, files }) {
  const url = (file) => `${base}/${file.name}`;
  const updater = (system, endings) => {
    const file = pick(files, system, endings);
    if (!file.sig) throw new Error(`${system} build has no signature`);
    return { url: url(file), signature: file.sig };
  };
  const windows = updater("windows", ["-setup.exe"]);
  const mac = updater("macos", [".app.tar.gz"]);
  const linux = updater("linux", [".AppImage"]);
  const latest = {
    version,
    notes,
    pub_date: pubDate,
    ...(minimum ? { minimum } : {}),
    platforms: {
      "windows-x86_64": windows,
      "darwin-aarch64": mac,
      "darwin-x86_64": mac,
      "linux-x86_64": linux,
    },
  };
  const downloads = {
    version,
    windows: windows.url,
    macos: url(pick(files, "macos", [".dmg"])),
    linuxAppImage: linux.url,
    linuxDeb: url(pick(files, "linux", [".deb"])),
  };
  return { latest, downloads };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [dir, version, base, minimum] = process.argv.slice(2);
  const names = readdirSync(dir).filter(
    (name) => !name.endsWith(".sig") && !name.endsWith(".json"),
  );
  const files = names.map((name) => {
    let sig = null;
    try {
      sig = readFileSync(join(dir, `${name}.sig`), "utf8").trim();
    } catch {}
    return { name, sig };
  });
  let notes = "";
  try {
    notes = readFileSync(new URL("../RELEASE_NOTES.md", import.meta.url), "utf8").trim();
  } catch {}
  const { latest, downloads } = buildManifests({
    version,
    notes,
    pubDate: new Date().toISOString(),
    minimum: minimum || null,
    base,
    files,
  });
  writeFileSync(join(dir, "latest.json"), `${JSON.stringify(latest, null, 2)}\n`);
  writeFileSync(join(dir, "downloads.json"), `${JSON.stringify(downloads, null, 2)}\n`);
}
