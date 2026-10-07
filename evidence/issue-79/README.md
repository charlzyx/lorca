# PR #101 UI evidence

These PNGs show the production AppKit `BudgetViewController` and
`ConnectorLimitsViewController` from the #79 branch. The opt-in XCTest harness
creates native fixture windows in Aqua appearance, waits for the controllers to
read synthetic localhost websocket responses, and captures each content view
with `NSView.cacheDisplay` / `NSBitmapImageRep` at the window's 2× backing scale.
The fixture windows are not ordered onto the user's desktop.

| Capture | State shown |
| --- | --- |
| [Task allowance](task-allowance.png) | Spending, tokens, runtime, retry and connector-call limits; separate API spending, subscription API-equivalent estimates and unknown-price call counts |
| [Routine exhausted](routine-budget-exhausted.png) | Orange exhaustion state, exhaustion reason, consumed allowance and explicit increase/resume or renew actions |
| [Task interrupted](task-interrupted.png) | Restart/interruption state and the instruction to check completed effects before explicit recovery |
| [Account cooldown](connector-account-cooldown.png) | Account call rate/window/concurrency controls and a synthetic service cooldown |
| [Shared service](connector-service.png) | The same controller switched to its service scope, with shared service limits and active-call count |

All data is synthetic: “Demo Assistant”, “Screenshot Runner”, public fixture ids,
usage values and cooldown timestamps. The server accepts only fixture info and
read-only budget/connector queries. The harness connects the app's CLI client to
that endpoint without starting the app store or the real CLI. Captures contain
no credentials, private chats, provider requests or relay/service connections.

The Task allowance image uses the implemented `taskID:` constructor hook. Its
Task-card navigation still depends on #72 consolidation, as the PR states.
These captures demonstrate native rendering and the connector scope selection;
they do not exercise Runner budget mutations, model resumption, external OAuth,
paired-device transport, or the unresolved combined task/routine/event paths.

Regenerate on macOS with Bun, Xcode and Swift installed:

```sh
bash evidence/issue-79/capture.sh
```

The script starts a temporary read-only fixture server on an ephemeral localhost
port, builds the Markdown FFI if missing, runs the single opt-in capture test,
and stops the fixture server. The harness skips during ordinary test runs unless
`LORCA_CAPTURE_DIR` is set. Assets and capture scripts stay in this evidence
directory; test code stays under `macos/Tests`, outside production bundles.
