# dockerlint

A Dockerfile linter for the handful of anti-patterns that show up in
almost every real Dockerfile that hasn't been through one already:
floating base image tags, `ADD` used where `COPY` would do, containers
that never drop root, missing healthchecks, and `apt-get install` lines
that bloat every layer they touch. `hadolint` (Haskell) already does
this well but drags a Haskell runtime along; this is a static binary
covering the rules that matter most, with no GHC anywhere near the
critical path.

## Usage

```bash
dockerlint path/to/Dockerfile              # human-readable findings, exits 1 if any
dockerlint path/to/Dockerfile --json        # machine-readable findings
```

Exit code `0` means clean, `1` means at least one finding, `2` means the
file couldn't even be read.

## Rules

- **`unpinned-base-image`** — `FROM ubuntu` (no tag) or `FROM
  ubuntu:latest` (explicit but equally unreproducible). A `FROM` that
  references an earlier build stage by name (`FROM builder AS final`),
  `FROM scratch`, an image pinned by digest (`@sha256:...`), or an
  unresolvable `${BUILD_ARG}` reference is not a floating tag and isn't
  flagged.
- **`add-instead-of-copy`** — `ADD` used for a plain local file or
  directory. `ADD`'s two genuinely special behaviors — fetching a
  remote URL and auto-extracting a local archive — are recognized and
  left alone; only ADD used as a same-as-COPY-but-with-surprises is
  flagged.
- **`runs-as-root`** — no `USER` instruction in the final stage (only
  the final stage is checked; a builder stage that never drops root is
  normal and not part of what ships), or an explicit `USER root` /
  `USER 0`.
- **`no-healthcheck`** — no `HEALTHCHECK` in the final stage.
  `HEALTHCHECK NONE` is treated as a deliberate, informed opt-out and
  not flagged.
- **`apt-missing-no-install-recommends`** / **`apt-missing-list-cleanup`**
  — an `apt-get install` (or `apt install`) line missing
  `--no-install-recommends`, or not followed in the *same* `RUN` by `rm
  -rf /var/lib/apt/lists/*`. Both are checked independently and can
  both fire on one line — a `RUN apt-get update && apt-get install -y
  curl` with neither flag produces two separate findings.

Multi-line `RUN ... \` continuations are joined into one logical
instruction before any rule runs, so a well-formed three-line
`apt-get update && apt-get install ... && rm -rf ...` block is
correctly recognized as compliant rather than tripping the cleanup rule
because the `rm -rf` is on a different physical line.

## Status: built, 26 unit tests passing, verified against both hand-built fixtures and real Dockerfiles in this monorepo

- **26 unit tests** (`cargo test --lib`): the line-continuation parser
  (joining, comment/blank-line skipping, case-insensitive keywords),
  every rule individually against small hand-built instruction lists
  (each true-positive shape and its corresponding true-negative — a
  pinned tag, a digest-pinned image, a stage-name reference, a
  URL/archive `ADD`, a non-root `USER`, `HEALTHCHECK NONE`, a
  well-formed apt block), and the "only the final stage counts for
  USER/HEALTHCHECK" rule against a real two-stage instruction sequence.
  Two fixture-level tests run the full pipeline against
  `fixtures/bad.Dockerfile` (constructed to hit all five rule
  categories in one file — 6 findings total, since the apt rule fires
  twice) and `fixtures/good.Dockerfile` (a realistic two-stage build
  meant to pass every rule cleanly) and assert the exact rule sets each
  produces, including that the good fixture produces **zero** findings.
- **`cargo clippy --all-targets -- -D warnings`**: clean.
- **Live-verified against real Dockerfiles already in this monorepo**,
  not just its own fixtures: ran against
  `apps/scoreboard/bff/Dockerfile` and
  `apps/job-search-app/bff/Dockerfile`, both real two-stage Rust/Debian
  builds already in production use. Both correctly produced exactly one
  finding each (`runs-as-root` — neither Dockerfile drops root) and
  correctly did **not** flag their `RUN apt-get update && apt-get
  install -y --no-install-recommends ... && rm -rf
  /var/lib/apt/lists/*` blocks (a real 2-line `\`-continued command,
  confirming the continuation-joining logic works outside its own
  fixtures), their pinned `rust:1-bookworm` / `debian:bookworm-slim`
  base images, or their real multi-line `HEALTHCHECK` instructions.
  Read both Dockerfiles by hand afterward to confirm the one finding
  each was a genuine true positive, not a parser artifact.

**Not done / deliberately deferred**: the `# escape=\`` directive
(switches the line-continuation character away from the default
backslash) isn't honored — a Dockerfile using it would have its
continuations misparsed; `ONBUILD`-wrapped instructions aren't unwrapped
and inspected as if they were the wrapped instruction; shell-form vs.
exec-form `CMD`/`ENTRYPOINT` isn't linted at all (hadolint has rules for
this — out of scope for v1); and there's no `--ignore <rule>` flag to
selectively suppress a rule per line or per file, so a deliberate,
documented exception still shows up as a finding.
