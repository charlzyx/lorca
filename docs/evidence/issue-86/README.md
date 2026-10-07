# Workflow feedback UI evidence

These PNGs capture the actual AppKit sheet controllers on this branch, using synthetic
request responses in `WorkflowFeedbackCaptureTests`. The sheet layout, controls, typography
and rendering come from production `WorkflowFeedbackViewController`,
`RecordWorkflowFeedbackViewController`, shared `SheetViewController`, `Build` and `Theme`.
The request closure is injected at the sheet boundary; normal callers still use the local CLI.

The harness creates borderless offscreen NSWindows, renders their content at 2× through
`NSView.cacheDisplay(in:to:)`, and saves lossless PNGs. It never connects the CLI, loads an
account, calls a model/relay or captures the user's desktop. All names, messages, source ids,
diffs, hashes, decisions and settings shown are test fixtures. PNGs have no image retouching
or added annotations. The review captures use the sheet's real scroll view to show the
relevant section. These are native sheet-content captures, rather than full-app window captures.

| Image | State and caption |
| --- | --- |
| [01-record-user-edit.png](01-record-user-edit.png) | Record workflow feedback: User edited, selected Morning brief target, a synthetic explanation and corrected draft, plus the sensitive-material exclusion checkbox. |
| [02-review-proposed-diff.png](02-review-proposed-diff.png) | Before acceptance: pending routine revision, source-work links, supporting explanation, exact proposed diff and Accept/Reject/Exclude controls. The sheet is scrolled to the proposal. |
| [03-accepted-rollback-preview.png](03-accepted-rollback-preview.png) | After a synthetic acceptance response: numbered revision history, reversal diff and rollback control. The test clicks the real Accept button and checks its submitted diff hash; this image does not establish backend application or persistence. |
| [04-exclusions-and-neutral-alert.png](04-exclusions-and-neutral-alert.png) | After a synthetic exclusion response: disabled “This chat is excluded,” an excluded edit with its text absent, no pending improvement, Weekly review, and an ignored alert from another included fixture chat that remains neutral. |

The fixture test also clicks Record and checks that the payload carries `edited` with
different before/after text. It verifies the exclusion control is disabled after the response.
Source navigation, rollback execution, real provider inference, relay/paired-Device operations
and the sibling review/playbook integrations still require the QA listed in draft PR #97.

From the repository root on macOS, with the project's Markdown framework built:

```sh
LORCA_UI_EVIDENCE_DIR="$PWD/docs/evidence/issue-86" \
  swift test --package-path macos --filter WorkflowFeedbackCaptureTests
```

A checkout without the generated Markdown framework can prepare it with the existing
`buildMarkdown` helper in `scripts/app.ts`. The evidence test is skipped without
`LORCA_UI_EVIDENCE_DIR`. The capture harness lives under `macos/Tests/Notifications/`,
and these assets live under `docs/evidence/`, outside production app bundles.
