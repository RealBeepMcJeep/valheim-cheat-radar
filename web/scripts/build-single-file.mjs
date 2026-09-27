// Build one self-contained HTML file: the normal bundle with the module worker and the WASM
// inlined, so a server admin can download one file and open it directly (file:// included).
//
// Three target-specific facts drive this script, all established empirically in a real browser:
//   - a `blob:` module worker is refused from a `file://` page, while a `data:` module worker runs;
//   - Chrome refuses URLs over 2 MiB, so the worker's data: URL cannot also carry the WASM (base64
//     twice over, it pushed the URL to 2.25 MiB and the worker silently failed to start);
//   - so the WASM rides in the page as a base64 global, and the page posts the bytes to the worker
//     before any work (see `ensureWorker` in src/scan.ts).
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const dist = resolve(import.meta.dirname, '../dist');
const outDir = resolve(import.meta.dirname, '../dist-single');
const outFile = resolve(outDir, 'valheim-cheat-radar.html');

const html = readFileSync(resolve(dist, 'index.html'), 'utf8');
const scriptMatch = html.match(/<script type="module"[^>]*src="([^"]+)"[^>]*><\/script>/);
const styleMatch = html.match(/<link rel="stylesheet"[^>]*href="([^"]+)"[^>]*>/);
if (!scriptMatch || !styleMatch) {
  throw new Error('dist/index.html has an unexpected shape; cannot inline the bundle');
}

const readAsset = (url) => readFileSync(resolve(dist, url.replace(/^\.\//, '')), 'utf8');
const mainJs = readAsset(scriptMatch[1]);
const css = readAsset(styleMatch[1]);

// The worker is created inline in the bundle as `new Worker(new URL(...), {type:"module"})`.
const workerMatch = mainJs.match(
  /new Worker\(new URL\(""\+new URL\("([^"]+\.js)",import\.meta\.url\)\.href,import\.meta\.url\),\{type:"module"\}\)/,
);
if (!workerMatch) {
  throw new Error('worker construction not found in the bundle; cannot inline the worker');
}

const workerJs = readFileSync(resolve(dist, 'assets', workerMatch[1]), 'utf8');
const wasmName = workerJs.match(/valheim_backup_cheat_scanner_bg-[\w-]+\.wasm/);
if (!wasmName) {
  throw new Error('wasm asset name not found in the worker bundle');
}
const wasm = readFileSync(resolve(dist, 'assets', wasmName[0]));

// Replacement *functions* throughout: a replacement string would expand `$&`, `$'` and friends,
// and minified code can contain them (a `$&` once spliced the <script> tag into the bundle).
const workerUrl = `data:text/javascript;base64,${Buffer.from(workerJs).toString('base64')}`;
const URL_LIMIT = 2 * 1024 * 1024;
if (workerUrl.length > URL_LIMIT * 0.9) {
  throw new Error(`worker data: URL is ${workerUrl.length} chars, too close to Chrome's ${URL_LIMIT}-char URL limit`);
}
const patchedJs = mainJs
  .replace(workerMatch[0], () => `new Worker("${workerUrl}",{type:"module"})`)
  .replace(/\/\/# sourceMappingURL=\S+\s*$/, '');
if (patchedJs.includes('import.meta.url')) {
  throw new Error('unexpected import.meta.url left in the bundle; the inline worker would break');
}

const inlined = html
  .replace(styleMatch[0], () => `<style>${css}</style>`)
  .replace(
    scriptMatch[0],
    () => `<script>globalThis.__VCR_WASM_BASE64__="${wasm.toString('base64')}";</script>`
      + `<script type="module">${patchedJs.replaceAll('</script', '<\\/script')}</script>`,
  );
if (inlined.includes(scriptMatch[0]) || inlined.includes(styleMatch[0])) {
  throw new Error('an original asset tag survived inlining; the single file would be broken');
}

mkdirSync(outDir, { recursive: true });
writeFileSync(outFile, inlined);
const kib = (inlined.length / 1024).toFixed(0);
console.log(`${outFile} (${kib} KiB, wasm inlined as base64)`);
