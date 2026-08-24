// swift-tools-version:5.10
import PackageDescription
import Foundation

// The Rust core is built separately (see the repository Makefile) into a
// static library; resolve its location relative to this manifest so builds
// work from any working directory and from editor tooling.
let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let coreLibDir = "\(packageDir)/../core/target/release"

let package = Package(
    name: "Textchum",
    platforms: [.macOS(.v14)],
    targets: [
        // Raw C interface to the core (generated header, no code).
        .target(
            name: "CTextchum"
        ),
        // Safe, idiomatic Swift API over CTextchum. Everything above this
        // layer is ordinary Swift with no pointers in sight.
        .target(
            name: "TextchumKit",
            dependencies: ["CTextchum"]
        ),
        // The macOS application.
        .executableTarget(
            name: "Textchum",
            dependencies: ["TextchumKit"],
            linkerSettings: [
                .unsafeFlags(["-L\(coreLibDir)", "-ltextchum"])
            ]
        ),
    ]
)
