// The MCP servers a Runner keeps in its mcp.json, after the macOS app's McpServers.swift: each
// one's entry as the file holds it, how it stands, and the tools it offered; the command line the
// server sheet's Command field reads and writes; and the form the sheet edits an entry through.

import { L } from "../l10n";
import type { InstalledPlugin } from "./models";

/** One server's entry, canonical: `command`, `args`, `env`, and `cwd` for a command; `type`, `url`,
 * and `headers` for a remote server; and any field Lorca does not know, as the file has it. */
export interface McpEntry {
  type?: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;
  url?: string;
  headers?: Record<string, string>;
  description?: string;
  disabled?: boolean;
  [key: string]: unknown;
}

export interface McpTool {
  name: string;
  title?: string;
  /** The first line of what it does. */
  description: string;
  /** Its server marked it read-only when it last connected: it runs without Auto-review. */
  readOnly: boolean;
  /** Kept from bots by the entry's `toolExposure`. */
  hidden?: boolean;
}

/** A server in a Runner's mcp.json, usable or not. */
export interface McpServer {
  /** Its key in the file: what the apps show and `lorca mcp` takes. */
  name: string;
  /** The plugin id it runs as, which its tools go by (`id__tool`). */
  id: string;
  enabled: boolean;
  entry: McpEntry;
  /** Why it cannot run: an entry the CLI cannot read, or a name another plugin's id takes. */
  problem?: string;
  /** Its plugin's state while it runs and is on. */
  status?: InstalledPlugin;
  /** A remote server that asked for a sign-in, or is signed in. */
  signsIn: boolean;
  signedIn: boolean;
  /** How many tools it offered when it last connected; none before it has. */
  toolCount?: number;
  /** `mcp.get`'s: the tools themselves. */
  tools?: McpTool[];
}

/** A Runner's mcp.json: where it is, why it cannot be read, and its servers in the file's order. */
export interface McpFile {
  path: string;
  error?: string;
  servers: McpServer[];
}

/** One server of pasted JSON: its name when the JSON gives one, and its entry or why it cannot
 * run. */
export interface ParsedServer {
  name?: string;
  entry?: McpEntry;
  problem?: string;
}

/** A plugin that is one of the Runner's mcp.json servers, which the server sheet edits. */
export function isMcpServer(plugin: InstalledPlugin): boolean {
  return plugin.source === "mcp.json";
}

export function isRemote(entry: McpEntry): boolean {
  return typeof entry.url === "string" && entry.type !== "stdio";
}

export function mcpSymbol(entry: McpEntry): string {
  return isRemote(entry) ? "globe" : "terminal";
}

/** How a server stands, in a few words, and the color they take. */
export function mcpState(server: McpServer): { text: string; color: string } {
  if (server.problem) return { text: server.problem, color: "var(--red)" };
  if (!server.enabled) return { text: L("Off"), color: "var(--label-3)" };
  const status = server.status;
  switch (status?.state) {
    case "ready":
      if (server.toolCount === undefined) return { text: L("Not connected yet"), color: "var(--label-2)" };
      return { text: server.toolCount === 1 ? L("1 tool") : L("%d tools", server.toolCount), color: "var(--green)" };
    case "needs_auth":
      return { text: L("Needs a sign-in"), color: "var(--orange)" };
    case "connecting":
      return { text: status.detail || L("Connecting…"), color: "var(--accent)" };
    case "error":
      return { text: status.detail || L("Couldn't connect"), color: "var(--red)" };
    default:
      return { text: status?.detail ?? "", color: "var(--label-2)" };
  }
}

/** The command line, or the URL, a row shows under the server's name. */
export function mcpAddress(entry: McpEntry): string {
  if (isRemote(entry)) return entry.url ?? "";
  return joinCommandLine([entry.command ?? "", ...(entry.args ?? [])]);
}

// MARK: - The command line

/** `npx -y "My Folder"` as its words, read the way Windows and a shell both read it: spaces part
 * words, double quotes group them with `\"` for a quote inside, single quotes group them as typed,
 * and a backslash is a backslash, which Windows paths need, except in a run before a quote, which
 * halves (Windows' own rule). */
export function splitCommandLine(text: string): string[] {
  const words: string[] = [];
  let word = "";
  let started = false;
  let quote: '"' | "'" | null = null;
  let index = 0;
  while (index < text.length) {
    const char = text[index]!;
    if (quote === "'") {
      if (char === "'") quote = null;
      else word += char;
      index += 1;
      continue;
    }
    if (char === "\\") {
      let run = 0;
      while (text[index + run] === "\\") run += 1;
      if (text[index + run] === '"') {
        // 2n backslashes and a quote are n backslashes and the quote; 2n + 1 are n and a quote mark.
        word += "\\".repeat(Math.floor(run / 2));
        if (run % 2 === 1) {
          word += '"';
          index += run + 1;
        } else {
          index += run;
        }
      } else {
        word += "\\".repeat(run);
        index += run;
      }
      started = true;
      continue;
    }
    if (char === '"') {
      quote = quote === '"' ? null : '"';
      started = true;
    } else if (quote === null && char === "'") {
      quote = "'";
      started = true;
    } else if (quote === null && /\s/.test(char)) {
      if (started) words.push(word);
      word = "";
      started = false;
    } else {
      word += char;
      started = true;
    }
    index += 1;
  }
  if (started) words.push(word);
  return words;
}

/** Words as one command line that `splitCommandLine` reads back as the same words: a word with a
 * space, a quote, or nothing in it in double quotes. */
export function joinCommandLine(words: string[]): string {
  return words.map(quoteWord).join(" ");
}

function quoteWord(word: string): string {
  if (word !== "" && !/[\s"']/.test(word)) return word;
  let out = '"';
  let backslashes = 0;
  for (const char of word) {
    if (char === "\\") {
      backslashes += 1;
      continue;
    }
    out += char === '"' ? "\\".repeat(backslashes * 2 + 1) + '"' : "\\".repeat(backslashes) + char;
    backslashes = 0;
  }
  return `${out}${"\\".repeat(backslashes * 2)}"`;
}

// MARK: - The sheet's form

/** A name that says its value is a key, a token, or a password, which the sheet hides. */
export function looksSecret(name: string): boolean {
  return /key|token|secret|passw|auth|credential|cookie|session/i.test(name);
}

/** An entry as the server sheet's form holds it: the command line as typed, and the environment
 * or headers as rows. */
export interface McpForm {
  remote: boolean;
  command: string;
  env: [string, string][];
  url: string;
  headers: [string, string][];
  description: string;
}

export function formOf(entry: McpEntry): McpForm {
  // No command yet is an empty field, not the quoted empty word `""`.
  const words = [entry.command ?? "", ...(entry.args ?? [])];
  return {
    remote: isRemote(entry),
    command: words.length === 1 && words[0] === "" ? "" : joinCommandLine(words),
    env: Object.entries(entry.env ?? {}),
    url: entry.url ?? "",
    headers: Object.entries(entry.headers ?? {}),
    description: entry.description ?? "",
  };
}

/** The keys the form owns; the entry's others (`cwd`, `oauth`, `disabled`, and fields other apps
 * write) stay as they are. */
const formKeys = ["type", "command", "args", "env", "environment", "url", "serverUrl", "httpUrl", "headers", "description"];

function rows(pairs: [string, string][]): Record<string, string> | undefined {
  const out: Record<string, string> = {};
  for (const [name, value] of pairs) {
    if (name.trim() !== "") out[name.trim()] = value;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

/** The entry the form describes, over `base`'s other fields. */
export function entryOf(form: McpForm, base: McpEntry = {}): McpEntry {
  const entry: McpEntry = {};
  if (form.remote) {
    entry.type = base.type === "sse" ? "sse" : "http";
    entry.url = form.url.trim();
    const headers = rows(form.headers);
    if (headers) entry.headers = headers;
  } else {
    if (base.type === "stdio") entry.type = "stdio";
    const [command = "", ...args] = splitCommandLine(form.command.trim());
    entry.command = command;
    if (args.length > 0) entry.args = args;
    const env = rows(form.env);
    if (env) entry.env = env;
  }
  if (form.description.trim() !== "") entry.description = form.description.trim();
  for (const [key, value] of Object.entries(base)) {
    if (!formKeys.includes(key) && value !== undefined) entry[key] = value;
  }
  return entry;
}

/** Why the form cannot be saved yet, or nothing. */
export function formProblem(name: string, form: McpForm): string | undefined {
  if (name.trim() === "") return L("Give the server a name.");
  if (form.remote) {
    const url = form.url.trim();
    if (url === "") return L("Give the server's URL.");
    if (!(url.startsWith("http://") || url.startsWith("https://") || url.startsWith("${"))) return L("The URL starts with http:// or https://.");
    return undefined;
  }
  if (splitCommandLine(form.command.trim()).length === 0) return L("Give the command to run.");
  return undefined;
}

/** Whether two entries say the same, whatever the order of their keys; an empty list or object is
 * no field at all. */
export function sameEntry(a: McpEntry, b: McpEntry): boolean {
  const normal = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(normal);
    if (typeof value !== "object" || value === null) return value;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) {
      const field = (value as Record<string, unknown>)[key];
      const empty = field === undefined || (Array.isArray(field) && field.length === 0) || (typeof field === "object" && field !== null && !Array.isArray(field) && Object.keys(field).length === 0);
      if (!empty) out[key] = normal(field);
    }
    return out;
  };
  return JSON.stringify(normal(a)) === JSON.stringify(normal(b));
}

/** An entry as the JSON field shows it: the keys in the order Lorca writes them, then the rest. */
export function entryJSON(entry: McpEntry): string {
  const order = ["type", "command", "args", "env", "cwd", "url", "headers", "oauth", "description", "disabled"];
  const sorted: Record<string, unknown> = {};
  for (const key of order) if (entry[key] !== undefined) sorted[key] = entry[key];
  for (const [key, value] of Object.entries(entry)) if (!(key in sorted) && value !== undefined) sorted[key] = value;
  return JSON.stringify(sorted, null, 2);
}

/** What the demo, which has no CLI, makes of pasted JSON: a server, or servers by name under
 * `mcpServers` or `servers`. The CLI reads every other app's shape (`mcp.parse`). */
export function parseLocally(text: string): ParsedServer[] {
  const value: unknown = JSON.parse(text);
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error(L("Not a server's JSON: expected an object."));
  const object = value as Record<string, unknown>;
  const looksLikeServer = (candidate: unknown) => typeof candidate === "object" && candidate !== null && ("command" in candidate || "url" in candidate);
  if (looksLikeServer(object)) return [{ entry: object as McpEntry }];
  const container = (object.mcpServers ?? object.servers ?? object) as Record<string, unknown>;
  const servers = Object.entries(container).filter(([, entry]) => looksLikeServer(entry));
  if (servers.length === 0) throw new Error(L("No MCP servers in that JSON: expected mcpServers, a server's command, or its url."));
  return servers.map(([name, entry]) => ({ name, entry: entry as McpEntry }));
}
