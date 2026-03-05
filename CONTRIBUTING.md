# Contributing to LMAO

## Pull Requests

Before opening a PR:

1. **Compile**: `cargo build` must pass
2. **Tests**: `cargo test` must pass
3. **Clippy**: `cargo clippy -- -D warnings` must be clean
4. **Format**: `cargo fmt --check`

## Tests are mandatory

Every PR that adds or changes behaviour **must** include tests:

- **New function/method** → unit test: happy path + at least one error case
- **New backend/transport** → integration test: roundtrip (store→retrieve, send→receive, etc.)
- **Bug fix** → regression test that would have caught the bug

No exceptions. A PR without tests for new code is incomplete.

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p logos-messaging-a2a-storage

# Run with feature flags
cargo test --features libstorage -p logos-messaging-a2a-storage
```

## Docs are mandatory

Every PR that adds a public API or new feature **must** update docs:

- **Public structs/traits/fns** → doc comments (`///`)
- **New feature** → usage example in README or relevant crate doc
- **New config/flags** → document the option

A feature without documentation doesn't exist for the next person.

## Branch naming

`jimmy/<short-description>` — e.g. `jimmy/libstorage-backend`

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):
- `feat(crate): add X`
- `fix(crate): fix Y`
- `test(crate): add tests for Z`
- `docs(crate): document W`

## Architecture

See [README.md](README.md) for the crate layout and design principles.
