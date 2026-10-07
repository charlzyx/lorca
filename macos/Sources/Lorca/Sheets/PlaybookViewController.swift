import AppKit

/// Native authoring and history. The reviewed Save action is the only activation path.
final class PlaybookViewController: SheetViewController {
    private let store = AppStore.shared
    private let scope: PlaybookScope
    private var record: PlaybookRecord?
    private let name = NSTextField()
    private let summary = NSTextField()
    private let instructions = NSTextView()
    private let examples = NSTextView()
    private var references: PlaybookResourcesView!
    private var scripts: PlaybookResourcesView!
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    var onSaved: (() -> Void)?

    init(scope: PlaybookScope, scopeName: String, record: PlaybookRecord? = nil) {
        self.scope = scope
        self.record = record
        super.init(title: record?.status == "draft" ? L("Review Skill Draft") : L("Edit Skill"),
                   subtitle: L("Scope: %@. Save makes this skill available on later turns. Instructions and scripts keep the bot's usual action permissions.", scopeName), width: 640)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let content = record?.content ?? PlaybookContent()
        name.placeholderString = L("Skill name (for example, weekly-review)")
        summary.placeholderString = L("Describe when to use this skill")
        name.stringValue = content.name
        summary.stringValue = content.description
        for (label, field) in [(L("Name"), name), (L("Description"), summary)] {
            let row = Build.stack([Build.label(label, font: Theme.Font.caption), field], spacing: 5)
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            field.widthAnchor.constraint(equalTo: row.widthAnchor).isActive = true
        }
        instructions.string = content.instructions
        examples.string = content.examples
        references = PlaybookResourcesView(kind: "references", resources: content.references)
        scripts = PlaybookResourcesView(kind: "scripts", resources: content.scripts)
        let tabs = NSTabView(frame: NSRect(x: 0, y: 0, width: 600, height: 330))
        tabs.translatesAutoresizingMaskIntoConstraints = false
        for (title, page) in [(L("Instructions"), Self.editor(instructions)), (L("Examples"), Self.editor(examples)),
                              (L("References"), references as NSView), (L("Scripts"), scripts as NSView)] {
            let item = NSTabViewItem(identifier: title)
            item.label = title
            item.view = page
            tabs.addTabViewItem(item)
        }
        if let record {
            let history = NSTextView()
            history.isEditable = false
            history.string = record.revisions.sorted { $0.revision > $1.revision }.map { revision in
                let date = Date(timeIntervalSince1970: revision.created_at).formatted()
                let evidence = revision.provenance.message_ids.isEmpty ? "" : "\n" + L("Source messages: %@", revision.provenance.message_ids.joined(separator: ", "))
                return L("Revision %d · %@ · %@", revision.revision, date, revision.status)
                    + "\n" + revision.provenance.kind + " · " + revision.provenance.note + evidence
                    + "\n\n" + (revision.content?.instructions ?? L("Removed"))
            }.joined(separator: "\n\n────────────\n\n")
            let item = NSTabViewItem(identifier: "history")
            item.label = L("History")
            item.view = Self.editor(history)
            tabs.addTabViewItem(item)
            status.stringValue = record.status == "draft" ? L("Draft · unavailable to the bot until you save") : L("Revision %d", record.revision)
        } else { status.stringValue = L("New skill · instructions are required") }
        contentStack.addArrangedSubview(tabs)
        contentStack.addArrangedSubview(status)
        NSLayoutConstraint.activate([
            tabs.widthAnchor.constraint(equalTo: contentStack.widthAnchor), tabs.heightAnchor.constraint(equalToConstant: 330),
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor)
        ])
        setButtons(confirm: L("Save Skill"))
    }

    static func editor(_ text: NSTextView) -> NSScrollView {
        text.isRichText = false
        text.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        text.textColor = .labelColor
        text.isAutomaticQuoteSubstitutionEnabled = false
        text.isAutomaticDashSubstitutionEnabled = false
        text.isAutomaticTextReplacementEnabled = false
        text.allowsUndo = true
        text.textContainerInset = NSSize(width: 8, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        let scroll = NSScrollView()
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        scroll.translatesAutoresizingMaskIntoConstraints = false
        return scroll
    }

    override func confirmTapped() {
        let content = PlaybookContent(name: name.stringValue, description: summary.stringValue, instructions: instructions.string,
                                      examples: examples.string, references: references.value, scripts: scripts.value)
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await store.savePlaybook(content, in: scope, previous: record)
                onSaved?()
                dismiss(nil)
            } catch {
                confirmButton.isEnabled = true
                status.stringValue = error.localizedDescription
                status.textColor = .systemRed
                if error.localizedDescription.contains("changed since") { resolveConflict() }
            }
        }
    }

    private func resolveConflict() {
        let alert = NSAlert()
        alert.messageText = L("This skill changed while you were editing")
        alert.informativeText = L("Keep your draft to copy it, or reload the current revision before editing again.")
        alert.addButton(withTitle: L("Keep Draft"))
        alert.addButton(withTitle: L("Reload"))
        guard let window = view.window, let record else { return }
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertSecondButtonReturn else { return }
            Task { [weak self] in
                guard let self else { return }
                do {
                    let fresh = try await store.playbook(record.id, in: scope)
                    self.record = fresh
                    guard let content = fresh.content else { status.stringValue = L("This skill was removed"); return }
                    name.stringValue = content.name
                    summary.stringValue = content.description
                    instructions.string = content.instructions
                    examples.string = content.examples
                    references.replace(content.references)
                    scripts.replace(content.scripts)
                    status.stringValue = L("Revision %d", fresh.revision)
                    status.textColor = .secondaryLabelColor
                } catch { status.stringValue = error.localizedDescription }
            }
        }
    }
}

/// A reference/script is an explicitly named bundled text file. Editing never reads or
/// writes a path on disk, and selecting a script never runs it.
final class PlaybookResourcesView: NSView {
    private let kind: String
    private var resources: [PlaybookResource]
    private var selected: Int?
    private let picker = NSPopUpButton()
    private let path = NSTextField()
    private let text = NSTextView()

    var value: [PlaybookResource] { commit(); return resources }

    init(kind: String, resources: [PlaybookResource]) {
        self.kind = kind
        self.resources = resources
        super.init(frame: .zero)
        let add = NSButton(title: L("Add File"), target: self, action: #selector(addFile))
        let remove = NSButton(title: L("Remove File"), target: self, action: #selector(removeFile))
        picker.target = self
        picker.action = #selector(pickFile)
        let row = Build.stack([picker, add, remove], orientation: .horizontal, spacing: 8)
        path.placeholderString = L("Relative file name (for example, checklist.md)")
        let note = Build.label(kind == "scripts" ? L("Scripts are saved as text. The bot's usual permissions apply when it runs them.") : L("Bundle reusable reference text with this skill."),
                               font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
        let editor = PlaybookViewController.editor(text)
        let stack = Build.stack([row, path, editor, note], spacing: 8)
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8), stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 8), stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            row.widthAnchor.constraint(equalTo: stack.widthAnchor), path.widthAnchor.constraint(equalTo: stack.widthAnchor),
            editor.widthAnchor.constraint(equalTo: stack.widthAnchor), editor.heightAnchor.constraint(greaterThanOrEqualToConstant: 180),
            note.widthAnchor.constraint(equalTo: stack.widthAnchor)
        ])
        refresh(select: resources.isEmpty ? nil : 0)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    func replace(_ resources: [PlaybookResource]) {
        self.resources = resources
        refresh(select: resources.isEmpty ? nil : 0)
    }
    private func commit() {
        guard let selected, resources.indices.contains(selected) else { return }
        resources[selected] = PlaybookResource(path: kind + "/" + path.stringValue, text: text.string)
    }
    private func refresh(select index: Int?) {
        selected = index
        picker.removeAllItems()
        picker.addItems(withTitles: resources.map(\.path))
        if let index {
            picker.selectItem(at: index)
            path.stringValue = String(resources[index].path.dropFirst(kind.count + 1))
            text.string = resources[index].text
        } else { path.stringValue = ""; text.string = "" }
        path.isEnabled = index != nil
        text.isEditable = index != nil
    }
    @objc private func pickFile() { commit(); refresh(select: picker.indexOfSelectedItem >= 0 ? picker.indexOfSelectedItem : nil) }
    @objc private func addFile() {
        commit()
        let base = kind == "scripts" ? "script" : "reference"
        resources.append(PlaybookResource(path: "\(kind)/\(base)-\(UUID().uuidString.prefix(6).lowercased()).\(kind == "scripts" ? "sh" : "md")", text: ""))
        refresh(select: resources.count - 1)
    }
    @objc private func removeFile() {
        guard let selected else { return }
        resources.remove(at: selected)
        refresh(select: resources.isEmpty ? nil : min(selected, resources.count - 1))
    }
}
