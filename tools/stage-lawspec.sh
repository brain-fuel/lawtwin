#!/usr/bin/env bash
# Stage a LawSpec CLI into .lawspec-cli/.
#
# By default it unpacks LAWSPEC_VERSION's npm package (npm pack lawspec@<v>),
# which runs as is: node .lawspec-cli/bin/lawspec.mjs.
#
# With LAWSPEC_HOME set, it stages from a source checkout instead, for LawSpec
# builds not yet on npm: it copies the checkout's npm/ folder and swaps the
# WASM launcher for one that calls the checkout's native compiler, so the
# compiler is the one `stack build` made from that checkout's sources. The
# checkout itself is not changed.
#
#   LAWSPEC_VERSION=0.31.0 tools/stage-lawspec.sh
#   LAWSPEC_HOME=/path/to/lawspec tools/stage-lawspec.sh
#   node .lawspec-cli/bin/lawspec.mjs check --config ...
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -z "${LAWSPEC_HOME:-}" ]; then
  : "${LAWSPEC_VERSION:?set LAWSPEC_VERSION to a LawSpec release on npm, or LAWSPEC_HOME to a checkout}"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  tgz="$(cd "$tmp" && npm pack --silent "lawspec@$LAWSPEC_VERSION")"
  tar -xzf "$tmp/$tgz" -C "$tmp"
  rm -rf .lawspec-cli
  mv "$tmp/package" .lawspec-cli
  npm install --prefix .lawspec-cli --omit=dev --ignore-scripts --no-audit --no-fund
  echo "staged LawSpec $(node .lawspec-cli/bin/lawspec.mjs --version 2>/dev/null || echo "$LAWSPEC_VERSION") from npm in .lawspec-cli"
  exit 0
fi
actual_version="$(node -p 'require(process.argv[1]).version' "$(cd "$LAWSPEC_HOME" && pwd)/npm/package.json")"
if [ -n "${LAWSPEC_VERSION:-}" ] && [ "$actual_version" != "$LAWSPEC_VERSION" ]; then
  echo "LawSpec checkout is $actual_version; contracts require $LAWSPEC_VERSION" >&2
  exit 1
fi
core="$(cd "$LAWSPEC_HOME" && stack --no-terminal path --local-install-root)/bin/lawspec-core"
test -x "$core" || { echo "no $core; run stack build in $LAWSPEC_HOME" >&2; exit 1; }
rm -rf .lawspec-cli
cp -R "$LAWSPEC_HOME/npm" .lawspec-cli
# The CLI fingerprints core.wasm for its cache; the native core stands in.
shasum -a 256 "$core" > .lawspec-cli/core.wasm
core_json="$(node -p 'JSON.stringify(process.argv[1])' "$core")"
cat > .lawspec-cli/launcher.mjs <<JS
// Written by tools/stage-lawspec.sh: each request runs the native compiler.
import { spawnSync } from "node:child_process";
const core = ${core_json};
export async function loadCore() {
  return async (input) => {
    const run = spawnSync(core, { input: JSON.stringify(input), maxBuffer: 1 << 30 });
    if (run.status !== 0) throw new Error("lawspec-core failed: " + run.stderr);
    return JSON.parse(run.stdout.toString());
  };
}
JS
echo "staged LawSpec $(cd "$LAWSPEC_HOME" && git describe --always --dirty) in .lawspec-cli"
