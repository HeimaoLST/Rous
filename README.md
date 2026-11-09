# Rous

Rous is a terminal client for [Saki](https://github.com/HeimaoLST/Saki). It provides a TUI and a line-oriented console. Saki runs models, tools, context compaction, titles, and session persistence in a separate process.

## Run Rous

Build Rous with Rust and a compatible prebuilt Saki runtime:

```sh
cargo build --locked --bin rous
SAKI_RUNTIME_BIN=/absolute/path/to/Saki ./target/debug/rous
```

Without `SAKI_RUNTIME_BIN`, Rous looks for `Saki` on `PATH`. Rous does not build Saki or search adjacent checkouts. It starts one exclusively owned stdio child and drains that child on shutdown.

Interactive terminals use the TUI. Pipes use the console. An unknown `SAKI_UI` value warns and falls back to the console. Existing configuration names remain supported:

```sh
SAKI_UI=console SAKI_RUNTIME_BIN=/absolute/path/to/Saki ./target/debug/rous
SAKI_SESSION=<session-id> SAKI_RUNTIME_BIN=/absolute/path/to/Saki ./target/debug/rous
```

Each launch creates a fresh session unless `SAKI_SESSION` names an existing session. `SAKI_SESSION=new` explicitly creates a fresh session. Missing or incompatible IDs return errors. `SAKI_THEME=auto|light|dark` controls the TUI theme. Press F1 for keyboard help or use `/help` in either frontend.

Runtime configuration, provider credentials, the working directory, and existing `SAKI_SESSION_DIR` settings pass to Saki. Session IDs and data paths remain unchanged. Rous neither reads journal files nor migrates stored data. Rous enables Saki's existing automatic title jobs for its child with `SAKI_AUTOMATIC_TITLES=true`.

## Verify a checkout

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
set -o pipefail
SAKI_TEST_RUNTIME_BIN=/absolute/path/to/Saki cargo test --locked --bin rous client::tests::real_runtime_contract -- --ignored --exact --nocapture --test-threads=1 2>&1 | tee /private/tmp/rous-contract.log
python3 scripts/check-rous-runtime-results.py /private/tmp/rous-contract.log
cargo test --locked --bin rous frontend::tui::tests::pty::pty_panic_restores_terminal -- --ignored --exact --nocapture --test-threads=1
cargo test --locked --bin rous frontend::tui::tests::pty::pty_reader_cancel_and_suspend -- --ignored --exact --nocapture --test-threads=1
```

Capture the real contract command's combined output in the file supplied to the checker. The contract fixture uses Rous's actual stdio client, an external Saki process, temporary session data, and a controlled local HTTP provider. It makes no external model requests. PTY fixtures exercise terminal cleanup and reader cancellation without opening a native terminal window.

CI selects Saki through the artifact pin in [.github/saki-runtime.json](.github/saki-runtime.json). The pin identifies a real Saki CI artifact by source commit, run, archive hash, and binary hash. Cross-repository artifact credentials require repository-owner setup. A green local test does not establish native clipboard behavior, human acceptance, or completion of the historical MVP.

## Product and protocol ownership

Rous's version comes from [Cargo.toml](Cargo.toml). Saki has its own build, version, and release. Compatibility uses version 1 of Saki's [runtime protocol](https://github.com/HeimaoLST/Saki/blob/dev/docs/design/runtime-api.md), including `session.snapshot` and `runtime.metadata.subscribe`.

Rous owns input, drafts, commands, terminal lifecycle, and display projections. Saki owns execution, canonical history, configuration, context estimates, cancellation, and title generation. Display projections do not implement an Agent loop. See the [documentation index](docs/README.md) for module boundaries and retained interaction behavior.

If a display packet times out, exceeds its byte limit, or a critical reply cannot enter the queue, Rous closes that display stream and stops accepting requests. Already accepted Runtime work drains normally. Rous does not silently resume an incomplete display or reconnect automatically.

The [source import manifest](docs/design/source-import.json) records the exact original candidate export. Its original commits remain in this repository's history. Historical Saki test and acceptance reports describe those historical builds, not this independent product.
