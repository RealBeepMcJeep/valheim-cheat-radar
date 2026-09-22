import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';

const root = resolve(import.meta.dirname, '../..');
const local = resolve(root, '.tools/wasm-pack/bin/wasm-pack.exe');
const command = existsSync(local) ? local : 'wasm-pack';
execFileSync(command, ['build', '--target', 'web', '--out-dir', 'web/wasm/pkg', '--release'], {
  cwd: root,
  stdio: 'inherit',
});

writeFileSync(
  resolve(root, 'web/wasm/pkg/.gitignore'),
  '# The generated WebAssembly package is intentionally committed for static deployment.\n',
);

// Normalize generated glue so repeated builds are idempotent and lint-clean.
// Constructing an Error with or without `new` yields the same instance.
const glue = resolve(root, 'web/wasm/pkg/valheim_backup_cheat_scanner.js');
writeFileSync(glue, readFileSync(glue, 'utf8').replaceAll('throw Error(', 'throw new Error('));
