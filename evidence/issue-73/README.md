# Review queue UI captures

These PNGs are native AppKit fixture captures of the production `ReviewViewController` and the actual `InspectorViewController` Review queue section. `ReviewScreenshotTests` renders them in offscreen windows using the repository's fictional mock bot/Runner and synthetic review items, with light appearance at 2× scale. The PNGs are unretouched view captures, including an explicitly cropped inspector section. The fixtures contain only sandbox labels, an `example.test` recipient, and a fictional guarded path.

| Capture | State shown |
| --- | --- |
| [00-inspector-queue.png](00-inspector-queue.png) | The new inspector section's empty state, rendered through the normal mock-store inspector path. |
| [01-editable-draft.png](01-editable-draft.png) | Saved version 1: account, resource, rationale, editable draft, and review controls. |
| [02-unsaved-edit-guard.png](02-unsaved-edit-guard.png) | The same sheet after a local unsaved edit and programmatic activation of its real Approve button: the local guard asks the user to save and review the new version. |
| [03-proposed-call.png](03-proposed-call.png) | A fixture-supplied version 2 plugin proposal with exact server/tool, JSON arguments, and guarded file. |
| [04-uncertain-outcome.png](04-uncertain-outcome.png) | A fixture-supplied uncertain outcome: inspection guidance, disabled mutation controls, and available Reload. |

The native capture test checks the unsaved-edit warning and disabled controls. It does not start the CLI, connect an account, send a message, or perform the depicted restart. It demonstrates rendered UI and a local approval guard; paired-Device/OAuth, combined permissions, backend save/approval/restart flows, and interactive AppKit QA retain the PR's stated validation limits.

Regenerate from the assigned worktree on macOS:

```sh
LORCA_MOCK=1 LORCA_REVIEW_SCREENSHOT_DIR="$PWD/evidence/issue-73" \
  swift test --package-path macos --filter ReviewScreenshotTests
```

The test skips unless explicitly enabled and requires mock mode. Its source lives in the test target; these evidence assets are outside the app's resources and production bundles.
