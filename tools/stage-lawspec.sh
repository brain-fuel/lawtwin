#!/usr/bin/env bash
# Stage a LawSpec CLI from a source checkout, for LawSpec builds not yet on
# npm. It copies the checkout's npm/ folder into .lawspec-cli/ and swaps the
# WASM launcher for one that calls the checkout's native compiler, so the
# compiler is the one `stack build` made from that checkout's sources. The
# checkout itself is not changed.
#
#   LAWSPEC_HOME=/path/to/lawspec tools/stage-lawspec.sh
#   node .lawspec-cli/bin/lawspec.mjs check --config ...
set -euo pipefail
cd "$(dirname "$0")/.."
: "${LAWSPEC_HOME:?set LAWSPEC_HOME to a LawSpec checkout built with stack build}"
core="$(cd "$LAWSPEC_HOME" && stack --no-terminal path --local-install-root)/bin/lawspec-core"
test -x "$core" || { echo "no $core; run stack build in $LAWSPEC_HOME" >&2; exit 1; }
rm -rf .lawspec-cli
cp -R "$LAWSPEC_HOME/npm" .lawspec-cli
# The CLI fingerprints core.wasm for its cache; the native core stands in.
shasum -a 256 "$core" > .lawspec-cli/core.wasm
cat > .lawspec-cli/launcher.mjs <<JS
// Written by tools/stage-lawspec.sh: each request runs the native compiler.
import { spawnSync } from "node:child_process";
const core = "${core}";
export async function loadCore() {
  return async (input) => {
    const run = spawnSync(core, { input: JSON.stringify(input), maxBuffer: 1 << 30 });
    if (run.status !== 0) throw new Error("lawspec-core failed: " + run.stderr);
    return JSON.parse(run.stdout.toString());
  };
}
JS
echo "staged LawSpec $(cd "$LAWSPEC_HOME" && git describe --always --dirty) in .lawspec-cli"
