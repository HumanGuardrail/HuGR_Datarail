# Contributing

This project is under active, fast-moving development — the roadmap lives
with the maintainer and big features land in coordinated waves. The most
useful external contributions right now:

- **Bug reports with reproductions** — especially platform-specific ones.
- **Benchmark scrutiny** — if you find a hole in a published number or its
  method, open an issue; the harness exists to be attacked.
- **Small, focused PRs** — docs fixes, portability fixes, test coverage.

## Ground rules

- Before a big PR, open an issue first — coordinated waves mean parallel
  work can collide.
- CI must be green: `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test --workspace`.
- Performance claims in docs must cite a measured, reproducible run — this
  repo does not merge unmeasured numbers (see the benchmark docs).
- Commit messages: conventional style (`fix:`, `docs:`, `feat:`).
