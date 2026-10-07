import Foundation

/// The additive marketplace contract. Older CLI indexes decode with no packs.
struct WorkflowPack: Decodable, Identifiable {
    struct Question: Decodable { var id: String; var label: String; var placeholder: String }
    struct Requirement: Decodable { var serviceId: String; var name: String }
    var id: String
    var name: String
    var outcome: String
    var description: String
    var questions: [Question]
    var connections: [Requirement]
}

struct WorkflowProgress: Decodable {
    struct Setup: Decodable {
        struct Sample: Decodable {
            var jobId: String
            var chatId: String
            var botId: String
            var state: String
            var messageIds: [String]
            var error: String?
        }
        var id: String
        var runnerId: String
        var pack: WorkflowPack
        var answers: [String: String]
        var botIds: [String: String]
        var connectionIds: [String: String]
        var phase: String
        var sample: Sample?
    }
    struct Connection: Decodable {
        struct Choice: Decodable {
            var id: String
            var name: String
            var serviceId: String?
            var accountName: String?
            var state: String
            var detail: String
            var label: String { accountName.map { "\(name) · \($0)" } ?? name }
        }
        var serviceId: String
        var name: String
        var selectedId: String?
        var choices: [Choice]
        var available: Bool
        var state: String
        var detail: String
    }
    struct Specialist: Decodable {
        struct Choice: Decodable { var id: String; var name: String }
        var id: String
        var name: String
        var selectedId: String?
        var choices: [Choice]
    }
    struct Routine: Decodable {
        var id: String
        var name: String
        var scheduleText: String
        var isEnabled: Bool
    }
    var setup: Setup
    var connections: [Connection]
    var specialists: [Specialist]
    var routines: [Routine]
    var sampleMessages: [Wire.Message]
    var isRunning: Bool
    var canSample: Bool
    var canEnable: Bool
    var blockedReason: String?
}

@MainActor
extension AppStore {
    func workflow(_ method: String, _ params: [String: Any]) async throws -> WorkflowProgress {
        try await client.request("workflows.\(method)", params, as: WorkflowProgress.self)
    }
}
