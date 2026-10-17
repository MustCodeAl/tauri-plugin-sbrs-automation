// Builds src/bridge.js from src/bridge.ts: one self-contained script, not
// minified, with a first line that holds the SHA-256 of bridge.ts. A Rust test
// (tests/bridge_js.rs) compares that hash with bridge.ts, so a stale
// bridge.js fails `cargo test`. Run it with `bun run bridge`.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const source = new URL('../src/bridge.ts', import.meta.url);
const target = new URL('../src/bridge.js', import.meta.url);

const build = spawnSync(
  process.execPath,
  ['build', source.pathname, '--target=browser', '--format=iife'],
  { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
);
if (build.status !== 0) {
  process.stderr.write(build.stderr);
  process.exit(build.status ?? 1);
}

const hash = createHash('sha256').update(readFileSync(source)).digest('hex');
const header = `// Generated from src/bridge.ts (sha256: ${hash}); edit bridge.ts and run "bun run bridge".\n`;
writeFileSync(target, header + build.stdout);
