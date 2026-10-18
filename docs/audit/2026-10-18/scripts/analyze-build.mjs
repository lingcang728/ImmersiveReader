#!/usr/bin/env node
// Read-only build-artifact analyzer for the 2026-10-18 performance audit.
// Parses the existing apps/desktop/build output (never triggers a build) and
// reports: total bytes, the first-load static closure, the markdown-worker
// closure, and how much code the worker bundle duplicates from the main bundle.
//
// Usage: node docs/audit/2026-10-18/scripts/analyze-build.mjs [--json]
import { readFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "../../../..");
const BUILD = join(ROOT, "apps", "desktop", "build");
const APP = join(BUILD, "_app", "immutable");

if (!existsSync(APP)) {
  console.error(`missing build output: ${APP} (this analyzer does not build)`);
  process.exit(1);
}

const STATIC_RE = /(?:^|[;{}])\s*(?:import\s*(?:[^"'()]*?\s?from\s*)?|export\s*(?:[^"'()]*?\s?from\s*))["']([^"']+)["']/g;
const DYNAMIC_RE = /import\(\s*["']([^"']+)["']\s*\)/g;

/** @returns {{rel:string, size:number, sha256:string, static:Set<string>, dynamic:string[]}} */
function parseFile(abs) {
  const source = readFileSync(abs, "utf8");
  const rel = relative(BUILD, abs).replace(/\\/g, "/");
  const staticImports = new Set();
  const dynamicImports = [];
  for (const match of source.matchAll(STATIC_RE)) {
    const spec = match[1];
    if (spec.startsWith("./") || spec.startsWith("../") || spec.startsWith("/")) {
      staticImports.add(resolvePath(rel, spec));
    }
  }
  for (const match of source.matchAll(DYNAMIC_RE)) {
    const spec = match[1];
    if (spec.startsWith("./") || spec.startsWith("../") || spec.startsWith("/")) {
      dynamicImports.push(resolvePath(rel, spec));
    }
  }
  return {
    rel,
    size: Buffer.byteLength(source),
    sha256: createHash("sha256").update(source).digest("hex"),
    static: staticImports,
    dynamic: [...new Set(dynamicImports)],
  };
}

function resolvePath(fromRel, spec) {
  if (spec.startsWith("/")) return spec.slice(1);
  return resolve(join(BUILD, fromRel, "..", spec))
    .replace(BUILD + "\\", "")
    .replace(BUILD + "/", "")
    .replace(/\\/g, "/");
}

const files = new Map();
const nonJs = new Map();
function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const abs = join(dir, entry.name);
    if (entry.isDirectory()) walk(abs);
    else if (entry.name.endsWith(".js")) {
      const parsed = parseFile(abs);
      files.set(parsed.rel, parsed);
    } else {
      nonJs.set(relative(BUILD, abs).replace(/\\/g, "/"), statSync(abs).size);
    }
  }
}
walk(APP);

function assetSize(rel) {
  const abs = join(BUILD, rel);
  return existsSync(abs) ? statSync(abs).size : 0;
}

const indexHtml = readFileSync(join(BUILD, "index.html"), "utf8");
const modulePreloads = [...indexHtml.matchAll(/<link[^>]+modulepreload[^>]*href="\/?([^"]+)"/g)].map(
  (m) => m[1],
);

const entryApp = [...files.keys()].find((rel) => /^_app\/immutable\/entry\/app\..*\.js$/.test(rel));
const entryStart = [...files.keys()].find((rel) => /^_app\/immutable\/entry\/start\..*\.js$/.test(rel));
const node0 = [...files.keys()].find((rel) => /^_app\/immutable\/nodes\/0\..*\.js$/.test(rel));

function closureOf(seedRels) {
  const seen = new Set();
  const stack = [...seedRels];
  while (stack.length) {
    const rel = stack.pop();
    if (!rel || seen.has(rel)) continue;
    const file = files.get(rel);
    if (!file) continue;
    seen.add(rel);
    for (const dep of file.static) stack.push(dep);
  }
  let bytes = 0;
  const list = [];
  for (const rel of [...seen].sort()) {
    const file = files.get(rel);
    if (!file) continue;
    bytes += file.size;
    list.push({ rel, size: file.size });
  }
  return { files: list, bytes };
}

// SvelteKit route boot: __vite__mapDeps lists the modules preloaded for the
// page node. Those include the route CSS, which is *not* in index.html.
const routePrefetch = [];
if (entryApp && files.has(entryApp)) {
  const appSource = readFileSync(join(BUILD, entryApp), "utf8");
  const mapDeps = /const __vite__mapDeps=\(i,m=__vite__mapDeps,d=\(m\.f\|\|\(m\.f=\[([^\]]*)\]\)\)\)=>i\.map\(i=>d\[i\]\)/
    .exec(appSource);
  if (mapDeps) {
    const listed = [...mapDeps[1].matchAll(/"([^"]+)"/g)]
      .map((m) => resolvePath(entryApp, m[1]));
    routePrefetch.push(...listed);
  }
}

const initialSeeds = [...new Set([...modulePreloads, entryStart, entryApp, node0, ...routePrefetch])]
  .filter(Boolean);
const initialJs = closureOf(initialSeeds.filter((rel) => files.has(rel)));
const initialCss = initialSeeds.filter((rel) => rel.endsWith(".css"));

const workerEntry = [...files.keys()].find((rel) => /^_app\/immutable\/workers\/markdown\.worker-.*\.js$/.test(rel));
const workerClosure = workerEntry ? closureOf([workerEntry]) : { files: [], bytes: 0 };

// ---- worker/main duplication ----
// The main and worker builds emit the same module bodies with different
// generated chunk names, wrappers and preload helpers. Compare the multiset of
// code tokens (identifier runs) instead of raw text: that survives chunk-name
// and wrapper differences, while unrelated chunks stay well below 0.95.
function tokenBag(rel) {
  const source = readFileSync(join(BUILD, rel), "utf8").replace(
    /\.\.?\/(?:\.\.\/)?(?:assets|chunks\/)?[A-Za-z0-9_.-]+\.(?:js|css)/g,
    '"__ASSET__"',
  );
  const bag = new Map();
  for (const token of source.match(/[A-Za-z0-9_$]{4,}/g) ?? []) {
    bag.set(token, (bag.get(token) ?? 0) + 1);
  }
  return bag;
}
function dice(a, b) {
  const total = (bag) => {
    let sum = 0;
    for (const count of bag.values()) sum += count;
    return sum;
  };
  const [small, large] = a.size <= b.size ? [a, b] : [b, a];
  let shared = 0;
  for (const [token, count] of small) shared += Math.min(count, large.get(token) ?? 0);
  return (2 * shared) / (total(a) + total(b));
}
const mainChunks = [...files.keys()].filter((rel) => rel.startsWith("_app/immutable/chunks/"));
const workerChunks = [...files.keys()].filter((rel) => rel.startsWith("_app/immutable/workers/"));
const workerTokens = new Map(workerChunks.map((rel) => [rel, tokenBag(rel)]));
const mainTokens = new Map(mainChunks.map((rel) => [rel, tokenBag(rel)]));
const duplicatePairs = [];
for (const workerRel of workerChunks) {
  const workerSize = files.get(workerRel)?.size ?? 0;
  const workerBag = workerTokens.get(workerRel);
  let best = { rel: null, score: 0, mainSize: 0 };
  for (const mainRel of mainChunks) {
    const mainSize = files.get(mainRel)?.size ?? 0;
    if (Math.min(mainSize, workerSize) / Math.max(mainSize, workerSize) < 0.9) continue;
    const score = dice(workerBag, mainTokens.get(mainRel));
    if (score > best.score) best = { rel: mainRel, score, mainSize };
  }
  if (best.score >= 0.95) {
    duplicatePairs.push({
      workerRel,
      workerSize,
      mainRel: best.rel,
      mainSize: best.mainSize,
      tokenDice: Number(best.score.toFixed(4)),
    });
  }
}
const duplicatedBytes = duplicatePairs.reduce((sum, pair) => sum + Math.min(pair.workerSize, pair.mainSize), 0);

// --- chunk content fingerprinting (occurrence thresholds reject one-off UI strings)
const MARKERS = {
  shiki: /shikiji|createHighlighter|codeToTokens|\bGrammarState\b/gi,
  "shiki-lang": /@shikijs\/langs|langs\/(bash|cpp|python|rust)/gi,
  mermaid: /mermaid\.render|mermaidAPI|flowchart|sequenceDiagram/gi,
  katex: /KaTeX|katex/gi,
  unified: /micromark|mdast-util|hast-util|unist-util/gi,
  remark: /remark-gfm|remark-parse|remark-math|remark-rehype/gi,
  rehype: /rehype-raw|rehype-sanitize|rehype-katex|rehype-stringify/gi,
  dompurify: /DOMPurify|ADD_ATTR|FORBID_TAGS/g,
  epub: /spine|publication\.json|nav\.xhtml|opf/gi,
  svelte: /svelte|hydrat/gi,
  tauri: /__TAURI_INTERNALS__|plugin:event|plugin:dialog/gi,
};
const fingerprints = [];
for (const [rel, file] of files) {
  const source = readFileSync(join(BUILD, rel), "utf8");
  const tags = [];
  for (const [tag, regex] of Object.entries(MARKERS)) {
    const count = (source.match(regex) ?? []).length;
    if (count >= 3) tags.push(`${tag}:${count}`);
  }
  if (tags.length) fingerprints.push({ rel, size: file.size, tags });
}

// --- reverse import graph + dynamic hook details ----------------------------
const importers = new Map();
for (const [rel, file] of files) {
  for (const dep of file.static) {
    if (!importers.has(dep)) importers.set(dep, []);
    importers.get(dep).push({ rel, kind: "static" });
  }
  for (const dep of file.dynamic) {
    if (!importers.has(dep)) importers.set(dep, []);
    importers.get(dep).push({ rel, kind: "dynamic" });
  }
}

// Route component chunk: the chunk statically imported by nodes/2.
const routeNode = [...files.keys()].find((rel) => /^_app\/immutable\/nodes\/2\..*\.js$/.test(rel));
const pageChunk = routeNode
  ? [...(files.get(routeNode)?.static ?? [])].find((rel) => files.has(rel))
  : routePrefetch.find((rel) => /chunks\/Cf-/.test(rel));
const hookDetails = (rel) => ({
  rel,
  size: files.get(rel)?.size ?? assetSize(rel),
  dynamic: files.get(rel)?.dynamic ?? [],
});

// Full closure following static *and* dynamic imports (upper bound of what a
// feature can pull in if every lazy branch executes).
function fullClosure(seedRels) {
  const seen = new Set();
  const stack = [...seedRels];
  let bytes = 0;
  while (stack.length) {
    const rel = stack.pop();
    if (!rel || seen.has(rel)) continue;
    const file = files.get(rel);
    if (!file) continue;
    seen.add(rel);
    bytes += file.size;
    for (const dep of file.static) stack.push(dep);
    for (const dep of file.dynamic) stack.push(dep);
  }
  return { bytes, fileCount: seen.size, rels: [...seen] };
}

const pageDynamic = pageChunk ? files.get(pageChunk)?.dynamic ?? [] : [];
const publicPrefixes = {
  markdown_fallback_main: "_app/immutable/chunks/B-yrRDoM.js",
  mermaid: "_app/immutable/chunks/8Nzaj9JP.js",
  epub_component: "_app/immutable/chunks/",
  dompurify: "_app/immutable/chunks/Csrj9YNg.js",
};
const graphClosures = {};
for (const dep of [...new Set([...pageDynamic, ...["_app/immutable/chunks/8Nzaj9JP.js","_app/immutable/chunks/B-yrRDoM.js"].filter((rel)=>files.has(rel))])]) {
  const closure = fullClosure([dep]);
  graphClosures[dep] = {
    root: dep,
    rootSize: files.get(dep)?.size ?? 0,
    ...closure,
  };
}
graphClosures.worker_full = workerEntry
  ? { root: workerEntry, ...fullClosure([workerEntry]) }
  : null;

// Distinct markdown pipeline copies: worker static graph vs main-thread fallback.
const workerFullSet = new Set(graphClosures.worker_full?.rels ?? []);
const fallbackFullSet = new Set(
  (() => {
    const closure = fullClosure(["_app/immutable/chunks/B-yrRDoM.js"]);
    return closure.rels;
  })(),
);
const overlap = [];
for (const rel of workerFullSet) {
  if (!fallbackFullSet.has(rel)) continue;
  const size = files.get(rel)?.size ?? 0;
  overlap.push({ rel, size });
}
const pipelineDuplication = {
  workerFullBytes: graphClosures.worker_full?.bytes ?? 0,
  fallbackFullBytes: graphClosures["_app/immutable/chunks/B-yrRDoM.js"]?.bytes ?? 0,
  sharedFileCount: overlap.length,
  sharedBytes: overlap.reduce((sum, entry) => sum + entry.size, 0),
  sharedFiles: overlap.sort((a, b) => b.size - a.size),
};

// --- totals ---------------------------------------------------------------
let totalBuildBytes = 0;
const walkAll = (dir) => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const abs = join(dir, entry.name);
    if (entry.isDirectory()) walkAll(abs);
    else totalBuildBytes += statSync(abs).size;
  }
};
walkAll(BUILD);
const sumBy = (predicate) =>
  [...files.values()].filter((file) => predicate(file.rel)).reduce((sum, file) => sum + file.size, 0);

const report = {
  generatedFrom: "apps/desktop/build (existing artifact; no build executed)",
  buildIndexMtime: statSync(join(BUILD, "index.html")).mtime.toISOString(),
  totals: {
    totalBuildBytes,
    allJsBytes: [...files.values()].reduce((sum, file) => sum + file.size, 0),
    chunksBytes: sumBy((rel) => rel.startsWith("_app/immutable/chunks/")),
    workersBytes: sumBy((rel) => rel.startsWith("_app/immutable/workers/")),
    nodesBytes: sumBy((rel) => rel.startsWith("_app/immutable/nodes/")),
    entryBytes: sumBy((rel) => rel.startsWith("_app/immutable/entry/")),
    cssFiles: [...nonJs.entries()].filter(([rel]) => rel.endsWith(".css")).map(([rel, size]) => ({ rel, size })),
    fontsBytes: [...nonJs.entries()].filter(([rel]) => /\.(ttf|woff2?)$/.test(rel)).reduce((sum, [, size]) => sum + size, 0),
    fontFileCount: [...nonJs.keys()].filter((rel) => /\.(ttf|woff2?)$/.test(rel)).length,
  },
  firstLoad: {
    modulePreloads,
    routePrefetch,
    staticClosure: initialJs,
    jsBytes: initialJs.bytes,
    css: initialCss.map((rel) => ({ rel, size: assetSize(rel) })),
    cssBytes: initialCss.reduce((sum, rel) => sum + assetSize(rel), 0),
    totalBytes: initialJs.bytes + initialCss.reduce((sum, rel) => sum + assetSize(rel), 0),
    fileCount: initialJs.files.length + initialCss.length,
  },
  worker: {
    entry: workerEntry,
    entrySize: workerEntry ? files.get(workerEntry)?.size ?? 0 : 0,
    dynamic: workerEntry ? files.get(workerEntry)?.dynamic ?? [] : [],
    staticClosureBytes: workerClosure.bytes,
    staticClosureFiles: workerClosure.files,
    dynamicClosure: (() => {
      const dynamic = workerEntry ? files.get(workerEntry)?.dynamic ?? [] : [];
      const dyn = closureOf(dynamic);
      return { bytes: dyn.bytes, fileCount: dyn.files.length, files: dyn.files };
    })(),
  },
  duplication: {
    pairs: duplicatePairs,
    duplicatedBytes,
    workerChunksBytes: [...files.values()]
      .filter((file) => file.rel.startsWith("_app/immutable/workers/"))
      .reduce((sum, file) => sum + file.size, 0),
    mainChunksBytes: sumBy((rel) => rel.startsWith("_app/immutable/chunks/")),
  },
  dynamicHooks: {
    pageChunk: pageChunk ? hookDetails(pageChunk) : null,
    pageDynamic: pageDynamic.map(hookDetails),
  },
  graphClosures,
  pipelineDuplication,
  biggestFiles: [...files.values()]
    .sort((a, b) => b.size - a.size)
    .slice(0, 25)
    .map((file) => ({
      rel: file.rel,
      size: file.size,
      inInitial: initialJs.files.some((entry) => entry.rel === file.rel),
      inWorkerStatic: workerClosure.files.some((entry) => entry.rel === file.rel),
      importedBy: importers.get(file.rel) ?? [],
    })),
  fingerprints: fingerprints.sort((a, b) => b.size - a.size),
};

if (process.argv.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const kb = (n) => `${(n / 1024).toFixed(1)} KiB`;
  console.log(`build index.html mtime: ${report.buildIndexMtime}`);
  console.log(`total build bytes: ${report.totals.totalBuildBytes} | js: ${kb(report.totals.allJsBytes)} | fonts: ${kb(report.totals.fontsBytes)} in ${report.totals.fontFileCount} files`);
  console.log(`  chunks: ${kb(report.totals.chunksBytes)} | workers: ${kb(report.totals.workersBytes)}`);
  console.log(`  css: ${report.totals.cssFiles.map((f) => `${f.rel} ${kb(f.size)}`).join(" | ")}`);
  console.log("");
  console.log(`first load: ${kb(report.firstLoad.totalBytes)} = js ${kb(report.firstLoad.jsBytes)} + css ${kb(report.firstLoad.cssBytes)} (${report.firstLoad.fileCount} files)`);
  console.log("first-load static closure:");
  for (const file of report.firstLoad.staticClosure.files) console.log(`  ${file.rel} ${kb(file.size)}`);
  for (const file of report.firstLoad.css) console.log(`  ${file.rel} ${kb(file.size)}`);
  console.log("");
  console.log(`worker entry: ${report.worker.entry} ${kb(report.worker.entrySize)}`);
  console.log(`worker static closure: ${kb(report.worker.staticClosureBytes)}`);
  for (const file of report.worker.staticClosureFiles) console.log(`  ${file.rel} ${kb(file.size)}`);
  console.log(`worker dynamic closure: ${kb(report.worker.dynamicClosure.bytes)} over ${report.worker.dynamicClosure.fileCount} files`);
  for (const file of report.worker.dynamicClosure.files) console.log(`  ${file.rel} ${kb(file.size)}`);
  console.log(`duplicated worker<->main: ${kb(report.duplication.duplicatedBytes)} over ${report.duplication.pairs.length} pairs`);
  console.log("");
  console.log("dynamic graph closures (static+dynamic, upper bound):"); for (const [root, closure] of Object.entries(report.graphClosures)) { if (!closure) continue; console.log(`  ${root}: root ${kb(closure.rootSize)} => full ${kb(closure.bytes)} over ${closure.fileCount} files`); } console.log(""); console.log("page dynamic imports:");
  for (const dep of report.dynamicHooks.pageDynamic) {
    const tags = report.fingerprints.find((entry) => entry.rel === dep.rel)?.tags.join(",") ?? "";
    console.log(`  page -> ${dep.rel} ${kb(dep.size)} [${tags}]`);
  }
  console.log("");
  console.log("fingerprinted chunks:");
  for (const entry of report.fingerprints) console.log(`  ${entry.rel} ${kb(entry.size)} [${entry.tags.join(",")}] importedBy=${JSON.stringify(entry.importedBy ?? report.biggestFiles.find((f) => f.rel === entry.rel)?.importedBy ?? [])}`);
}





