import CTextchum
import Foundation

/// Where local history keeps the saved versions of files, and dropping
/// the old ones. The versions themselves are read through the document.
public enum CoreLocalHistory {
    /// Where versions are kept from now on; nil keeps none.
    public static func setDirectory(_ path: String?) {
        let path = path ?? ""
        path.withCString { pointer in
            tc_local_history_set_dir(pointer, UInt(strlen(pointer)))
        }
    }

    /// Drops versions older than local history keeps them; how many went.
    @discardableResult
    public static func sweep() -> Int {
        Int(tc_local_history_sweep())
    }
}
