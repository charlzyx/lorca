import Foundation

@main
struct ProjectContextTests {
    static func main() throws {
        let data = Data(#"""
        {"revision":"hash","has_more":true,"conflicts":{"old":["a","b"]},"entries":[{
          "id":"ctx-1","kind":"fact","title":"Reference","text":"snapshot",
          "source":{"kind":"output","label":"Verified build","message_id":"message-1",
            "output":{"chat_id":"project-a","message_id":"version-message","output_id":"output-1","version":2,"task_id":"task-1"}},
          "verification":"unavailable","freshness":"unavailable","current":true,
          "updated_at":100,"verified_at":90,"fetched_at":80,"max_age_secs":3600,
          "refresh_error":"source offline","asset":{"name":"reference.pdf"}
        }]}
        """#.utf8)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let page = try decoder.decode(ProjectContextWire.Page.self, from: data)
        precondition(page.hasMore && page.conflicts["old"]?.count == 2)
        let entry = page.entries[0]
        precondition(entry.updatedAt == 100 && entry.verifiedAt == 90 && entry.fetchedAt == 80)
        precondition(entry.maxAgeSecs == 3600 && entry.refreshError == "source offline")
        precondition(entry.source.output?.messageId == "version-message" && entry.source.output?.taskId == "task-1")
        let encoded = try entry.source.parameters()
        precondition(encoded["message_id"] as? String == "message-1")
        let output = encoded["output"] as! [String: Any]
        precondition(output["chat_id"] as? String == "project-a")
        precondition(output["message_id"] as? String == "version-message")
        precondition(output["task_id"] as? String == "task-1")
        let reread = try decoder.decode(ProjectContextWire.Source.self, from: JSONSerialization.data(withJSONObject: encoded))
        precondition(reread.output?.version == 2)
        print("Project context decoding and provenance round-trip passed")
    }
}
