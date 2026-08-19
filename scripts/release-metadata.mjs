#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

function fail(message) {
  throw new Error(message);
}

function argument(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || index + 1 >= process.argv.length) fail(`missing ${name}`);
  return process.argv[index + 1];
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stable(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function parseLock(lockText) {
  const packages = [];
  let current = null;
  for (const line of lockText.split("\n")) {
    if (line === "[[package]]") {
      if (current) packages.push(current);
      current = {};
      continue;
    }
    if (!current) continue;
    const match = /^(name|version|source|checksum) = "(.*)"$/.exec(line);
    if (match) current[match[1]] = match[2];
  }
  if (current) packages.push(current);
  return new Map(
    packages
      .filter((item) => item.name && item.version)
      .map((item) => [`${item.name}\u0000${item.version}\u0000${item.source ?? ""}`, item]),
  );
}

function sourceFor(pkg) {
  return pkg.source ?? "workspace";
}

function componentRef(pkg) {
  return `pkg:cargo/${pkg.name}@${pkg.version}?source=${sha256(sourceFor(pkg)).slice(0, 16)}`;
}

function component(pkg, lockPackages) {
  const lock = lockPackages.get(`${pkg.name}\u0000${pkg.version}\u0000${pkg.source ?? ""}`);
  const properties = [{ name: "cargo:source", value: sourceFor(pkg) }];
  if (lock?.checksum) properties.push({ name: "cargo:checksum", value: lock.checksum });
  return {
    "bom-ref": componentRef(pkg),
    name: pkg.name,
    type: pkg.source ? "library" : "application",
    version: pkg.version,
    ...(pkg.license ? { licenses: [{ license: { name: pkg.license } }] } : {}),
    ...(lock?.checksum ? { hashes: [{ alg: "SHA-256", content: lock.checksum }] } : {}),
    properties,
  };
}

function deterministicSerial(releaseTag, target, sourceCommit) {
  const digest = sha256(`${releaseTag}\n${target}\n${sourceCommit}`);
  return `urn:uuid:${digest.slice(0, 8)}-${digest.slice(8, 12)}-5${digest.slice(13, 16)}-a${digest.slice(17, 20)}-${digest.slice(20, 32)}`;
}

const target = argument("--target");
const releaseTag = argument("--release-tag");
const sourceCommit = argument("--source-commit");
const outputDirectory = resolve(argument("--out-dir"));
if (!/^[0-9a-f]{40}$/.test(sourceCommit)) fail("source commit must be a lowercase 40-character SHA-1");

const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], { encoding: "utf8" }),
);
const lockBytes = readFileSync("Cargo.lock");
const lockPackages = parseLock(lockBytes.toString("utf8"));
const packages = [...metadata.packages]
  .sort((left, right) => componentRef(left).localeCompare(componentRef(right)));
const componentByPackageId = new Map(packages.map((pkg) => [pkg.id, component(pkg, lockPackages)]));
const dependencies = [...metadata.resolve.nodes]
  .map((node) => ({
    ref: componentByPackageId.get(node.id)?.["bom-ref"] ?? fail(`unknown package ID ${node.id}`),
    dependsOn: node.deps
      .map((dependency) => componentByPackageId.get(dependency.pkg)?.["bom-ref"] ?? fail(`unknown dependency ${dependency.pkg}`))
      .sort(),
  }))
  .sort((left, right) => left.ref.localeCompare(right.ref));
const rootRefs = metadata.workspace_members
  .map((member) => componentByPackageId.get(member)?.["bom-ref"] ?? fail(`unknown workspace member ${member}`))
  .sort();
const components = [...componentByPackageId.values()];
const application = packages.find((pkg) => pkg.name === "second-observer") ?? fail("missing second-observer application package");
const applicationRef = componentByPackageId.get(application.id)?.["bom-ref"] ?? fail("missing application component");
if (components.length <= 10 || dependencies.length <= 10) {
  fail("locked dependency graph is unexpectedly incomplete");
}
const record = {
  contract_version: "second-observer.dependency-record/v1",
  cargo_lock_sha256: sha256(lockBytes),
  components: components.map((entry) => ({
    ref: entry["bom-ref"],
    name: entry.name,
    version: entry.version,
    source: entry.properties.find((property) => property.name === "cargo:source")?.value,
    checksum: entry.properties.find((property) => property.name === "cargo:checksum")?.value ?? null,
    license: entry.licenses?.[0]?.license.name ?? null,
  })),
  dependencies,
  release_tag: releaseTag,
  roots: rootRefs,
  source_commit: sourceCommit,
  target,
};
const sbom = {
  $schema: "http://cyclonedx.org/schema/bom-1.6.schema.json",
  bomFormat: "CycloneDX",
  components,
  dependencies: dependencies.map(({ ref, dependsOn }) => ({ ref, dependsOn })),
  metadata: {
    component: {
      "bom-ref": applicationRef,
      name: "second-observer",
      type: "application",
      version: releaseTag,
    },
    properties: [
      { name: "second-observer:source-commit", value: sourceCommit },
      { name: "second-observer:target", value: target },
      { name: "second-observer:cargo-lock-sha256", value: record.cargo_lock_sha256 },
    ],
  },
  serialNumber: deterministicSerial(releaseTag, target, sourceCommit),
  specVersion: "1.6",
  version: 1,
};

for (const value of [record, sbom]) {
  if (stable(value).match(/(?:^|[\\/])(?:home|Users|runner|target)(?:[\\/]|$)/)) {
    fail("release metadata contains a local path or build-host identifier");
  }
}
mkdirSync(outputDirectory, { recursive: true });
writeFileSync(resolve(outputDirectory, `second-observer-${target}.dependency-record.json`), `${stable(record)}\n`);
writeFileSync(resolve(outputDirectory, `second-observer-${target}.cdx.json`), `${stable(sbom)}\n`);
