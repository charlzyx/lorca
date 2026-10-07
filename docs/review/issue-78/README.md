# PR #91 — native UI evidence

These PNGs render the implemented AppKit controllers with synthetic account, Runner, routine, and check-history data. An opt-in XCTest fixture applies the data through the same snapshot mapper the local CLI uses. The native views render in an opaque offscreen fixture window, without the main-window chrome, and `NSView.cacheDisplay` captures their pixels at Retina resolution. Overview and offline-state images capture the sheet's details-and-actions region; the other images capture the complete fixture content view. Each scene uses a separate test process.

The before image uses the actual `RoutineViewController.swift` from baseline commit `02a11cd8d596dfa60d5be28a3156ad7f2c8a6bb4`, generated temporarily into the test target. The current images use the #78 implementation introduced in `22f5ce1575d097bcda5c9fc0fc3162fab1940bd4`. The before and quiet images receive the same synthetic routine instant: the prior sheet formats it in the viewer's local time; the current sheet identifies the routine's IANA timezone and UTC offset and exposes missed-run policy and independent check health.

| Image | Captured view |
| --- | --- |
| [routine-before.png](routine-before.png) | Baseline routine sheet, same synthetic routine definition and UTC occurrence. |
| [routine-quiet.png](routine-quiet.png) | Current sheet with New York timezone, Run once policy, quiet check success history, and Last run: Never. |
| [routine-failed-overview.png](routine-failed-overview.png) | Failed state and retry history in the current sheet. |
| [routine-failed.png](routine-failed.png) | The same failed fixture scrolled to the retry time and connection recovery action. |
| [routine-blocked-overview.png](routine-blocked-overview.png) | Authentication-blocked state, no next check, disabled Run Now, and Resume. |
| [routine-blocked.png](routine-blocked.png) | The same blocked fixture scrolled to reconnect-then-resume instructions. |
| [routine-waiting_for_runner.png](routine-waiting_for_runner.png) | Offline assigned Runner, Skip policy, disabled Run Now, and recovery instructions. |
| [runner-service.png](runner-service.png) | Devices pane with a synthetic Runner and the mocked Not installed response after Check status, plus install/status commands. |

Reproduce from this worktree:

```sh
bash macos/Tests/Captures/capture-routine-evidence.sh
```

The harness requires `LORCA_MOCK=1`; the script sets it. Its fixed clock data is October 8, 2026 at 13:00 UTC, with a next occurrence of October 9 at 09:00 in America/New_York. The sample Runner key, relay URL, bot, and inbox text are synthetic. The capture does not start a CLI or check, read account files or private chats, connect a provider or relay, or install a service. The assets and harness live outside production bundle resources.

These images verify presentation with fixtures. The PR's existing limitations for real paired-Device outages, reauthentication, OS service lifecycle, and sibling consolidation remain in force.
