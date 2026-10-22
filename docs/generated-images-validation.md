# Image parts in conversation threads

The shared protocol includes `AgentEvent::GeneratedImage` and structured image
parts. The engine can import a raster file into profile uploads before persisting
the event; desktop and iOS clients render the resulting attachment reference.
The Native Agent currently has no raster image-generation tool. These data and
rendering paths do not imply a model image-generation integration.

## Storage and display

Image import accepts PNG, JPEG, WebP and GIF within the encoded-file limit,
validates the source path and signature, and publishes the uploads reference.
The transcript loads images through the attachment RPC/cache, trying the message
owner and chat host as appropriate. Loading and unavailable states use the
existing retry path; image activation opens the shared lightbox.

Generated-image decoding validates actual MIME type, bounds source dimensions
and decoded allocation, and produces a static downsampled preview. Its cache
accounts for encoded and decoded data and is separate from ordinary attachment
aliases. History entries remain eligible for eviction.

## Validation

```sh
cargo test --locked -p rcode-proto -p rcode-doc --lib generated_image
cargo test --locked -p rcode-engine --lib generated_image
cargo test --locked -p rcode-ui --lib generated_image
cargo test --locked -p rcode-ui --lib attachments
```

The protocol, upload and UI tests cover event serialization, file validation,
import idempotence, cache policy and rendering state. They do not exercise a
live image-generation provider. iOS has `GeneratedImageTests` and transcript
layout cases that require Xcode/simulator validation; see the
[iOS README](../apps/ios/README.md).
