import AppKit

/// One server's entry in a Runner's mcp.json, canonical as the CLI writes it: `command`, `args`,
/// `env`, and `cwd` for a command; `type`, `url`, and `headers` for a remote server; and any field
/// Lorca does not know, as the file has it. The fields come through `JSONSerialization`, never the
/// snake-case decoder, which would read the variable `GITHUB_TOKEN` as `GITHUBToken`.
struct McpEntry {
    var fields: [String: Any]

    init(_ fields: [String: Any] = [:]) {
        self.fields = fields
    }

    var type: String? { fields["type"] as? String }
    var command: String? { fields["command"] as? String }
    var args: [String] { (fields["args"] as? [Any])?.map(Self.text) ?? [] }
    var env: [(String, String)] { Self.pairs(fields["env"]) }
    var url: String? { fields["url"] as? String }
    var headers: [(String, String)] { Self.pairs(fields["headers"]) }
    /// What it is for, which bots read beside its tools.
    var about: String { fields["description"] as? String ?? "" }
    var isDisabled: Bool { fields["disabled"] as? Bool == true }

    var isRemote: Bool { url != nil && type != "stdio" }
    var symbolName: String { isRemote ? "globe" : "terminal" }

    /// The command line or the URL a row shows under the server's name.
    var address: String {
        isRemote ? url ?? "" : McpCommandLine.join([command ?? ""] + args)
    }

    private static func text(_ value: Any) -> String {
        if let string = value as? String { return string }
        if value is NSNull { return "" }
        return (value as? NSNumber)?.stringValue ?? "\(value)"
    }

    /// Names to strings, in the names' order, which is how the CLI sends them.
    private static func pairs(_ value: Any?) -> [(String, String)] {
        guard let object = value as? [String: Any] else { return [] }
        return object.keys.sorted().map { ($0, text(object[$0] ?? "")) }
    }

    /// Whether two entries say the same, whatever the order of their keys; an empty list or
    /// object is no field at all.
    func saysTheSame(as other: McpEntry) -> Bool {
        NSDictionary(dictionary: Self.normal(fields)).isEqual(to: Self.normal(other.fields))
    }

    private static func normal(_ fields: [String: Any]) -> [String: Any] {
        fields.filter { _, value in
            if let list = value as? [Any] { return !list.isEmpty }
            if let object = value as? [String: Any] { return !object.isEmpty }
            return !(value is NSNull)
        }
    }

    /// The entry as the JSON view shows it: the keys in the order Lorca writes them, then the rest.
    var json: String {
        let order = ["type", "command", "args", "env", "cwd", "url", "headers", "oauth", "description", "disabled"]
        let keys = order.filter { fields[$0] != nil } + fields.keys.filter { !order.contains($0) }.sorted()
        guard !keys.isEmpty else { return "{}" }
        let lines = keys.map { key in "  \(Self.jsonText(key, indent: "  ")): \(Self.jsonText(fields[key] ?? NSNull(), indent: "  "))" }
        return "{\n\(lines.joined(separator: ",\n"))\n}"
    }

    /// One JSON value as text, two spaces deeper for each level, as the CLI writes the file.
    private static func jsonText(_ value: Any, indent: String) -> String {
        let deeper = indent + "  "
        switch value {
        case let string as String:
            let data = (try? JSONSerialization.data(withJSONObject: string, options: [.fragmentsAllowed, .withoutEscapingSlashes])) ?? Data("\"\"".utf8)
            return String(decoding: data, as: UTF8.self)
        case let number as NSNumber:
            // A JSON boolean comes as an NSNumber too, which only its type tells apart.
            if CFGetTypeID(number) == CFBooleanGetTypeID() { return number.boolValue ? "true" : "false" }
            return number.stringValue
        case let list as [Any]:
            guard !list.isEmpty else { return "[]" }
            return "[\n" + list.map { deeper + jsonText($0, indent: deeper) }.joined(separator: ",\n") + "\n\(indent)]"
        case let object as [String: Any]:
            guard !object.isEmpty else { return "{}" }
            let lines = object.keys.sorted().map { key in "\(deeper)\(jsonText(key, indent: deeper)): \(jsonText(object[key] ?? NSNull(), indent: deeper))" }
            return "{\n" + lines.joined(separator: ",\n") + "\n\(indent)}"
        default:
            return "null"
        }
    }
}

/// A tool a server offered when it last connected.
struct McpTool {
    var name: String
    var title: String?
    /// The first line of what it does.
    var about: String
    /// Its server marked it read-only then: it runs without Auto-review.
    var isReadOnly: Bool
}

/// A server in a Runner's mcp.json, usable or not.
struct McpServer {
    /// Its key in the file: what the apps show and `lorca mcp` takes.
    var name: String
    /// The plugin id it runs as, which its tools go by (`id__tool`).
    var id: String
    var isEnabled: Bool
    var entry: McpEntry
    /// Why it cannot run: an entry the CLI cannot read, or a name another plugin's id takes.
    var problem: String?
    /// Its plugin's state while it runs and is on.
    var status: InstalledPlugin?
    /// A remote server that asked for a sign-in, or is signed in.
    var signsIn: Bool
    var isSignedIn: Bool
    /// How many tools it offered when it last connected; none before it has.
    var toolCount: Int?
    /// `mcp.get`'s: the tools themselves.
    var tools: [McpTool]?

    /// `mcp.list`'s and `mcp.get`'s server, read from the JSON object the CLI sent.
    init?(json: [String: Any]) {
        guard let name = json["name"] as? String, let id = json["id"] as? String else { return nil }
        self.name = name
        self.id = id
        isEnabled = json["enabled"] as? Bool ?? true
        entry = McpEntry(json["config"] as? [String: Any] ?? [:])
        problem = json["problem"] as? String
        status = (json["status"] as? [String: Any]).flatMap(InstalledPlugin.init(json:))
        signsIn = json["signs_in"] as? Bool ?? false
        isSignedIn = json["signed_in"] as? Bool ?? false
        toolCount = (json["tool_count"] as? NSNumber)?.intValue
        tools = (json["tools"] as? [[String: Any]])?.compactMap { tool in
            guard let name = tool["name"] as? String else { return nil }
            return McpTool(name: name, title: tool["title"] as? String, about: tool["description"] as? String ?? "", isReadOnly: tool["read_only"] as? Bool ?? false)
        }
    }

    init(name: String, id: String, isEnabled: Bool, entry: McpEntry, status: InstalledPlugin?, toolCount: Int? = nil, tools: [McpTool]? = nil, signsIn: Bool = false) {
        self.name = name
        self.id = id
        self.isEnabled = isEnabled
        self.entry = entry
        self.status = status
        self.toolCount = toolCount
        self.tools = tools
        self.signsIn = signsIn
        isSignedIn = false
    }

    /// How it stands, in a few words, and the color they take.
    var state: (text: String, color: NSColor) {
        if let problem { return (problem, .systemRed) }
        guard isEnabled else { return (L("Off"), .tertiaryLabelColor) }
        switch status?.state {
        case .ready:
            guard let toolCount else { return (L("Not connected yet"), .secondaryLabelColor) }
            return (toolCount == 1 ? L("1 tool") : L("%d tools", toolCount), .systemGreen)
        case .needsAuth:
            return (L("Needs a sign-in"), .systemOrange)
        case .connecting:
            return (status?.detail.isEmpty == false ? status?.detail ?? "" : L("Connecting…"), .controlAccentColor)
        case .error:
            return (status?.detail.isEmpty == false ? status?.detail ?? "" : L("Couldn't connect"), .systemRed)
        default:
            return (status?.detail ?? "", .secondaryLabelColor)
        }
    }

    var needsSignIn: Bool { status?.state == .needsAuth }
}

/// A Runner's mcp.json: where it is, why it cannot be read, and its servers in the file's order.
struct McpFile {
    var path: String
    var error: String?
    var servers: [McpServer]

    init(json: [String: Any]) {
        path = json["path"] as? String ?? ""
        error = json["error"] as? String
        servers = (json["servers"] as? [[String: Any]] ?? []).compactMap(McpServer.init(json:))
    }

    init(path: String, servers: [McpServer]) {
        self.path = path
        self.servers = servers
    }
}

/// One server of pasted JSON: its name when the JSON gives one, and its entry or why it cannot run.
struct ParsedServer {
    var name: String?
    var entry: McpEntry?
    var problem: String?
}

extension InstalledPlugin {
    /// A plugin status the CLI sent inside an `mcp.*` reply.
    init?(json: [String: Any]) {
        guard let id = json["id"] as? String, let name = json["name"] as? String else { return nil }
        self.init(
            id: id, name: name, description: json["description"] as? String ?? "", version: json["version"] as? String ?? "",
            icon: json["icon"] as? String ?? "", state: State(rawValue: json["state"] as? String ?? "") ?? .unknown,
            detail: json["detail"] as? String ?? "", source: json["source"] as? String)
    }
}

// MARK: - The command line

/// A command line as words, read the way Windows and a shell both read it: spaces part words,
/// double quotes group them with `\"` for a quote inside, single quotes group them as typed, and a
/// backslash is a backslash, which Windows paths need, except in a run before a quote, which
/// halves (Windows' own rule).
enum McpCommandLine {
    static func split(_ text: String) -> [String] {
        let chars = Array(text)
        var words: [String] = []
        var word = ""
        var started = false
        var quote: Character?
        var index = 0
        while index < chars.count {
            let char = chars[index]
            if quote == "'" {
                if char == "'" { quote = nil } else { word.append(char) }
                index += 1
                continue
            }
            if char == "\\" {
                var run = 0
                while index + run < chars.count, chars[index + run] == "\\" { run += 1 }
                if index + run < chars.count, chars[index + run] == "\"" {
                    // 2n backslashes and a quote are n backslashes and the quote; 2n + 1 are n and a quote mark.
                    word += String(repeating: "\\", count: run / 2)
                    if run % 2 == 1 {
                        word.append("\"")
                        index += run + 1
                    } else {
                        index += run
                    }
                } else {
                    word += String(repeating: "\\", count: run)
                    index += run
                }
                started = true
                continue
            }
            if char == "\"" {
                quote = quote == "\"" ? nil : "\""
                started = true
            } else if quote == nil, char == "'" {
                quote = "'"
                started = true
            } else if quote == nil, char.isWhitespace {
                if started { words.append(word) }
                word = ""
                started = false
            } else {
                word.append(char)
                started = true
            }
            index += 1
        }
        if started { words.append(word) }
        return words
    }

    /// Words as one line that `split` reads back as the same words: a word with a space, a quote,
    /// or nothing in it in double quotes.
    static func join(_ words: [String]) -> String {
        words.map(quoted).joined(separator: " ")
    }

    private static func quoted(_ word: String) -> String {
        if !word.isEmpty, !word.contains(where: { $0.isWhitespace || $0 == "\"" || $0 == "'" }) { return word }
        var out = "\""
        var backslashes = 0
        for char in word {
            if char == "\\" {
                backslashes += 1
                continue
            }
            if char == "\"" {
                out += String(repeating: "\\", count: backslashes * 2 + 1) + "\""
            } else {
                out += String(repeating: "\\", count: backslashes) + String(char)
            }
            backslashes = 0
        }
        return out + String(repeating: "\\", count: backslashes * 2) + "\""
    }
}

// MARK: - The sheet's form

/// A name that says its value is a key, a token, or a password, which the sheet hides.
func looksSecret(_ name: String) -> Bool {
    name.range(of: "key|token|secret|passw|auth|credential|cookie|session", options: [.regularExpression, .caseInsensitive]) != nil
}

/// An entry as the server sheet's form holds it: the command line as typed, and the environment
/// or headers as rows.
struct McpForm {
    var isRemote: Bool
    var command: String
    var env: [(String, String)]
    var url: String
    var headers: [(String, String)]
    var about: String

    init(entry: McpEntry) {
        isRemote = entry.isRemote
        command = entry.command.map { McpCommandLine.join([$0] + entry.args) } ?? ""
        env = entry.env
        url = entry.url ?? ""
        headers = entry.headers
        about = entry.about
    }

    /// The keys the form owns; the entry's others (`cwd`, `oauth`, `disabled`, and fields other
    /// apps write) stay as they are.
    private static let keys = ["type", "command", "args", "env", "environment", "url", "serverUrl", "httpUrl", "headers", "description"]

    private static func rows(_ pairs: [(String, String)]) -> [String: String] {
        var out: [String: String] = [:]
        for (name, value) in pairs {
            let name = name.trimmingCharacters(in: .whitespaces)
            if !name.isEmpty { out[name] = value }
        }
        return out
    }

    /// The entry the form describes, over `base`'s other fields.
    func entry(over base: McpEntry = McpEntry()) -> McpEntry {
        var fields = base.fields.filter { !Self.keys.contains($0.key) }
        if isRemote {
            fields["type"] = base.type == "sse" ? "sse" : "http"
            fields["url"] = url.trimmingCharacters(in: .whitespaces)
            let headers = Self.rows(headers)
            if !headers.isEmpty { fields["headers"] = headers }
        } else {
            if base.type == "stdio" { fields["type"] = "stdio" }
            let words = McpCommandLine.split(command.trimmingCharacters(in: .whitespaces))
            fields["command"] = words.first ?? ""
            if words.count > 1 { fields["args"] = Array(words.dropFirst()) }
            let env = Self.rows(env)
            if !env.isEmpty { fields["env"] = env }
        }
        let about = about.trimmingCharacters(in: .whitespacesAndNewlines)
        if !about.isEmpty { fields["description"] = about }
        return McpEntry(fields)
    }

    /// Why the form cannot be saved yet, or nil.
    func problem(name: String) -> String? {
        if name.trimmingCharacters(in: .whitespaces).isEmpty { return L("Give the server a name.") }
        if isRemote {
            let url = url.trimmingCharacters(in: .whitespaces)
            if url.isEmpty { return L("Give the server's URL.") }
            if !(url.hasPrefix("http://") || url.hasPrefix("https://") || url.hasPrefix("${")) { return L("The URL starts with http:// or https://.") }
            return nil
        }
        return McpCommandLine.split(command.trimmingCharacters(in: .whitespaces)).isEmpty ? L("Give the command to run.") : nil
    }
}
