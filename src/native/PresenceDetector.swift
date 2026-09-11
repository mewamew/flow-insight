import Foundation
import Vision
import CoreImage

// Images never leave this process and are not retained by the detector.
// This detects a visible person, not identity, attention, mood, or health.
enum PresenceDetector {
    static func detect(_ image: CGImage) throws -> [String: Any] {
        let context = CIContext()
        let ci = CIImage(cgImage: image)
        let average = ci.applyingFilter("CIAreaAverage", parameters: [kCIInputExtentKey: CIVector(cgRect: ci.extent)])
        var rgba = [UInt8](repeating: 0, count: 4)
        context.render(average, toBitmap: &rgba, rowBytes: 4, bounds: CGRect(x: 0, y: 0, width: 1, height: 1), format: .RGBA8, colorSpace: CGColorSpaceCreateDeviceRGB())
        let brightness = (Double(rgba[0]) + Double(rgba[1]) + Double(rgba[2])) / 765
        let at = Int64(Date().timeIntervalSince1970 * 1000)
        guard brightness > 0.06 && brightness < 0.97 else {
            return ["state":"unknown", "confidence":0.0, "observed_at":at, "reason":"光线不足、过曝或镜头被遮挡，无法判断", "regions":[[String: Any]]()]
        }
        let body = VNDetectHumanRectanglesRequest()
        body.upperBodyOnly = true
        let face = VNDetectFaceRectanglesRequest()
        try VNImageRequestHandler(cgImage: image, options: [:]).perform([body, face])
        let bodies = body.results ?? [], faces = face.results ?? []
        let scores = bodies.map { $0.confidence } + faces.map { $0.confidence }
        let confidence = scores.max() ?? 0
        // Only where a person is in the frame leaves the detector, as normalized boxes; never what the frame shows.
        let regions = regionList(faces, kind: "face") + regionList(bodies, kind: "body")
        if confidence >= 0.5 {
            return ["state":"present", "confidence":Double(confidence), "observed_at":at, "reason":"本地检测到人脸或上半身；不代表专注", "regions":regions]
        }
        if !scores.isEmpty {
            return ["state":"unknown", "confidence":Double(confidence), "observed_at":at, "reason":"检测结果不确定", "regions":regions]
        }
        return ["state":"not_detected", "confidence":0.0, "observed_at":at, "reason":"本帧未检测到人；需要结合持续时间与键鼠活动", "regions":regions]
    }
    // Vision boxes are normalized with a bottom-left origin; the page expects top-left.
    private static func regionList(_ observations: [VNDetectedObjectObservation], kind: String) -> [[String: Any]] {
        let unit = { (value: CGFloat) -> Double in (Double(min(max(value, 0), 1)) * 1000).rounded() / 1000 }
        return observations.prefix(4).map { observation in
            let box = observation.boundingBox
            return ["kind":kind, "x":unit(box.minX), "y":unit(1 - box.maxY), "w":unit(box.width), "h":unit(box.height), "confidence":unit(CGFloat(observation.confidence))]
        }
    }
}
