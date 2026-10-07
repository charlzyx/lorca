import AppKit

/// User-owned grants. Saving updates the CLI profile; a chat's request only opens this sheet.
final class BotAccessViewController: SheetViewController {
    private let store = AppStore.shared
    private let botID: Bot.ID
    private let initialPolicy: BotPermissions?
    private let allConnections = NSButton(checkboxWithTitle: L("All connections"), target: nil, action: nil)
    private let allTools = NSButton(checkboxWithTitle: L("All local tools"), target: nil, action: nil)
    private let shell = NSButton(checkboxWithTitle: L("Allow shell commands"), target: nil, action: nil)
    private let filesystem = NSPopUpButton()
    private let connections = Build.stack([], spacing: 12)
    private let localTools = Build.stack([], spacing: 3)
    private let note = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private var toolButtons: [String: NSButton] = [:]

    private struct ConnectionControls {
        var capabilities: [String: NSButton]
        var allTools: NSButton
        var tools: [String: NSButton]
    }
    private var connectionControls: [String: ConnectionControls] = [:]

    init(botID: Bot.ID) {
        self.botID = botID
        initialPolicy = AppStore.shared.bot(botID)?.permissions
        super.init(title: L("Access"), subtitle: AppStore.shared.bot(botID)?.name ?? "", width: 580)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let policy = initialPolicy ?? BotPermissions()
        allConnections.state = policy.connections == nil ? .on : .off
        allTools.state = policy.tools == nil ? .on : .off
        shell.state = policy.shell ? .on : .off
        filesystem.addItems(withTitles: [L("None"), L("Read"), L("Read and write")])
        filesystem.selectItem(at: ["none", "read", "write"].firstIndex(of: policy.filesystem) ?? 0)

        let explanation = Build.label(
            L("These limits apply before Auto-review. Only you can change them. Read, draft, and write are separate grants."),
            font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
        let boundary = Build.label(
            L("Shell commands and filesystem tools use the Runner's user account. Shell access can reach credentials and bypass connection limits. A working directory provides no isolation; use an isolated process or a dedicated Runner for stronger separation."),
            font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)

        let body = Build.stack([], spacing: 12)
        body.alignment = .leading
        let filesystemRow = Build.stack([Build.label(L("Filesystem"), font: Theme.Font.caption), filesystem], orientation: .horizontal, spacing: 10)
        for child in [explanation, allConnections, connections, allTools, localTools, filesystemRow, shell, boundary] {
            body.addArrangedSubview(child)
            child.widthAnchor.constraint(equalTo: body.widthAnchor).isActive = true
        }
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(body)
        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = document
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        contentStack.addArrangedSubview(scroll)
        contentStack.addArrangedSubview(note)
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 440),
            document.widthAnchor.constraint(equalTo: scroll.widthAnchor, constant: -16),
            body.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            body.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            body.topAnchor.constraint(equalTo: document.topAnchor),
            body.bottomAnchor.constraint(equalTo: document.bottomAnchor),
            note.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])
        for button in [allConnections, allTools] {
            button.target = self
            button.action = #selector(modesChanged)
        }
        setButtons(confirm: L("Save"))
        // The saved policy and Runner advertisement are useful even while that Runner is offline.
        let runner = store.bot(botID).flatMap { store.device($0.runnerID) }
        let fallback = (runner?.plugins ?? []).map { BotPermissionCatalog.Connection(id: $0.id, name: $0.name, tools: []) }
        renderConnections(fallback)
        renderLocalTools([])
        note.stringValue = L("Loading available tools…")
        Task { [weak self] in
            guard let self else { return }
            do {
                let catalog = try await store.botPermissionCatalog(botID)
                renderConnections(catalog.connections)
                renderLocalTools(catalog.localTools)
                note.stringValue = L("Unlisted connections and tools are denied when an allowlist is selected. Saved tool labels are informational; the CLI checks live capabilities before a call.")
            } catch {
                note.stringValue = error.localizedDescription
            }
        }
    }

    private func checkbox(_ title: String, checked: Bool) -> NSButton {
        let button = NSButton(checkboxWithTitle: title, target: self, action: #selector(modesChanged))
        button.state = checked ? .on : .off
        return button
    }

    private func renderLocalTools(_ names: [String]) {
        let policy = initialPolicy ?? BotPermissions()
        let previous = toolButtons.mapValues { $0.state == .on }
        localTools.arrangedSubviews.forEach { localTools.removeArrangedSubview($0); $0.removeFromSuperview() }
        toolButtons.removeAll()
        let available = Set(names).union(policy.tools ?? []).union(previous.keys)
        for name in available.sorted() {
            let button = checkbox(name, checked: previous[name] ?? (policy.tools?.contains(name) ?? true))
            toolButtons[name] = button
            localTools.addArrangedSubview(button)
        }
        modesChanged()
    }

    private func renderConnections(_ listed: [BotPermissionCatalog.Connection]) {
        let policy = initialPolicy ?? BotPermissions()
        // Keep the user's in-progress choices if catalog discovery finishes after an edit.
        let previous = currentConnections()
        connections.arrangedSubviews.forEach { connections.removeArrangedSubview($0); $0.removeFromSuperview() }
        connectionControls.removeAll()
        var available = Dictionary(listed.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        for id in (policy.connections ?? [:]).keys where available[id] == nil {
            available[id] = .init(id: id, name: id, tools: [])
        }
        for connection in available.values.sorted(by: { $0.name.localizedStandardCompare($1.name) == .orderedAscending }) {
            let grant = previous[connection.id] ?? policy.connections?[connection.id]
                ?? BotPermissions.Connection(capabilities: policy.connections == nil ? ["read", "draft", "write"] : [])
            let heading = Build.label(connection.name, font: .systemFont(ofSize: 12, weight: .semibold))
            heading.toolTip = connection.id
            let id = Build.label(connection.id, font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
            let caps = ["read": checkbox(L("Read"), checked: grant.capabilities.contains("read")),
                        "draft": checkbox(L("Draft"), checked: grant.capabilities.contains("draft")),
                        "write": checkbox(L("Write"), checked: grant.capabilities.contains("write"))]
            let row = Build.stack([caps["read"]!, caps["draft"]!, caps["write"]!], orientation: .horizontal, spacing: 18)
            let all = checkbox(L("All connection tools"), checked: grant.tools == nil)
            let group = Build.stack([heading, id, row, all], spacing: 5)
            group.alignment = .leading
            var tools: [String: NSButton] = [:]
            let names = Set(connection.tools.filter { $0.hidden != true }.map(\.name)).union(grant.tools ?? [])
            for name in names.sorted() {
                let metadata = connection.tools.first { $0.name == name }
                let capability = metadata?.capability ?? "write"
                let label = ["read": L("Read"), "draft": L("Draft"), "write": L("Write")][capability] ?? L("Write")
                let button = checkbox("\(name) · \(label)", checked: grant.tools?.contains(name) ?? true)
                button.toolTip = metadata?.description
                tools[name] = button
                group.addArrangedSubview(button)
            }
            connectionControls[connection.id] = ConnectionControls(capabilities: caps, allTools: all, tools: tools)
            connections.addArrangedSubview(group)
            group.widthAnchor.constraint(equalTo: connections.widthAnchor).isActive = true
        }
        modesChanged()
    }

    private func currentConnections() -> [String: BotPermissions.Connection] {
        connectionControls.mapValues { controls in
            BotPermissions.Connection(
                capabilities: Set(controls.capabilities.filter { $0.value.state == .on }.keys),
                tools: controls.allTools.state == .on ? nil : Set(controls.tools.filter { $0.value.state == .on }.keys))
        }
    }

    @objc private func modesChanged() {
        for button in toolButtons.values { button.isEnabled = allTools.state != .on }
        for controls in connectionControls.values {
            for button in controls.capabilities.values { button.isEnabled = allConnections.state != .on }
            controls.allTools.isEnabled = allConnections.state != .on
            for button in controls.tools.values { button.isEnabled = allConnections.state != .on && controls.allTools.state != .on }
        }
    }

    override func confirmTapped() {
        guard store.bot(botID)?.permissions == initialPolicy else {
            note.stringValue = L("Access changed while this sheet was open. Reopen it to review the current settings.")
            return
        }
        var policy = BotPermissions()
        policy.connections = allConnections.state == .on ? nil : currentConnections()
        policy.tools = allTools.state == .on ? nil : Set(toolButtons.filter { $0.value.state == .on }.keys)
        policy.filesystem = ["none", "read", "write"][max(0, filesystem.indexOfSelectedItem)]
        policy.shell = shell.state == .on
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                try await store.setBotPermissions(botID, policy: policy)
                dismiss(nil)
            } catch {
                note.stringValue = error.localizedDescription
                confirmButton.isEnabled = true
            }
        }
    }
}
