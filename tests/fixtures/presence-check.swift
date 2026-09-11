import AppKit
import Foundation
@main enum Check {
    static func main() throws {
        for path in CommandLine.arguments.dropFirst() {
            guard let image = NSImage(contentsOfFile:path)?.cgImage(forProposedRect:nil, context:nil, hints:nil) else { fatalError("fixture unavailable") }
            let result = try PresenceDetector.detect(image)
            // Emit only detector results, never image contents or local paths.
            let data=try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys])
            print(String(data:data,encoding:.utf8)!)
        }
    }
}
