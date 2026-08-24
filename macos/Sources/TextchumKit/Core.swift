import CTextchum

/// Namespace for information about the linked core library.
public enum Core {
    /// Version of the compiled core (semver).
    public static var version: String {
        String(cString: tc_version())
    }
}

/// An edit or query the core validated and rejected.
///
/// The core's contract is transactional: a rejected operation changed
/// nothing, so callers can surface the error and carry on.
public struct CoreRejectedOperation: Error, CustomStringConvertible {
    public let operation: String

    public var description: String {
        "core rejected operation: \(operation)"
    }
}
