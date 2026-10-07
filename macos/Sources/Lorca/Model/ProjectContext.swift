import Foundation

/// Read with Wire.decoder's snake-case conversion; writes use the matching encoder strategy.
enum ProjectContextWire {
    struct Page: Decodable {
        var revision: String
        var entries: [Entry]
        var hasMore: Bool
        var conflicts: [String: [String]]
    }
    struct Entry: Decodable {
        var id: String
        var kind: String
        var title: String
        var text: String
        var source: Source
        var verification: String
        var freshness: String
        var current: Bool
        var updatedAt: Int64
        var verifiedAt: Int64?
        var fetchedAt: Int64?
        var maxAgeSecs: Int64?
        var asset: Asset?
        var refreshError: String?
    }
    struct Source: Codable {
        var kind: String
        var label: String
        var url: String?
        var messageId: String?
        var output: OutputReference?

        func parameters() throws -> [String: Any] {
            let encoder = JSONEncoder()
            encoder.keyEncodingStrategy = .convertToSnakeCase
            let data = try encoder.encode(self)
            return try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        }
    }
    struct OutputReference: Codable {
        var chatId: String
        var messageId: String
        var outputId: String
        var version: Int
        var taskId: String?
    }
    struct Asset: Decodable { var name: String }
    struct AssetPath: Decodable { var path: String }
}
