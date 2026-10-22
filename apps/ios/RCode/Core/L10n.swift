import Foundation
import Darwin

/// Native labels and the Rust transcript use the app's selected localization.
enum L10n {
    static func text(_ key: String) -> String {
        NSLocalizedString(key, comment: "")
    }

    static func format(_ key: String, _ arguments: CVarArg...) -> String {
        String(format: text(key), locale: Locale.current, arguments: arguments)
    }

    static func count(_ key: String, _ count: Int) -> String {
        String.localizedStringWithFormat(text(key), count)
    }

    static func error(_ error: Error) -> String {
        guard let core = error as? CoreError else { return error.localizedDescription }
        switch core {
        case let .NotFound(message): return format("error.not-found", message)
        case let .InvalidArgument(message): return format("error.invalid-argument", message)
        case let .HostUnavailable(message): return format("error.host-unavailable", message)
        case let .Unsupported(message): return format("error.unsupported", message)
        case let .HostError(message): return format("error.host", message)
        case let .Network(message): return format("error.network", message)
        case let .Auth(message): return format("error.auth", message)
        case let .Storage(message): return format("error.storage", message)
        case let .NotImplemented(message): return format("error.not-implemented", message)
        case .Closed: return text("error.closed")
        case let .Internal(message): return format("error.internal", message)
        }
    }

    /// Set before any core worker starts; language changes take effect on launch.
    static func configureCoreLanguage() {
        let language = Bundle.main.preferredLocalizations.first ?? "en"
        setenv("RCODE_DISPLAY_LANGUAGE", language, 1)
    }
}
