import CTextchum
import Foundation

/// What the Markdown preview does with a link.
///
/// A click would take the pane somewhere else with no way back, so
/// links go to the browser. The exception is a link into the page
/// already on screen, which is a place to scroll to. The rule is the
/// core's, so both previews treat the same links the same way.
public enum CorePreview {
    public static func isPlaceInPage(here: String, target: String) -> Bool {
        here.withCString { herePointer in
            target.withCString { targetPointer in
                tc_preview_is_place_in_page(
                    herePointer, UInt(strlen(herePointer)),
                    targetPointer, UInt(strlen(targetPointer)))
            }
        }
    }
}
