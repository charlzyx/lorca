// A Runner's MCP servers in the model: the command line the server sheet reads and writes, the form
// it edits an entry through, and how a server's state reads.

import { expect, test } from "bun:test";
import { entryJSON, entryOf, formOf, formProblem, joinCommandLine, looksSecret, mcpAddress, mcpState, parseLocally, sameEntry, splitCommandLine, type McpServer } from "./mcp";

test("a command line splits the way Windows and a shell both read it", () => {
  expect(splitCommandLine("npx -y @modelcontextprotocol/server-filesystem ~/Documents")).toEqual(["npx", "-y", "@modelcontextprotocol/server-filesystem", "~/Documents"]);
  expect(splitCommandLine('"C:\\Program Files\\nodejs\\node.exe" server.js')).toEqual(["C:\\Program Files\\nodejs\\node.exe", "server.js"]);
  // A backslash is a backslash, unless a run of them comes before a quote.
  expect(splitCommandLine("C:\\Users\\me\\mcp.exe \\\\server\\share")).toEqual(["C:\\Users\\me\\mcp.exe", "\\\\server\\share"]);
  expect(splitCommandLine('"say \\"hi\\"" \'single quoted\' it\'s')).toEqual(['say "hi"', "single quoted", "its"]);
  expect(splitCommandLine('"C:\\dir\\\\" next')).toEqual(["C:\\dir\\", "next"]);
  expect(splitCommandLine('a "" b')).toEqual(["a", "", "b"]);
  expect(splitCommandLine("  spaced   out  ")).toEqual(["spaced", "out"]);
  expect(splitCommandLine("")).toEqual([]);
});

test("words joined into a line read back as the same words", () => {
  const cases = [
    ["npx", "-y", "@scope/pkg"],
    ["C:\\Program Files\\nodejs\\node.exe", "C:\\My Server\\index.js"],
    ["echo", 'a "quoted" word', "it's", ""],
    ["trailing\\", "dir with space\\", "\\\\unc\\path"],
    ["docker", "run", "-i", "--rm", "-e", "GITHUB_TOKEN", "ghcr.io/github/github-mcp-server"],
  ];
  for (const words of cases) expect(splitCommandLine(joinCommandLine(words))).toEqual(words);
  expect(joinCommandLine(["npx", "-y", "My Folder"])).toBe('npx -y "My Folder"');
  expect(joinCommandLine(["C:\\Program Files\\x.exe"])).toBe('"C:\\Program Files\\x.exe"');
});

test("the form edits an entry and keeps the fields it does not show", () => {
  const entry = { command: "npx", args: ["-y", "pkg", "My Folder"], env: { API_KEY: "secret", DEBUG: "1" }, cwd: "~/work", alwaysAllow: ["read"], disabled: true };
  const form = formOf(entry);
  expect(form).toEqual({ remote: false, command: 'npx -y pkg "My Folder"', env: [["API_KEY", "secret"], ["DEBUG", "1"]], url: "", headers: [], description: "" });
  // Untouched, it says what the entry said.
  expect(sameEntry(entryOf(form, entry), entry)).toBe(true);
  // A new command and an emptied environment row; cwd, alwaysAllow, and disabled stay.
  const edited = entryOf({ ...form, command: "uvx server --port 1", env: [["DEBUG", "2"], ["", "dropped"]] }, entry);
  expect(edited).toEqual({ command: "uvx", args: ["server", "--port", "1"], env: { DEBUG: "2" }, cwd: "~/work", alwaysAllow: ["read"], disabled: true });
  // To a URL: the command's fields go, a type comes.
  const remote = entryOf({ ...form, remote: true, url: " https://mcp.example.com/mcp ", headers: [["Authorization", "Bearer x"]] }, entry);
  expect(remote).toEqual({ type: "http", url: "https://mcp.example.com/mcp", headers: { Authorization: "Bearer x" }, cwd: "~/work", alwaysAllow: ["read"], disabled: true });
  // An SSE server stays one.
  expect(entryOf(formOf({ type: "sse", url: "https://x.test/sse" }), { type: "sse", url: "https://x.test/sse" }).type).toBe("sse");
});

test("what stops a save, in words", () => {
  const form = formOf({ command: "npx" });
  expect(formProblem("", form)).toBe("Give the server a name.");
  expect(formProblem("fs", { ...form, command: "   " })).toBe("Give the command to run.");
  expect(formProblem("fs", form)).toBeUndefined();
  const remote = { ...form, remote: true };
  expect(formProblem("api", remote)).toBe("Give the server's URL.");
  expect(formProblem("api", { ...remote, url: "mcp.example.com" })).toBe("The URL starts with http:// or https://.");
  expect(formProblem("api", { ...remote, url: "${BASE}/mcp" })).toBeUndefined();
});

test("entries compare by what they say", () => {
  expect(sameEntry({ command: "a", args: [] }, { command: "a" })).toBe(true);
  expect(sameEntry({ env: { B: "2", A: "1" }, command: "a" }, { command: "a", env: { A: "1", B: "2" } })).toBe(true);
  expect(sameEntry({ command: "a", args: ["x"] }, { command: "a" })).toBe(false);
  expect(entryJSON({ args: ["-y"], command: "npx", trust: true })).toBe('{\n  "command": "npx",\n  "args": [\n    "-y"\n  ],\n  "trust": true\n}');
});

test("names that hold a secret, and how servers read", () => {
  expect(["API_KEY", "GITHUB_PERSONAL_ACCESS_TOKEN", "Authorization", "client_secret", "DB_PASSWORD"].every(looksSecret)).toBe(true);
  expect(["DEBUG", "PORT", "X-Team", "ROOT_DIR"].some(looksSecret)).toBe(false);
  expect(mcpAddress({ command: "npx", args: ["-y", "My Folder"] })).toBe('npx -y "My Folder"');
  expect(mcpAddress({ type: "http", url: "https://x.test/mcp" })).toBe("https://x.test/mcp");
  const server: McpServer = { name: "fs", id: "fs", enabled: true, entry: { command: "npx" }, signsIn: false, signedIn: false };
  const status = { id: "fs", name: "fs", description: "", version: "", icon: "terminal", detail: "Ready", source: "mcp.json" };
  expect(mcpState({ ...server, status: { ...status, state: "ready" }, toolCount: 1 }).text).toBe("1 tool");
  expect(mcpState({ ...server, status: { ...status, state: "ready" }, toolCount: 14 }).text).toBe("14 tools");
  expect(mcpState({ ...server, status: { ...status, state: "ready" } }).text).toBe("Not connected yet");
  expect(mcpState({ ...server, status: { ...status, state: "error", detail: "Cannot start npx" } }).text).toBe("Cannot start npx");
  expect(mcpState({ ...server, enabled: false }).text).toBe("Off");
  expect(mcpState({ ...server, problem: "Give the server a command to run or a URL to connect to." }).color).toBe("var(--red)");
});

test("the demo reads pasted JSON without a CLI", () => {
  expect(parseLocally('{"command": "npx"}')).toEqual([{ entry: { command: "npx" } }]);
  expect(parseLocally('{"mcpServers": {"a": {"url": "https://a.test"}, "b": {"command": "b"}}}').map((server) => server.name)).toEqual(["a", "b"]);
  expect(() => parseLocally('{"theme": "dark"}')).toThrow();
});
