// Isolated native fixture host. Production AttentionViewController and SheetViewController
// are compiled unchanged by capture-appkit.py; this host supplies synthetic store/CLI data.
import AppKit

func L(_ key: String, _ args: CVarArg...) -> String { String(format: key, arguments: args) }
final class FlippedView: NSView { override var isFlipped: Bool { true } }
struct FixtureBot: Decodable { var id: String; var name: String }
struct FixtureChat: Decodable { var id: String; var title: String }
struct Fixture: Decodable { var bots: [FixtureBot]; var chats: [FixtureChat]; var attention: AttentionView }
enum StoreEvent { case attentionChanged, snapshotReplaced, rosterChanged, chatsChanged }
@MainActor final class FixtureClient {
    func request(_ method: String, _ params: [String: Any]) async throws -> Bool {
        let store = AppStore.shared
        if method == "attention.resolve", let id = params["id"] as? String { store.attention.items.removeAll { $0.id == id } }
        if method == "attention.preferences" {
            if let value = params["summaries"] as? Bool { store.attention.preferences.summaries = value }
            if let value = params["urgent_direct"] as? Bool { store.attention.preferences.urgentDirect = value }
            store.attention.preferences.defaultCoordinatorBotId = params["default_coordinator_bot_id"] as? String
        }
        store.publish()
        return true
    }
}
@MainActor final class AppStore {
    static let shared = AppStore()
    var attention = AttentionView()
    var bots: [FixtureBot] = []
    var chats: [FixtureChat] = []
    let client = FixtureClient()
    private var observers: [(StoreEvent) -> Void] = []
    func observe(_ owner: AnyObject, _ handler: @escaping (StoreEvent) -> Void) { observers.append(handler) }
    func publish() { observers.forEach { $0(.attentionChanged) } }
    func bot(_ id: String) -> FixtureBot? { bots.first { $0.id == id } }
    func chat(_ id: String) -> FixtureChat? { chats.first { $0.id == id } }
    func title(for chat: FixtureChat) -> String { chat.title }
}
@MainActor final class RootSplitViewController: NSViewController { func open(_ id: String) {} }

@main struct Capture {
    @MainActor static func main() throws {
        let arguments = CommandLine.arguments
        let data = try Data(contentsOf: URL(fileURLWithPath: arguments[1]))
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let fixture = try decoder.decode(Fixture.self, from: data)
        let store = AppStore.shared
        store.attention = fixture.attention; store.bots = fixture.bots; store.chats = fixture.chats
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        app.appearance = NSAppearance(named: .aqua)
        app.finishLaunching()
        let controller = AttentionViewController()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 660, height: 640), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = controller
        window.contentView?.wantsLayer = true
        window.contentView?.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        controller.view.frame = NSRect(x: 0, y: 0, width: 660, height: 640)
        func settle() {
            controller.view.layoutSubtreeIfNeeded()
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.2))
            controller.view.layoutSubtreeIfNeeded()
        }
        func views(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(views) }
        func capture(_ name: String) throws {
            settle()
            let view = controller.view
            guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { fatalError("No native bitmap") }
            view.cacheDisplay(in: view.bounds, to: rep)
            guard let bytes = rep.representation(using: .png, properties: [:]) else { fatalError("No PNG") }
            try bytes.write(to: URL(fileURLWithPath: arguments[2]).appendingPathComponent(name))
            print("Captured \(name): \(rep.pixelsWide)×\(rep.pixelsHigh)")
        }
        try capture("macos-attention-active.png")
        let scroll = views(controller.view).compactMap { $0 as? NSScrollView }.first!
        scroll.contentView.scroll(to: NSPoint(x: 0, y: max(0, (scroll.documentView?.frame.height ?? 0) - 430)))
        scroll.reflectScrolledClipView(scroll.contentView)
        try capture("macos-attention-followups.png")
        // Exercise the real controller's action selectors through the fixture API boundary.
        for button in views(controller.view).compactMap({ $0 as? NSButton }).filter({ $0.title == "Mark resolved" }) {
            button.performClick(nil); settle()
        }
        if let summary = views(controller.view).compactMap({ $0 as? NSButton }).first(where: { $0.title == "Coordinator summaries" }) {
            summary.performClick(nil); settle()
        }
        scroll.contentView.scroll(to: .zero); scroll.reflectScrolledClipView(scroll.contentView)
        try capture("macos-attention-resolved.png")
        guard store.attention.items.isEmpty, !store.attention.preferences.summaries else { fatalError("Fixture actions did not apply") }
        print("Verified: all four synthetic items resolved; coordinator summaries switched off.")
        window.close()
    }
}
