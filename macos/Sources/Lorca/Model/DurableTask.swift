import Foundation

/// Durable work from the CLI, independent of a single running bot turn or terminal command.
struct DurableTask: Codable, Hashable, Identifiable {
    var id: String
    var revision: UInt64
    var authorityRunnerId: String
    var ownerBotId: String
    var runnerId: String
    var goal: String
    var acceptanceCriteria: [String]
    var dependencies: [String]
    var nextAction: String
    var chatIds: [String]
    var links: [Link]
    var state: State
    var reason: String?
    var result: String?
    var evidence: [Evidence]
    var activeRun: Run?
    var createdAt: Double
    var updatedAt: Double

    enum State: String, Codable, CaseIterable {
        case queued, working, blocked, awaitingReview = "awaiting_review", completed, cancelled
        var title: String {
            switch self {
            case .queued: return L("Queued")
            case .working: return L("Working")
            case .blocked: return L("Blocked")
            case .awaitingReview: return L("Awaiting review")
            case .completed: return L("Completed")
            case .cancelled: return L("Cancelled")
            }
        }
        var symbol: String {
            switch self {
            case .queued: return "clock"
            case .working: return "arrow.trianglehead.2.clockwise.rotate.90"
            case .blocked: return "exclamationmark.circle"
            case .awaitingReview: return "eye"
            case .completed: return "checkmark.circle"
            case .cancelled: return "xmark.circle"
            }
        }
        var canRun: Bool { self == .queued || self == .blocked || self == .working }
    }
    struct Link: Codable, Hashable { var label: String; var url: String }
    struct Run: Codable, Hashable {
        var id: String; var botId: String; var runnerId: String; var chatId: String; var startedAt: Double
    }
    struct Evidence: Codable, Hashable {
        var kind: String
        var label: String
        var chatId: String? = nil
        var messageId: String? = nil
        var attachmentId: String? = nil
        var url: String? = nil
        var outputId: String? = nil
        var version: UInt64? = nil
        var reviewId: String? = nil

        var params: [String: Any] {
            var result: [String: Any] = ["kind": kind, "label": label]
            for (key, value) in [("chat_id", chatId), ("message_id", messageId), ("attachment_id", attachmentId), ("url", url), ("output_id", outputId), ("review_id", reviewId)] {
                if let value { result[key] = value }
            }
            if let version { result["version"] = version }
            return result
        }
    }
}
