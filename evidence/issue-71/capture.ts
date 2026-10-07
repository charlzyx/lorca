// Opt-in native screenshot fixture. Run from any directory with:
// bun run evidence/issue-71/capture.ts
// Only this test process receives the temporary home and loopback fixture port.
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { resolve, join } from "node:path"

const root = resolve(import.meta.dir, "../..")
const output = import.meta.dir
const index = await Bun.file(join(root, "crates/cli/marketplace/index.json")).json()
const serviceIDs = ["slack", "gmail", "google-calendar", "google-drive"]
const manifests = index.plugins.filter((plugin: any) => serviceIDs.includes(plugin.id))
const status = (service: string, label: string, state: string, detail: string, n: number) => {
  const manifest = manifests.find((plugin: any) => plugin.id === service)
  return {
    id: `${service}-${n.toString(16).padStart(32, "0")}`,
    service_id: service,
    account_name: label,
    name: `${manifest.name} · ${label}`,
    description: manifest.description,
    version: manifest.version,
    icon: manifest.icon,
    state,
    detail,
  }
}
const plugins = [
  status("gmail", "Work", "ready", "Connected", 1),
  status("gmail", "Personal", "needs_auth", "Sign in", 2),
  status("gmail", "Shared", "insufficient_access", "Sign in again to grant the required access", 3),
  status("gmail", "New account", "needs_setup", "Needs GOOGLE_CLIENT_ID", 4),
  status("gmail", "Offline", "error", "Unable to reach the service. Try again.", 5),
  status("slack", "Work", "error", "Unable to reach the service. Try again.", 6),
]
const runner = {
  id: "fixture-runner",
  name: "Fixture Runner",
  model: "Test Mac",
  os: "macos",
  os_version: "Test fixture",
  machine_key: "fixture-public-key",
  is_this_device: true,
  status: "online",
  last_seen: 0,
  plugins,
}
const snapshot = {
  version: "0.0.0-fixture",
  has_identity: true,
  is_identity_device: false,
  relay_connected: false,
  devices: [runner], bots: [], chats: [], routines: [], providers: [], models: [],
  auto_review: { is_enabled: true, rules: [] },
  running_chat_ids: [], running_turns: [],
}
const detail = (id: string) => {
  const account = plugins.find(plugin => plugin.id === id)
  if (!account) throw new Error("Unknown fixture account")
  const manifest = manifests.find((plugin: any) => plugin.id === account.service_id)
  const configured = account.state !== "needs_setup"
  return {
    manifest, status: account,
    variables: manifest.variables.map((variable: any) => ({
      ...variable, is_set: configured,
      value: variable.secret || !configured ? null : "fixture-client-id",
    })),
    servers: Object.entries(manifest.servers).map(([name, spec]: [string, any]) => ({
      name, kind: "http", auth: { url: spec.url, oauth: true,
        signed_in: ["ready", "error"].includes(account.state) },
    })),
    skills: manifest.skills.map((skill: any) => ({ name: skill.name, description: skill.description })),
  }
}

const home = await mkdtemp(join(tmpdir(), "lorca-71-ui-"))
const server = Bun.serve({
  hostname: "127.0.0.1", port: 0,
  fetch(request, server) {
    const path = new URL(request.url).pathname
    if (path === "/capture-fixture") return Response.json({ issue: 71, runner })
    if (path === "/ws" && server.upgrade(request)) return
    return new Response("Lorca issue 71 read-only screenshot fixture")
  },
  websocket: {
    message(ws, raw) {
      const request = JSON.parse(String(raw))
      try {
        const result = request.method === "bootstrap" ? snapshot
          : request.method === "marketplace" ? { plugins: manifests, bots: [] }
          : request.method === "plugins.detail" ? detail(request.params.plugin_id)
          : (() => { throw new Error("The screenshot fixture accepts reads only") })()
        ws.send(JSON.stringify({ id: request.id, result }))
      } catch (error) {
        ws.send(JSON.stringify({ id: request.id, error: { message: String(error) } }))
      }
    },
  },
})
try {
  const test = Bun.spawn(["swift", "test", "--package-path", join(root, "macos"), "--filter", "IntegrationScreenshotTests"], {
    cwd: root,
    env: { ...process.env, LORCA_PORT: String(server.port), LORCA_HOME: home, LORCA_MOCK: "0",
      LORCA_TRACE_STARTUP: "0", LORCA_CAPTURE_71_DIR: output },
    stdout: "inherit", stderr: "inherit",
  })
  const result = await test.exited
  if (result !== 0) process.exitCode = result
} finally {
  server.stop(true)
  await rm(home, { recursive: true, force: true })
}
