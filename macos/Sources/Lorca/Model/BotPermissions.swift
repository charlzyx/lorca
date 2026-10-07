import Foundation

/// The user's policy, carried by the CLI in the encrypted bot roster.
struct BotPermissions: Decodable, Hashable {
    struct Connection: Decodable, Hashable {
        var capabilities: Set<String> = []
        var tools: Set<String>? = nil

        var json: [String: Any] {
            var result: [String: Any] = ["capabilities": capabilities.sorted()]
            if let tools { result["tools"] = tools.sorted() }
            return result
        }
    }

    var connections: [String: Connection]? = nil
    var tools: Set<String>? = nil
    var filesystem: String = "write"
    var shell: Bool = true

    init() {}

    enum CodingKeys: CodingKey { case connections, tools, filesystem, shell }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        connections = try values.decodeIfPresent([String: Connection].self, forKey: .connections)
        tools = try values.decodeIfPresent(Set<String>.self, forKey: .tools)
        filesystem = try values.decodeIfPresent(String.self, forKey: .filesystem) ?? "write"
        shell = try values.decodeIfPresent(Bool.self, forKey: .shell) ?? true
    }

    var json: [String: Any] {
        var result: [String: Any] = ["filesystem": filesystem, "shell": shell]
        if let tools { result["tools"] = tools.sorted() }
        if let connections { result["connections"] = connections.mapValues(\.json) }
        return result
    }

    var summary: String {
        let accounts = connections.map { L("%d connections", $0.values.filter { !$0.capabilities.isEmpty }.count) } ?? L("All connections")
        let localTools = tools.map { L("%d local tools", $0.count) } ?? L("All local tools")
        return [accounts, localTools, shell ? L("Shell allowed") : L("Shell denied")].joined(separator: " · ")
    }
}

struct BotPermissionCatalog: Decodable {
    struct Connection: Decodable {
        struct Tool: Decodable {
            var name: String
            var description: String?
            var capability: String?
            var hidden: Bool?
        }
        var id: String
        var name: String
        var tools: [Tool]
    }
    var localTools: [String]
    var connections: [Connection]
}
