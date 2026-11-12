#!/usr/bin/env node
/**
 * @ai-generated
 * 校验版本号在 package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml 三处一致。
 * 可选 `--tag <tag>`：额外校验发布标签与版本号匹配（标签形如 v1.4.1）。
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SEMVER_PATTERN = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

function readJson(relativePath) {
  return JSON.parse(readFileSync(resolve(ROOT, relativePath), "utf8"));
}

function readCargoVersion(relativePath) {
  const lines = readFileSync(resolve(ROOT, relativePath), "utf8").split(/\r?\n/);
  let inPackage = false;
  for (const line of lines) {
    if (/^\s*\[/.test(line)) {
      inPackage = line.trim() === "[package]";
      continue;
    }
    if (!inPackage) {
      continue;
    }
    const match = line.match(/^\s*version\s*=\s*"([^"]+)"/);
    if (match) {
      return match[1];
    }
  }
  throw new Error(`${relativePath}: 未找到 [package] version`);
}

const sources = [
  { name: "package.json", version: readJson("package.json").version },
  {
    name: "src-tauri/tauri.conf.json",
    version: readJson("src-tauri/tauri.conf.json").version,
  },
  {
    name: "src-tauri/Cargo.toml",
    version: readCargoVersion("src-tauri/Cargo.toml"),
  },
];

const tagIndex = process.argv.indexOf("--tag");
const tag = tagIndex >= 0 ? process.argv[tagIndex + 1] : null;

const details = sources.map(({ name, version }) => `  ${name}: ${version}`).join("\n");
const versions = new Set(sources.map(({ version }) => version));

if (versions.size > 1) {
  console.error(`版本号不一致：\n${details}`);
  process.exit(1);
}

const version = sources[0].version;
if (!SEMVER_PATTERN.test(version)) {
  console.error(`版本号格式非法（应为 X.Y.Z 或带预发布后缀）：${version}`);
  process.exit(1);
}

if (tag !== null && tag !== `v${version}`) {
  console.error(`发布标签与版本号不一致：tag=${tag}，期望 v${version}`);
  process.exit(1);
}

console.log(`版本号校验通过：${version}${tag ? `（${tag}）` : ""}`);
