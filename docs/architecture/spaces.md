# Spaces

A Space is a roster entity on one Runner. Bots join it with `space_id`. A bot with no `space_id` is solo. The relay holds Spaces only inside the encrypted roster blob, with the bots.

Standard [Agent Skills](playbooks.md#standard-agent-skills) are discovered at the fixed Space root. Playbooks retain bot/group scopes.

## Roster

`Space` is `id`, `name`, `runner_id`, optional `sandbox`, and `created_at`, and optional `archived`. Archiving hides the Space in the sidebar until restored; membership, file boundaries, bots, and transcripts remain active. The name is one line of at most 60 characters. The Runner is the one the Space was created for.

`sandbox` is a file boundary on that Runner: `{ root? }` or absent. Absent means members use the Runner the way a solo bot does. `root`, when set, is an absolute path. When the sandbox is on and `root` is absent, the boundary is `<LORCA_HOME>/spaces/<id>`.

The roster field is `spaces`. A Device that writes Spaces always sends the array, including an empty one. A roster that omits `spaces` was written by a build without them: this Device keeps the Spaces it has, puts `space_id` and `approval` back on bots that roster dropped, and uploads the roster again (`spaces::take_roster`).

The CLI keeps Spaces in SQLite (`spaces`). The snapshot and `roster.changed` carry `spaces`.

## API

From any Device:

- `spaces.workdir { id }` asks the assigned Runner for the absolute sandbox workdir, including its default `<LORCA_HOME>/spaces/<id>` directory. An open Space also has this root; turning its boundary off changes access restrictions, not its root.
- `spaces.list` answers `{ spaces }`.
- `spaces.create { id?, name, runner_id, sandbox? }` answers `{ space }`. `runner_id` is a Runner. An app may mint `id`.
- `spaces.update { id, name?, sandbox?, archived? }` answers `{ space }`. `sandbox: null` turns the boundary off. A Space does not change Runner.
- `spaces.delete { id }` removes the Space and clears `space_id` on its bots. Their `approval` stays.

`bots.create` and `bots.update` take `space_id` (a Space id, or `null` for solo) and `approval` (`ask`, `never`, or `null`). The bot's Runner is fixed at creation. Profile updates reject a changed Runner before publishing the roster, including an update that also changes Space. The bot's Runner and the Space's Runner are the same. An account holds at most 100 Spaces.

## Approval Never

`approval` on the bot is `ask` when absent, or `never`. `never` does not require a sandbox. A solo bot and a Space bot whose sandbox is off both accept it (`spaces::approval_requires_sandbox` is false for `never`).

Review reads the bot's current profile on each call. `never` skips the approval layer before exact-tool rules and model review, including when account Auto-review is disabled. Access checks and the Space file boundary remain active. A deleted or moved bot cannot reuse an older `never` profile to skip review.

## Runner boundary

`spaces::isolation` is what a Runner reads for a bot. It is open for a solo bot and for a Space with no sandbox. It is the sandbox root when the sandbox is on.

The turn's `review_workdir` is the coding tools' actual directory. Shell review resolves relative paths and its Lorca-folder fast path there, including when the Space root is outside the CLI home. The bot's original working directory remains the base for transcript attachments and steering messages.

`read`, `write`, `edit`, and `bash` use that result, and so do `grep`, `find`, and `ls`, which take the same path. Space execution keeps an explicitly selected child directory when it lies in the Space root, otherwise uses that root; solo execution uses the agent workspace. Changing `space_id` resets an old workdir override unless the same update explicitly supplies another workdir, and moves no files. Approval `never` does not turn Open into a boundary, and it does not require one. A boundary keeps the working directory when it already lies inside the root, and otherwise uses the root. A path is resolved against that directory, then as its real path, and a path outside the root is refused. A shell command runs only when every path it names stays inside the root. A command the parser cannot prove — a substitution, a variable, a glob, or a bare `cd` — is refused. `bash_input` is checked the same way, because it can type another command. `publish_output` resolves in that same directory, and a path outside the root is refused. `coding_agent` starts in that directory. Its folder stays inside the root, and a repository's worktree is created under `<root>/worktrees`.
