import Foundation

/// Exact review payloads come from the local CLI. Dictionary keys inside tool arguments
/// retain their spelling; only the surrounding wire model uses snake-case conversion.
enum ReviewJSON: Codable, Equatable {
    case null, bool(Bool), integer(Int64), unsigned(UInt64), number(Double), string(String), array([ReviewJSON]), object([String: ReviewJSON])

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let bool = try? value.decode(Bool.self) { self = .bool(bool) }
        else if let integer = try? value.decode(Int64.self) { self = .integer(integer) }
        else if let unsigned = try? value.decode(UInt64.self) { self = .unsigned(unsigned) }
        else if let number = try? value.decode(Double.self) { self = .number(number) }
        else if let string = try? value.decode(String.self) { self = .string(string) }
        else if let array = try? value.decode([ReviewJSON].self) { self = .array(array) }
        else { self = .object(try value.decode([String: ReviewJSON].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .null: try value.encodeNil()
        case let .bool(bool): try value.encode(bool)
        case let .integer(integer): try value.encode(integer)
        case let .unsigned(unsigned): try value.encode(unsigned)
        case let .number(number): try value.encode(number)
        case let .string(string): try value.encode(string)
        case let .array(array): try value.encode(array)
        case let .object(object): try value.encode(object)
        }
    }

    var object: Any {
        switch self {
        case .null: NSNull()
        case let .bool(value): value
        case let .integer(value): value
        case let .unsigned(value): value
        case let .number(value): value
        case let .string(value): value
        case let .array(value): value.map(\.object)
        case let .object(value): value.mapValues(\.object)
        }
    }

    var pretty: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return (try? encoder.encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }
}

struct ReviewItem: Decodable, Identifiable {
    struct Origin: Decodable {
        var chatId: String
        var messageId: String?
        var routineId: String?
        var taskId: String?
    }
    struct Target: Decodable { var account: String; var resource: String }
    struct Payload: Decodable {
        var kind: String
        var text: String?
        var pluginId: String?
        var serverName: String?
        var tool: String?
        var arguments: ReviewJSON?

        var editorText: String { kind == "draft" ? text ?? "" : arguments?.pretty ?? "{}" }

        func parameters(editedText: String) throws -> [String: Any] {
            if kind == "draft" { return ["kind": kind, "text": editedText] }
            let arguments = try JSONSerialization.jsonObject(with: Data(editedText.utf8))
            guard arguments is [String: Any] else { throw ReviewEditError.argumentsObject }
            if kind == "shell" { return ["kind": kind, "arguments": arguments] }
            return ["kind": kind, "plugin_id": pluginId ?? "", "server_name": serverName ?? "", "tool": tool ?? "", "arguments": arguments]
        }
    }
    struct Outcome: Decodable { var summary: String; var result: ReviewJSON?; var messageId: String }
    struct Preconditions: Decodable {
        struct File: Decodable { var path: String; var hash: String? }
        var workdir: String
        var files: [File]
    }

    var id: String
    var runnerId: String
    var botId: String
    var origin: Origin
    var target: Target
    var rationale: String
    var payload: Payload
    var version: UInt64
    var revision: UInt64
    var preconditions: Preconditions
    var state: String
    var outcome: Outcome?

    var isEditable: Bool { state == "pending" || state == "approved" }
    var stateText: String {
        switch state {
        case "pending": L("Needs review")
        case "approved": L("Approved")
        case "executing": L("Executing")
        case "succeeded": L("Completed")
        case "failed": L("Failed")
        case "rejected": L("Rejected")
        case "cancelled": L("Cancelled")
        case "uncertain": L("Check the outcome")
        default: state
        }
    }
}

enum ReviewEditError: LocalizedError {
    case argumentsObject
    var errorDescription: String? { L("The proposed call needs a JSON object of arguments.") }
}
