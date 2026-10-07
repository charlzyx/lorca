# AppKit evidence for PR #94

These PNGs capture the implemented AppKit controllers on branch
`codex/issue-81-workflow-onboarding`. `WorkflowEvidenceTests` injects presentation
fixtures through DEBUG-only constructors and renders the normal view hierarchy
with `NSView.cacheDisplay`, using offscreen, inactive, borderless windows in Aqua
appearance. The images show content views at 2× scale: onboarding is 660×560 pt;
marketplace/workflow pages are 800×700 pt. They do not include OS window chrome.

The catalog comes from this branch's bundled marketplace index. The account,
Runner, repository, sign-in failures and sample replies are synthetic. The
harness starts `AppStore` in mock mode and supplies decoded progress directly;
it makes no CLI, provider, OAuth or relay requests. The connection recovery
image illustrates the implemented renderer's response to fixture account states,
including the named-account contract from #71. It does not demonstrate a
completed Workspace integration or actual authentication.

| Image | State shown |
| --- | --- |
| `01-onboarding-complete.png` | The first-run closing page offers Choose a Workflow beside Open Lorca. |
| `02-outcomes.png` | Outcome selection lists the three bundled packs. |
| `03-required-questions.png` | Repository scope and specialist reuse choice; only this pack's required question is asked. |
| `04-connection-recovery.png` | A selected fixture calendar account needs sign-in; Drive is unavailable; the sample stays disabled and progress remains resumable. |
| `05-before-sample-review.png` | A completed fixture sample is displayed for explicit review; schedule activation is absent. |
| `06-after-sample-review.png` | After fixture review, the routine is still paused; Enable Schedules and Finish with Schedules Paused are separate choices. |
| `07-cancelled-setup.png` | Cancelled setup retains resources and offers Resume Setup. |

The review comparison is before/after the review **state**, not a comparison
with an older application release. The capture test asserts the corresponding
review/enable/resume controls. All seven images are visually inspected for
readable labels, complete controls and test-only data before publication.

Regenerate from the repository root:

```sh
LORCA_MOCK=1 \
LORCA_UI_EVIDENCE_DIR="$PWD/macos/Tests/Evidence/issue-81" \
swift test --package-path macos --filter WorkflowEvidenceTests
```

The test is opt-in and skips when the output directory is unset. DEBUG capture
constructors compile out of release builds. PNGs and this README sit outside
`macos/Sources` and the production app's resources/bundles.

These captures provide renderer/layout evidence. The draft PR's live OAuth,
paired-Runner/relay, manual navigation, service lifecycle and sibling
consolidation checks remain pending. Static fixture inspection does not replace
those checks.
