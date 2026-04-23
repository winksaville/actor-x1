# actor-x1

An experiment in the actor model in Rust (Communicating
Sequential Processes, Hoare 1978). Part of the
[actor-x1 workspace](../..); see the workspace
[README](../../README.md) for overview, usage, and
contribution conventions.

## Binaries

- **`goal1`** — two actors on one thread, ping-ponging an
  empty message through a shared `VecDeque`.
- **`goal2`** — two actors on two threads, ping-ponging
  over `std::sync::mpsc` channels.

Install from the workspace root:

```
cargo install --path crates/actor-x1
```

## Notes

- [`notes/design.md`](notes/design.md) — staged actor-model
  design. Stage 1 shipped at `0.1.0`; Stage 2 is next.

## License

Dual-licensed under [MIT](LICENSE-MIT) OR
[Apache-2.0](LICENSE-APACHE).
