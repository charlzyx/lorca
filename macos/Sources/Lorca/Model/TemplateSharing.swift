import Foundation

/// The production sheets use the local CLI; native fixtures can supply deterministic replies.
typealias TemplateReply = @MainActor (String, [String: Any]) async throws -> [String: Any]

/// The CLI owns validation and file access. The app displays its allowlisted preview and
/// sends selection identifiers and recipient connection choices back to that same CLI.
struct TemplateContents {
    struct Item {
        let id: String
        let title: String
    }
    let profileName: String
    let skills: [Item]
    let memories: [Item]
    let routines: [Item]
    let requirements: [Item]
    let notes: [String]

    init(json: [String: Any]) {
        profileName = (json["profile"] as? [String: Any])?["name"] as? String ?? ""
        skills = Self.items(json["skills"]) { ($0 as? [String: Any])?["name"] as? String ?? "" }
        memories = Self.items(json["memories"]) { $0 as? String ?? "" }
        routines = Self.items(json["routines"]) { ($0 as? [String: Any])?["name"] as? String ?? "" }
        requirements = (json["requirements"] as? [[String: Any]] ?? []).compactMap {
            guard let id = $0["service_id"] as? String else { return nil }
            return Item(id: id, title: id)
        }
        notes = json["notes"] as? [String] ?? []
    }

    private static func items(_ value: Any?, title: (Any) -> String) -> [Item] {
        (value as? [[String: Any]] ?? []).compactMap {
            guard let id = $0["id"] as? String, let content = $0["content"] else { return nil }
            return Item(id: id, title: title(content))
        }
    }
}

struct TemplatePreview {
    struct Connection {
        let id: String
        let name: String
        let state: String
        let detail: String
    }
    struct Requirement {
        let serviceID: String
        let candidates: [Connection]
    }
    let digest: String
    let canImport: Bool
    let issues: [String]
    let requirements: [Requirement]
    let name: String
    let text: String

    init(json: [String: Any]) {
        digest = json["digest"] as? String ?? ""
        canImport = json["can_import"] as? Bool ?? false
        issues = json["issues"] as? [String] ?? []
        requirements = (json["requirements"] as? [[String: Any]] ?? []).compactMap { requirement in
            guard let serviceID = requirement["service_id"] as? String else { return nil }
            let candidates = (requirement["candidates"] as? [[String: Any]] ?? []).compactMap { candidate -> Connection? in
                guard let id = candidate["id"] as? String else { return nil }
                return Connection(id: id, name: candidate["name"] as? String ?? id,
                    state: candidate["state"] as? String ?? "", detail: candidate["detail"] as? String ?? "")
            }
            return Requirement(serviceID: serviceID, candidates: candidates)
        }
        let template = json["template"] as? [String: Any] ?? [:]
        let profile = template["profile"] as? [String: Any]
        name = profile?["name"] as? String ?? ""
        var sections: [String] = []
        if let profile {
            sections.append(L("Profile") + "\n" + (profile["name"] as? String ?? "")
                + "\n" + (profile["description"] as? String ?? "")
                + "\n" + L("Look: %@ · %@", profile["symbol_name"] as? String ?? "", profile["accent"] as? String ?? ""))
        }
        for skill in template["skills"] as? [[String: Any]] ?? [] {
            var lines = [L("Skill: %@", skill["name"] as? String ?? ""), skill["description"] as? String ?? "", skill["instructions"] as? String ?? ""]
            if let examples = skill["examples"] as? String, !examples.isEmpty { lines += [L("Examples"), examples] }
            for kind in ["references", "scripts"] {
                for resource in skill[kind] as? [[String: Any]] ?? [] {
                    lines += [resource["path"] as? String ?? "", resource["text"] as? String ?? ""]
                }
            }
            sections.append(lines.joined(separator: "\n"))
        }
        for memory in template["memories"] as? [String] ?? [] { sections.append(L("Memory") + "\n" + memory) }
        for routine in template["routines"] as? [[String: Any]] ?? [] {
            var lines = [L("Routine: %@", routine["name"] as? String ?? ""), routine["schedule"] as? String ?? "", routine["prompt"] as? String ?? ""]
            if let timezone = routine["timezone"] as? String { lines.append(L("Time zone: %@", timezone)) }
            if let policy = routine["missed_run_policy"] as? String { lines.append(L("Missed runs: %@", policy)) }
            if let check = routine["check"] as? String, !check.isEmpty { lines += [L("Check script"), check] }
            lines.append(L("Imported paused"))
            sections.append(lines.joined(separator: "\n"))
        }
        let services = (template["requirements"] as? [[String: Any]] ?? []).compactMap { $0["service_id"] as? String }
        if !services.isEmpty { sections.append(L("Integration requirements") + "\n" + services.joined(separator: "\n")) }
        let warnings = (json["warnings"] as? [[String: Any]] ?? []).compactMap { $0["message"] as? String }
        if !warnings.isEmpty { sections.insert(L("Review before sharing") + "\n" + warnings.map { "• " + $0 }.joined(separator: "\n"), at: 0) }
        text = sections.joined(separator: "\n\n")
    }
}

extension AppStore {
    func templateReply(_ method: String, _ params: [String: Any]) async throws -> [String: Any] {
        guard !isMock else { throw CLIClient.RequestError(message: L("Templates require a running Lorca CLI.")) }
        let data = try await client.request(method, params)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw CLIClient.RequestError(message: L("Couldn't read the template response."))
        }
        return json
    }
}
