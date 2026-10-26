# HTTP twin 0.2.1: fresh independent provider conformance

These reports were produced by the public `lawtwin conform http` command against
`lawtwin/http:0.2.1`, using the independently written Python Petstore and the
native Swagger Petstore 3 image. `context.json` records the CLI/core binary
hashes, local image IDs, source and contract identities, exact commands,
settings and scope. The declared contract remains `lawtwin.petstore` **0.1.0**.
No historical receipt or image version was relabelled.

Each run uses seed 1, two rounds and retries. Both sides receive the same
selection; the twin is reset before its session. The modeled-resource runs use
44 requests. Adding `--invalid` produces 84 requests, of which 44 are valid
contract calls. Every side checks the seven resource laws for Pet, Order and
User. The real service starts as a fresh process/container; Swagger's image
initialization is preserved.

| Real provider / selection | Requests per side | Real law violations | Real findings | Answer divergences | Report level | Exit |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| python-invalid | 84 | 0 | 0 | 6 | lawful | 0 |
| python | 44 | 0 | 0 | 0 | conformant | 0 |
| python-forget-delete | 44 | 6 | 0 | 6 | divergent | 1 |
| swagger-petstore3 | 84 | 0 | 2 | 36 | lawful | 0 |

All twin sessions have zero law violations, findings and response problems.
All real sessions have zero response problems for this selection. Raw JSON and
command output are retained beside this summary.

## What differs

- **Python, modeled resource traffic:** conformant, with no answer differences.
- **Python with invalid traffic:** lawful, with six differences. The invalid
  `updatePetWithForm`, `uploadFile` and `createUsersWithListInput` calls receive
  `501 Unmodelled` from the hand-written service and `400 BadRequest` from the
  twin, once per round. This does not establish valid behavior for those
  unmodelled operations.
- **Swagger Petstore 3:** lawful, with 36 differences and no resource-law
  violations. The full differences are in `swagger-petstore3.json`; they include
  server-supplied defaults, normalized dates, duplicate/create behavior, missing
  resource status/response differences and omitted required fields. It accepts
  two pets without their required name, recorded as two document findings.
  Consequently its report's `holds` field is false even though its resource-law
  violation list is empty. The lawful report exits 0, as the CLI specifies.
- **Independent negative control:** the same Python implementation started with
  `--bug forget-delete` returns success from deletion while retaining records.
  Six observed violations (pet, order and user in each round) name the deleted
  resource law and the subsequent successful read. It reports divergent and
  exits 1. The twin remains clean. This failure is behavioral, not a build or
  setup error.

## Limits

These seeded runs support the reported finite observations, not complete
provider fidelity. Valid behavior for the eight operations marked unmodelled
in `context.json` is outside the selected resource sequences. Invalid requests
do not fill that gap. Duplicates, abandoned-response/timeouts and
`--all-operations` modes were not selected. The Python provider and Swagger
implementation are independent of the generated reference model; the twin and
its reference model still share generated contract code.

The tested images were local images, identified by immutable local image IDs;
this report does not assert publication or a registry digest. Only the owned
Python processes and named containers were stopped after these runs. No whole
release CI or other provider-family acceptance is claimed here.
