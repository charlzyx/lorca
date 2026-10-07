import AppKit

/// The accounts of one marketplace service on the selected Runner. Opening a row chooses its
/// stable installed instance; adding an account never replaces another account's sign-in.
final class PluginAccountsViewController: SheetViewController {
    private let store = AppStore.shared
    private let serviceID: String
    private let serviceName: String
    private let runner: Device
    private let accounts = SectionView(title: L("Accounts"))
    private let addButton = NSButton()
    private var added: [InstalledPlugin] = []

    init(serviceID: String, name: String, runner: Device) {
        self.serviceID = serviceID
        serviceName = name
        self.runner = runner
        super.init(title: name, subtitle: L("Accounts on %@. Choose a name such as Work or Personal.", runner.name), width: 520)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        contentStack.addArrangedSubview(accounts)
        accounts.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        addButton.title = L("Add Account…")
        addButton.bezelStyle = .rounded
        addButton.target = self
        addButton.action = #selector(addAccount)
        contentStack.addArrangedSubview(addButton)
        setButtons(confirm: L("Done"), cancel: nil)
        render()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.render()
            default: break
            }
        }
    }

    private func render() {
        let current = (store.device(runner.id) ?? runner).plugins.filter { $0.serviceID == serviceID }
        // The sealed install reply can precede the Runner's encrypted machine advertisement.
        added.removeAll { fresh in current.contains { $0.id == fresh.id } }
        let plugins = current + added
        var rows: [NSView] = plugins.map { plugin in
            let row = ActionRow(key: plugin.accountName ?? plugin.name, value: plugin.detail,
                                tint: plugin.stateColor, actionTitle: L("Manage…"))
            row.onAction = { [weak self] in
                guard let self else { return }
                PluginViewController.present(pluginID: plugin.id, runner: self.store.device(self.runner.id) ?? self.runner, bot: nil, from: self)
            }
            return row
        }
        if rows.isEmpty { rows = [KeyValueRow(key: L("Accounts"), value: L("No accounts connected"), tint: .secondaryLabelColor)] }
        accounts.setRows(rows)
        fitSheetToContent()
    }

    @objc private func addAccount() {
        AccountNameSheet.show(serviceName: serviceName, from: self) { [weak self] name in
            guard let self else { return }
            self.addButton.isEnabled = false
            Task { [weak self] in
                guard let self else { return }
                defer { self.addButton.isEnabled = true }
                do {
                    let status = try await self.store.installPlugin(self.serviceID, on: self.runner.id, accountName: name)
                    self.added.append(status)
                    self.render()
                    PluginViewController.present(pluginID: status.id, runner: self.store.device(self.runner.id) ?? self.runner, bot: nil, from: self)
                } catch {
                    let alert = NSAlert()
                    alert.messageText = L("Couldn't add the account")
                    alert.informativeText = error.localizedDescription
                    if let window = self.view.window { alert.beginSheetModal(for: window, completionHandler: nil) }
                }
            }
        }
    }
}

/// A native name prompt shared by the marketplace and an installed account's sheet.
enum AccountNameSheet {
    static func show(serviceName: String, from controller: NSViewController, completion: @escaping (String) -> Void) {
        guard let window = controller.view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Add an account to %@", serviceName)
        alert.informativeText = L("Name this account so your bots can choose it explicitly.")
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 300, height: 24))
        field.placeholderString = L("Work or Personal")
        field.setAccessibilityLabel(L("Account name"))
        alert.accessoryView = field
        alert.addButton(withTitle: L("Add"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { response in
            guard response == .alertFirstButtonReturn else { return }
            let name = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            // The Runner checks length, control characters, and duplicate names.
            completion(name)
        }
        alert.window.initialFirstResponder = field
    }
}
