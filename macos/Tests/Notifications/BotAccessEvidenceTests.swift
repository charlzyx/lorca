import AppKit
import XCTest
@testable import Lorca

/// Opt-in screenshots of production AppKit views. The fixture connects only to a loopback
/// tool-catalog stub; mock profile saves never reach a real CLI or account.
@MainActor
final class BotAccessEvidenceTests: XCTestCase {
    private func descendants(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap(descendants)
    }

    private func button(_ title: String, in view: NSView) throws -> NSButton {
        try XCTUnwrap(descendants(view).compactMap { $0 as? NSButton }.first { $0.title == title }, title)
    }

    private func labels(in view: NSView) -> [String] {
        descendants(view).compactMap { ($0 as? NSTextField)?.stringValue }
    }

    private func waitFor(_ description: String, _ predicate: () -> Bool) async throws {
        let end = Date().addingTimeInterval(8)
        while !predicate() && Date() < end { try await Task.sleep(nanoseconds: 20_000_000) }
        XCTAssertTrue(predicate(), description)
    }

    private func capture(_ view: NSView, named name: String, in directory: URL) throws {
        view.window?.displayIfNeeded()
        view.layoutSubtreeIfNeeded()
        if name.hasPrefix("access-"), let window = view.window {
            let capture = Process()
            capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), directory.appendingPathComponent(name).path]
            try capture.run()
            capture.waitUntilExit()
            XCTAssertEqual(capture.terminationStatus, 0, "capture only the fixture sheet window")
            try validateImage(at: directory.appendingPathComponent(name))
            return
        }
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: directory.appendingPathComponent(name))
        try validateImage(at: directory.appendingPathComponent(name))
    }

    private func validateImage(at url: URL) throws {
        let image = try XCTUnwrap(NSBitmapImageRep(data: Data(contentsOf: url)))
        var colors = Set<String>()
        for y in stride(from: 0, to: image.pixelsHigh, by: max(1, image.pixelsHigh / 40)) {
            for x in stride(from: 0, to: image.pixelsWide, by: max(1, image.pixelsWide / 40)) {
                if let color = image.colorAt(x: x, y: y) { colors.insert(color.description) }
            }
        }
        XCTAssertGreaterThan(colors.count, 10, "reject blank/transparent captures: \(url.lastPathComponent)")
    }

    func testCaptureProfileEditorAndRefusedAccess() async throws {
        guard let path = ProcessInfo.processInfo.environment["LORCA_UI_EVIDENCE_DIR"] else {
            throw XCTSkip("Run bun macos/Tests/Fixtures/capture-bot-access.ts to capture fixture UI")
        }
        let directory = URL(fileURLWithPath: path, isDirectory: true)
        let store = AppStore.shared
        guard store.isMock, let rawPort = ProcessInfo.processInfo.environment["LORCA_PORT"],
              let fixturePort = Int(rawPort), fixturePort == Preferences.cliPort, fixturePort != AppInfo.defaultCLIPort else {
            throw XCTSkip("Evidence requires mock data and the isolated loopback fixture port")
        }
        NSApplication.shared.appearance = NSAppearance(named: .aqua)
        NSApplication.shared.setActivationPolicy(.accessory)
        store.resetMockData()
        let botID = store.createBot(name: "Inbox Assistant", description: "Read the Work inbox and stage drafts for review.",
                                    symbolName: "envelope", accent: .indigo, runnerID: "dev-workbench", provider: .deepseek)
        let chatID = store.dm(with: botID)
        store.client.connect()
        defer { store.client.disconnect() }
        try await waitFor("fixture websocket is connected") { store.client.state == .connected }

        let inspector = InspectorViewController()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 380, height: 900),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.title = "Lorca · AppKit permission fixture"
        window.setFrameOrigin(NSPoint(x: 80, y: 80))
        window.contentViewController = inspector
        window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)
        defer { window.orderOut(nil) }
        inspector.viewWillAppear()
        inspector.show(selection: .chat(chatID))
        window.displayIfNeeded()
        let profile = try XCTUnwrap(descendants(inspector.view).compactMap { $0 as? SectionView }
            .first { $0.title == L("Profile") })
        try capture(profile, named: "profile-before-policy.png", in: directory)

        let access = try XCTUnwrap(descendants(profile).compactMap { $0 as? SummaryActionRow }
            .first { labels(in: $0).contains(L("Access")) })
        try button(L("Edit…"), in: access).performClick(nil)
        try await waitFor("Profile Access opens its native sheet") { window.attachedSheet != nil }
        let sheet = try XCTUnwrap(window.attachedSheet)
        let editor = try XCTUnwrap(sheet.contentViewController as? BotAccessViewController)
        try await waitFor("fixture account and local tools rendered") {
            labels(in: editor.view).contains("Gmail · Work")
                && descendants(editor.view).contains { ($0 as? NSButton)?.title == "stage_review" }
        }
        // AppKit's sheet transition commits its backing layers after the catalog is ready.
        try await Task.sleep(nanoseconds: 350_000_000)

        // Drive real native controls into a Work-only read/draft policy.
        try button(L("All connections"), in: editor.view).performClick(nil)
        let groups = descendants(editor.view).compactMap { $0 as? NSStackView }
        let personal = try XCTUnwrap(groups.first { ($0.arrangedSubviews.first as? NSTextField)?.stringValue == "Gmail · Personal" })
        let work = try XCTUnwrap(groups.first { ($0.arrangedSubviews.first as? NSTextField)?.stringValue == "Gmail · Work" })
        for title in [L("Read"), L("Draft"), L("Write")] { try button(title, in: personal).performClick(nil) }
        try button(L("Write"), in: work).performClick(nil)
        try button(L("All connection tools"), in: work).performClick(nil)
        try button("send_message · \(L("Write"))", in: work).performClick(nil)
        try button(L("All local tools"), in: editor.view).performClick(nil)
        let selected = Set(["codemode", "read", "recall", "stage_review"])
        let catalog = try await store.botPermissionCatalog(botID)
        for name in catalog.localTools where !selected.contains(name) { try button(name, in: editor.view).performClick(nil) }
        let fileModes = descendants(editor.view).compactMap { $0 as? NSPopUpButton }
        let filesystem = try XCTUnwrap(fileModes.first { $0.itemTitles.contains(L("Read and write")) })
        filesystem.selectItem(at: 1)
        let shell = try button(L("Allow shell commands"), in: editor.view)
        shell.performClick(nil)
        XCTAssertEqual(shell.state, .off)
        XCTAssertEqual(try button(L("Read"), in: work).state, .on)
        XCTAssertEqual(try button(L("Draft"), in: work).state, .on)
        XCTAssertEqual(try button(L("Write"), in: work).state, .off)
        let scroll = try XCTUnwrap(descendants(editor.view).compactMap { $0 as? NSScrollView }.first)
        let document = try XCTUnwrap(scroll.documentView)
        scroll.contentView.scroll(to: .zero)
        scroll.reflectScrolledClipView(scroll.contentView)
        sheet.displayIfNeeded()
        try capture(editor.view, named: "access-connections.png", in: directory)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: max(0, document.bounds.height - scroll.contentView.bounds.height)))
        scroll.reflectScrolledClipView(scroll.contentView)
        sheet.displayIfNeeded()
        try capture(editor.view, named: "access-local-controls.png", in: directory)
        try button(L("Save"), in: editor.view).performClick(nil)
        try await waitFor("native Save applies fixture policy") { store.bot(botID)?.permissions?.shell == false }
        try await waitFor("native sheet closes") { window.attachedSheet == nil }
        inspector.reload()
        window.displayIfNeeded()
        try capture(profile, named: "profile-after-policy.png", in: directory)
        let policy = try XCTUnwrap(store.bot(botID)?.permissions)
        XCTAssertEqual(policy.tools, selected)
        XCTAssertEqual(policy.filesystem, "read")
        XCTAssertEqual(policy.connections?["gmail-" + String(repeating: "1", count: 32)]?.capabilities, ["read", "draft"])

        let request = PermissionRequest(pluginID: "gmail-" + String(repeating: "1", count: 32), pluginName: "Bot access", tool: "access",
            summary: "Inbox Assistant needs access to send_message", decision: .pending,
            reason: "Access refused for send_message: write access to the Work connection is disabled. Change this bot's Access settings in its profile. Auto-review and Always allow cannot override this restriction.")
        let width: CGFloat = 500
        let height = PermissionCellView.height(for: request, rowWidth: width, indent: 0)
        let card = PermissionCellView()
        card.translatesAutoresizingMaskIntoConstraints = true
        card.frame = NSRect(x: 0, y: 0, width: width, height: height)
        // The unused command block normally keeps a cell's prior frame. Give the fresh
        // fixture cell a finite hidden-block frame before AppKit lays out its padding.
        for block in descendants(card).compactMap({ $0 as? CommandBlockView }) {
            block.frame = NSRect(x: 0, y: 0, width: 100, height: 40)
        }
        card.configure(request: request, botName: "Inbox Assistant", avatar: nil, groupStart: false)
        let cardWindow = NSWindow(contentRect: card.frame, styleMask: .borderless, backing: .buffered, defer: false)
        cardWindow.contentView = card
        cardWindow.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        defer { cardWindow.orderOut(nil) }
        var clicked: String?
        card.onDecision = { clicked = $0 }
        cardWindow.displayIfNeeded()
        try capture(card, named: "refused-access-request.png", in: directory)
        let visibleChoices = descendants(card).compactMap { $0 as? NSButton }.filter { !$0.isHidden }.map(\.title)
        XCTAssertFalse(visibleChoices.contains(L("Allow once")))
        XCTAssertFalse(visibleChoices.contains(L("Always allow")))
        try button(L("Edit Access…"), in: card).performClick(nil)
        XCTAssertEqual(clicked, "access")
    }
}
