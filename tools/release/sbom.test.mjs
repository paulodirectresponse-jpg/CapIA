import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  buildSbom,
  cargoComponents,
  ffmpegComponent,
  licenseReport,
  pnpmComponents,
  thirdPartyNotices,
} from "./sbom.mjs";

test("cargo components skip workspace members and path crates, keep license", () => {
  const meta = {
    workspace_members: ["ws#1"],
    packages: [
      { id: "ws#1", name: "capia-x", version: "0.6.0", source: null, license: null },
      {
        id: "r#1",
        name: "serde",
        version: "1.0.1",
        source: "registry+x",
        license: "MIT OR Apache-2.0",
      },
      { id: "r#2", name: "weird", version: "0.1.0", source: "registry+x", license: null },
    ],
  };
  const c = cargoComponents(meta);
  assert.deepEqual(
    c.map((x) => x.name),
    ["serde", "weird"],
  );
  assert.equal(c[0].licenses[0].expression, "MIT OR Apache-2.0");
  assert.equal(c[1].licenses[0].expression, "NOASSERTION");
});

test("pnpm lock v9 keys are parsed, including scoped packages and peer suffixes", () => {
  const lock = `lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      react: 1
packages:

  '@scope/pkg@1.2.3':
    resolution: {integrity: x}

  react@19.3.0:
    resolution: {integrity: y}

  vite@8.3.2(@types/node@22.0.0):
    resolution: {integrity: z}

snapshots:
  react@19.3.0: {}
`;
  const c = pnpmComponents(lock);
  assert.deepEqual(
    c.map((x) => `${x.name}@${x.version}`),
    ["@scope/pkg@1.2.3", "react@19.3.0", "vite@8.3.2"],
  );
  assert.equal(c[0].purl, "pkg:npm/%40scope/pkg@1.2.3");
});

test("sbom is CycloneDX with the app version; report flags unknown and copyleft", () => {
  const comps = [
    ...cargoComponents({
      workspace_members: [],
      packages: [
        { id: "1", name: "a", version: "1", source: "r", license: "MIT" },
        { id: "2", name: "b", version: "1", source: "r", license: "MPL-2.0" },
        { id: "3", name: "c", version: "1", source: "r", license: null },
      ],
    }),
  ];
  const s = buildSbom({ version: "0.6.0-rc.1", components: comps, now: "2026-01-01T00:00:00Z" });
  assert.equal(s.bomFormat, "CycloneDX");
  assert.equal(s.metadata.component.version, "0.6.0-rc.1");
  assert.equal(s.components.length, 3);
  assert.ok(!("ecosystem" in s.components[0]));
  const r = licenseReport(comps);
  assert.equal(r.unknown, 1);
  assert.deepEqual(r.needs_review, ["MPL-2.0"]);
});

test("notices include the FFmpeg LGPL section only when FFmpeg metadata exists", () => {
  const ff = ffmpegComponent({ source_tag: "n8.1.3", source_sha256: "abc" });
  const withFf = thirdPartyNotices({ version: "1", components: [], ffmpeg: ff });
  assert.match(withFf, /FFmpeg/);
  assert.match(withFf, /LGPL/);
  assert.match(withFf, /n8\.1\.3/);
  assert.doesNotMatch(thirdPartyNotices({ version: "1", components: [], ffmpeg: null }), /FFmpeg/);
  assert.equal(ffmpegComponent(null), null);
});
