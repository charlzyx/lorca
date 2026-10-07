# Event triggers

An event subscription starts work in a bot's DM on its assigned Runner. The user configures a `gateway_hmac` source, an event-type allowlist, JSON-pointer equality filters, a target bot and optional routine, and the task prompt. `crates/cli/src/event_triggers.rs` owns subscriptions, authenticated delivery, health, and the durable inbox. Routines keep their scheduled checks alongside these events; a service event runs the configured task directly.

## Configuration and health

`lorca events add <config.json>` creates a subscription on this Runner. A configuration is:

```json
{
  "name": "PR changes",
  "source": "gateway_hmac",
  "bot_id": "bot-12345678",
  "routine_id": null,
  "prompt": "Summarize new PR activity. Read the PR to verify its current state, and tell me what needs attention.",
  "event_types": ["github.pull_request"],
  "filters": [
    {"pointer": "/repository/full_name", "equals": "acme/project"},
    {"pointer": "/action", "equals": "synchronize"}
  ],
  "queue_policy": "fifo",
  "is_enabled": true,
  "expires_at": null
}
```

The CLI generates the subscription id and a random gateway signing secret. A routine target belongs to the selected bot; the job carries its `routine_id` through the normal runtime. The subscription has its own prompt, so an event can ask for work that differs from the routine's polling task. An event does not run the routine's check or move its schedule cursor. A paused routine holds event work that targets it.

`events edit <id> <config.json>` replaces the configuration with the same target; another target uses another subscription. `events list` shows configuration, state (`idle`, `ready`, `running`, `paused`, `expired`, `blocked`, `attention`), pending count, the twenty newest delivery ids and states, last authenticated receipt, last successful model outcome, and safe recovery text. Replies omit signing secrets and payloads. `events pause <id>` retains incoming and queued deliveries; `events resume <id>` resumes them. A turn already executing settles normally. A turn waiting for its chat lock rechecks pause, expiry and the target routine before inference.

`events reconnect <id> [--expires-at <unix-seconds>]` rotates the signing secret and generation, clears authentication health, and replaces the expiry (omitting it removes expiry). Export a fresh route afterwards. A route from the old generation fails authentication. Already authenticated inbox items remain queued; deliveries still on the gateway or relay with an old signature need redelivery signed with the new route. `events edit` can extend expiry while keeping the current signing key. Expiry retains pending work and stops execution. A week without a user message since the subscription was created or resumed pauses it, using the same seven-day boundary as routines.

`events remove <id>` deletes a subscription and its inbox once its active turn has stopped. `events retry <delivery-id>` explicitly retries failed or uncertain work after the user reviews the chat; `events discard <delivery-id>` removes pending, failed or uncertain work from the queue while retaining its deduplication mark.

The local API uses `events.list/create/update/pause/resume/reconnect/route/delete/retry/discard`, plus `events.ingest { envelope }` for signed direct delivery and `events.forward { route, envelope }` for a gateway. Lifecycle requests use `id`, creation uses `config`, and update uses both. `runner_id` routes management through the existing sealed `request`/`response` mechanism, so paired Devices inspect and manage a Runner through their local CLI. Subscription ownership and integrations remain on the assigned Runner.

## User-controlled gateway

The gateway is a paired Device the user controls, or the assigned Runner itself. It receives plaintext from the service over the user's HTTPS endpoint, verifies the service's signature, constructs a signed Lorca envelope, and invokes `lorca events forward <route.json>` with the envelope on stdin. Lorca verifies the gateway signature and seals the envelope to the Runner's X25519 box key before queuing it in the gateway's durable relay outbox. A gateway on the assigned Runner verifies and persists the delivery directly. A running `lorca serve` drains the gateway's outbox; queued ciphertext survives both a relay outage and a gateway restart.

The gateway uses its existing paired machine and relay bearer. Its pairing is an account Device pairing, with the account's encrypted credential sync described in [Identity](identity.md); the route does not register another identity or connection. The service's token and webhook secret stay on the gateway. The AppKit app accesses Runner state through the local CLI.

To set up the included GitHub PR gateway:

1. On the assigned Runner, create the subscription above, using its bot id and repository. Use `lorca events list` to read the new id, then `lorca events route <id> /private/path/pr-route.json`. The route contains the signing secret and Runner public keys. The CLI creates a new file with mode 0600 and refuses to overwrite an existing file.
2. Put that route on the user-controlled paired gateway, or keep it on the Runner. Keep a random GitHub webhook secret of at least 32 bytes in another private file. Run `lorca serve` on this Device with its usual `LORCA_HOME`, port and relay URL.
3. Run `python3 scripts/event-gateway.py --route /private/path/pr-route.json --github-secret-file /private/path/github-secret --lorca /absolute/path/lorca`. The script listens on `127.0.0.1:8984/github`. Put the user's HTTPS reverse proxy in front of that path.
4. In the GitHub repository's webhook settings, set that HTTPS URL, JSON content, the same GitHub secret, and Pull requests events. Send a test PR event and inspect `lorca events list` on the Runner. Subscribe to the desired actions with JSON filters.

The script compares `X-Hub-Signature-256` against HMAC-SHA256 of the exact request body in constant time, following [GitHub's verification contract](https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries). It recognizes `pull_request` from the signed body. GitHub's event and delivery headers are outside that signature, so the script derives the Lorca delivery id from SHA-256 of the signed body and uses the fixed `github.pull_request` type. Identical bodies become the same delivery, even when their unsigned headers differ. The gateway acknowledges with `202` only after Lorca durably accepts or queues ciphertext. Invalid signatures get `403`; oversized input gets `413`; unavailable persistence gets `503`. GitHub's delivery history supplies explicit redelivery after a rejected request. Requests and errors log no bodies or secrets.

Other adapters for incoming messages or completed meeting transcripts use the same gateway contract. They verify their provider's signature, timestamp/challenge and stable event id before signing the Lorca envelope. These adapters keep service credentials and plaintext on that gateway. The subscription's type and equality filters operate on the authenticated envelope without executing service content or a model.

## Signed delivery contract

An envelope is JSON:

```json
{
  "version": 1,
  "subscription_id": "ev-uuid",
  "generation": 1,
  "delivery_id": "provider-stable-delivery-id",
  "occurred_at": 1791395200,
  "event_type": "github.pull_request",
  "payload": "{\"repository\":{\"full_name\":\"acme/project\"}}",
  "signature": "base64url-hmac-sha256-without-padding"
}
```

`payload` is the exact JSON text. Signing input is compact UTF-8 JSON of this array, with non-ASCII characters unescaped:

```text
[version, subscription_id, generation, delivery_id, occurred_at, event_type, payload]
```

`signature` is unpadded base64url of HMAC-SHA256 over those bytes, keyed by the UTF-8 route secret. This signs every routing, replay and filter field as well as the body. The Runner verifies it in constant time before evaluating filters, including for relay deliveries. Unknown versions, invalid signatures, mismatched generations, malformed JSON, payloads over 64 KiB, timestamps over five minutes ahead, and deliveries over seven days old start no work. The seven-day ingress window matches relay envelope retention; clocks use UTC Unix seconds. The signature establishes authenticity from the configured gateway, while service content remains untrusted data.

## Durable ordering and execution

SQLite `event_subscriptions` and `event_inbox` store account-key-encrypted ciphertext with `event_subscription` and `event_inbox` associated data. Only opaque subscription ids, delivery hashes and queue positions are columns. Secrets, filters, target ids, prompts, payloads and runtime health are encrypted. These tables clear when the identity is forgotten. The inbox and dedup record commit atomically before relay acknowledgement; a persistence failure leaves the sync cursor before that envelope and retries it. Relay deletion is queued durably after receipt, independently of whether a model turn runs.

The dedup key hashes `(subscription_id, delivery_id)` and does not include the relay blob id or signing-key generation. Duplicate upload, redelivery or rotation creates one item. Terminal items retain dedup marks for thirty days, beyond the seven-day ingress window. Completed, filtered and coalesced items discard their payloads. Pending, failed and uncertain work remains until execution or explicit removal. FIFO receipt persists valid deliveries even when execution is paused, so a paused subscription's backlog does not stop other account synchronization. A storage failure retains the relay envelope for retry.

FIFO executes in durable arrival order, one item per subscription at a time. Timestamps do not reorder delivery. A failure or uncertain outcome blocks later items until explicit retry or discard. `latest` replaces all still-pending items of that subscription with the newest arrival, retaining tombstones for replaced deliveries; it leaves a running item alone. A subscription spans all its allowed event types, so latest coalesces across those types. Different subscriptions have independent ordering. The runtime serializes their jobs with all other work in the bot's DM.

Admission snapshots the owner's configured prompt independently of service data. The `event` Job passes through `runtime::start_turn`, the assigned Runner, the chat lock, the account's providers, the normal TurnHooks, nested codemode hooks and tool Auto-review. It opens an `Event · Name` marker containing the trusted task. The payload is an ephemeral, bounded JSON cue explicitly described as untrusted data, never instructions, approval, permissions or credentials. Auto-review reads the configured task and the turn's steps; it never treats the payload as a request from the user. Event jobs are unattended, so actions that need a user's answer are refused as they are in routine turns.

Routine-targeted events carry the routine scope into the existing runtime admission boundary; bot-targeted events are normal ad-hoc jobs at that boundary. The runtime's spending records and unattended script timeout apply to event turns as to routines. The event inbox supplies the trusted job, target and routine scope; a generic relay `job` cannot admit an event turn. Event subscriptions use the existing task and connection model.

A Runner records a delivery as running before admitting a Job. On process restart, such an item becomes `uncertain`: a tool action may already have taken effect, so the Runner does not automatically repeat it. The user reads the existing chat and chooses Retry or Discard. This defines durable delivery and deduplication without promising exactly-once external effects.

## Relay transport

Protocol 3 adds `kind=event` to the relay and Device poll lists. Deploy a protocol-3 relay before clients that request this kind. The envelope is sealed to `recipient_machine_pubkey`; it has no slot or group, and the relay refuses an unaddressed event. The relay stores opaque ciphertext, public keys, ids, sequence, size and timestamp, just as it does other machine envelopes. Only the recipient's machine can list or open it. Unconsumed event envelopes expire after seven days with other sealed work. The recipient transfers accepted work to its encrypted local inbox before deleting the relay copy.
