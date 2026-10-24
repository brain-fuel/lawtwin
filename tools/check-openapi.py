#!/usr/bin/env python3
"""Check importer reproducibility, or compile its fixtures on the pinned compiler.

Fresh outputs live outside the maintained contracts. Handwritten files remain
owned by their authors and are never compared to new-import placeholders.

ref:DEC-http-contract-registry ref:DEC-dependency-contracts-in-lawspec
"""
import argparse
import os
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent

def run(args, **kwargs):
    """Run one check, preserving a failing command as a failing acceptance step."""
    subprocess.run([str(x) for x in args], cwd=ROOT, check=True, **kwargs)

def main():
    """Compare owned imports or compile fresh contracts through their public adapters."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["reproducible", "compile"])
    parser.add_argument("--lawtwin", default="target/release/lawtwin")
    parser.add_argument("--lawspec", default=".lawspec-cli/bin/lawspec.mjs")
    args = parser.parse_args()
    artifact = ROOT / ".artifacts"
    artifact.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="openapi-check-", dir=artifact) as scratch:
        scratch = Path(scratch)
        docs = [("openapi/petstore3.json", "lawtwin.petstore", "petstore"),
                ("openapi/github-labels-milestones.json", "lawtwin.github", "github")]
        if args.mode == "compile":
            docs += [("crates/lawtwin/tests/fixtures/petstore.yaml", "lawtwin.small", "small"),
                     ("crates/lawtwin/tests/fixtures/features-3.1.yaml", "lawtwin.tour", "tour")]
        for document, package, name in docs:
            dest = scratch / name
            run([args.lawtwin, "import", "openapi", document, "--package", package, "--out", dest])
            if args.mode == "reproducible":
                paths = ["lawspec.edn", "src/api.lawspec", "src/model.lawspec", "lawtwin-openapi.json", "lawtwin-embed.rs",
                         f"test/rust/src/{package.replace('.', '/')}.rs", f"test/python/src/{package.replace('.', '/')}/__init__.py"]
                stale = [p for p in paths if not (ROOT / "contracts" / name / p).is_file()
                         or (dest / p).read_bytes() != (ROOT / "contracts" / name / p).read_bytes()]
                if stale:
                    raise SystemExit(f"{name}: stale importer outputs: {', '.join(stale)}; run make openapi-import")
                continue
            config = dest / "test/lawspec.edn"
            run(["node", args.lawspec, "check", "--config", config])
            run(["node", args.lawspec, "generate", "--target", "rust", "--config", config])
            # Compile the emitted adapter as part of a tiny HTTP-core harness,
            # with the generated model linked as a normal Cargo dependency.
            core = dest / "adapter-check"
            core.mkdir()
            alias = package.replace(".", "_") + "_model"
            (core / "Cargo.toml").write_text(f'''[package]
name = "adapter-{name}"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
{alias} = {{package="{package.replace('.', '-')}-model", path="../test/rust"}}
lawtwin-wire = {{path={str(ROOT / 'crates/lawtwin-wire')!r}}}
''')
            (core / "src").mkdir()
            (core / "src/lib.rs").write_text(f'''pub use lawtwin_wire::V;
#[path = {json.dumps(str(ROOT / 'twins/http/rust/src/model.rs'), ensure_ascii=False)}]
pub mod model;
#[allow(warnings, clippy::all)]
#[path = "../../lawtwin-embed.rs"]
pub mod adapter;
''')
            run(["cargo", "check", "--manifest-path", core / "Cargo.toml"],
                env={**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/openapi-check")})
    print(f"OpenAPI {args.mode}: {len(docs)} documents passed")

if __name__ == "__main__":
    main()
