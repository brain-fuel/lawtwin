# LawTwin development tasks. `make` lists them. Every check runs locally;
# there is no hosted CI.

# The contracts and twin model consumers use LawSpec 0.26.0, configuration
# version 2 and profile 1.0-rc.2, with explicit verification layers.
# `make lawspec-stage` stages the CLI from npm; set LAWSPEC_HOME to
# stage it from a LawSpec checkout instead (for builds not yet on npm), or set
# LAWSPEC to another way of running it.
LAWSPEC_VERSION  := 0.26.0
LAWSPEC_REQUIRED := LawSpec $(LAWSPEC_VERSION)
LAWSPEC_HOME ?=
LAWSPEC   ?= node .lawspec-cli/bin/lawspec.mjs
CONTRACTS := contracts/kafka contracts/petstore contracts/github
LAWTWIN   := target/release/lawtwin
TARGET    ?=
TARGETARG := $(if $(TARGET),--target $(TARGET),)
PYTHON313 ?= python3.13
GRADLE_HOME ?=
CANON     ?= stack --no-terminal --stack-yaml $(CANON_HOME)/stack.yaml exec canon --
PODMAN    ?= podman
IMAGE     ?= lawtwin/kafka:0.1.0
# The Kafka core the conformance kit checks: a command, or image:<image>.
CORE      ?= target/release/lawtwin-kafka
HTTP_IMAGE ?= lawtwin/http:0.1.0
OPENAPI_GENERATOR ?= docker.io/openapitools/openapi-generator-cli:v7.9.0
# The HTTP core core-conformance-http checks: CORE when it names one,
# otherwise the Rust core.
HTTP_CORE := $(if $(filter target/release/lawtwin-kafka,$(CORE)),target/release/lawtwin-http,$(CORE))
# The real broker conform-kafka and fidelity-test start (tools/real-kafka.sh),
# and the twin release a conformance report is for.
KAFKA_IMAGE ?= docker.io/apache/kafka:4.3.1
KAFKA_PORT  ?= 39092
RELEASE     ?= 0.1.0

comma := ,

# Kotlin test projects run `gradle`; GRADLE_HOME puts one on the path.
export PATH := $(if $(GRADLE_HOME),$(abspath $(GRADLE_HOME))/bin:,)$(PATH)

# The OpenAPI documents the importer is tested on, and the contracts made
# from them: document, package, directory.
OPENAPI_IMPORTS := \
  openapi/petstore3.json,lawtwin.petstore,contracts/petstore \
  openapi/github-labels-milestones.json,lawtwin.github,contracts/github

.DEFAULT_GOAL := help
.PHONY: help lawspec-version lawspec-stage build test openapi-import openapi-import-check openapi-compile-check contract-check contract-generate contract-generate-check python-env contract-test core-conformance golden image compose-up compose-down core-conformance-http openapi-client-http golden-http image-http compose-up-http compose-down-http kafka-up kafka-down conform-kafka fidelity-test canon ci

help: ## List the tasks
	@awk -F':.*## ' '/^[a-z-]+:.*## / {printf "  %-24s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

lawspec-version: ## Say which LawSpec the contracts need
	@echo "$(LAWSPEC_REQUIRED)"

lawspec-stage: ## Stage the LawSpec CLI into .lawspec-cli, from npm or from LAWSPEC_HOME
	LAWSPEC_VERSION="$(LAWSPEC_VERSION)" LAWSPEC_HOME="$(LAWSPEC_HOME)" tools/stage-lawspec.sh

build: ## Build the lawtwin command and the twins' Rust cores
	cargo build --release -p lawtwin -p lawtwin-kafka -p lawtwin-http -p lawtwin-kafka-fidelity

test: ## Run the lawtwin command's and the Rust cores' own tests
	cargo test -p lawtwin -p lawtwin-wire -p lawtwin-kafka -p lawtwin-http -p lawtwin-kafka-fidelity

openapi-import: build ## Import each vendored OpenAPI document into its contract
	@for i in $(OPENAPI_IMPORTS); do \
	  set -- $$(echo $$i | tr , ' '); \
	  $(LAWTWIN) import openapi $$1 --package $$2 --out $$3 || exit 1; \
	done

openapi-import-check: build ## Check importer outputs without changing maintained files
	python3 tools/check-openapi.py reproducible --lawtwin $(LAWTWIN)

openapi-compile-check: build ## Compile fresh OpenAPI 3.0/3.1 imports and their public Rust adapters
	python3 tools/check-openapi.py compile --lawtwin $(LAWTWIN)

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

contract-test: python-env ## Run each contract's laws on every target (TARGET=rust, python, go, java, kotlin, fsharp or csharp for one)
	@for c in $(CONTRACTS); do $(LAWSPEC) test $(TARGETARG) --config $$c/test/lawspec.json || exit 1; done

core-conformance: ## Check a Kafka core with the conformance kit (CORE=<command> or CORE=image:<image>)
	@if [ "$(CORE)" = "target/release/lawtwin-kafka" ]; then cargo build --release -p lawtwin-kafka; fi
	LAWTWIN_CORE="$(if $(filter target/release/lawtwin-kafka,$(CORE)),$(CURDIR)/$(CORE),$(CORE))" LAWTWIN_REQUIRE_CLIENTS=1 \
	  cargo test -p lawtwin-kafka-conformance -- --test-threads=2

golden: ## Record the golden session from the Rust core into twins/kafka/shared/golden
	LAWTWIN_WRITE_GOLDEN=1 cargo test -p lawtwin-kafka-conformance --test golden

image: ## Build the Kafka twin's image with podman, as $(IMAGE) and docker.io/$(IMAGE)
	$(PODMAN) build -f twins/kafka/rust/Containerfile -t $(IMAGE) -t docker.io/$(IMAGE) .

compose-up: ## Run the Kafka twin with podman compose (examples/kafka/compose.yaml)
	$(PODMAN) compose -f examples/kafka/compose.yaml up -d

compose-down: ## Stop the compose example
	$(PODMAN) compose -f examples/kafka/compose.yaml down

kafka-up: ## Start a real Kafka broker ($(KAFKA_IMAGE)) in podman on port $(KAFKA_PORT)
	KAFKA_IMAGE=$(KAFKA_IMAGE) KAFKA_PORT=$(KAFKA_PORT) PODMAN=$(PODMAN) tools/real-kafka.sh up

kafka-down: ## Stop the real Kafka broker
	PODMAN=$(PODMAN) tools/real-kafka.sh down

conform-kafka: build ## Conformance of the Kafka twin against real Kafka; writes twins/kafka/conformance/reports/$(RELEASE)
	@$(MAKE) --no-print-directory kafka-up
	$(LAWTWIN) conform kafka --real 127.0.0.1:$(KAFKA_PORT) --real-name $(patsubst docker.io/%,%,$(KAFKA_IMAGE)) \
	  --release $(RELEASE) --frames --out twins/kafka/conformance/reports/$(RELEASE); status=$$?; \
	  $(MAKE) --no-print-directory kafka-down; exit $$status

fidelity-test: ## Record and replay and the peers against a real Kafka broker in podman
	@$(MAKE) --no-print-directory kafka-up
	LAWTWIN_KAFKA=127.0.0.1:$(KAFKA_PORT) LAWTWIN_REQUIRE_KAFKA=1 cargo test -p lawtwin-kafka-fidelity; status=$$?; \
	  $(MAKE) --no-print-directory kafka-down; exit $$status

core-conformance-http: ## Check an HTTP core with the conformance kit (CORE=<command> or CORE=image:<image>)
	@if [ "$(HTTP_CORE)" = "target/release/lawtwin-http" ]; then cargo build --release -p lawtwin-http; fi
	LAWTWIN_CORE="$(if $(filter target/release/lawtwin-http,$(HTTP_CORE)),$(CURDIR)/$(HTTP_CORE),$(HTTP_CORE))" \
	  cargo test -p lawtwin-http-conformance -- --test-threads=4

openapi-client-http: ## Generate a Python client from the Petstore document with openapi-generator, for the HTTP kit
	mkdir -p target/openapi-client
	$(PODMAN) run --rm -v $(CURDIR)/openapi:/spec:ro -v $(CURDIR)/target/openapi-client:/out $(OPENAPI_GENERATOR) \
	  generate -i /spec/petstore3.json -g python -o /out/python --package-name petstore_client
	python3 -m venv target/openapi-client/venv && target/openapi-client/venv/bin/pip install -q target/openapi-client/python

golden-http: ## Record the HTTP golden sessions from the Rust core into twins/http/shared/golden
	LAWTWIN_WRITE_GOLDEN=1 cargo test -p lawtwin-http-conformance --test golden

image-http: ## Build the HTTP twin's image with podman, as $(HTTP_IMAGE) and docker.io/$(HTTP_IMAGE)
	$(PODMAN) build -f twins/http/rust/Containerfile -t $(HTTP_IMAGE) -t docker.io/$(HTTP_IMAGE) .

compose-up-http: ## Run the HTTP twin with podman compose (examples/http/compose.yaml)
	$(PODMAN) compose -f examples/http/compose.yaml up -d

compose-down-http: ## Stop the HTTP compose example
	$(PODMAN) compose -f examples/http/compose.yaml down

canon: ## Check the canonical format with canon 0.2.2 (needs CANON_HOME)
	@test -n "$(CANON_HOME)" || { echo "set CANON_HOME to a canon 0.2.2 checkout built with stack build"; exit 1; }
	@ln -sfn "$(CANON_HOME)" .canon
	$(CANON) check

ci: test openapi-import-check openapi-compile-check contract-check contract-generate-check contract-test core-conformance core-conformance-http fidelity-test canon ## Every check
