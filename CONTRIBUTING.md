# Contributing

Issues and pull requests are welcome. A few ground rules keep the project
coherent:

- **Point-in-time correctness is non-negotiable.** Any change touching the
  schema or views must preserve the invariant that a query as-of date D can
  only see facts knowable at D. PRs that trade this away for convenience will
  be declined regardless of other merits.
- **`cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` must pass** —
  CI enforces all three.
- **No new data source without its redistribution terms checked.** Free to
  *access* is not free to *redistribute*; note the terms in the PR.
- **Match the existing style**: comments explain constraints the code can't
  show, not what the next line does.

For anything substantial, open an issue first so the design conversation
happens before the code.
