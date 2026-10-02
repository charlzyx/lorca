// Adds or edits one of a Runner's MCP servers, after the macOS app's McpServerViewController: its
// name, the command that starts it or its URL, the environment or headers, and what it is for, in a
// form or as the JSON of its mcp.json entry; JSON pasted anywhere in the form, as READMEs and other
// apps' configs write it, opens the JSON view. Once saved, the sheet follows the server: how it
// stands, Reconnect or Sign In, a switch to turn it off, and the tools it offered.

import { createMemo, createSignal, For, onCleanup, onSettled, Show } from "solid-js";
import { L } from "../../l10n";
import {
  entryJSON,
  entryOf,
  formOf,
  formProblem,
  isRemote,
  looksSecret,
  mcpAddress,
  mcpState,
  sameEntry,
  type McpEntry,
  type McpForm,
  type McpServer,
  type ParsedServer,
} from "../../model/mcp";
import type { Device } from "../../model/models";
import { onStoreEvent } from "../../model/reactive";
import { errorText, store } from "../../model/store";
import { Button, HoverButton, LinkButton, Segmented, Spinner, Switch, TextArea, TextField } from "../controls";
import { Icon } from "../icons";
import { alert, presentSheet, Sheet } from "../overlay";
import { AccessoryRow, Section } from "../sections";

let fetching = false;

/** Opens a server of `runner`'s mcp.json (fetched first, with its tools, so the sheet opens filled
 * in), or an empty sheet to add one. */
export async function presentMcpServer(runner: Device, name?: string): Promise<void> {
  if (fetching) return;
  let server: McpServer | undefined;
  if (name !== undefined) {
    fetching = true;
    try {
      server = await store.mcpServer(name, runner.id);
    } catch (error) {
      void alert({ message: L("Couldn't open %@", name), informative: errorText(error) });
      return;
    } finally {
      fetching = false;
    }
  }
  presentSheet((dismiss) => <McpServerSheet runner={runner} server={server} dismiss={dismiss} />);
}

/** Text that reads as JSON rather than a command, a URL, or a name: what a README's snippet is. */
function looksLikeJSON(text: string): boolean {
  const trimmed = text.trim();
  return trimmed.startsWith("{") || /^"[^"]+"\s*:\s*\{/.test(trimmed);
}

/** Names and values as rows of fields, a row to add, and a button to remove each: a command's
 * environment, or a remote server's headers. A value whose name says it is a key is hidden until
 * its eye shows it. */
function PairsEditor(props: {
  pairs: [string, string][];
  namePlaceholder: string;
  valuePlaceholder: string;
  addTitle: string;
  label: string;
  disabled?: boolean;
  onChange: (pairs: [string, string][]) => void;
}) {
  const [revealed, setRevealed] = createSignal<ReadonlySet<number>>(new Set());
  const set = (index: number, pair: [string, string]) => props.onChange(props.pairs.map((each, at) => (at === index ? pair : each)));
  const remove = (index: number) => {
    props.onChange(props.pairs.filter((_, at) => at !== index));
    setRevealed(new Set([...revealed()].filter((at) => at !== index).map((at) => (at > index ? at - 1 : at))));
  };
  const toggle = (index: number) => {
    const next = new Set(revealed());
    if (next.has(index)) next.delete(index);
    else next.add(index);
    setRevealed(next);
  };
  let list: HTMLDivElement | undefined;
  const add = () => {
    props.onChange([...props.pairs, ["", ""]]);
    // The new row's name takes the keyboard.
    requestAnimationFrame(() => [...(list?.querySelectorAll<HTMLInputElement>(".pair-name") ?? [])].at(-1)?.focus());
  };
  return (
    <div ref={(element) => (list = element)} class="pairs" role="group" aria-label={props.label}>
      <For each={props.pairs} keyed={false}>
        {(pair, index) => {
          const secret = () => looksSecret(pair()[0]);
          return (
            <div class="pair-row">
              <input
                class="text-field mono small pair-name"
                value={pair()[0]}
                placeholder={props.namePlaceholder}
                disabled={props.disabled}
                spellcheck="false"
                autocomplete="off"
                aria-label={L("Name")}
                onInput={(event) => set(index, [event.currentTarget.value, pair()[1]])}
              />
              <span class="key-field pair-value">
                <input
                  class="text-field mono small"
                  type={secret() && !revealed().has(index) ? "password" : "text"}
                  value={pair()[1]}
                  placeholder={props.valuePlaceholder}
                  disabled={props.disabled}
                  spellcheck="false"
                  autocomplete="off"
                  aria-label={L("Value")}
                  onInput={(event) => set(index, [pair()[0], event.currentTarget.value])}
                />
                <Show when={secret()}>
                  <HoverButton
                    class="key-reveal"
                    symbol={revealed().has(index) ? "eye.slash" : "eye"}
                    size={13}
                    tooltip={revealed().has(index) ? L("Hide value") : L("Show value")}
                    disabled={props.disabled}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => toggle(index)}
                  />
                </Show>
              </span>
              <HoverButton symbol="minus.circle" size={14} tooltip={L("Remove")} disabled={props.disabled} onClick={() => remove(index)} />
            </div>
          );
        }}
      </For>
      <LinkButton class="pairs-add" disabled={props.disabled} onClick={add}>
        <Icon name="plus" size={11} strokeWidth={2.4} />
        {props.addTitle}
      </LinkButton>
    </div>
  );
}

/** What pasted JSON came to: the servers in it, or why it could not be read. */
type Reading = { kind: "empty" } | { kind: "reading" } | { kind: "read"; servers: ParsedServer[] } | { kind: "failed"; message: string };

function McpServerSheet(props: { runner: Device; server?: McpServer; dismiss: () => void }) {
  const runner = props.runner;
  /** The server as the Runner last answered for it; none while it is being added. */
  const [saved, setSaved] = createSignal<McpServer | undefined>(props.server);
  const initial = props.server?.entry ?? {};
  const [name, setName] = createSignal(props.server?.name ?? "");
  const [mode, setMode] = createSignal<"form" | "json">("form");
  const [form, setForm] = createSignal<McpForm>(formOf(initial));
  const [json, setJSON] = createSignal("");
  const [reading, setReading] = createSignal<Reading>({ kind: "empty" });
  const [busy, setBusy] = createSignal(false);
  const [connecting, setConnecting] = createSignal(false);
  const [status, setStatus] = createSignal<{ text: string; color: string; spinning?: boolean } | null>(null);
  let closed = false;
  onCleanup(() => {
    closed = true;
    clearTimeout(readTimer);
  });

  const base = () => saved()?.entry ?? {};
  /** The entry the form, or the one server of the JSON, describes. */
  const entry = (): McpEntry | undefined => {
    if (mode() === "form") return entryOf(form(), base());
    const state = reading();
    return state.kind === "read" && state.servers.length === 1 ? state.servers[0]!.entry : undefined;
  };
  /** The JSON's servers when it holds more than one, which an Add adds together. */
  const several = () => {
    const state = reading();
    return mode() === "json" && saved() === undefined && state.kind === "read" && state.servers.length > 1 ? state.servers : undefined;
  };
  /** What stops a save, in words, or nothing. */
  const problem = (): string | undefined => {
    if (mode() === "form") return formProblem(name(), form());
    const state = reading();
    switch (state.kind) {
      case "empty":
        return L("Paste a server's JSON.");
      case "reading":
        return L("Reading…");
      case "failed":
        return state.message;
      case "read": {
        // Several are added together, the ones that can run; the note names the others.
        const all = several();
        if (all) return all.every((server) => server.problem) ? all[0]!.problem : undefined;
        if (state.servers.length > 1) return L("That JSON has %d servers. Add them from an empty sheet, or keep one.", state.servers.length);
        const server = state.servers[0]!;
        if (server.problem) return server.problem;
        return name().trim() === "" ? L("Give the server a name.") : undefined;
      }
    }
  };
  /** Whether what the sheet shows differs from what the Runner has. */
  const dirty = createMemo(() => {
    const current = saved();
    if (!current) return true;
    if (name().trim() !== current.name) return true;
    const next = entry();
    return next !== undefined && !sameEntry(next, current.entry);
  });
  const confirmTitle = () => {
    if (several()) return L("Add %d Servers", several()!.length);
    if (!saved()) return L("Add");
    return dirty() ? L("Save") : L("Done");
  };
  const confirmDisabled = () => busy() || (dirty() && problem() !== undefined);

  // MARK: The JSON

  let readTimer: ReturnType<typeof setTimeout> | undefined;
  let reads = 0;
  /** Reads the JSON once it stops changing, through the CLI here, which knows every app's spelling.
   * It takes the text, since what was just set reads back only after the update lands. */
  const read = (delay: number, text: string) => {
    clearTimeout(readTimer);
    if (text.trim() === "") {
      setReading({ kind: "empty" });
      return;
    }
    setReading({ kind: "reading" });
    const current = ++reads;
    readTimer = setTimeout(async () => {
      try {
        const servers = await store.parseMcpJSON(text);
        if (closed || current !== reads) return;
        setReading({ kind: "read", servers });
        // One named server: its name fills an empty Name.
        const only = servers.length === 1 ? servers[0] : undefined;
        if (only?.name && name().trim() === "") setName(only.name);
      } catch (error) {
        if (closed || current !== reads) return;
        setReading({ kind: "failed", message: errorText(error) });
      }
    }, delay);
  };

  const showJSON = (pasted?: string) => {
    const text = pasted ?? entryJSON(entryOf(form(), base()));
    setJSON(text);
    setMode("json");
    read(0, text);
  };

  /** Back to the form: the JSON's one server fills it. */
  const showForm = async () => {
    const text = json();
    if (text.trim() === "") {
      setMode("form");
      return;
    }
    try {
      const servers = await store.parseMcpJSON(text);
      if (servers.length !== 1) {
        setStatus({ text: L("The form holds one server, and that JSON has %d.", servers.length), color: "var(--orange)" });
        return;
      }
      const [server] = servers;
      if (!server!.entry) {
        setStatus({ text: server!.problem ?? L("That JSON is not a server."), color: "var(--red)" });
        return;
      }
      setForm(formOf(server!.entry));
      if (server!.name && name().trim() === "") setName(server!.name);
      setStatus(null);
      setMode("form");
    } catch (error) {
      setStatus({ text: errorText(error), color: "var(--red)" });
    }
  };

  /** JSON pasted into a field of the form opens the JSON view with it. */
  const pasteJSON = (event: ClipboardEvent) => {
    const text = event.clipboardData?.getData("text/plain") ?? "";
    if (!looksLikeJSON(text)) return;
    event.preventDefault();
    showJSON(text);
  };

  // MARK: The server

  let loads = 0;
  /** The server as the Runner has it now: its state and its tools. */
  const reload = async () => {
    const current = saved();
    if (!current) return;
    const load = ++loads;
    try {
      const fresh = await store.mcpServer(current.name, runner.id);
      if (!closed && load === loads) setSaved(fresh);
    } catch {
      // The list says when the Runner cannot be reached; the sheet keeps what it has.
    }
  };

  onSettled(() =>
    // The Runner's state moved: a sign-in finished, a connection came up or failed.
    onStoreEvent((event) => {
      if ((event.kind === "rosterChanged" || event.kind === "snapshotReplaced") && !connecting()) void reload();
    }),
  );

  /** Connects the server and waits for how it went: from scratch for Reconnect, or taking the
   * connection a save started. A save names the server it just saved, which `saved` reads back
   * only once the update lands. */
  const connect = async (fresh: boolean, server = saved()) => {
    const current = server;
    if (!current) return;
    setConnecting(true);
    loads += 1;
    try {
      const answered = await store.reconnectMcpServer(runner.id, current.name, fresh);
      if (!closed) setSaved(answered);
    } catch (error) {
      if (!closed) setStatus({ text: errorText(error), color: "var(--red)" });
    } finally {
      if (!closed) setConnecting(false);
    }
  };

  const signIn = async () => {
    const current = saved();
    if (!current) return;
    try {
      // The Runner notes the sign-in on the plugin, so the state reads it.
      await store.connectPlugin(current.id, runner.id);
      void reload();
    } catch (error) {
      void alert({ message: L("Couldn't start the sign-in"), informative: errorText(error) });
    }
  };

  /** Offers a tool to bots or keeps it from them; the switch moves at once, and back on a failure. */
  const showTool = async (tool: string, shown: boolean) => {
    const current = saved();
    if (!current) return;
    setSaved({ ...current, tools: current.tools?.map((each) => (each.name === tool ? { ...each, hidden: !shown } : each)) });
    try {
      const answered = await store.setMcpToolHidden(runner.id, current.name, tool, !shown);
      if (!closed) setSaved(answered);
    } catch (error) {
      if (!closed) setSaved(current);
      void alert({ message: shown ? L("Couldn't offer %@ to bots", tool) : L("Couldn't hide %@", tool), informative: errorText(error) });
    }
  };

  /** Forgets the sign-in on the Runner. Nothing is revoked at the server; its next use asks again. */
  const signOut = async () => {
    const current = saved();
    if (!current) return;
    try {
      const answered = await store.signOutMcpServer(runner.id, current.name);
      if (!closed) setSaved(answered);
    } catch (error) {
      void alert({ message: L("Couldn't sign out of %@", current.name), informative: errorText(error) });
    }
  };

  const setEnabled = async (on: boolean) => {
    const current = saved();
    if (!current) return;
    setSaved({ ...current, enabled: on });
    try {
      const answered = await store.setMcpServerEnabled(runner.id, current.name, on);
      setSaved(answered);
      if (on) void connect(false, answered);
    } catch (error) {
      if (!closed) setSaved(current);
      void alert({ message: on ? L("Couldn't turn %@ on", current.name) : L("Couldn't turn %@ off", current.name), informative: errorText(error) });
    }
  };

  // MARK: Saving

  const begin = (text: string) => {
    setBusy(true);
    setStatus({ text, color: "var(--label-2)", spinning: true });
  };
  const fail = (error: unknown) => {
    setBusy(false);
    setStatus({ text: errorText(error), color: "var(--red)" });
  };

  const confirm = async () => {
    if (confirmDisabled()) return;
    if (!dirty()) {
      props.dismiss();
      return;
    }
    const servers = several();
    if (servers) {
      const usable = servers.filter((server) => server.name && server.entry && !server.problem);
      begin(L("Adding %d servers…", usable.length));
      const failed: string[] = servers.filter((server) => !usable.includes(server)).map((server) => `${server.name ?? "?"}: ${server.problem ?? L("That JSON is not a server.")}`);
      for (const server of usable) {
        try {
          await store.saveMcpServer(runner.id, server.name!, server.entry!);
        } catch (error) {
          failed.push(`${server.name}: ${errorText(error)}`);
        }
        if (closed) return;
      }
      setBusy(false);
      if (failed.length === 0) props.dismiss();
      else setStatus({ text: failed.join("\n"), color: "var(--red)" });
      return;
    }
    const next = entry();
    if (!next) return;
    const previous = saved()?.name;
    begin(previous ? L("Saving %@…", name().trim()) : L("Adding %@…", name().trim()));
    try {
      const server = await store.saveMcpServer(runner.id, name().trim(), next, previous);
      if (closed) return;
      setSaved(server);
      setName(server.name);
      setForm(formOf(server.entry));
      setMode("form");
      setBusy(false);
      setStatus(null);
      // The Runner starts it once to list its tools; the sheet waits on that connection.
      if (server.enabled) void connect(false, server);
    } catch (error) {
      if (!closed) fail(error);
    }
  };

  const confirmRemove = async () => {
    const current = saved();
    if (!current) return;
    const answer = await alert({
      message: L("Remove %@ from %@?", current.name, runner.name),
      informative: L("Its entry leaves mcp.json on %@, every bot there loses its tools, and its sign-in is forgotten.", runner.name),
      style: "warning",
      buttons: [{ title: L("Remove") }, { title: L("Cancel") }],
    });
    if (answer !== 0) return;
    begin(L("Removing…"));
    try {
      await store.removeMcpServer(runner.id, current.name);
      if (!closed) props.dismiss();
    } catch (error) {
      if (!closed) fail(error);
    }
  };

  const update = (patch: Partial<McpForm>) => setForm({ ...form(), ...patch });
  const remote = () => form().remote;
  const runnerNote = () =>
    runner.isThisDevice
      ? L("Starts on this computer from your login shell, so npx, uvx, and docker resolve as they do in a terminal.")
      : L("Starts on %@ from its login shell, so npx, uvx, and docker resolve as they do in a terminal there.", runner.name);

  return (
    <Sheet
      title={saved()?.name ?? L("Add MCP Server")}
      subtitle={L("A command %@ runs, or a remote server's URL, speaking the Model Context Protocol. Every bot on %@ can use its tools.", runner.name, runner.name)}
      width={560}
      confirm={confirmTitle()}
      confirmDisabled={confirmDisabled()}
      onConfirm={() => void confirm()}
      onCancel={props.dismiss}
      leading={
        saved() ? (
          <Button kind="destructive" disabled={busy()} onClick={() => void confirmRemove()}>
            {L("Remove…")}
          </Button>
        ) : undefined
      }
    >
      <Show when={saved()}>{(server) => <ServerStatus server={server()} runner={runner} connecting={connecting()} busy={busy()} onReconnect={() => void connect(true)} onSignIn={() => void signIn()} onSignOut={() => void signOut()} onEnable={(on) => void setEnabled(on)} onShowTool={(tool, shown) => void showTool(tool, shown)} />}</Show>
      <div class="mcp-mode">
        <Segmented
          label={L("Edit as")}
          options={[
            { value: "form" as const, label: L("Form") },
            { value: "json" as const, label: L("JSON") },
          ]}
          value={mode()}
          onChange={(next) => (next === "json" ? showJSON() : void showForm())}
        />
      </div>
      <div class="custom-provider-form mcp-form">
        <Show when={!several()}>
          <span class="custom-form-label">{L("Name")}</span>
          <div class="form-control">
            <TextField value={name()} placeholder="github" disabled={busy()} autofocus={!saved()} label={L("Name")} onInput={setName} />
          </div>
          <span />
          <div class="field-note">{L("Bots call its tools as %@__tool.", (name().trim() || "name").toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "") || "name")}</div>
        </Show>
        <Show
          when={mode() === "form"}
          fallback={
            <>
              <span class="custom-form-label">{L("JSON")}</span>
              <div class="form-control">
                <TextArea
                  value={json()}
                  rows={11}
                  monospaced
                  disabled={busy()}
                  class="mcp-json"
                  label={L("The server's JSON")}
                  placeholder={'{\n  "command": "npx",\n  "args": ["-y", "@modelcontextprotocol/server-memory"]\n}'}
                  onInput={(value) => {
                    setJSON(value);
                    read(300, value);
                  }}
                />
              </div>
              <span />
              <div class={["field-note", { error: reading().kind === "failed" }]}>
                <JSONNote reading={reading()} several={several()} />
              </div>
            </>
          }
        >
          <span class="custom-form-label">{L("Type")}</span>
          <div class="form-control">
            <Segmented
              label={L("Type")}
              options={[
                { value: false, label: L("Command") },
                { value: true, label: L("URL") },
              ]}
              value={remote()}
              onChange={(next) => update({ remote: next })}
            />
          </div>
          <Show
            when={remote()}
            fallback={
              <>
                <span class="custom-form-label">{L("Command")}</span>
                <div class="form-control" onPaste={pasteJSON}>
                  <TextField
                    value={form().command}
                    placeholder="npx -y @modelcontextprotocol/server-filesystem ~/Documents"
                    monospaced
                    disabled={busy()}
                    label={L("Command")}
                    onInput={(command) => update({ command })}
                  />
                </div>
                <span />
                <div class="field-note">{runnerNote()}</div>
                <span class="custom-form-label pairs-label">{L("Environment")}</span>
                <div class="form-control">
                  <PairsEditor
                    pairs={form().env}
                    namePlaceholder="API_KEY"
                    valuePlaceholder={L("value")}
                    addTitle={L("Add Variable")}
                    label={L("Environment")}
                    disabled={busy()}
                    onChange={(env) => update({ env })}
                  />
                </div>
              </>
            }
          >
            <span class="custom-form-label">{L("URL")}</span>
            <div class="form-control" onPaste={pasteJSON}>
              <TextField value={form().url} placeholder="https://mcp.example.com/mcp" monospaced disabled={busy()} label={L("URL")} onInput={(url) => update({ url })} />
            </div>
            <span />
            <div class="field-note">{L("Streamable HTTP. When the server asks for a sign-in, Lorca signs in with OAuth; or send a token in a header.")}</div>
            <span class="custom-form-label pairs-label">{L("Headers")}</span>
            <div class="form-control">
              <PairsEditor
                pairs={form().headers}
                namePlaceholder="Authorization"
                valuePlaceholder="Bearer …"
                addTitle={L("Add Header")}
                label={L("Headers")}
                disabled={busy()}
                onChange={(headers) => update({ headers })}
              />
            </div>
          </Show>
          <span class="custom-form-label">{L("About")}</span>
          <div class="form-control">
            <TextField
              value={form().description}
              placeholder={L("What it is for, which bots read (optional)")}
              disabled={busy()}
              label={L("About")}
              spellcheck
              onInput={(description) => update({ description })}
            />
          </div>
        </Show>
      </div>
      <Show when={mode() === "form" && !saved()}>
        <div class="sheet-note">{L("Paste the JSON a server's README gives, or a whole mcpServers block from Claude Desktop or Cursor, into any field.")}</div>
      </Show>
      <Show when={status()}>
        {(current) => (
          <div class="status-line">
            <Show when={current().spinning}>
              <Spinner size={14} />
            </Show>
            <span class="wrap-lines" style={{ color: current().color }}>
              {current().text}
            </span>
          </div>
        )}
      </Show>
    </Sheet>
  );
}

/** Under the JSON: what it holds, or why it cannot be read. */
function JSONNote(props: { reading: Reading; several?: ParsedServer[] }) {
  const text = () => {
    const state = props.reading;
    switch (state.kind) {
      case "empty":
        return L("One server's JSON, or the mcpServers block of Claude Desktop, Cursor, or a README.");
      case "reading":
        return L("Reading…");
      case "failed":
        return state.message;
      case "read": {
        if (props.several) return L("%d servers: %@", props.several.length, props.several.map((server) => server.name ?? "?").join(", "));
        const server = state.servers[0];
        if (!server) return "";
        if (server.problem) return server.problem;
        const address = server.entry ? mcpAddress(server.entry) : "";
        return server.entry && isRemote(server.entry) ? L("A remote server at %@", address) : L("A command: %@", address);
      }
    }
  };
  return <>{text()}</>;
}

/** How a saved server stands, with what to do about it, whether it is on, and its tools. */
function ServerStatus(props: {
  server: McpServer;
  runner: Device;
  connecting: boolean;
  busy: boolean;
  onReconnect: () => void;
  onSignIn: () => void;
  onSignOut: () => void;
  onEnable: (on: boolean) => void;
  onShowTool: (tool: string, shown: boolean) => void;
}) {
  const state = () => (props.connecting ? { text: L("Connecting…"), color: "var(--accent)" } : mcpState(props.server));
  const needsSignIn = () => !props.connecting && props.server.status?.state === "needs_auth";
  return (
    <>
      <Section title={L("Status")}>
        <div class="row mcp-status-row" data-label={L("State")}>
          <span class="row-key">{L("State")}</span>
          <span class="mcp-status-value">
            <Show when={props.connecting} fallback={<span class="mcp-status-dot" style={{ background: state().color }} />}>
              <Spinner size={12} />
            </Show>
            <span class="row-value selectable" style={{ color: state().color }}>
              {state().text}
            </span>
          </span>
          <Show when={props.server.enabled && !props.server.problem}>
            <LinkButton disabled={props.connecting || props.busy} onClick={() => (needsSignIn() ? props.onSignIn() : props.onReconnect())}>
              {needsSignIn() ? L("Sign in") : L("Reconnect")}
            </LinkButton>
          </Show>
        </div>
        <Show when={props.server.signsIn && props.server.signedIn && !needsSignIn()}>
          <div class="row action-row">
            <span class="row-key">{L("Account")}</span>
            <span class="row-value truncate" style={{ color: "var(--green)" }}>
              {L("Signed in")}
            </span>
            <LinkButton disabled={props.busy} onClick={props.onSignIn}>
              {L("Sign in again")}
            </LinkButton>
            <LinkButton disabled={props.busy} onClick={props.onSignOut}>
              {L("Sign Out")}
            </LinkButton>
          </div>
        </Show>
        <Show when={!props.server.problem}>
          <AccessoryRow label={L("On")} tooltip={L("Off, no bot on %@ sees it and it never starts.", props.runner.name)}>
            <Switch small checked={props.server.enabled} disabled={props.busy} label={L("On")} onChange={props.onEnable} />
          </AccessoryRow>
        </Show>
      </Section>
      <Show when={(props.server.tools?.length ?? 0) > 0}>
        <Section title={L("Tools")}>
          <div class="mcp-tools">
            <For each={props.server.tools ?? []}>
              {(tool) => (
                <div class={["mcp-tool", { hidden: !!tool.hidden }]} title={tool.description}>
                  <span class="mcp-tool-name mono">{tool.name}</span>
                  <span class="mcp-tool-about truncate">{tool.description}</span>
                  <Show when={tool.readOnly}>
                    <span class="mcp-tool-tag" title={L("It only reads, so it runs without Auto-review.")}>
                      {L("Reads only")}
                    </span>
                  </Show>
                  <Switch
                    small
                    checked={!tool.hidden}
                    disabled={props.busy}
                    label={L("Offer %@ to bots", tool.name)}
                    tooltip={tool.hidden ? L("Hidden from bots") : L("Offered to bots")}
                    onChange={(on) => props.onShowTool(tool.name, on)}
                  />
                </div>
              )}
            </For>
          </div>
        </Section>
      </Show>
    </>
  );
}
