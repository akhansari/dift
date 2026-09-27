# Dift

## Rust Standards

- Use feature-based structure.
- Prefer expression-oriented and functional code.
- Write idiomatic modern Rust with clear ownership and minimal to zero cloning.
- Prefer borrowing over cloning, but optimize for clarity before micro-optimizing allocations.
- Avoid `unwrap`, `expect`, and `panic!`.
- Use your judgment, if code or design quality degrades or refactoring feels warranted, pause and ask before proceeding.

## Coding Patterns

- Builder Pattern: For constructing complex objects step by step
- RAII Pattern: For resource management tied to object lifetimes
- Newtype Pattern: For type safety and abstraction
- Decorator Pattern: For adding behavior to objects dynamically
- Command Pattern: For encapsulating operations as objects

## Testing

- Read the Unit Testing skill before modifying or adding unit tests.
- Smoke and perf tests are opt-in (excluded from `cargo test`) and require Unix:
  - Smoke: after changes to terminal UI, input, Git workflows, or file watching, run `cargo test --locked --test smoke`.
  - Perf: when changes may affect startup, collection, watching, or rendering performance, run `cargo test --release --locked --test perf -- --repo /path/to/repository --runs 5` before and after on the same representative repository; compare medians. See `docs/startup.md` for metrics.

## Ask Before

- Adding runtime dependencies.
- Introducing persisted configuration or changing its format.

## Development Commands

Before finishing code changes, run:

```bash
cargo fmt
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
```
