// Native AppKit evidence only. The loopback fixture supplies tool metadata, never secrets.
import { readFileSync } from "node:fs"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"

const root = resolve(import.meta.dir, "../../..")
const output = resolve(root, "macos/Tests/Evidence/issue75")
await mkdir(output, { recursive: true })
const policySource = readFileSync(resolve(root, "crates/cli/src/permissions.rs"), "utf8")
const localTools = [...policySource.match(/pub const LOCAL_TOOLS:.*?=\s*&\[(.*?)\];/s)![1].matchAll(/"([^"]+)"/g)].map(match => match[1])
const tools = [
  { name: "list_messages", description: "Read the fixture inbox.", capability: "read", hidden: false },
  { name: "create_draft", description: "Stage a fixture draft without sending it.", capability: "draft", hidden: false },
  { name: "send_message", description: "Send a message (fixture metadata only).", capability: "write", hidden: false },
]
const catalog = {
  local_tools: localTools,
  connections: [
    { id: `gmail-${"1".repeat(32)}`, name: "Gmail · Work", tools },
    { id: `gmail-${"2".repeat(32)}`, name: "Gmail · Personal", tools },
  ],
}
const server = Bun.serve({
  hostname: "127.0.0.1", port: 0,
  fetch(request, server) {
    if (new URL(request.url).pathname === "/ws" && server.upgrade(request)) return
    return new Response("AppKit tool-catalog fixture", { status: 404 })
  },
  websocket: {
    message(socket, raw) {
      const request = JSON.parse(String(raw))
      if (request.method === "bots.permissions") {
        socket.send(JSON.stringify({ id: request.id, result: catalog }))
      } else {
        socket.send(JSON.stringify({ id: request.id, error: { message: `Fixture does not execute ${request.method}` } }))
      }
    },
  },
})
const test = Bun.spawn(["swift", "test", "--package-path", "macos", "--filter", "BotAccessEvidenceTests"], {
  cwd: root,
  env: { ...process.env, LORCA_MOCK: "1", LORCA_PORT: String(server.port), LORCA_UI_EVIDENCE_DIR: output },
  stdin: "ignore", stdout: "inherit", stderr: "inherit",
})
process.on("SIGINT", () => { test.kill(); server.stop(true) })
try {
  const code = await test.exited
  console.log(`Native AppKit evidence: ${output}`)
  process.exitCode = code
} finally {
  server.stop(true)
}
