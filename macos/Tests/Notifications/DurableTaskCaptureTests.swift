import AppKit
import XCTest
@testable import Lorca

/// Opt-in PR evidence. The runner supplies an isolated echo websocket; the store's existing
/// mock roster/chats and explicit task fixtures contain no user data or provider credentials.
final class DurableTaskCaptureTests: XCTestCase {
    @MainActor
    func testCaptureDurableTaskScreens() async throws {
        guard let destination = ProcessInfo.processInfo.environment["LORCA_TASK_CAPTURE_DIR"] else {
            throw XCTSkip("Run macos/Tests/Fixtures/DurableTasks/capture.py for native PR evidence.")
        }
        XCTAssertEqual(ProcessInfo.processInfo.environment["LORCA_MOCK"], "1")
        XCTAssertNotEqual(Preferences.cliPort, 4862)
        XCTAssertNotEqual(Preferences.cliPort, 4863)
        let output = URL(fileURLWithPath: destination, isDirectory: true)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        NSApplication.shared.appearance = NSAppearance(named: .aqua)
        let store = AppStore.shared
        store.start() // Mock mode returns before launching or connecting the real CLI.
        store.client.connect()
        defer { store.client.disconnect() }
        for _ in 0..<200 where store.client.state != .connected {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertEqual(store.client.state, .connected)

        let group = try XCTUnwrap(store.chat("chat-relay"))
        let owner = try XCTUnwrap(store.bot("bot-nova"))
        let phases: [DurableTask.State] = [.queued, .working, .blocked, .awaitingReview, .completed, .cancelled]
        let goals = ["Checklist", "Notes", "Resume", "Review", "Sync", "Old draft"]
        var fixtures: [DurableTask] = []
        for (index, phase) in phases.enumerated() {
            let id = String(format: "task-00000000-0000-0000-0000-%012d", index + 1)
            let evidence = phase == .awaitingReview || phase == .completed
                ? [DurableTask.Evidence(kind: "url", label: "Demo smoke-check summary", url: "https://example.invalid/evidence/smoke-checks")]
                : []
            let task = DurableTask(
                id: id, revision: 7, authorityRunnerId: owner.runnerID, ownerBotId: owner.id, runnerId: owner.runnerID,
                goal: goals[index], acceptanceCriteria: ["Ownership remains stable after restart", "Result and evidence are recorded"],
                dependencies: phase == .queued ? ["task-00000000-0000-0000-0000-000000000005"] : [],
                nextAction: phase == .blocked ? "Inspect the interrupted command before starting a fresh run." : "Review the result and attach supporting evidence.",
                chatIds: [group.id, "chat-patch"], links: [.init(label: "Demo tracking issue", url: "https://example.invalid/issues/72")],
                state: phase,
                reason: phase == .blocked ? "Runner restarted during this run. Its effects may have occurred; inspect them before requeueing." : phase == .cancelled ? "Replaced by the current release checklist." : nil,
                result: phase == .awaitingReview || phase == .completed ? "Demo result: ownership and replay checks are ready for review. The supporting summary is linked below." : nil,
                evidence: evidence,
                activeRun: phase == .working ? .init(id: "task-run-demo", botId: owner.id, runnerId: owner.runnerID, chatId: group.id, startedAt: 100) : nil,
                createdAt: 100, updatedAt: Double(100 + index)
            )
            let encoder = JSONEncoder()
            encoder.keyEncodingStrategy = .convertToSnakeCase
            let payload = try JSONSerialization.jsonObject(with: encoder.encode(task))
            fixtures.append(try await store.taskRequest("tasks.get", params: ["id": id, "fixture": payload]))
        }
        XCTAssertEqual(store.tasks(in: group.id).count, phases.count)

        let chat = ChatViewController()
        let inspector = InspectorViewController()
        let chatView = chat.view
        let inspectorView = inspector.view
        let columns = NSStackView(views: [chatView, inspectorView])
        columns.orientation = .horizontal
        columns.spacing = 0
        columns.alignment = .top
        chatView.widthAnchor.constraint(equalToConstant: 750).isActive = true
        inspectorView.widthAnchor.constraint(equalToConstant: 320).isActive = true
        chatView.heightAnchor.constraint(equalToConstant: 870).isActive = true
        inspectorView.heightAnchor.constraint(equalToConstant: 870).isActive = true
        let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1070, height: 870), styleMask: [.borderless], backing: .buffered, defer: false)
        host.isReleasedWhenClosed = false
        host.appearance = NSAppearance(named: .aqua)
        host.contentView = columns
        chat.viewWillAppear()
        inspector.viewWillAppear()
        chat.show(chatID: group.id)
        inspector.show(selection: .chat(group.id))
        columns.frame = NSRect(x: 0, y: 0, width: 1070, height: 870)
        columns.layoutSubtreeIfNeeded()
        await settle()
        if let tasks = descendants(inspectorView).compactMap({ $0 as? SectionView }).first(where: { $0.title == "Tasks" }) {
            tasks.scrollToVisible(tasks.bounds)
        }
        columns.layoutSubtreeIfNeeded()
        try capture(columns, named: "group-task-states.png", in: output)

        for (state, filename) in [(DurableTask.State.blocked, "blocked-after-restart.png"), (.awaitingReview, "review-result-evidence.png")] {
            let task = try XCTUnwrap(fixtures.first { $0.state == state })
            let editor = DurableTaskViewController(chatID: group.id, task: task)
            let view = editor.view
            view.layoutSubtreeIfNeeded()
            let size = view.fittingSize
            let sheet = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless], backing: .buffered, defer: false)
            sheet.isReleasedWhenClosed = false
            sheet.appearance = NSAppearance(named: .aqua)
            sheet.contentView = view
            view.frame = NSRect(origin: .zero, size: size)
            view.layoutSubtreeIfNeeded()
            await settle()
            if let form = editor.contentStack.arrangedSubviews.first as? NSScrollView, let document = form.documentView {
                // Capture actual scrolled sheet states, with its fixed title/actions retained.
                let marker = state == .blocked ? "Next action" : "Result · required for completion"
                if let label = descendants(document).compactMap({ $0 as? NSTextField }).first(where: { $0.stringValue == marker }) {
                    let rect = label.convert(label.bounds, to: document)
                    form.contentView.scroll(to: NSPoint(x: 0, y: max(0, rect.minY - 8)))
                    form.reflectScrolledClipView(form.contentView)
                }
            }
            view.layoutSubtreeIfNeeded()
            try capture(view, named: filename, in: output)
            sheet.close()
        }

        // Exercise the real save handler/error rendering against a scripted revision conflict.
        let task = try XCTUnwrap(fixtures.first { $0.state == .queued })
        let editor = DurableTaskViewController(chatID: group.id, task: task)
        let view = editor.view
        view.layoutSubtreeIfNeeded()
        let size = view.fittingSize
        let sheet = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless], backing: .buffered, defer: false)
        sheet.isReleasedWhenClosed = false
        sheet.appearance = NSAppearance(named: .aqua)
        sheet.contentView = view
        view.frame = NSRect(origin: .zero, size: size)
        editor.confirmTapped()
        for _ in 0..<200 {
            if descendants(view).compactMap({ $0 as? NSTextField }).contains(where: { $0.stringValue.contains("Task revision conflict") }) { break }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertTrue(descendants(view).compactMap({ $0 as? NSTextField }).contains(where: { $0.stringValue.contains("Task revision conflict") }))
        view.layoutSubtreeIfNeeded()
        await settle()
        try capture(view, named: "stale-edit-keeps-form.png", in: output)
        sheet.close()
        host.close()
    }

    @MainActor
    private func settle() async { try? await Task.sleep(nanoseconds: 150_000_000) }

    @MainActor
    private func descendants(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap(descendants)
    }

    @MainActor
    private func capture(_ view: NSView, named name: String, in output: URL) throws {
        let size = view.bounds.size
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size.width * 2), pixelsHigh: Int(size.height * 2), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        bitmap.size = size
        view.cacheDisplay(in: view.bounds, to: bitmap)
        // Offscreen views have transparent gaps where an ordinary window supplies its
        // background. Composite native pixels over that standard Aqua window color.
        let opaque = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: bitmap.pixelsWide, pixelsHigh: bitmap.pixelsHigh, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        opaque.size = size
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: opaque))
        NSColor.windowBackgroundColor.setFill()
        NSBezierPath(rect: NSRect(origin: .zero, size: size)).fill()
        NSImage(cgImage: try XCTUnwrap(bitmap.cgImage), size: size).draw(in: NSRect(origin: .zero, size: size))
        NSGraphicsContext.restoreGraphicsState()
        let png = try XCTUnwrap(opaque.representation(using: .png, properties: [:]))
        try png.write(to: output.appendingPathComponent(name))
        print("Native task fixture capture: \(name) (\(bitmap.pixelsWide)x\(bitmap.pixelsHigh))")
    }
}
