import UIKit

/// Pair this device with the host its user runs.
///
/// This screen used to start a WorkOS web-auth session against a fixed edge:
/// the system sheet returned a code, the app exchanged it for an access/refresh
/// pair, and the host was a constant in this file. RCode has no account and no
/// hosted service — the host is one the user runs and reaches through a tunnel
/// they set up, and the bearer is one that host was told to expect — so the
/// screen asks for both and stores them.
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
        let tagline = UILabel()
        tagline.text = "Your coding agents, from anywhere."
        tagline.font = Fonts.ui(.sans, 17)
        tagline.textColor = Palette.secondary

        // The host is an address the user's own tunnel exposes; the token is
        // the one the desktop app shows for pairing.
        hostField.placeholder = "Host, e.g. https://laptop.tailnet.ts.net:27655"
        hostField.accessibilityIdentifier = "pair-host"
        hostField.borderStyle = .roundedRect
        hostField.autocapitalizationType = .none
        hostField.autocorrectionType = .no
        hostField.keyboardType = .URL
        hostField.textContentType = .URL
        hostField.text = Self.lastHost

        tokenField.placeholder = "Pairing token"
        tokenField.accessibilityIdentifier = "pair-token"
        tokenField.borderStyle = .roundedRect
        tokenField.autocapitalizationType = .none
        tokenField.autocorrectionType = .no

        var primary = UIButton.Configuration.filled()
        primary.title = "Pair"
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

        let top = UIStackView(arrangedSubviews: [mark, title, tagline])
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
        // The token is `user@org`: that is the bearer the desktop host expects,
        // and its two halves also name the store this device caches into.
        let token = (tokenField.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: host), url.scheme == "http" || url.scheme == "https", url.host != nil else {
            status.text = "That host is not a URL."
            return
        }
        let parts = token.split(separator: "@", maxSplits: 1, omittingEmptySubsequences: false)
        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty else {
            status.text = "The token looks like `user@org`."
            return
        }
        status.text = nil
        pair.configuration?.showsActivityIndicator = true
        Task { @MainActor in
            let started = app.pair(host: host, userId: String(parts[0]), orgId: String(parts[1]))
            pair.configuration?.showsActivityIndicator = false
            if !started {
                status.text = "Could not reach that host with that token."
            }
        }
    }
}
