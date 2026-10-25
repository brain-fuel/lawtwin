#!/usr/bin/env bash
# Stage a LawSpec CLI into .lawspec-cli/.
#
# By default it unpacks LAWSPEC_VERSION's npm package (npm pack lawspec@<v>),
# which runs as is: node .lawspec-cli/bin/lawspec.mjs.
# LAWSPEC_ARCHIVE stages a locally packed release through the same WASM path.
#
# With LAWSPEC_HOME set, it stages from a source checkout instead, for LawSpec
# builds not yet on npm: it copies the checkout's npm/ folder and swaps the
# WASM launcher for one that calls the checkout's native compiler, so the
# compiler is the one `stack build` made from that checkout's sources. The
# checkout itself is not changed.
#
#   LAWSPEC_VERSION=0.31.0 tools/stage-lawspec.sh
#   LAWSPEC_ARCHIVE=/path/to/lawspec-0.31.0.tgz tools/stage-lawspec.sh
#   LAWSPEC_HOME=/path/to/lawspec tools/stage-lawspec.sh
#   node .lawspec-cli/bin/lawspec.mjs check --config ...
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -n "${LAWSPEC_ARCHIVE:-}" ] && [ -n "${LAWSPEC_HOME:-}" ]; then
  echo "choose LAWSPEC_ARCHIVE or LAWSPEC_HOME, not both" >&2
  exit 1
fi
if [ -z "${LAWSPEC_HOME:-}" ]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  if [ -n "${LAWSPEC_ARCHIVE:-}" ]; then
    cp "$LAWSPEC_ARCHIVE" "$tmp/lawspec.tgz"
    tgz="lawspec.tgz"
    source_description="local npm archive"
  else
    : "${LAWSPEC_VERSION:?set LAWSPEC_VERSION, LAWSPEC_ARCHIVE or LAWSPEC_HOME}"
    tgz="$(cd "$tmp" && npm pack --silent "lawspec@$LAWSPEC_VERSION")"
    source_description="npm"
  fi
  tar -xzf "$tmp/$tgz" -C "$tmp"
  actual_version="$(node -p 'require(process.argv[1]).version' "$tmp/package/package.json")"
  if [ -n "${LAWSPEC_VERSION:-}" ] && [ "$actual_version" != "$LAWSPEC_VERSION" ]; then
    echo "LawSpec package is $actual_version; contracts require $LAWSPEC_VERSION" >&2
    exit 1
  fi
  npm install --prefix "$tmp/package" --omit=dev --ignore-scripts --no-audit --no-fund
  rm -rf .lawspec-cli
  mv "$tmp/package" .lawspec-cli
  echo "staged LawSpec $actual_version from $source_description in .lawspec-cli"
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
