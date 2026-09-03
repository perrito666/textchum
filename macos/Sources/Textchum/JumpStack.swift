import Foundation

/// One remembered position: a file and an LSP-style caret.
struct JumpLocation: Equatable {
    let path: String
    let line: Int
    let character: Int
}

/// The jump stack — vim's jumplist by another name. Every jump (go to
/// definition, a search or reference result, an outline entry, a chum
/// open) records where it left from; Go Back walks those origins, Go
/// Forward retraces. A fresh jump rewrites the future: the forward
/// history is discarded, because the trail now continues from here.
@MainActor
final class JumpStack {
    private var back: [JumpLocation] = []
    private var forward: [JumpLocation] = []
    private static let capacity = 100

    var canGoBack: Bool { !back.isEmpty }
    var canGoForward: Bool { !forward.isEmpty }
    /// The places left behind, the most recent first.
    var backTrail: [JumpLocation] { back.reversed() }
    /// The places gone back from, the nearest first.
    var forwardTrail: [JumpLocation] { forward.reversed() }

    /// Records `origin` as the place a jump left from.
    func noteJump(from origin: JumpLocation) {
        forward.removeAll()
        if back.last != origin {
            back.append(origin)
        }
        if back.count > Self.capacity {
            back.removeFirst()
        }
    }

    /// Where to go back to, exchanging `current` into the forward trail.
    func goBack(from current: JumpLocation) -> JumpLocation? {
        guard let target = back.popLast() else { return nil }
        forward.append(current)
        return target
    }

    /// Where to go forward to, exchanging `current` into the back trail.
    func goForward(from current: JumpLocation) -> JumpLocation? {
        guard let target = forward.popLast() else { return nil }
        back.append(current)
        return target
    }
}
