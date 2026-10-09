# LawTwin development tasks. `make` lists them. Every check runs locally;
# there is no hosted CI.

LAWSPEC   := npx --yes lawspec@0.21.0
CONTRACTS := contracts/kafka
TARGET    ?=
TARGETARG := $(if $(TARGET),--target $(TARGET),)
PYTHON313 ?= python3.13
CANON     ?= stack --no-terminal --stack-yaml $(CANON_HOME)/stack.yaml exec canon --

.DEFAULT_GOAL := help
.PHONY: help contract-check contract-generate contract-generate-check python-env contract-test canon ci

help: ## List the tasks
	@awk -F':.*## ' '/^[a-z-]+:.*## / {printf "  %-24s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

contract-check: ## Compile each contract package and its laws
	@for c in $(CONTRACTS); do \
	  $(LAWSPEC) package --project $$c && \
	  $(LAWSPEC) check --config $$c/test/lawspec.json || exit 1; \
	done

contract-generate: ## Regenerate each contract's test project
	@for c in $(CONTRACTS); do $(LAWSPEC) generate $(TARGETARG) --config $$c/test/lawspec.json || exit 1; done

contract-generate-check: ## Fail if a generated file of a contract's test project is stale
	@for c in $(CONTRACTS); do $(LAWSPEC) generate --check $(TARGETARG) --config $$c/test/lawspec.json || exit 1; done

python-env: ## Make each contract test project's Python environment
	@for c in $(CONTRACTS); do \
	  test -x $$c/test/python/.venv/bin/python || \
	    ( $(PYTHON313) -m venv $$c/test/python/.venv && \
	      $$c/test/python/.venv/bin/pip install -q -e "$$c/test/python[test]" ) || exit 1; \
	done

contract-test: python-env ## Run each contract's laws (TARGET=rust or python for one)
	@for c in $(CONTRACTS); do $(LAWSPEC) test $(TARGETARG) --config $$c/test/lawspec.json || exit 1; done

canon: ## Check the canonical format with canon 0.2.0 (needs CANON_HOME)
	@test -n "$(CANON_HOME)" || { echo "set CANON_HOME to a canon 0.2.0 checkout built with stack build"; exit 1; }
	@ln -sfn "$(CANON_HOME)" .canon
	$(CANON) check

ci: contract-check contract-generate-check contract-test canon ## Every check
