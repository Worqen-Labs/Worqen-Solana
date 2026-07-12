#!/usr/bin/env bun
import { existsSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { rootNodeFromAnchor, type AnchorIdl } from "@codama/nodes-from-anchor";
import { renderVisitor } from "@codama/renderers-js";
import { createFromRoot } from "codama";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const idlPath = join(repoRoot, "target", "idl", "worqen_escrow.json");
const packageFolder = join(repoRoot, "..", "frontend", "apps", "dashboard");
const generatedFolder = join("lib", "solana-wallet", "generated");

if (!existsSync(idlPath)) {
  console.error(`IDL not found at ${idlPath} — run \`anchor build\` first.`);
  process.exit(1);
}

const idl = (await Bun.file(idlPath).json()) as AnchorIdl;
const codama = createFromRoot(rootNodeFromAnchor(idl));

await codama.accept(
  renderVisitor(packageFolder, {
    generatedFolder,
    deleteFolderBeforeRendering: true,
    formatCode: false,
    syncPackageJson: false,
  }),
);

const outDir = join(packageFolder, generatedFolder);
console.log(
  `Generated @solana/kit client from ${relative(repoRoot, idlPath)} into ${outDir}`,
);
