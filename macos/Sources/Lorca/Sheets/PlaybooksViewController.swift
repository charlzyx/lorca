import AppKit

/// Lists only metadata. A double click loads the selected skill body and history.
final class PlaybooksViewController: SheetViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let store = AppStore.shared
    private let scopes: [(PlaybookScope, String)]
    private let scopePicker = NSPopUpButton()
    private let table = NSTableView()
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private var items: [PlaybookSummary] = []
    private var generation = 0

    init(chat: Chat) {
        var scopes: [(PlaybookScope, String)] = []
        if chat.isGroup {
            scopes.append((PlaybookScope(kind: "project", id: chat.id), L("Project · %@", chat.customTitle ?? L("Group"))))
        }
        for id in chat.botIDs {
            scopes.append((PlaybookScope(kind: "bot", id: id), L("Bot · %@", AppStore.shared.bot(id)?.name ?? id)))
        }
        self.scopes = scopes
        super.init(title: L("Playbooks"), subtitle: L("Reusable instructions, examples, references, and scripts. Drafts wait for your review; saved skills are available on later turns."), width: 620)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    private var pickedScope: (PlaybookScope, String) { scopes[max(0, scopePicker.indexOfSelectedItem)] }

    override func loadView() {
        super.loadView()
        scopePicker.addItems(withTitles: scopes.map { $0.1 })
        scopePicker.target = self
        scopePicker.action = #selector(scopeChanged)
        contentStack.addArrangedSubview(scopePicker)
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("skill"))
        table.addTableColumn(column)
        table.headerView = nil
        table.rowHeight = 50
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(editSelected)
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        scroll.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(scroll)
        let actions = Build.stack([], orientation: .horizontal, spacing: 8)
        for (title, action) in [(L("New Skill"), #selector(newSkill)), (L("Edit / Review"), #selector(editSelected)),
                                (L("Export…"), #selector(exportSelected)), (L("Remove"), #selector(removeSelected)),
                                (L("Reload"), #selector(scopeChanged))] {
            actions.addArrangedSubview(NSButton(title: title, target: self, action: action))
        }
        contentStack.addArrangedSubview(actions)
        contentStack.addArrangedSubview(status)
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor), scroll.heightAnchor.constraint(equalToConstant: 280),
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor)
        ])
        setButtons(confirm: L("Done"), cancel: nil)
        reload()
    }

    private func reload() {
        guard !scopes.isEmpty else { return }
        generation += 1
        let requested = generation
        let scope = pickedScope.0
        status.stringValue = L("Loading…")
        Task { [weak self] in
            guard let self else { return }
            do {
                let fresh = try await store.playbooks(in: scope)
                guard generation == requested else { return }
                items = fresh
                table.reloadData()
                status.stringValue = fresh.isEmpty ? L("No skills in this scope. Create one or save a completed chat workflow.") : L("%d skills", fresh.count)
                status.textColor = .secondaryLabelColor
            } catch {
                guard generation == requested else { return }
                items = []
                table.reloadData()
                show(error)
            }
        }
    }
    @objc private func scopeChanged() { reload() }
    @objc private func newSkill() {
        let (scope, label) = pickedScope
        let editor = PlaybookViewController(scope: scope, scopeName: label)
        editor.onSaved = { [weak self] in self?.reload() }
        presentAsSheet(editor)
    }
    private var selected: PlaybookSummary? { items.indices.contains(table.selectedRow) ? items[table.selectedRow] : nil }
    @objc private func editSelected() {
        guard let item = selected else { return }
        let scopeName = pickedScope.1
        Task { [weak self] in
            guard let self else { return }
            do {
                let record = try await store.playbook(item.id, in: item.scope)
                let editor = PlaybookViewController(scope: item.scope, scopeName: scopeName, record: record)
                editor.onSaved = { [weak self] in self?.reload() }
                presentAsSheet(editor)
            } catch { show(error) }
        }
    }
    @objc private func exportSelected() {
        guard let item = selected else { return }
        Task { [weak self] in
            guard let self else { return }
            do {
                let data = try await store.client.request("playbooks.export", ["scope": item.scope.params, "id": item.id])
                let object = try JSONSerialization.jsonObject(with: data)
                let formatted = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
                let panel = NSSavePanel()
                panel.nameFieldStringValue = item.name + ".lorca-playbook.json"
                panel.message = L("Export this skill's current instructions and bundled files. Revision history and chat provenance stay private.")
                guard let window = view.window else { return }
                panel.beginSheetModal(for: window) { [weak self] response in
                    guard response == .OK, let url = panel.url else { return }
                    do { try formatted.write(to: url, options: .atomic) }
                    catch { self?.show(error) }
                }
            } catch { show(error) }
        }
    }
    @objc private func removeSelected() {
        guard let item = selected, let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Remove %@?", item.name)
        alert.informativeText = L("This skill becomes unavailable to the bot. Its revision history records the removal.")
        alert.addButton(withTitle: L("Remove"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            Task { [weak self] in
                guard let self else { return }
                do {
                    // Use the version the list displayed; no silent deletion of an intervening edit.
                    let fresh = try await store.playbook(item.id, in: item.scope)
                    guard fresh.revision == item.revision, fresh.hash == item.hash else {
                        throw CLIClient.RequestError(message: L("Playbook changed since you opened it; reload before removing"))
                    }
                    try await store.removePlaybook(fresh)
                    reload()
                } catch { show(error) }
            }
        }
    }
    private func show(_ error: Error) { status.stringValue = error.localizedDescription; status.textColor = .systemRed }
    func numberOfRows(in tableView: NSTableView) -> Int { items.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let item = items[row]
        let title = Build.label(item.name + (item.status == "draft" ? " · " + L("Draft") : ""), font: .systemFont(ofSize: 13, weight: .medium))
        let detail = Build.label(item.description, font: Theme.Font.caption, color: .secondaryLabelColor)
        let stack = Build.stack([title, detail], spacing: 3)
        stack.setAccessibilityLabel(item.name + ". " + item.description)
        return stack
    }
}

/// Explicitly selected evidence goes into a draft; this sheet has no automatic activation.
final class CapturePlaybookViewController: SheetViewController {
    private let store = AppStore.shared
    private let chat: Chat
    private let botID: Bot.ID
    private let kind: String
    private let scopes: [(PlaybookScope, String)]
    private let scopePicker = NSPopUpButton()
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private var picks: [(Message, NSButton)] = []

    init(chat: Chat, message: Message) {
        self.chat = chat
        botID = message.author.botID ?? chat.owner ?? chat.botIDs.first ?? ""
        kind = message.author.isYou ? "corrections" : "workflow"
        let botLabel = L("Bot · %@", AppStore.shared.bot(botID)?.name ?? botID)
        var choices = [(PlaybookScope(kind: "bot", id: botID), botLabel)]
        if chat.isGroup { choices.append((PlaybookScope(kind: "project", id: chat.id), L("Project · %@", chat.customTitle ?? L("Group")))) }
        scopes = choices
        super.init(title: kind == "workflow" ? L("Save Workflow as Skill") : L("Propose Standing Instructions"),
                   subtitle: kind == "workflow" ? L("Select the request and successful replies to capture. Review and edit the draft before saving.")
                    : L("Select at least two related user corrections. Review the proposed standing instruction before saving; action permissions stay the same."), width: 640)
        let eligible = chat.messages.filter { $0.canBeQuoted && !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (kind == "corrections" ? $0.author.isYou : ($0.author.isYou || $0.author.botID == botID)) }
        var candidates = Array(eligible.suffix(20))
        if !candidates.contains(where: { $0.id == message.id }) { candidates.insert(message, at: 0) }
        let preceding = eligible.last(where: { $0.author.isYou && $0.createdAt < message.createdAt })?.id
        picks = candidates.map { candidate in
            let button = NSButton(checkboxWithTitle: (candidate.author.isYou ? L("You") : AppStore.shared.bot(botID)?.name ?? L("Bot"))
                                  + ": " + String(candidate.text.replacingOccurrences(of: "\n", with: " ").prefix(150)), target: nil, action: nil)
            button.state = candidate.id == message.id || (kind == "workflow" && candidate.id == preceding) ? .on : .off
            button.toolTip = candidate.text
            return (candidate, button)
        }
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        scopePicker.addItems(withTitles: scopes.map { $0.1 })
        contentStack.addArrangedSubview(scopePicker)
        let rows = Build.stack(picks.map { $0.1 }, spacing: 10)
        for (_, button) in picks {
            button.cell?.lineBreakMode = .byTruncatingTail
            button.widthAnchor.constraint(equalTo: rows.widthAnchor).isActive = true
        }
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(rows)
        let scroll = NSScrollView()
        scroll.documentView = document
        scroll.hasVerticalScroller = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(scroll)
        contentStack.addArrangedSubview(status)
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor), scroll.heightAnchor.constraint(equalToConstant: 270),
            document.widthAnchor.constraint(equalTo: scroll.widthAnchor),
            rows.topAnchor.constraint(equalTo: document.topAnchor, constant: 8), rows.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -8),
            rows.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 8), rows.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -8),
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor)
        ])
        setButtons(confirm: L("Draft for Review"))
    }
    override func confirmTapped() {
        let ids = picks.filter { $0.1.state == .on }.map { $0.0.id }
        if ids.isEmpty || ids.count > 20 || (kind == "corrections" && ids.count < 2) {
            status.stringValue = kind == "corrections" ? L("Select 2–20 related user corrections") : L("Select 1–20 source messages")
            return
        }
        let (scope, label) = scopes[max(0, scopePicker.indexOfSelectedItem)]
        confirmButton.isEnabled = false
        scopePicker.isEnabled = false
        status.stringValue = L("Drafting…")
        Task { [weak self] in
            guard let self else { return }
            defer { confirmButton.isEnabled = true; scopePicker.isEnabled = true }
            do {
                let record = try await store.capturePlaybook(scope: scope, botID: botID, chatID: chat.id, kind: kind, messageIDs: ids)
                status.stringValue = L("Draft created. It remains in Playbooks until saved or removed.")
                let editor = PlaybookViewController(scope: scope, scopeName: label, record: record)
                editor.onSaved = { [weak self] in self?.dismiss(nil) }
                presentAsSheet(editor)
            } catch { status.stringValue = error.localizedDescription; status.textColor = .systemRed }
        }
    }
}
