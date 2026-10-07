# Issue 76 UI evidence

All names, chats, task/review references and text come from `fixture.json`. No identity, credentials, private chat data, model calls or relay connection is used. Images are evidence assets outside the production app resource directories.

The AppKit captures compile the production `AttentionViewController.swift`, `SheetViewController.swift` and their label/stack helpers unchanged into an isolated native host. `macos/Tests/UIEvidence/AttentionCapture.swift` supplies an in-memory AppStore/client fixture. Native AppKit draws the PNGs off screen at 2× scale. The harness also invokes the real controller's Mark resolved and summary-toggle action selectors and verifies the fixture projection changes. It does not exercise the Rust API behind those actions.

- `macos-attention-active.png`: current brief, urgent blocker, pending review, source references and next actions.
- `macos-attention-followups.png`: the same native scroll view scrolled to commitments and important changes.
- `macos-attention-resolved.png`: all four synthetic items resolved and coordinator summaries switched off; the previously published brief remains, as it does until a coordinator updates it.

The phone captures render the production `mobile/app/attention.tsx`, forms, localization and zustand store in an arm64 iPhone Simulator running iOS 27. The small Expo fixture has its own navigation shell and substitutes only the core API boundary with an in-memory fixture. The offline bundle runs in a copied SDK-57 Lorca development runtime; a fixture-only native hook calls its embedded-bundle loader. The runtime copy, bundles and node_modules are ignored and are not shipped. Simulator screenshots are unedited native framebuffer captures, not web approximations.

- `phone-attention-before.png`: original phone screen from commit `ac1d9b7`, before automatic native scroll insets; the top notification controls sit under the navigation bar.
- `phone-attention-active.png`: notification preferences and the coordinator brief, followed by active attention.
- `phone-attention-followups.png`: a synthetic projection containing commitments/changes with summaries off.
- `phone-attention-empty.png`: an empty synthetic projection with summaries off.

The phone fixture is UI rendering evidence. It does not validate the production Rust native module, pairing, provider turns, source adapters, multi-Runner sync or APNs/FCM. PR draft/consolidation limitations still apply.

## Reproduce

AppKit, from the assigned checkout:

```sh
python3 macos/Tests/UIEvidence/capture-appkit.py
```

Phone: use a dedicated arm64 iPhone Simulator with iOS 27 and an existing SDK-57 Lorca development `.app`. Symlink this fixture's `node_modules` to the checkout's installed `mobile/node_modules`; dependency manifests and lockfiles in the production project stay unchanged. From the repository root:

```sh
python3 .github/evidence/issue-76/phone-fixture/capture-phone.py \
  --runtime-app /path/to/LorcaDev.app \
  --device YOUR_DEDICATED_SIMULATOR_UUID \
  --scenario active
```

Repeat with `--scenario followups`, `--scenario empty`, and `--scenario before` for the inset comparison. The before variant reads only the original screen source from `ac1d9b7`; the production checkout stays on the fixed version. `capture-phone.py` builds the fixture-only offline bundle, copies the supplied runtime, enables embedded loading, installs only that copy on the supplied test device, and captures its framebuffer. The native startup hook disables the fixture runtime's onboarding menu. Verify every resulting image before publishing it.
