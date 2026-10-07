import AppKit

/// The CLI owns project scope, immutable corrections, source freshness, and encrypted sync.
final class ProjectContextViewController: SheetViewController {
    private typealias Page = ProjectContextWire.Page
    private typealias Entry = ProjectContextWire.Entry
    private typealias Source = ProjectContextWire.Source
    private typealias AssetPath = ProjectContextWire.AssetPath

    private let chatID: Chat.ID
    private let store = AppStore.shared
    private let picker = NSPopUpButton()
    private let kindPicker = NSPopUpButton()
    private let statusPicker = NSPopUpButton()
    private let titleField = NSTextField(string: "")
    private let sourceField = NSTextField(string: "")
    private let urlField = NSTextField(string: "")
    private let textView = NSTextView()
    private let metadata = Build.label("", font: .systemFont(ofSize: 11), color: .secondaryLabelColor, lines: 0)
    private let gauge = Build.label("", font: .systemFont(ofSize: 11), color: .secondaryLabelColor, lines: 0)
    private let history = NSButton(checkboxWithTitle: L("Show revision history"), target: nil, action: nil)
    private let refreshButton = NSButton(title: L("Refresh source"), target: nil, action: nil)
    private let openButton = NSButton(title: L("Open asset"), target: nil, action: nil)
    private let removeButton = NSButton(title: L("Remove"), target: nil, action: nil)
    private let reloadButton = NSButton(title: L("Reload"), target: nil, action: nil)
    private let kinds = ["brief", "goal", "constraint", "decision", "fact", "document", "asset"]
    private let statuses = ["agreed", "verified", "unverified"]
    private var entries: [Entry] = []
    private var revision = ""
    private var busy = false
    private var needsReload = false
    private var selected: Entry? {
        let index = picker.indexOfSelectedItem - 1
        return entries.indices.contains(index) ? entries[index] : nil
    }

    init(chatID: Chat.ID) {
        self.chatID = chatID
        super.init(title: L("Project context"), subtitle: L("Shared with this group's bots. Corrections keep source and revision history."), width: 580)
    }
    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        picker.target = self; picker.action = #selector(selectionChanged)
        kindPicker.addItems(withTitles: [L("Brief"), L("Goal"), L("Constraint"), L("Decision"), L("Fact"), L("Document link"), L("Reference asset")])
        statusPicker.addItems(withTitles: [L("Agreed"), L("Verified"), L("Unverified")])
        history.target = self; history.action = #selector(reloadTapped)
        titleField.placeholderString = L("Title")
        sourceField.placeholderString = L("Source or decision author")
        urlField.placeholderString = "https://…"
        let fields: [(String, NSView)] = [(L("Entry"), picker), (L("Type"), kindPicker), (L("Title"), titleField), (L("Source"), sourceField), (L("Document URL"), urlField), (L("Status"), statusPicker)]
        for (label, control) in fields {
            let row = Build.stack([Build.label(label, font: .systemFont(ofSize: 12)), control], orientation: .horizontal, spacing: 12)
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            control.widthAnchor.constraint(greaterThanOrEqualToConstant: 350).isActive = true
        }
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder; scroll.documentView = textView
        textView.isRichText = false
        textView.isAutomaticQuoteSubstitutionEnabled = false; textView.isAutomaticDashSubstitutionEnabled = false
        textView.font = .systemFont(ofSize: 12); textView.textContainerInset = NSSize(width: 8, height: 8)
        textView.isVerticallyResizable = true; textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]; textView.textContainer?.widthTracksTextView = true
        textView.delegate = self
        contentStack.addArrangedSubview(scroll)
        scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        scroll.heightAnchor.constraint(equalToConstant: 210).isActive = true
        contentStack.addArrangedSubview(gauge); contentStack.addArrangedSubview(metadata)
        for button in [refreshButton, openButton, removeButton, reloadButton] { button.bezelStyle = .rounded; button.target = self }
        refreshButton.action = #selector(refreshTapped); openButton.action = #selector(openTapped)
        removeButton.action = #selector(removeTapped); reloadButton.action = #selector(reloadTapped)
        let attach = NSButton(title: L("Add reference file…"), target: self, action: #selector(attachTapped))
        attach.bezelStyle = .rounded
        contentStack.addArrangedSubview(Build.stack([refreshButton, openButton, removeButton, reloadButton, attach], orientation: .horizontal, spacing: 8))
        contentStack.addArrangedSubview(history)
        setButtons(confirm: L("Save"), cancel: L("Close"))
        loadEntries()
    }

    private func loadEntries(select id: String? = nil) {
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do {
                var all: [Entry] = []
                var after: String?
                var page: Page
                repeat {
                    var params: [String: Any] = ["chat_id": self.chatID, "history": self.history.state == .on, "limit": 100]
                    if let after { params["after"] = after }
                    page = try await self.store.client.request("projects.get", params, as: Page.self)
                    all += page.entries; after = page.entries.last?.id
                } while page.hasMore && after != nil
                self.entries = all.sorted { $0.updatedAt > $1.updatedAt }
                self.revision = page.revision; self.needsReload = false
                self.picker.removeAllItems(); self.picker.addItem(withTitle: L("New entry"))
                self.picker.addItems(withTitles: self.entries.map { ($0.current ? "" : "↳ ") + $0.title + " · " + self.freshnessName($0.freshness) })
                if let id, let index = self.entries.firstIndex(where: { $0.id == id }) { self.picker.selectItem(at: index + 1) }
                else { self.picker.selectItem(at: 0) }
                self.setBusy(false); self.selectionChanged()
                if !page.conflicts.isEmpty { self.metadata.stringValue += "\n" + L("Concurrent corrections are visible. Review each current version before deciding which to keep.") }
            } catch { self.setBusy(false); self.needsReload = true; self.confirmButton.isEnabled = false; self.showError(error) }
        }
    }

    @objc private func selectionChanged() {
        let entry = selected
        titleField.stringValue = entry?.title ?? ""; textView.string = entry?.text ?? ""
        sourceField.stringValue = entry?.source.label ?? L("User"); urlField.stringValue = entry?.source.url ?? ""
        if entry?.kind == "asset" {
            if kindPicker.numberOfItems == 6 { kindPicker.addItem(withTitle: L("Reference asset")) }
        } else if kindPicker.numberOfItems == 7 { kindPicker.removeItem(at: 6) }
        kindPicker.selectItem(at: kinds.firstIndex(of: entry?.kind ?? "brief") ?? 0)
        statusPicker.selectItem(at: statuses.firstIndex(of: entry?.verification ?? "agreed") ?? 2)
        if let entry {
            let when = Date(timeIntervalSince1970: TimeInterval(entry.updatedAt)).formatted(date: .abbreviated, time: .shortened)
            metadata.stringValue = "\(freshnessName(entry.freshness)) · \(entry.source.label) · \(when)"
            if let at = entry.verifiedAt { metadata.stringValue += "\n" + L("Verified: %@", Date(timeIntervalSince1970: TimeInterval(at)).formatted()) }
            if let at = entry.fetchedAt { metadata.stringValue += "\n" + L("Fetched: %@", Date(timeIntervalSince1970: TimeInterval(at)).formatted()) }
            if let error = entry.refreshError { metadata.stringValue += "\n" + error }
            if let asset = entry.asset { metadata.stringValue += "\n" + asset.name }
        } else { metadata.stringValue = L("Fetched sources are evidence to verify. Select Agreed for decisions the user accepts.") }
        kindPicker.isEnabled = !busy && entry == nil
        refreshButton.isEnabled = !busy && entry?.current == true && entry?.source.url != nil
        openButton.isEnabled = !busy && entry?.current == true && entry?.asset != nil
        removeButton.isEnabled = !busy && entry?.current == true
        confirmButton.isEnabled = !busy && !needsReload && (entry == nil || entry?.current == true)
        updateGauge()
    }
    private func setBusy(_ value: Bool) {
        busy = value
        for button in [confirmButton, picker, kindPicker, refreshButton, openButton, removeButton, reloadButton, history] { button.isEnabled = !value }
    }
    private func freshnessName(_ state: String) -> String {
        switch state {
        case "agreed": return L("Agreed")
        case "verified": return L("Verified")
        case "fetched": return L("Fetched")
        case "stale": return L("Stale")
        case "unavailable": return L("Unavailable")
        default: return L("Unverified")
        }
    }
    private func updateGauge() {
        let count = textView.string.utf8.count
        gauge.stringValue = L("%d / %d bytes · bots discover up to %d bytes each turn", count, 32_000, 8_000)
        gauge.textColor = count > 32_000 ? .systemRed : .secondaryLabelColor
    }
    private func showError(_ error: Error) {
        guard let window = view.window else { metadata.stringValue = error.localizedDescription; return }
        let alert = NSAlert(); alert.messageText = L("Couldn't update project context"); alert.informativeText = error.localizedDescription
        alert.beginSheetModal(for: window, completionHandler: nil)
    }
    override func confirmTapped() { save(removing: false) }
    @objc private func removeTapped() { save(removing: true) }
    private func save(removing: Bool) {
        guard !busy else { return }
        let entry = selected
        var source = entry?.source ?? Source(kind: "user", label: sourceField.stringValue)
        source.label = sourceField.stringValue
        let url = urlField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        if source.url != (url.isEmpty ? nil : url) { source = Source(kind: url.isEmpty ? "user" : "url", label: sourceField.stringValue, url: url.isEmpty ? nil : url) }
        guard let sourceJSON = try? source.parameters() else { return }
        var params: [String: Any] = ["chat_id": chatID, "kind": entry?.kind ?? kinds[kindPicker.indexOfSelectedItem], "title": titleField.stringValue, "text": textView.string, "source": sourceJSON, "verification": statuses[statusPicker.indexOfSelectedItem], "removed": removing]
        if let entry { params["supersedes"] = [entry.id]; params["expected_revision"] = revision; if let age = entry.maxAgeSecs { params["max_age_secs"] = age } }
        perform("projects.save", params, selectResult: !removing)
    }
    private func perform(_ method: String, _ params: [String: Any], selectResult: Bool = true) {
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do {
                let data = try await self.store.client.request(method, params)
                let result = try JSONSerialization.jsonObject(with: data) as? [String: Any]
                self.loadEntries(select: selectResult ? result?["id"] as? String : nil)
            } catch { self.setBusy(false); self.selectionChanged(); self.showError(error) }
        }
    }
    @objc private func reloadTapped() { loadEntries(select: selected?.id) }
    @objc private func refreshTapped() {
        guard let entry = selected, !busy else { return }
        perform("projects.refresh", ["chat_id": chatID, "entry_id": entry.id])
    }
    @objc private func openTapped() {
        guard let entry = selected, !busy else { return }
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do {
                let path = try await self.store.client.request("projects.asset_path", ["chat_id": self.chatID, "entry_id": entry.id], as: AssetPath.self)
                NSWorkspace.shared.open(URL(fileURLWithPath: path.path))
            } catch { self.showError(error) }
            self.setBusy(false); self.selectionChanged()
        }
    }
    @objc private func attachTapped() {
        guard !busy, let window = view.window else { return }
        let panel = NSOpenPanel(); panel.canChooseDirectories = false; panel.allowsMultipleSelection = false
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            self.perform("projects.asset", ["chat_id": self.chatID, "file": ["path": url.path]])
        }
    }
}

extension ProjectContextViewController: NSTextViewDelegate {
    func textDidChange(_ notification: Notification) { updateGauge() }
}
