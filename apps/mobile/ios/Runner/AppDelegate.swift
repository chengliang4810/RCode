import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    guard let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "RCodeImageClipboard") else { return }
    let channel = FlutterMethodChannel(
      name: "com.rcode.mobile/image_clipboard",
      binaryMessenger: registrar.messenger()
    )
    channel.setMethodCallHandler { call, result in
      guard call.method == "readImage" else {
        result(FlutterMethodNotImplemented)
        return
      }
      // This handler is invoked only by the explicit Paste image action.
      let pasteboard = UIPasteboard.general
      let maxBytes = 24 * 1024 * 1024
      var png = pasteboard.data(forPasteboardType: "public.png")
      if png == nil, let image = pasteboard.image {
        let scale = min(1, 2560 / max(image.size.width, image.size.height))
        let size = CGSize(width: max(1, image.size.width * scale), height: max(1, image.size.height * scale))
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        png = UIGraphicsImageRenderer(size: size, format: format).image { _ in
          image.draw(in: CGRect(origin: .zero, size: size))
        }.pngData()
      }
      guard let png else {
        result(nil)
        return
      }
      guard png.count <= maxBytes else {
        result(FlutterError(code: "image_too_large", message: "Image exceeds 24 MiB", details: nil))
        return
      }
      result(["bytes": FlutterStandardTypedData(bytes: png), "name": "clipboard.png"])
    }
  }
}
