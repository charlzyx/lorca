# Bot permissions UI evidence

These PNGs show the production AppKit views from the #75 branch in a native XCTest fixture. The fixture creates an invented **Inbox Assistant** bot and supplies invented **Gmail · Work** and **Gmail · Personal** tool catalogs through a loopback-only WebSocket stub. It uses `LORCA_MOCK=1`: profile saves stay in the in-memory mock store, with no real credentials, inbox contents, private chats, or provider calls.

The harness opens the actual Profile → Access sheet and operates native checkboxes, the filesystem pop-up, and Save. It verifies that the selected policy reaches the mock profile and that the refusal card's Edit Access action is available without Allow once/Always allow. The account and local-control images are macOS window captures scoped to the harness's own sheet window ID. The profile cards and refusal card use AppKit's native view bitmap capture. Images have no raster edits, annotations, or generated artwork.

| Image | State shown |
| --- | --- |
| [profile-before-policy.png](profile-before-policy.png) | Current Profile card before applying fixture restrictions: all connections and local tools, shell allowed. This is a policy-state comparison, not a screenshot of an earlier code version. |
| [profile-after-policy.png](profile-after-policy.png) | Same card after native Save: one connection with capabilities, four local tools, shell denied. |
| [access-connections.png](access-connections.png) | Actual Access sheet with Personal capabilities off, Work read/draft on, write off, and send_message excluded from Work's tool selection. |
| [access-local-controls.png](access-local-controls.png) | Same sheet scrolled to local tools, explicit stage_review selection, filesystem Read, shell off, and the Runner/isolation explanation. |
| [refused-access-request.png](refused-access-request.png) | Actual PermissionCellView rendering a fixture refusal payload with Edit Access and Dismiss. |

Reproduce on macOS from the worktree root:

```sh
bun macos/Tests/Fixtures/capture-bot-access.ts
```

The script derives the local tool names from the Rust policy module, hosts only fixture tool metadata on a random loopback port, runs `BotAccessEvidenceTests`, then stops that fixture. The opt-in test is skipped during ordinary test runs. The capture test validates that every PNG contains rendered content and asserts the selected policy and refusal controls. It passes with nonfatal geometry warnings from the hidden command-block view; all visible captures are inspected separately.

This is evidence of native rendering and an automated fixture control/save path. It does **not** establish production-account OAuth, actual CLI/paired-Device save or authorization behavior, manual human interaction, or combined #71/#73/#85 execution. Those PR draft/QA/consolidation limits remain. The evidence assets and fixture code live under `macos/Tests/`, outside the production target and bundles.
