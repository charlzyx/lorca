# Spaces

A Space is a roster entity on one Runner. Bots join it with `space_id`. A bot with no `space_id` is solo. The relay holds Spaces only inside the encrypted roster blob, with the bots.

Skills stay [playbooks](playbooks.md) on a bot or a group. A Space does not hold them.

## Roster

`Space` is `id`, `name`, `runner_id`, optional `sandbox`, and `created_at`. The name is one line of at most 60 characters. The Runner is the one the Space was created for.

`sandbox` is a file boundary on that Runner: `{ root? }` or absent. Absent means members use the Runner the way a solo bot does. `root`, when set, is an absolute path. When the sandbox is on and `root` is absent, the boundary is `<LORCA_HOME>/spaces/<id>`.

The roster field is `spaces`. A Device that writes Spaces always sends the array, including an empty one. A roster that omits `spaces` was written by a build without them: this Device keeps the Spaces it has, puts `space_id` and `approval` back on bots that roster dropped, and uploads the roster again (`spaces::take_roster`).

The CLI keeps Spaces in SQLite (`spaces`). The snapshot and `roster.changed` carry `spaces`.

## API

From any Device:

- `spaces.list` answers `{ spaces }`.
- `spaces.create { id?, name, runner_id, sandbox? }` answers `{ space }`. `runner_id` is a Runner. An app may mint `id`.
- `spaces.update { id, name?, sandbox? }` answers `{ space }`. `sandbox: null` turns the boundary off. A Space does not change Runner.
- `spaces.delete { id }` removes the Space and clears `space_id` on its bots. Their `approval` stays.

`bots.create` and `bots.update` take `space_id` (a Space id, or `null` for solo) and `approval` (`ask`, `never`, or `null`). The bot's Runner and the Space's Runner are the same. An account holds at most 100 Spaces.

## Approval Never

`approval` on the bot is `ask` when absent, or `never`. `never` does not require a sandbox. A solo bot and a Space bot whose sandbox is off both accept it (`spaces::approval_requires_sandbox` is false for `never`).

## Runner boundary

`spaces::isolation` is what a Runner reads for a bot. It is open for a solo bot and for a Space with no sandbox. It is the sandbox root when the sandbox is on.

`read`, `write`, `edit`, and `bash` use that result, and so do `grep`, `find`, and `ls`, which take the same path. Open keeps the bot's working directory. Approval `never` does not turn Open into a boundary, and it does not require one. A boundary keeps the working directory when it already lies inside the root, and otherwise uses the root. A path is resolved against that directory, then as its real path, and a path outside the root is refused. A shell command runs only when every path it names stays inside the root. A command the parser cannot prove — a substitution, a variable, a glob, or a bare `cd` — is refused. `bash_input` is checked the same way, because it can type another command. `publish_output` resolves in that same directory, and a path outside the root is refused. `coding_agent` starts in that directory. Its folder stays inside the root, and a repository's worktree is created under `<root>/worktrees`.
