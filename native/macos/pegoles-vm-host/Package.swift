// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "pegoles-vm-host",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "pegoles-vm-host", targets: ["pegoles-vm-host"]),
    ],
    targets: [
        .executableTarget(
            name: "pegoles-vm-host",
            path: "Sources"
        ),
    ],
    swiftLanguageVersions: [.v5]
)
