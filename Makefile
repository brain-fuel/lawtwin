# LawTwin development tasks. `make` lists them. Every check runs locally;
# there is no hosted CI.

# The contracts need LawSpec main after the lawspec-gaps and lawspec-codegen
# merges; its npm release is pending. Until it is published, stage the CLI
# from a LawSpec checkout with `make lawspec-stage LAWSPEC_HOME=<checkout>`,
# or set LAWSPEC to another way of running it.
LAWSPEC_REQUIRED := LawSpec main after the lawspec-gaps and lawspec-codegen merges (npm release pending)
LAWSPEC_HOME ?=
LAWSPEC   ?= node .lawspec-cli/bin/lawspec.mjs
CONTRACTS := contracts/kafka contracts/petstore contracts/github
LAWTWIN   := target/release/lawtwin
TARGET    ?=
TARGETARG := $(if $(TARGET),--target $(TARGET),)
PYTHON313 ?= python3.13
GRADLE_HOME ?=
CANON     ?= stack --no-terminal --stack-yaml $(CANON_HOME)/stack.yaml exec canon --

comma := ,

# Kotlin test projects run `gradle`; GRADLE_HOME puts one on the path.
export PATH := $(if $(GRADLE_HOME),$(GRADLE_HOME)/bin:,)$(PATH)

# The OpenAPI documents the importer is tested on, and the contracts made
# from them: document, package, directory.
OPENAPI_IMPORTS := \
  openapi/petstore3.json,lawtwin.petstore,contracts/petstore \
  openapi/github-labels-milestones.json,lawtwin.github,contracts/github

.DEFAULT_GOAL := help
.PHONY: help lawspec-version lawspec-stage build test openapi-import openapi-import-check contract-check contract-generate contract-generate-check python-env contract-test canon ci

help: ## List the tasks
	@awk -F':.*## ' '/^[a-z-]+:.*## / {printf "  %-24s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

lawspec-version: ## Say which LawSpec the contracts need
	@echo "$(LAWSPEC_REQUIRED)"

lawspec-stage: ## Stage the LawSpec CLI from LAWSPEC_HOME into .lawspec-cli
	@test -n "$(LAWSPEC_HOME)" || { echo "set LAWSPEC_HOME to a checkout of $(LAWSPEC_REQUIRED), built with stack build"; exit 1; }
	LAWSPEC_HOME="$(LAWSPEC_HOME)" tools/stage-lawspec.sh

build: ## Build the lawtwin command
	cargo build --release

test: ## Run the lawtwin command's own tests
	cargo test

openapi-import: build ## Import each vendored OpenAPI document into its contract
	@for i in $(OPENAPI_IMPORTS); do \
	  set -- $$(echo $$i | tr , ' '); \
	  $(LAWTWIN) import openapi $$1 --package $$2 --out $$3 || exit 1; \
	done

openapi-import-check: openapi-import ## Fail if an imported contract differs from what the importer writes
	@git diff --exit-code -- $(foreach i,$(OPENAPI_IMPORTS),$(word 3,$(subst $(comma), ,$(i)))) || \
	  { echo "an imported contract is stale: run make openapi-import"; exit 1; }

contract-check: ## Compile each contract package and its laws
	@for c in $(CONTRACTS); do \
	  $(LAWSPEC) package --project $$c && \
	  $(LAWSPEC) check --config $$c/test/lawspec.json || exit 1; \
	done

contract-generate: ## Regenerate each contract's test project (TARGET=... for one target)
	@for c in $(CONTRACTS); do $(LAWSPEC) generate $(TARGETARG) --config $$c/test/lawspec.json || exit 1; done

contract-generate-check: ## Fail if a generated file of a contract's test project is stale
	@for c in $(CONTRACTS); do $(LAWSPEC) generate --check $(TARGETARG) --config $$c/test/lawspec.json || exit 1; done

python-env: ## Make each contract test project's Python environment
	@for c in $(CONTRACTS); do \
	  test -x $$c/test/python/.venv/bin/python || \
	    ( $(PYTHON313) -m venv $$c/test/python/.venv && \
	      $$c/test/python/.venv/bin/pip install -q -e "$$c/test/python[test]" ) || exit 1; \
	done

contract-test: python-env ## Run each contract's laws on every target (TARGET=rust, python, go, java or kotlin for one)
	@for c in $(CONTRACTS); do $(LAWSPEC) test $(TARGETARG) --config $$c/test/lawspec.json || exit 1; done

canon: ## Check the canonical format with canon 0.2.0 (needs CANON_HOME)
	@test -n "$(CANON_HOME)" || { echo "set CANON_HOME to a canon 0.2.0 checkout built with stack build"; exit 1; }
	@ln -sfn "$(CANON_HOME)" .canon
	$(CANON) check

ci: test openapi-import-check contract-check contract-generate-check contract-test canon ## Every check
