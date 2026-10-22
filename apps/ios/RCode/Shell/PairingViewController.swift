import UIKit

/// Pair this device with the host its user runs.
///
/// The user supplies a reachable protocol endpoint and the bearer it expects.
/// The screen stores both for the Rust client's pairing configuration.
final class PairingViewController: UIViewController {
    private let app: AppModel
    private let status = UILabel()
    private let hostField = UITextField()
    private let tokenField = UITextField()
    private let pair = UIButton(type: .system)

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        let mark = UIImageView(image: UIImage(systemName: "sparkle", withConfiguration: UIImage.SymbolConfiguration(pointSize: 44, weight: .light)))
        mark.tintColor = Palette.text
        let title = UILabel()
        title.text = "RCode"
        title.font = Fonts.ui(.sansSemibold, 34)
        title.textColor = Palette.text
        hostField.placeholder = L10n.text("Server, e.g. wss://host.example:27655")
        hostField.accessibilityIdentifier = "pair-host"
        hostField.borderStyle = .roundedRect
        hostField.autocapitalizationType = .none
        hostField.autocorrectionType = .no
        hostField.keyboardType = .URL
        hostField.textContentType = .URL
        hostField.text = Self.lastHost

        tokenField.placeholder = L10n.text("API Key")
        tokenField.accessibilityLabel = L10n.text("API Key")
        tokenField.accessibilityIdentifier = "pair-token"
        tokenField.borderStyle = .roundedRect
        tokenField.autocapitalizationType = .none
        tokenField.autocorrectionType = .no
        tokenField.isSecureTextEntry = true
        tokenField.textContentType = .password

        var primary = UIButton.Configuration.filled()
        primary.title = L10n.text("Connect")
        primary.baseBackgroundColor = Palette.text
        primary.baseForegroundColor = Palette.background
        primary.cornerStyle = .capsule
        primary.contentInsets = NSDirectionalEdgeInsets(top: 15, leading: 20, bottom: 15, trailing: 20)
        primary.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { a in
            var a = a
            a.font = Fonts.ui(.sansSemibold, 17)
            a.foregroundColor = Palette.background
            return a
        }
        pair.configuration = primary
        pair.accessibilityIdentifier = "pair"
        pair.addAction(UIAction { [weak self] _ in self?.attemptPair() }, for: .touchUpInside)

        status.font = Fonts.ui(.sans, 14)
        status.textColor = Palette.danger
        status.numberOfLines = 0
        status.textAlignment = .center

        let top = UIStackView(arrangedSubviews: [mark, title])
        top.axis = .vertical
        top.alignment = .center
        top.spacing = 12
        let form = UIStackView(arrangedSubviews: [status, hostField, tokenField, pair])
        form.axis = .vertical
        form.spacing = 12
        for v in [top, form] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        NSLayoutConstraint.activate([
            top.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            top.centerYAnchor.constraint(equalTo: view.centerYAnchor, constant: -80),
            form.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            form.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            form.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -24),
        ])
    }

    /// The host from the last pairing, so re-pairing after a token change does
    /// not mean retyping an address.
    private static var lastHost: String {
        AppModel.host
    }

    private func attemptPair() {
        let host = (hostField.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let token = (tokenField.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: host), let scheme = url.scheme,
              ["http", "https", "ws", "wss"].contains(scheme), url.host != nil,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil else {
            status.text = L10n.text("That host is not a URL.")
            return
        }
        guard token.range(of: "^[A-Za-z0-9]{8,}$", options: .regularExpression) != nil,
              token.range(of: "[A-Za-z]", options: .regularExpression) != nil,
              token.range(of: "[0-9]", options: .regularExpression) != nil else {
            status.text = L10n.text("Use at least 8 characters, including letters and numbers.")
            return
        }
        status.text = nil
        pair.configuration?.showsActivityIndicator = true
        Task { @MainActor in
            let started = app.connect(host: host, apiKey: token)
            pair.configuration?.showsActivityIndicator = false
            if !started {
                status.text = L10n.text("Could not reach that host with that token.")
            }
        }
    }
}
