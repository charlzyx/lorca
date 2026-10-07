import AppKit

// These substitutes provide only fixture data and localization. All views, controls, layout,
// actions, and wire decoding below use the production sources passed to swiftc by capture.sh.
enum Chat { typealias ID = String }
func L(_ key: String) -> String { key }
func L(_ key: String, _ args: CVarArg...) -> String {
    String(format: key, locale: Locale(identifier: "en_US"), arguments: args)
}

final class AppStore {
    static let shared = AppStore()
    let client = FixtureClient()
}

final class FixtureClient {
    private let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    func request(_ method: String, _ params: [String: Any] = [:]) async throws -> Data {
        guard method == "projects.get", params["chat_id"] as? String == "project-fixture" else {
            throw NSError(domain: "FixtureOnly", code: 1, userInfo: [NSLocalizedDescriptionKey: "The capture fixture only reads synthetic project context."])
        }
        let historical = params["history"] as? Bool == true
        let entries = Fixtures.entries.filter { historical || ($0["current"] as? Bool == true) }
        return try JSONSerialization.data(withJSONObject: ["revision": "fixture-revision", "entries": entries, "has_more": false, "conflicts": [:]])
    }

    func request<T: Decodable>(_ method: String, _ params: [String: Any] = [:], as type: T.Type) async throws -> T {
        try decoder.decode(type, from: await request(method, params))
    }
}

enum Fixtures {
    static func entry(_ id: String, kind: String, title: String, text: String, state: String, current: Bool = true, at: Int64 = 1_791_447_300, extra: [String: Any] = [:]) -> [String: Any] {
        var record: [String: Any] = ["id": id, "kind": kind, "title": title, "text": text,
            "source": ["kind": "user", "label": "Project owner · fixture"], "verification": state,
            "freshness": state, "current": current, "updated_at": at, "removed": false]
        if state == "agreed" { record["verified_at"] = at }
        record.merge(extra) { _, new in new }
        return record
    }

    static let entries: [[String: Any]] = [
        entry("ctx-fixture-decision", kind: "decision", title: "Pilot release decision", text: "Release the Harbor pilot to the internal team on 16 October.\n\n• Keep the scope to project briefs and reference assets.\n• Require a successful smoke test before announcing availability.\n• Treat fetched snapshots as evidence until the project owner verifies them.", state: "agreed"),
        entry("ctx-fixture-source", kind: "fact", title: "Release status reference", text: "Last retrieved release status:\n\nBuild: pilot-2026.10\nReadiness: awaiting owner review\nVerification: pending smoke test\n\nThis earlier snapshot remains available while its live source is unreachable.", state: "unavailable", extra: [
            "source": ["kind": "url", "label": "Release status page · fixture", "url": "https://example.com/harbor/release-status"],
            "fetched_at": Int64(1_791_356_400), "max_age_secs": 86_400,
            "refresh_error": "Source unavailable (fixture): simulated timeout. Previous snapshot retained."]),
        entry("ctx-fixture-asset", kind: "asset", title: "Launch checklist", text: "Use this reference checklist when verifying the pilot build.\n\nReference files belong to this project and stay available independently of transcript compaction.", state: "agreed", extra: [
            "asset": ["id": "att-fixture-checklist", "name": "launch-checklist.pdf", "mime": "application/pdf", "size": 28_416], "asset_available": true]),
        entry("ctx-fixture-history", kind: "decision", title: "Initial rollout plan", text: "Initial plan: release the Harbor pilot on 9 October.\n\nSuperseded after the project owner moved the release to 16 October to allow time for smoke-test verification. The original decision and its source remain in revision history.", state: "agreed", current: false, at: 1_791_356_400)
    ]
}

@main
struct Capture {
    @MainActor
    static func main() throws {
        guard CommandLine.arguments.count == 2 else { fatalError("Pass the evidence output directory") }
        let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.prohibited)
        NSApp.appearance = NSAppearance(named: .aqua)
        let controller = ProjectContextViewController(chatID: "project-fixture")
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 580, height: 700), styleMask: [.borderless], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: .aqua)
        window.backgroundColor = .windowBackgroundColor
        window.contentViewController = controller
        let view = controller.view
        view.wantsLayer = true
        window.orderBack(nil)
        let readyBy = Date().addingTimeInterval(10)
        while !descendants(view).compactMap({ $0 as? NSPopUpButton }).contains(where: { $0.itemTitles.contains("New entry") }) && Date() < readyBy { pump() }
        let controls = descendants(view).compactMap { $0 as? NSPopUpButton }
        guard let picker = controls.first(where: { $0.itemTitles.contains("New entry") }),
              let history = descendants(view).compactMap({ $0 as? NSButton }).first(where: { $0.title == "Show revision history" }) else {
            fatalError("Production view did not load the fixture response")
        }
        let scenarios = [
            ("agreed-decision.png", "Pilot release decision", false),
            ("source-unavailable.png", "Release status reference", false),
            ("reference-asset.png", "Launch checklist", false),
            ("revision-history.png", "Initial rollout plan", true)
        ]
        for (file, title, showsHistory) in scenarios {
            if showsHistory {
                history.state = .on
                NSApp.sendAction(history.action!, to: history.target, from: history)
                pump()
            }
            guard let index = picker.itemTitles.firstIndex(where: { $0.contains(title) }) else {
                fatalError("Missing fixture entry: \(title)")
            }
            picker.selectItem(at: index)
            NSApp.sendAction(picker.action!, to: picker.target, from: picker)
            pump()
            precondition(controller.confirmButton.isEnabled != showsHistory, "Historical entries must be read-only")
            view.layoutSubtreeIfNeeded()
            let size = view.fittingSize
            window.setContentSize(NSSize(width: 580, height: max(size.height, 600)))
            view.layoutSubtreeIfNeeded()
            pump()
            window.makeFirstResponder(nil)
            for text in descendants(view).compactMap({ $0 as? NSTextView }) {
                if let container = text.textContainer { text.layoutManager?.ensureLayout(for: container) }
                text.scrollRangeToVisible(NSRange(location: 0, length: 0))
                text.needsDisplay = true
                text.display()
            }
            view.display()
            let scale: CGFloat = 2
            let width = Int(view.bounds.width * scale)
            let height = Int(view.bounds.height * scale)
            guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
                    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
                  let context = NSGraphicsContext(bitmapImageRep: bitmap)?.cgContext,
                  let layer = view.layer else { fatalError("No native layer backing") }
            context.scaleBy(x: scale, y: scale)
            window.effectiveAppearance.performAsCurrentDrawingAppearance {
                context.setFillColor(NSColor.windowBackgroundColor.cgColor)
                context.fill(view.bounds)
                layer.render(in: context)
            }
            guard let png = bitmap.representation(using: .png, properties: [:]) else { fatalError("PNG encoding failed") }
            try png.write(to: output.appendingPathComponent(file))
            print("Captured \(file): \(bitmap.pixelsWide) × \(bitmap.pixelsHigh)")
        }
        window.contentViewController = nil
        window.close()
    }

    @MainActor
    static func descendants(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap(descendants)
    }

    @MainActor
    static func pump() {
        let until = Date().addingTimeInterval(0.2)
        while Date() < until { RunLoop.main.run(mode: .default, before: until) }
    }
}
