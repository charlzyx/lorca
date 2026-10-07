import AppKit
import UniformTypeIdentifiers

final class TemplateImportViewController: SheetViewController {
    private let store = AppStore.shared
    private let url: URL
    private let onCreate: (Chat.ID) -> Void
    private let reply: TemplateReply
    private let nameField = NSTextField()
    private let runnerPopup = NSPopUpButton()
    private let providerPopup = NSPopUpButton()
    private let accounts = Build.stack([], spacing: 6)
    private let previewText = TemplatePreviewText(height: 270)
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let reviewed = NSButton(checkboxWithTitle: L("I reviewed the contents and selected my own connections."), target: nil, action: nil)
    private var runners: [Device] = []
    private var providers: [ProviderCredential.Kind] = []
    private var mappings: [String: String] = [:]
    private var preview: TemplatePreview?
    private var generation = 0
    private var loadedName = false
    private var importing = false

    init(url: URL, reply: TemplateReply? = nil, onCreate: @escaping (Chat.ID) -> Void) {
        self.url = url
        self.onCreate = onCreate
        self.reply = reply ?? { method, params in try await AppStore.shared.templateReply(method, params) }
        super.init(title: L("Import Bot Template"),
            subtitle: L("Review %@ and choose your own Runner and connections. Import creates an independent bot with its routines paused.", url.lastPathComponent), width: 640)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        nameField.placeholderString = L("New bot name")
        nameField.delegate = self
        runners = store.runners
        for runner in runners { runnerPopup.addItem(withTitle: runner.name) }
        if let local = runners.firstIndex(where: \.isThisDevice) { runnerPopup.selectItem(at: local) }
        runnerPopup.target = self
        runnerPopup.action = #selector(runnerChanged)
        providers = store.providerKinds
        for provider in providers { providerPopup.addItem(withTitle: provider.name) }
        if let preferred = providers.firstIndex(of: store.preferredProvider) { providerPopup.selectItem(at: preferred) }
        let nameRow = field(L("Name"), nameField)
        let runnerRow = field(L("Runner"), runnerPopup)
        let providerRow = field(L("Provider"), providerPopup)
        let accountList = TemplateChoiceScroll(stack: accounts, height: 95)
        for row in [nameRow, runnerRow, providerRow, accountList, previewText, status, reviewed] as [NSView] {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        reviewed.target = self
        reviewed.action = #selector(reviewChanged)
        let refresh = NSButton(title: L("Refresh Preview"), target: self, action: #selector(refreshTapped))
        refresh.bezelStyle = .rounded
        setButtons(confirm: L("Create Independent Bot"), leading: refresh)
        confirmButton.isEnabled = false
        refreshPreview()
    }

    private func field(_ title: String, _ control: NSView) -> NSView {
        control.translatesAutoresizingMaskIntoConstraints = false
        let label = Build.label(title, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        let row = Build.stack([label, control], orientation: .horizontal, spacing: 12)
        label.widthAnchor.constraint(equalToConstant: 70).isActive = true
        control.widthAnchor.constraint(equalToConstant: 470).isActive = true
        return row
    }

    private var params: [String: Any] {
        var params: [String: Any] = ["path": url.path, "mappings": mappings]
        let index = runnerPopup.indexOfSelectedItem
        if runners.indices.contains(index) { params["runner_id"] = runners[index].id }
        if loadedName { params["name"] = nameField.stringValue }
        return params
    }

    @objc private func runnerChanged() {
        mappings.removeAll()
        refreshPreview()
    }

    @objc private func refreshTapped() { refreshPreview() }

    private func refreshPreview() {
        guard !importing else { return }
        generation += 1
        let current = generation
        reviewed.state = .off
        reviewed.isEnabled = false
        confirmButton.isEnabled = false
        status.stringValue = L("Validating the private file and recipient connections…")
        status.textColor = .secondaryLabelColor
        let request = params
        Task { [weak self] in
            guard let self else { return }
            do {
                let reply = try await self.reply("templates.import.preview", request)
                guard self.generation == current else { return }
                let preview = TemplatePreview(json: reply)
                self.preview = preview
                if !self.loadedName {
                    self.nameField.stringValue = preview.name
                    self.loadedName = true
                }
                self.previewText.text = preview.text
                self.showAccounts(preview.requirements)
                self.status.stringValue = preview.issues.isEmpty
                    ? L("Routines stay paused. Review instructions and scripts before running the new bot.")
                    : preview.issues.joined(separator: "\n")
                self.status.textColor = preview.issues.isEmpty ? .secondaryLabelColor : .systemOrange
                self.reviewed.isEnabled = preview.canImport
                self.reviewChanged()
            } catch {
                guard self.generation == current else { return }
                self.preview = nil
                self.status.stringValue = error.localizedDescription
                self.status.textColor = .systemRed
            }
        }
    }

    private func showAccounts(_ requirements: [TemplatePreview.Requirement]) {
        accounts.arrangedSubviews.forEach { accounts.removeArrangedSubview($0); $0.removeFromSuperview() }
        if requirements.isEmpty { accounts.addArrangedSubview(Build.label(L("No integration requirements"), font: Theme.Font.caption, color: .secondaryLabelColor)) }
        for requirement in requirements {
            let popup = NSPopUpButton()
            popup.addItem(withTitle: L("Choose your connection…"))
            popup.identifier = NSUserInterfaceItemIdentifier(requirement.serviceID)
            for connection in requirement.candidates {
                let title = connection.state == "ready" ? connection.name : connection.name + " · " + connection.detail
                popup.addItem(withTitle: title)
                popup.lastItem?.representedObject = connection.id
            }
            if let chosen = mappings[requirement.serviceID], let item = popup.itemArray.first(where: { $0.representedObject as? String == chosen }) {
                popup.select(item)
            }
            popup.target = self
            popup.action = #selector(accountChanged(_:))
            popup.setAccessibilityLabel(L("Connection for %@", requirement.serviceID))
            accounts.addArrangedSubview(field(requirement.serviceID, popup))
        }
    }

    @objc private func accountChanged(_ popup: NSPopUpButton) {
        guard let service = popup.identifier?.rawValue else { return }
        mappings[service] = popup.selectedItem?.representedObject as? String
        refreshPreview()
    }

    @objc private func reviewChanged() {
        confirmButton.isEnabled = preview?.canImport == true && reviewed.state == .on && !importing
    }

    override func confirmTapped() {
        guard let preview, preview.canImport, reviewed.state == .on, !importing else { return }
        importing = true
        confirmButton.isEnabled = false
        nameField.isEnabled = false
        runnerPopup.isEnabled = false
        providerPopup.isEnabled = false
        var request = params
        request["expected_digest"] = preview.digest
        request["reviewed"] = true
        let provider = providerPopup.indexOfSelectedItem
        if providers.indices.contains(provider) { request["provider"] = providers[provider].wireValue }
        Task { [weak self] in
            guard let self else { return }
            do {
                let chatID = try await self.store.importTemplate(request)
                self.dismiss(nil)
                self.onCreate(chatID)
            } catch {
                self.status.stringValue = error.localizedDescription
                self.status.textColor = .systemRed
                self.reviewed.state = .off
                self.importing = false
                self.nameField.isEnabled = true
                self.runnerPopup.isEnabled = true
                self.providerPopup.isEnabled = true
                self.reviewChanged()
            }
        }
    }

    override func cancelOperation(_ sender: Any?) { if !importing { super.cancelOperation(sender) } }
    override func dismissSheet() { if !importing { super.dismissSheet() } }
}

extension TemplateImportViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) { refreshPreview() }
}
