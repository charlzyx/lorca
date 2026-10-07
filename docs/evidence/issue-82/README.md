# Template sharing — native UI evidence

These PNGs show the production `TemplateExportViewController` and `TemplateImportViewController`, rendered by AppKit in offscreen native test windows with a light appearance at 640 points wide (2× PNGs). `TemplateSharingCaptureTests` supplies deterministic synthetic CLI replies through the controllers' optional reply dependency; normal app construction uses the local CLI.

The fixture uses a fictional Release Reviewer bot, the existing mock Runner/provider list, a reserved `review@example.test` address, a simulated ready GitHub connection, and a paused-routine preview. It reads no account files or real credentials and shows no real conversations. The connection-choice and missing-capability replies are simulated. Save/Open panels, a completed import, live providers/OAuth, relay transport, and sibling consolidation are outside this capture; the PR's existing limitations remain applicable.

| Capture | State |
| --- | --- |
| [Export selection](export-selection.png) | Every category starts unchecked; Preview and personal-content review are disabled until selection. The standalone playbook capability note is visible. |
| [Export review](export-review.png) | Selected profile, memory, routine, and requirement; scrollable contents and personal-information warning; explicit review enables Save Private File. |
| [Import needs connection](import-needs-connection.png) | Recipient Runner/provider and an explicit connection choice; Create Independent Bot stays disabled. |
| [Reviewed import](import-reviewed.png) | Simulated recipient connection chosen and review acknowledged; paused-routine wording remains visible and Create Independent Bot becomes enabled. No import is performed by the fixture. |
| [Missing playbook capability](import-missing-capability.png) | A file containing a reusable skill reports the standalone CLI capability blocker; creation remains disabled. |

Reproduce from this worktree:

```sh
LORCA_MOCK=1 LORCA_UI_CAPTURE_DIR="$PWD/target/template-ui-captures" \
  swift test --package-path macos --filter TemplateSharing
```

The three native fixture tests assert selection payloads, review/connection gating, a visible confirmation button, and capability blocking. The existing three JSON-preview tests also run. Capture uses `NSView.cacheDisplay` on the native window frame; images are visually inspected before publication. Assets live here outside the app's production resources and bundles.
