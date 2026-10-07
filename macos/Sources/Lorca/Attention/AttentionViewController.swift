import AppKit

struct AttentionRevision: Codable, Equatable {
    var counter: UInt64
    var deviceId: String
    var params: [String: Any] { ["counter": counter, "device_id": deviceId] }
}
struct AttentionSource: Decodable, Equatable {
    var chatId: String
    var taskId: String?
    var messageId: String?
    var reviewId: String?
}
struct AttentionItem: Decodable, Equatable {
    var id: String
    var category: String
    var title: String
    var summary: String
    var nextAction: String
    var coordinatorBotId: String
    var sources: [AttentionSource]
    var reporters: [String]
    var urgent: Bool
    var revision: AttentionRevision
}
struct AttentionBrief: Decodable, Equatable {
    var coordinatorBotId: String
    var chatId: String
    var decisions: [String]
    var changes: [String]
    var nextAction: String
    var itemIds: [String]
    var messageId: String
}
struct AttentionPreferences: Decodable, Equatable {
    var summaries = true
    var urgentDirect = true
    var defaultCoordinatorBotId: String?
    var coordinators: [String: String] = [:]
}
struct AttentionView: Decodable, Equatable {
    var items: [AttentionItem] = []
    var briefs: [AttentionBrief] = []
    var preferences = AttentionPreferences()
}

/// Account attention, maintained by ordinary bots and rendered from the local CLI.
final class AttentionViewController: SheetViewController {
    private let store = AppStore.shared
    private let rows = Build.stack([], spacing: 16)
    private let errorLabel = Build.label("", font: .systemFont(ofSize: 12), color: .systemRed, lines: 0)
    private let summaries = NSButton(checkboxWithTitle: L("Coordinator summaries"), target: nil, action: nil)
    private let urgent = NSButton(checkboxWithTitle: L("Urgent direct alerts"), target: nil, action: nil)
    private let coordinator = NSPopUpButton()
    var onOpenSource: ((AttentionSource) -> Void)?
    private var rendered: AttentionView?

    init() {
        super.init(title: L("Attention"), subtitle: L("Decisions, reviews, blockers and commitments across your chats"), width: 660)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let preferences = Build.stack([summaries, urgent], orientation: .horizontal, spacing: 18)
        contentStack.addArrangedSubview(preferences)
        coordinator.target = self
        coordinator.action = #selector(preferencesChanged)
        let choice = Build.stack([Build.label(L("Default coordinator"), font: .systemFont(ofSize: 12)), coordinator], orientation: .horizontal, spacing: 12)
        contentStack.addArrangedSubview(choice)
        for button in [summaries, urgent] { button.target = self; button.action = #selector(preferencesChanged) }
        contentStack.addArrangedSubview(errorLabel)
        errorLabel.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        errorLabel.isHidden = true
        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(rows)
        scroll.documentView = document
        contentStack.addArrangedSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 430),
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            rows.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 2),
            rows.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -12),
            rows.topAnchor.constraint(equalTo: document.topAnchor, constant: 4),
            rows.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -8),
        ])
        setButtons(confirm: L("Done"), cancel: nil)
        store.observe(self) { [weak self] event in
            switch event {
            case .attentionChanged, .snapshotReplaced, .rosterChanged, .chatsChanged: self?.reload()
            default: break
            }
        }
        reload()
    }

    private func reload() {
        summaries.state = store.attention.preferences.summaries ? .on : .off
        urgent.state = store.attention.preferences.urgentDirect ? .on : .off
        coordinator.removeAllItems()
        coordinator.addItem(withTitle: L("Chat owner"))
        for bot in store.bots {
            coordinator.addItem(withTitle: bot.name)
            coordinator.lastItem?.representedObject = bot.id
        }
        if let id = store.attention.preferences.defaultCoordinatorBotId,
            let index = coordinator.itemArray.firstIndex(where: { $0.representedObject as? String == id }) {
            coordinator.selectItem(at: index)
        }
        guard rendered != store.attention else { return }
        rendered = store.attention
        sources.removeAll()
        for view in rows.arrangedSubviews { rows.removeArrangedSubview(view); view.removeFromSuperview() }
        for brief in store.attention.briefs {
            let name = store.bot(brief.coordinatorBotId)?.name ?? L("Coordinator")
            let card = Build.stack([], spacing: 6)
            card.addArrangedSubview(Build.label(L("%@’s brief", name), font: .systemFont(ofSize: 14, weight: .semibold)))
            for decision in brief.decisions { card.addArrangedSubview(label(L("Decision: %@", decision))) }
            for change in brief.changes { card.addArrangedSubview(label(L("Changed: %@", change))) }
            card.addArrangedSubview(label(L("Next: %@", brief.nextAction)))
            card.addArrangedSubview(sourceButton(AttentionSource(chatId: brief.chatId, messageId: brief.messageId), title: L("Open brief")))
            add(card)
        }
        if store.attention.items.isEmpty { add(label(L("Nothing needs attention"))) }
        for item in store.attention.items {
            let card = Build.stack([], spacing: 6)
            let category: String
            switch item.category {
            case "review": category = L("Pending review")
            case "blocker": category = L("Blocker")
            case "commitment": category = L("Commitment")
            default: category = L("Important change")
            }
            let name = store.bot(item.coordinatorBotId)?.name ?? L("Coordinator")
            card.addArrangedSubview(Build.label([item.urgent ? L("Urgent") : category, name].joined(separator: " · "), font: .systemFont(ofSize: 11), color: item.urgent ? .systemRed : .secondaryLabelColor))
            card.addArrangedSubview(Build.label(item.title, font: .systemFont(ofSize: 13, weight: .semibold), lines: 0))
            card.addArrangedSubview(label(item.summary))
            card.addArrangedSubview(label(L("Next: %@", item.nextAction)))
            for source in item.sources {
                let chat = store.chat(source.chatId)
                let title = chat.map(store.title(for:)) ?? L("Source chat")
                let reference = source.reviewId ?? source.taskId
                card.addArrangedSubview(sourceButton(source, title: reference.map { "\(title) · \($0)" } ?? title))
            }
            let resolve = NSButton(title: L("Mark resolved"), target: self, action: #selector(resolveItem(_:)))
            resolve.bezelStyle = .rounded
            resolve.identifier = NSUserInterfaceItemIdentifier(item.id)
            card.addArrangedSubview(resolve)
            add(card)
        }
    }

    private func label(_ text: String) -> NSTextField { Build.label(text, font: .systemFont(ofSize: 12), lines: 0) }
    private func add(_ row: NSView) {
        rows.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: rows.widthAnchor).isActive = true
        if let stack = row as? NSStackView {
            for child in stack.arrangedSubviews {
                if child is NSTextField { child.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true }
                else { child.widthAnchor.constraint(lessThanOrEqualTo: stack.widthAnchor).isActive = true }
            }
        }
    }
    private var sources: [String: AttentionSource] = [:]
    private func sourceButton(_ source: AttentionSource, title: String) -> NSButton {
        let button = NSButton(title: title, target: self, action: #selector(openSource(_:)))
        button.bezelStyle = .inline
        button.alignment = .left
        button.cell?.lineBreakMode = .byTruncatingMiddle
        let id = UUID().uuidString
        button.identifier = NSUserInterfaceItemIdentifier(id)
        sources[id] = source
        return button
    }
    @objc private func openSource(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue, let source = sources[id] else { return }
        dismissSheet()
        onOpenSource?(source)
    }
    @objc private func preferencesChanged() {
        let picked: Any = (coordinator.selectedItem?.representedObject as? String).map { $0 as Any } ?? NSNull()
        let params: [String: Any] = [
            "summaries": summaries.state == .on,
            "urgent_direct": urgent.state == .on,
            "default_coordinator_bot_id": picked,
        ]
        perform { _ = try await self.store.client.request("attention.preferences", params) }
    }
    @objc private func resolveItem(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue, let item = store.attention.items.first(where: { $0.id == id }) else { return }
        perform { _ = try await self.store.client.request("attention.resolve", ["id": id, "expected_revision": item.revision.params]) }
    }
    private func perform(_ action: @escaping () async throws -> Void) {
        Task { @MainActor in
            do { try await action(); errorLabel.isHidden = true }
            catch { errorLabel.stringValue = error.localizedDescription; errorLabel.isHidden = false; reload() }
        }
    }
}

extension RootSplitViewController {
    @objc func showAttention(_ sender: Any?) {
        let sheet = AttentionViewController()
        sheet.onOpenSource = { [weak self] source in self?.open(source.chatId) }
        presentAsSheet(sheet)
    }
}
