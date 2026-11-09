# Local command system

Updated: 2026-11-09. Status: implemented for TUI and console. Persistent session operations are reused; no new model/tool loop is introduced.

## Boundary

A local command is user input addressed to the host application. It is not a model message, an LLM function call, or a new session-management implementation.

```text
Composer / console input
  -> classify and parse
     -> ordinary message -> SessionRequest{request_id, Send} -> Runtime
     -> local command -> application handler
                        -> SessionRequest{request_id, session operation}
     <- SessionEventPacket{event, association, request_id}
        Accepted (send) / Opened / Listed / Renamed / TitleUpdated / Error
        Agent (full session/task/message) / Finished (unique terminal)
```

Parsing must happen before the frontend removes the draft, appends a visible user message, or marks the Agent as running. A submitted message is not removed or displayed as running until the Runtime `Accepted` reply carries its full association. Successful local commands do not enter Conversation/history.jsonl as user messages. Session operations can still create their normal metadata/history changes.

The parser accepts text and returns a typed intent or an argument error. It does not import Ratatui, access the filesystem, call a model, execute shell commands, or hold an Agent. Session lifecycle and persistence remain in the [Saki service](https://github.com/HeimaoLST/Saki/blob/dev/docs/design/runtime-api.md); the terminal manager only adapts request IDs and packets.

## Initial vocabulary

| Command | Contract |
| --- | --- |
| `/new` | Create and open a session using the host defaults; no arguments |
| `/sessions` | TUI opens the existing picker; console displays the session list |
| `/resume <id>` | Open the exact ID through the existing session operation |
| `/rename <title>` | Rename the current session; the rest of the line is the title |
| `/help` | Display command and interaction help through the current frontend |

`/resume <id>` gives console users a switching path without depending on a graphical picker. The session picker and slash commands must dispatch the same core requests. `/help` has no model or session mutation requirement.

Do not add `/compact`, `/model`, `/cancel`, `/delete`, or plugin commands in this first vocabulary. They need their own core operations and behavior contracts. Existing automatic compact continues independently.

## Input rules

- Classify a command only for a single-line input whose first non-whitespace character is `/`.
- Match command names exactly, without case folding or guessed aliases.
- For `/rename`, use the remaining trimmed text as the title; no shell expansion or quoting language is needed.
- `/resume` requires one exact ID. Missing/extra arguments produce local usage feedback.
- An unknown simple command such as `/resuem` produces a local error and keeps the draft. It is not silently sent to the model.
- A token that is clearly an absolute path, such as `/Users/me/src/main.rs`, remains ordinary input. To explicitly send command-looking text, use `//`; remove one leading slash from the outgoing message.
- Multiline pasted content remains a model message, even when its first line starts with `/`. Pasting text never executes a command before the user presses Send.
- A recognized command with image attachments reports a local error and retains the attachments. Commands must not silently discard images or redirect them into a new session.

These rules are covered by parser and frontend behavior tests. A root filename containing a dot, such as `/README.md`, also stays a message. A bare single-component path such as `/tmp` is ambiguous and uses the unknown-command path; `//tmp` explicitly sends it as text. Multiline submission applies to the TUI/API parser: the plain console remains line-oriented and handles each input line separately.

## Dispatch and feedback

Keep a small static definition list for command name, description and usage. Help and completion read this list so their supported vocabulary stays consistent. A straightforward typed enum and match-based dispatcher are sufficient; no dynamic plugin registry is needed.

Session actions use host-assigned monotonic `request_id` values on the bounded `SessionRequest` channel. Replies are matched by `request_id`; Agent/Finished events are matched by the packet’s full `(session_id, task_id, message_id)` association. Frontend actions such as help are applied by the host. Agent, Conversation, client, protocol and tools do not understand slash syntax.

While another session is running:

- `/help`, `/sessions`, `/rename`, `/new`, and `/resume` remain locally usable; viewing/creating/switching B does not cancel A.
- An ordinary message to a session with an active task (or full Runtime capacity) is rejected with the Runtime’s verified `busy` code/reason; the exact draft text/images/cursor stays editable and is not auto-retried. Explicit retry is another Send.
- Command/Agent feedback never fabricates a second terminal; only `Finished` finalizes the task. Cancel acknowledgment (`requested`/`already_requested`/`not_running`) is not a terminal.

The TUI command draft is consumed only after its matching `request_id` successful response and only if the draft still equals the dispatched snapshot. A parse error, busy response, missing session, failed rename, or a draft changed/switched after dispatch preserves the current editor. The plain console echoes rejected input with `Input kept:` so it can be retried; it does not auto-requeue. Opening a session through `/new` or `/resume` consumes the command text before caching the old session state; otherwise returning could restore an executed command as an unsent draft.

## TUI

A small completion list appears above the composer while typing a command name. Tab or a mouse click completes the selected name, Up/Down navigate the list, and Escape closes it without erasing input. Enter executes the exact typed command; selecting a suggestion does not execute it. Command completion must take priority over input-history navigation only while this list is active. Ordinary text retains the current editing and history behavior.

Descriptions and usage are shown on demand. No permanent keyboard legend or command catalog is added to the status line. Ctrl+S and clicking the session title remain direct routes to the picker.

## Implementation sequence

1. Implement the pure input classifier/parser and command definitions, with boundary tests.
2. Integrate dispatch before submission in console and TUI, reusing session requests and feedback paths.
3. Add completion/help presentation and verify draft preservation, Busy behavior, literal text, and zero model requests for local commands.

## Implementation and validation

- [Command module](../../src/command/mod.rs): pure text classification, argument validation, one static vocabulary, and translation to typed session requests.
- [Session requests](../../src/frontend/session_requests.rs): host request IDs, bounded `try_send`, pending-send/command/cancel ownership.
- [TUI commands](../../src/frontend/tui/commands.rs): completion presentation and command dispatch; no eager draft consumption.
- [TUI submission](../../src/frontend/tui/mod.rs): classification before `Accepted`, visible user records, or Agent execution; per-session projections route associated packets.
- [Console](../../src/frontend/console.rs): bounded request/event consumption, current-task Ctrl-C cancel, associated HistoryRestored append-only repair, and piped EOF draining.
- [Saki Runtime service](https://github.com/HeimaoLST/Saki/blob/dev/docs/design/runtime-api.md): authoritative sessions, task IDs/message IDs, unique Finished terminal, cancellation and title jobs. `TitleUpdated` remains non-acknowledging; `Renamed` is the explicit rename response.

Help and completion read the same definitions. Product feedback remains English. The command layer adds no API calls, database, plugin system, permanent footer legend, or persistence format.

Historical validation reports below belong to their named Saki commits. They are not evidence for the independent Rous build. The independent product uses the external-runtime fixture and commands in [README](../../README.md).

Validation status: the `7f87e44` checkpoint passed 596 main + 70 support tests with 0 failed/11 ignored; its earlier parallel Runtime title failure is preserved as issue evidence. Runtime `8f7831a` then added the explicit MetadataLock unlock correction with controlled duplicated-descriptor RED/GREEN coverage, not a consumer-side title change. The joint fixed candidate `527b402e69fcdf76e5a4f84eb906a233788f0f31` has a Runtime author report of **606 main + 70 support tests, 0 failed, 12 ignored** and exact CI `39832826642` green. Raw failures and the withdrawn native-candidate attempt remain issue evidence; they are not erased or rewritten. This is joint-candidate validation, not completed `dev` integration.

New behavior tests live in [parser tests](../../tests/command/mod.rs), [TUI command tests](../../tests/tui/commands.rs), [TUI session projections](../../tests/tui/sessions.rs), and [console tests](../../tests/console/mod.rs). They cover literal paths, multiline/slash escapes, attachments/cursor retention, bounded request rejection, full task/message association guards, A-running/B-view isolation, unique Finished, associated HistoryRestored canonical repair, targeted cancel acknowledgment (not terminal), automatic-title races, outgoing-session draft caching, keyboard/mouse completion, dispatch during jobs, and EOF draining. Handoff and evidence index: [#109](https://github.com/HeimaoLST/Saki/issues/109).

Actual joint-candidate PTY validation on the fixed `527b402` build: 28/28 checks across five scoped scenarios (new/explicit resume/rename/title, A-running/B-view, TUI cancel, console active-cancel→one interrupted→help→idle SIGINT, editor), plus a separate idle-SIGINT-with-open-stdin exit. The historical `7f87e44` checkpoint is distinguished from this current candidate validation. Raw failed artifacts remain in the issue handoff; they are not deleted.

2026-10-23 Runtime consumer facts: terminal requests are bounded `SessionRequest{request_id, command}` with host IDs and `try_send`; replies are matched by request_id and task events by full `(session_id,task_id,message_id)`. Send is committed/displayed only on `Accepted`; rejection retains the exact editor draft and creates no User/Activity/running record. Console Ctrl-C cancels only the current observed task with its exact session/task and its acknowledgment is not terminal. Console `HistoryRestored` appends the authoritative canonical body for the associated task without replaying that task’s terminal; the one ensuing `Finished` reports status/error/persistence/history_required. Rous consumes the public stdio protocol. Its raw event queue has an 8 MiB byte budget; UI request and event channels remain bounded.

Console shutdown semantics (#109): production keeps the single unchanged Tokio stdin source; the host `main`/`app` owns post-app drain and invokes `shutdown_background` only after the UI manager and its owned Runtime child and forwarders drain. An idle SIGINT with stdin still open therefore exits without requiring EOF/Enter. The console run loop and task association logic are unchanged. See [main.rs](../../src/main.rs), [app.rs](../../src/app.rs), [session terminal](../../src/session/terminal.rs), and [runtime API](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/design/runtime-api.md); no second stdin reader/thread or input framework is introduced. Ctrl+C acknowledgement is never treated as a terminal.

Rous display failure semantics, 2026-11-09: enqueue timeout, oversize, or an undeliverable critical reply closes the local display even if its receiver still exists. `is_closed` and `closed` expose that state. Later sends reject; the receiver discards the incomplete queued projection. The manager stops admission and drains its existing Runtime work without cancelling accepted tasks. A controlled queue-full process fixture verifies the task completes and remains restorable through the public snapshot.

Remaining scope: argument/session-ID completion, aliases, persistent command history, plugins, model switching, manual compact, and background/offscreen cancellation UI.
