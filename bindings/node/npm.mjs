// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// The npm side of the native build. The names come from the parsec-html
// package's package.json (bindings/wasm/npm/package.json): its version,
// and its `napi.packageName`, which each platform's package adds its
// platform to, as napi-rs names them.
//
//   node npm.mjs platform PLATFORM LIBRARY OUT
//     Makes OUT/PLATFORM, the package for one platform (linux-x64-gnu,
//     darwin-arm64, win32-x64-msvc...), from the library cargo built.
//   node npm.mjs optional PACKAGE_DIR PLATFORM...
//     Lists those platforms' packages as optional dependencies of the
//     package in PACKAGE_DIR, at its version, for npm to pick from.
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = new URL("../../", import.meta.url);
const main = JSON.parse(readFileSync(new URL("bindings/wasm/npm/package.json", root), "utf8"));
const { binaryName, packageName } = main.napi;

const [command, ...args] = process.argv.slice(2);
if (command === "platform" && args.length === 3) {
  const [platform, library, out] = args;
  const [os, cpu, abi] = platform.split("-");
  const file = `${binaryName}.${platform}.node`;
  const dir = join(out, platform);
  mkdirSync(dir, { recursive: true });
  copyFileSync(library, join(dir, file));
  for (const f of ["LICENSE", "NOTICE"]) copyFileSync(new URL(f, root), join(dir, f));
  const pkg = {
    name: `${packageName}-${platform}`,
    version: main.version,
    description: `${main.name} built for ${platform}: npm installs it with ${main.name} on that platform.`,
    license: main.license,
    repository: { ...main.repository, directory: "bindings/node" },
    os: [os],
    cpu: [cpu],
    ...(os === "linux" && { libc: [abi === "musl" ? "musl" : "glibc"] }),
    main: file,
    files: [file, "LICENSE", "NOTICE"],
  };
  writeFileSync(join(dir, "package.json"), `${JSON.stringify(pkg, null, 2)}\n`);
  writeFileSync(
    join(dir, "README.md"),
    `# ${pkg.name}\n\nThe native build of [${main.name}](https://www.npmjs.com/package/${main.name}) for ${platform}. Install ${main.name}, not this.\n`,
  );
} else if (command === "optional" && args.length >= 2) {
  const [dir, ...platforms] = args;
  const path = join(dir, "package.json");
  const pkg = JSON.parse(readFileSync(path, "utf8"));
  pkg.optionalDependencies = Object.fromEntries(platforms.map((p) => [`${packageName}-${p}`, pkg.version]));
  writeFileSync(path, `${JSON.stringify(pkg, null, 2)}\n`);
} else {
  console.error("usage: node npm.mjs platform PLATFORM LIBRARY OUT | optional PACKAGE_DIR PLATFORM...");
  process.exit(2);
}
