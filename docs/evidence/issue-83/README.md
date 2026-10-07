# Shared project context UI captures

These PNGs show the production `ProjectContextViewController` and `SheetViewController` rendered by a native AppKit fixture with synthetic project data. The capture compiles the production view controllers, wire model, and unchanged `Build` / `BackgroundView` helpers. Its test-only `AppStore` supplies fixed `projects.get` responses; it accesses no account files, credentials, private chats, relay, or provider.

The fixture uses English, Aqua appearance, UTC, a background capture window, and a 2× native Core Animation render. It selects entries through the production popup action and enables revision history through the production checkbox action. It asserts that Save is enabled for current entries and disabled for the historical entry. The host never activates the application. Capture code lives under `macos/Tests/ProjectContextCapture`, and PNGs live here, outside production bundle resources.

| Image | State |
| --- | --- |
| [Agreed decision](agreed-decision.png) | Current decision, owner provenance, verification time, context budget, and correction controls. |
| [Unavailable source](source-unavailable.png) | Simulated unavailable live source, previous snapshot retained, last retrieval time, and explicit timeout notice. |
| [Reference asset](reference-asset.png) | Synthetic reference-file metadata and enabled Open asset action. The fixture does not open or download a file. |
| [Revision history](revision-history.png) | Superseded decision selected with history enabled; Save and Remove are disabled. |

Reproduce on macOS from the repository root:

```sh
macos/Tests/ProjectContextCapture/capture.sh
```

Pass an output directory as the first argument to keep regenerated files elsewhere. The script builds its standalone fixture in a temporary directory and removes the binary afterward. Its data adapter only implements `projects.get`; save, refresh, asset transfer, and full application navigation remain outside this capture check. These images do not replace the PR's pending live workflow and sibling consolidation checks.
