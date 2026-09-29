// swift-tools-version: 6.0
import PackageDescription
let package = Package(
    name: "Lightmail",
    platforms: [.macOS(.v15)],
    products: [.executable(name: "Lightmail", targets: ["Lightmail"])],
    targets: [
        .systemLibrary(name: "lightmail_coreFFI", path: "Generated/FFI"),
        .executableTarget(name: "Lightmail", dependencies: ["lightmail_coreFFI"], path: "Sources/Lightmail", linkerSettings: [.linkedFramework("Security"), .linkedFramework("SystemConfiguration")])
    ],
    swiftLanguageModes: [.v5]
)
