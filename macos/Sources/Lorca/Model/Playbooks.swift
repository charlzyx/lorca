import Foundation

struct PlaybookScope: Codable, Equatable {
    let kind: String
    let id: String
    var params: [String: Any] { ["kind": kind, "id": id] }
}

struct PlaybookResource: Codable {
    var path: String
    var text: String
}

struct PlaybookContent: Codable {
    var name: String = ""
    var description: String = ""
    var instructions: String = ""
    var examples: String = ""
    var references: [PlaybookResource] = []
    var scripts: [PlaybookResource] = []

    func params() throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(self)) as! [String: Any]
    }
}

struct PlaybookProvenance: Codable {
    var kind: String
    var chat_id: String?
    var message_ids: [String]
    var note: String
}

struct PlaybookSummary: Decodable {
    let id: String
    let scope: PlaybookScope
    let name: String
    let description: String
    let path: String
    let revision: Int
    let hash: String
    let status: String
}

struct PlaybookRevision: Decodable {
    let revision: Int
    let status: String
    let content: PlaybookContent?
    let provenance: PlaybookProvenance
    let created_at: Double
}

struct PlaybookRecord: Decodable {
    let id: String
    let scope: PlaybookScope
    let revision: Int
    let hash: String
    let status: String
    let content: PlaybookContent?
    let provenance: PlaybookProvenance
    let revisions: [PlaybookRevision]
}

extension AppStore {
    func playbooks(in scope: PlaybookScope) async throws -> [PlaybookSummary] {
        struct Listing: Decodable { let items: [PlaybookSummary] }
        return try await client.request("playbooks.list", ["scope": scope.params, "include_drafts": true], as: Listing.self).items
    }

    func playbook(_ id: String, in scope: PlaybookScope) async throws -> PlaybookRecord {
        try await client.request("playbooks.get", ["scope": scope.params, "id": id], as: PlaybookRecord.self)
    }

    func savePlaybook(_ content: PlaybookContent, in scope: PlaybookScope, previous: PlaybookRecord?) async throws -> PlaybookRecord {
        var params: [String: Any] = ["scope": scope.params, "content": try content.params(),
                                   "expected_revision": previous?.revision ?? 0, "expected_hash": previous?.hash ?? ""]
        if let previous {
            params["id"] = previous.id
            var provenance = previous.provenance
            provenance.kind = "reviewed_edit"
            provenance.note = L("Saved after user review")
            params["provenance"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(provenance))
        }
        return try await client.request("playbooks.save", params, as: PlaybookRecord.self)
    }

    func removePlaybook(_ record: PlaybookRecord) async throws {
        _ = try await client.request("playbooks.remove", ["scope": record.scope.params, "id": record.id,
                                                         "expected_revision": record.revision, "expected_hash": record.hash])
    }

    func capturePlaybook(scope: PlaybookScope, botID: Bot.ID, chatID: Chat.ID, kind: String, messageIDs: [Message.ID]) async throws -> PlaybookRecord {
        try await client.request("playbooks.draft", ["scope": scope.params, "bot_id": botID, "chat_id": chatID,
                                                    "kind": kind, "message_ids": messageIDs], as: PlaybookRecord.self)
    }
}
