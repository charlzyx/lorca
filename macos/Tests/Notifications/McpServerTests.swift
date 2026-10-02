import XCTest
@testable import Lorca

final class McpServerTests: XCTestCase {
    func testACommandLineSplitsTheWayWindowsAndAShellBothReadIt() {
        XCTAssertEqual(McpCommandLine.split("npx -y @modelcontextprotocol/server-filesystem ~/Documents"), ["npx", "-y", "@modelcontextprotocol/server-filesystem", "~/Documents"])
        XCTAssertEqual(McpCommandLine.split(#""C:\Program Files\nodejs\node.exe" server.js"#), [#"C:\Program Files\nodejs\node.exe"#, "server.js"])
        // A backslash is a backslash, unless a run of them comes before a quote.
        XCTAssertEqual(McpCommandLine.split(#"C:\Users\me\mcp.exe \\server\share"#), [#"C:\Users\me\mcp.exe"#, #"\\server\share"#])
        XCTAssertEqual(McpCommandLine.split(#""say \"hi\"" 'single quoted'"#), [#"say "hi""#, "single quoted"])
        XCTAssertEqual(McpCommandLine.split(#""C:\dir\\" next"#), [#"C:\dir\"#, "next"])
        XCTAssertEqual(McpCommandLine.split(#"a "" b"#), ["a", "", "b"])
        XCTAssertEqual(McpCommandLine.split(""), [])
    }

    func testWordsJoinedIntoALineReadBackAsTheSameWords() {
        let cases = [
            ["npx", "-y", "@scope/pkg"],
            [#"C:\Program Files\nodejs\node.exe"#, #"C:\My Server\index.js"#],
            ["echo", #"a "quoted" word"#, "it's", ""],
            [#"trailing\"#, #"dir with space\"#, #"\\unc\path"#],
        ]
        for words in cases { XCTAssertEqual(McpCommandLine.split(McpCommandLine.join(words)), words) }
        XCTAssertEqual(McpCommandLine.join(["npx", "-y", "My Folder"]), #"npx -y "My Folder""#)
    }

    func testTheFormEditsAnEntryAndKeepsTheFieldsItDoesNotShow() {
        let entry = McpEntry(["command": "npx", "args": ["-y", "pkg", "My Folder"], "env": ["API_KEY": "secret", "DEBUG": "1"], "cwd": "~/work", "disabled": true])
        let form = McpForm(entry: entry)
        XCTAssertEqual(form.command, #"npx -y pkg "My Folder""#)
        XCTAssertEqual(form.env.map { $0.0 }, ["API_KEY", "DEBUG"])
        XCTAssertTrue(form.entry(over: entry).saysTheSame(as: entry), "untouched, it says what the entry said")

        var edited = form
        edited.command = "uvx server --port 1"
        edited.env = [("DEBUG", "2"), ("", "dropped")]
        let saved = edited.entry(over: entry)
        XCTAssertEqual(saved.command, "uvx")
        XCTAssertEqual(saved.args, ["server", "--port", "1"])
        XCTAssertEqual(saved.env.map { $0.0 }, ["DEBUG"])
        XCTAssertEqual(saved.fields["cwd"] as? String, "~/work")
        XCTAssertTrue(saved.isDisabled)

        var remote = form
        remote.isRemote = true
        remote.url = " https://mcp.example.com/mcp "
        let http = remote.entry(over: entry)
        XCTAssertEqual(http.type, "http")
        XCTAssertEqual(http.url, "https://mcp.example.com/mcp")
        XCTAssertNil(http.command)
        XCTAssertTrue(http.isRemote)

        // No command yet stays an empty field through the JSON view and back, not `""`.
        XCTAssertEqual(McpForm(entry: McpForm(entry: McpEntry()).entry()).command, "")
        XCTAssertEqual(McpForm(entry: McpEntry(["command": "", "args": ["--flag"]])).command, "\"\" --flag")
    }

    func testWhatStopsASave() {
        var form = McpForm(entry: McpEntry(["command": "npx"]))
        XCTAssertEqual(form.problem(name: ""), "Give the server a name.")
        XCTAssertNil(form.problem(name: "fs"))
        form.command = "   "
        XCTAssertEqual(form.problem(name: "fs"), "Give the command to run.")
        form.isRemote = true
        form.url = "mcp.example.com"
        XCTAssertEqual(form.problem(name: "api"), "The URL starts with http:// or https://.")
    }

    func testTheJSONViewWritesTheKeysInLorcasOrder() {
        let entry = McpEntry(["args": ["-y"], "command": "npx", "trust": true, "env": ["A": "1"]])
        XCTAssertEqual(entry.json, "{\n  \"command\": \"npx\",\n  \"args\": [\n    \"-y\"\n  ],\n  \"env\": {\n    \"A\": \"1\"\n  },\n  \"trust\": true\n}")
        XCTAssertTrue(looksSecret("GITHUB_PERSONAL_ACCESS_TOKEN") && looksSecret("Authorization") && !looksSecret("DEBUG"))
    }

    func testAServerReadsFromTheCLIsJSON() {
        let config: [String: Any] = ["command": "npx", "env": ["GITHUB_TOKEN": "x"]]
        let status: [String: Any] = ["id": "fs", "name": "fs", "state": "ready", "detail": "Ready", "source": "mcp.json"]
        let tool: [String: Any] = ["name": "read_file", "description": "Read a file.", "read_only": true]
        let json: [String: Any] = ["name": "fs", "id": "fs", "enabled": true, "config": config, "status": status, "tool_count": 14, "tools": [tool]]
        let server = McpServer(json: json)
        XCTAssertEqual(server?.entry.env.map { $0.0 }, ["GITHUB_TOKEN"], "a variable's name stays as the file has it")
        XCTAssertEqual(server?.state.text, "14 tools")
        XCTAssertEqual(server?.status?.isMcpServer, true)
        XCTAssertEqual(server?.tools?.first?.isReadOnly, true)
    }
}
