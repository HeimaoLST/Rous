# Rous contributor instructions

Updated: 2026-11-09.

Rous is an independent terminal product. Read [README.md](README.md) and [docs/design/tui.md](docs/design/tui.md) before changing startup, session interaction, or display ownership.

- Keep model calls, tool execution, persistence, replay, context estimation, and title generation in Saki. Consume its public version 1 stdio contract. Do not copy execution modules or depend on a neighboring Saki checkout.
- `src/client/` owns public wire DTOs, response routing, and the child process. `src/session/` adapts that protocol to UI packets. `src/input.rs`, `src/frontend/`, and `src/command/` own terminal behavior. Tests belong with their modules under `tests/<module>/`.
- Preserve existing `SAKI_UI`, `SAKI_SESSION`, `SAKI_THEME`, data paths, draft behavior, and exact task associations unless an approved task changes the contract.
- Check current ownership before writing. QA owns `.github/workflows/`, the Saki artifact pin, and artifact fetch/checker scripts. Product owners maintain production code, behavior tests, and product documentation.
- Use the task's authorized coordination channel. When using HC, discover the current terminal and recipient dynamically. Keep stable instance identities; do not put terminal addresses in permanent routing rules.
- Follow the task's Git authorization. Normal development uses a task branch and a reviewable PR; never treat passing CI as permission to expand scope, publish, or rewrite history.
- Test observable behavior with controlled fixtures. Run formatting, clippy, applicable tests, and the external-runtime contract before delivery. Run long commands in HC background terminals when HC is the active coordination channel. Preserve failed evidence and distinguish local providers, PTY checks, native interaction, and human acceptance.
- Keep one implementation of each responsibility. Read existing helpers before adding abstractions. Update the existing module document when behavior or configuration changes.
