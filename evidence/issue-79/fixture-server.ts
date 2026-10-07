// Synthetic localhost responses for the opt-in native AppKit screenshot harness.
// No credentials, private chats, provider calls, relay, or external service connections.
const portFileIndex = Bun.argv.indexOf("--port-file")
const portFile = Bun.argv[portFileIndex + 1]
if (portFileIndex < 0 || !portFile) throw new Error("Pass --port-file <temporary path>")

const base = {
  runner_id: "fixture-runner", bot_id: "fixture-bot", chat_id: "fixture-chat",
  updated_at: 1_900_000_000, task_id: null,
}
const usage = {
  tokens: 7_500, api_cost_usd: 1.25, subscription_estimate_usd: 0.75,
  unknown_price_calls: 2, estimated_calls: 1, model_calls: 6,
  runtime_secs: 120, retries: 1, connector_calls: 20,
}
const limits = {
  max_usd: 5.0, max_tokens: 20_000, max_runtime_secs: 900,
  max_retries: 3, max_connector_calls: 80,
}
const taskID = "task-00000000-0000-4000-8000-000000000079"
const interruptedID = "task-00000000-0000-4000-8000-000000000080"
const budgets = [
  { ...base, kind: "task", id: taskID, task_id: taskID, job_kind: "task",
    state: "ready", reason: null, limits, usage },
  { ...base, kind: "routine", id: "routine-fixture", job_kind: "routine",
    state: "budget_exhausted",
    reason: "Budget exhausted: the token allowance is used. Increase the allowance or explicitly renew it to resume.",
    limits: { ...limits, max_tokens: 10_000, max_runtime_secs: 600, max_connector_calls: 50 },
    usage: { ...usage, tokens: 10_000, api_cost_usd: 2.6, subscription_estimate_usd: 1.0,
      runtime_secs: 590, retries: 3, connector_calls: 40 } },
  { ...base, kind: "task", id: interruptedID, task_id: interruptedID, job_kind: "task",
    state: "interrupted",
    reason: "The Runner restarted during work. Check its completed effects, then explicitly resume.",
    limits: { ...limits, max_usd: 2.5, max_tokens: 5_000, max_runtime_secs: 300 },
    usage: { ...usage, tokens: 3_000, api_cost_usd: 0.4, subscription_estimate_usd: 0.2,
      runtime_secs: 64, retries: 0, connector_calls: 5 } },
]
const cooldown = Math.floor(Date.now() / 1000) + 5 * 60

const server = Bun.serve({
  hostname: "127.0.0.1", port: 0,
  fetch(request, server) {
    if (new URL(request.url).pathname === "/ws" && server.upgrade(request)) return
    return new Response("issue79-native-fixture")
  },
  websocket: {
    message(socket, raw) {
      const frame = JSON.parse(String(raw))
      let result: unknown
      if (frame.method === "fixture.info") result = { fixture: "issue79-native-fixture" }
      else if (frame.method === "budgets.list") result = { budgets }
      else if (frame.method === "connector_limits.get") {
        result = frame.params?.scope === "service"
          ? { limits: { max_calls: 60, window_secs: 60, max_concurrency: 4 }, active_calls: 3, retry_at: null }
          : { limits: { max_calls: 20, window_secs: 60, max_concurrency: 2 }, active_calls: 2, retry_at: cooldown }
      } else {
        socket.send(JSON.stringify({ id: frame.id, error: { message: "Read-only screenshot fixture: unsupported method" } }))
        return
      }
      socket.send(JSON.stringify({ id: frame.id, result }))
    },
  },
})
await Bun.write(portFile, String(server.port))
console.log(`issue79-native-fixture ready on 127.0.0.1:${server.port}`)
