import AppKit
import AVFoundation
import CoreImage
import ScreenCaptureKit

struct CaptureFailure: Error { let message: String }
func failure(_ message: String) -> CaptureFailure { CaptureFailure(message: message) }

func jpeg(_ image: CGImage, maxWidth: CGFloat) throws -> String {
    let source = CIImage(cgImage: image)
    let scale = min(1, maxWidth / source.extent.width, 1440 / source.extent.height)
    let resized = source.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
    guard let cg = CIContext().createCGImage(resized, from: resized.extent),
          let data = NSBitmapImageRep(cgImage: cg).representation(using: .jpeg, properties: [.compressionFactor: 0.72]) else {
        throw failure("无法编码采样图片")
    }
    return "data:image/jpeg;base64," + data.base64EncodedString()
}

// One AVCaptureSession is kept for an entire recording, not recreated per frame.
// Session/configuration are confined to queue; latest frames are protected by lock.
final class Camera: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    private let queue = DispatchQueue(label: "activity.camera.session")
    private let frames = DispatchQueue(label: "activity.camera.frames")
    private let inference = DispatchQueue(label: "activity.camera.presence", qos: .utility)
    private let lock = NSLock()
    private let session = AVCaptureSession()
    private var configured = false
    private var deviceName = ""
    private var latest: (CVPixelBuffer, Date)?
    private var observers: [NSObjectProtocol] = []

    override init() {
        super.init()
        for name in [AVCaptureSession.wasInterruptedNotification, AVCaptureSession.runtimeErrorNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: session, queue: nil) { [weak self] _ in
                guard let self else { return }; self.lock.lock(); self.latest = nil; self.lock.unlock()
            })
        }
    }
    deinit { observers.forEach { NotificationCenter.default.removeObserver($0) } }

    private func start() async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            queue.async { [self] in
                do {
                    guard AVCaptureDevice.authorizationStatus(for: .video) == .authorized else { throw failure("摄像头权限不可用，请授权本地程序或关闭摄像头选项") }
                    if !configured {
                        guard let device = AVCaptureDevice.default(for: .video) else { throw failure("未找到摄像头") }
                        deviceName = device.localizedName
                        let input = try AVCaptureDeviceInput(device: device)
                        let output = AVCaptureVideoDataOutput()
                        output.alwaysDiscardsLateVideoFrames = true
                        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA]
                        output.setSampleBufferDelegate(self, queue: frames)
                        session.beginConfiguration()
                        session.sessionPreset = .vga640x480
                        guard session.canAddInput(input), session.canAddOutput(output) else {
                            session.commitConfiguration(); throw failure("摄像头无法配置，可能正被其他程序占用")
                        }
                        session.addInput(input); session.addOutput(output)
                        session.commitConfiguration()
                        if device.activeFormat.videoSupportedFrameRateRanges.contains(where: { $0.minFrameRate <= 5 && $0.maxFrameRate >= 5 }) {
                            try device.lockForConfiguration()
                            device.activeVideoMinFrameDuration = CMTime(value: 1, timescale: 5)
                            device.activeVideoMaxFrameDuration = CMTime(value: 1, timescale: 5)
                            device.unlockForConfiguration()
                        }
                        configured = true
                    }
                    if !session.isRunning { session.startRunning() }
                    continuation.resume()
                } catch { continuation.resume(throwing: error) }
            }
        }
    }
    func stop() async {
        await withCheckedContinuation { continuation in
            queue.async { [self] in
                if session.isRunning { session.stopRunning() }
                lock.lock(); latest = nil; lock.unlock()
                continuation.resume()
            }
        }
    }
    /// Whether the capture session is currently running, so the page can say "camera on" from fact rather than from an error string.
    func status() async -> (active: Bool, device: String) {
        await withCheckedContinuation { (continuation: CheckedContinuation<(active: Bool, device: String), Never>) in
            queue.async { [self] in continuation.resume(returning: (session.isRunning, deviceName)) }
        }
    }
    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard CMSampleBufferDataIsReady(sampleBuffer), let pixel = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        lock.lock(); latest = (pixel, Date()); lock.unlock()
    }
    private func freshFrame() -> CVPixelBuffer? {
        lock.lock(); defer { lock.unlock() }
        guard let (pixel, date) = latest, Date().timeIntervalSince(date) < 2 else { return nil }
        return pixel
    }
    /// Latest frame as a small JPEG for the local page only. Never starts the session and keeps nothing.
    func snapshot(maxWidth: CGFloat) async -> String? {
        guard let pixel = freshFrame() else { return nil }
        let image = CIImage(cvPixelBuffer: pixel)
        guard let cg = CIContext().createCGImage(image, from: image.extent) else { return nil }
        return await withCheckedContinuation { (continuation: CheckedContinuation<String?, Never>) in
            inference.async { continuation.resume(returning: try? jpeg(cg, maxWidth: maxWidth)) }
        }
    }
    func presence() async throws -> [String: Any] {
        try await start()
        for _ in 0..<50 {
            if let pixel = freshFrame() {
                let image = CIImage(cvPixelBuffer: pixel)
                guard let cg = CIContext().createCGImage(image, from: image.extent) else { throw failure("摄像头画面无法读取") }
                return try await withCheckedThrowingContinuation { continuation in
                    inference.async {
                        do { continuation.resume(returning: try PresenceDetector.detect(cg)) }
                        catch { continuation.resume(throwing: error) }
                    }
                }
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        throw failure("摄像头未提供新画面，可能被占用或中断；未使用旧画面替代")
    }
}

@MainActor
final class CaptureWorker {
    let camera = Camera()
    let desktop = DesktopUI()
    var events: [[String: Any]] = []
    var since = Int64(Date().timeIntervalSince1970 * 1000)
    var observer: NSObjectProtocol?
    var observing = false
    init() {
        observer = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { [weak self] note in
            guard let app=note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else {return}
            Task { @MainActor in self?.activated(app) }
        }
    }
    func activated(_ app: NSRunningApplication) {
        guard observing else {return}
        events.append(["at":Int64(Date().timeIntervalSince1970 * 1000),"app_name":app.localizedName ?? "未知应用","bundle_id":app.bundleIdentifier ?? ""])
        if events.count>500 {events.removeFirst(events.count-500)}
    }
    func ensureCaptureAllowed(expected: String, excluded: [String]) async throws {
        let session = CGSessionCopyCurrentDictionary() as? [String: Any]
        var reason: String?
        if !CGPreflightScreenCaptureAccess() { reason = "录屏权限未授权或已撤销，后台已暂停采样" }
        else if session?["CGSSessionScreenIsLocked"] as? Bool == true || session?["kCGSessionOnConsoleKey"] as? Bool == false {
            reason = "屏幕已锁定或用户会话已切换，后台暂停采样"
        }
        let bundle = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? ""
        if excluded.contains(bundle) { reason = "当前应用已排除，暂不采集屏幕" }
        if let reason = reason {
            events = []; since = Int64(Date().timeIntervalSince1970 * 1000)
            await camera.stop(); throw failure(reason)
        }
        guard bundle == expected else { throw failure("应用刚刚切换，等待下一次稳定采样") }
    }
    func frontWindow(_ front: NSRunningApplication?) -> [String: Any]? {
        let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        return windows.first { ($0[kCGWindowOwnerPID as String] as? Int32) == front?.processIdentifier && ($0[kCGWindowLayer as String] as? Int) == 0 }
    }
    func displayForWindow(_ window: [String: Any]?, displays: [SCDisplay]) -> UInt32? {
        guard let bounds = window?[kCGWindowBounds as String] as? [String: Any],
              let frame = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return nil }
        let overlaps = displays.map { display -> (UInt32, CGFloat) in
            let rect = frame.intersection(display.frame)
            return (display.displayID, rect.isNull ? 0 : rect.width * rect.height)
        }.filter { $0.1 > 0 }.sorted { $0.1 > $1.1 }
        guard let first = overlaps.first else { return nil }
        if overlaps.count > 1 && overlaps[1].1 == first.1 { return nil }
        return first.0
    }
    func permissions() -> [String: Any] {
        let cameraStatus: String
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: cameraStatus = "authorized"
        case .notDetermined: cameraStatus = "not_determined"
        case .denied: cameraStatus = "denied"
        case .restricted: cameraStatus = "restricted"
        @unknown default: cameraStatus = "unknown"
        }
        let displays: [[String: Any]] = NSScreen.screens.compactMap { screen in
            guard let id = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? UInt32 else { return nil }
            return ["id": id, "name": screen.localizedName, "primary": id == CGMainDisplayID()]
        }
        return ["ok": true, "screen_permission": CGPreflightScreenCaptureAccess(), "camera_permission": cameraStatus,
                "displays": displays, "bundle_id": Bundle.main.bundleIdentifier ?? "unbundled", "engine": "ScreenCaptureKit + AVFoundation"]
    }
    func request(_ command: [String: Any]) async throws -> [String: Any] {
        switch command["command"] as? String {
        case "permissions": return permissions()
        case "begin":
            observing=true; events=[]; since=Int64(Date().timeIntervalSince1970*1000)

            desktop.dismiss(); return ["ok":true]
        case "show_card": desktop.show(command); return ["ok":true]
        case "dismiss_card": desktop.dismiss(); return ["ok":true]
        case "request_permission":
            if command["kind"] as? String == "screen" {
                if !CGPreflightScreenCaptureAccess() {
                    _ = CGRequestScreenCaptureAccess()
                    if !CGPreflightScreenCaptureAccess(), let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture") { NSWorkspace.shared.open(url) }
                }
            } else if command["kind"] as? String == "camera" {
                if AVCaptureDevice.authorizationStatus(for: .video) == .notDetermined { _ = await AVCaptureDevice.requestAccess(for: .video) }
                else if AVCaptureDevice.authorizationStatus(for: .video) != .authorized, let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera") { NSWorkspace.shared.open(url) }
            } else { throw failure("权限类型无效") }
            return permissions()
        case "preview":
            // Live view for the local page: a downscaled copy of the frame already in memory. Never starts the camera.
            let width = min(max(CGFloat((command["max_width"] as? Double) ?? 320), 64), 640)
            guard let image = await camera.snapshot(maxWidth: width) else {
                return ["ok":true, "image":NSNull(), "reason":"摄像头未运行或暂无新画面"]
            }
            return ["ok":true, "image":image, "captured_at":Int64(Date().timeIntervalSince1970 * 1000)]
        case "stop": observing=false; events=[]; desktop.dismiss(); await camera.stop(); return ["ok": true]
        case "observe":
            observing = true // Recover event collection if the IPC worker restarted.
            let at = Int64(Date().timeIntervalSince1970 * 1000)
            let front = NSWorkspace.shared.frontmostApplication
            let session = CGSessionCopyCurrentDictionary() as? [String: Any]
            var blocked: String? = nil
            if session?["CGSSessionScreenIsLocked"] as? Bool == true || session?["kCGSessionOnConsoleKey"] as? Bool == false { blocked = "locked" }
            else if !CGPreflightScreenCaptureAccess() { blocked = "screen_permission" }
            else if (command["excluded_apps"] as? [String] ?? []).contains(front?.bundleIdentifier ?? "") { blocked = "excluded" }
            let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
            let title = windows.first(where: { ($0[kCGWindowOwnerPID as String] as? Int32) == front?.processIdentifier && ($0[kCGWindowLayer as String] as? Int) == 0 })?[kCGWindowName as String] as? String ?? ""
            var presence: [String: Any] = ["state":"disabled", "confidence":0.0, "observed_at":at, "reason":"未启用本地在座检测"]
            if blocked != nil { await camera.stop(); presence["state"] = "unknown"; presence["reason"] = "采集暂停" }
            else if command["camera"] as? Bool == true {
                do { presence = try await camera.presence() }
                catch { await camera.stop(); presence = ["state":"unknown", "confidence":0.0, "observed_at":at, "reason":"摄像头或本地检测暂不可用；继续应用与屏幕观察"] }
            } else { await camera.stop() }
            // Detection geometry travels beside the result so stored samples keep the plain presence record.
            let regions = presence.removeValue(forKey: "regions") ?? [[String: Any]]()
            let cameraState = await camera.status()
            var activity: [String: Any] = ["app_name":front?.localizedName ?? "未知应用", "bundle_id":front?.bundleIdentifier ?? "", "window_title":String(title.prefix(300)), "idle_seconds":CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType:.null), "switches":events, "observed_since":since]
            if blocked != nil { activity = ["switches":[], "idle_seconds":0.0, "observed_since":at] }
            events = []; since = at
            return ["ok":true, "captured_at":at, "activity":activity, "presence":presence, "presence_regions":regions, "camera_active":cameraState.active, "camera_device":cameraState.active ? cameraState.device : "", "blocked_reason":blocked as Any? ?? NSNull()]
        case "sample":
            let front = NSWorkspace.shared.frontmostApplication
            let excluded = command["excluded_apps"] as? [String] ?? []
            let expected = command["expected_bundle_id"] as? String ?? front?.bundleIdentifier ?? ""
            try await ensureCaptureAllowed(expected: expected, excluded: excluded)
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            // Rust freezes this list at recording start. Never add newly connected displays here.
            let selected = command["displays"] as? [[String: Any]] ?? []
            guard !selected.isEmpty && selected.count <= 16 else { throw failure("请选择 1 至 16 块显示器") }
            let ids = selected.compactMap { $0["id"] as? UInt32 }
            guard ids.count == selected.count && Set(ids).count == ids.count else { throw failure("显示器选择无效") }
            let window = frontWindow(front)
            let foregroundDisplay = displayForWindow(window, displays: content.displays)
            let title = window?[kCGWindowName as String] as? String ?? ""
            let capturedAt = Int64(Date().timeIntervalSince1970 * 1000)
            var screens: [[String: Any]] = []
            for item in selected {
                try await ensureCaptureAllowed(expected: expected, excluded: excluded)
                let id = item["id"] as! UInt32
                let name = item["name"] as? String ?? "显示器 \(id)"
                var result: [String: Any] = ["display_id": id, "display_name": name,
                    "captured_at": Int64(Date().timeIntervalSince1970 * 1000)]
                if let display = content.displays.first(where: { $0.displayID == id }) {
                    do {
                        let config = SCStreamConfiguration()
                        let ratio = min(1.0, 1440.0 / Double(display.width), 1440.0 / Double(display.height))
                        config.width = Int(Double(display.width) * ratio)
                        config.height = Int(Double(display.height) * ratio)
                        config.showsCursor = false; config.capturesAudio = false
                        // Exclude entire applications so new windows from excluded apps stay hidden too.
                        let hiddenApps = content.applications.filter { excluded.contains($0.bundleIdentifier) }
                        let filter = SCContentFilter(display: display, excludingApplications: hiddenApps, exceptingWindows: [])
                        let screen = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
                        result["captured_at"] = Int64(Date().timeIntervalSince1970 * 1000)
                        result["image"] = try jpeg(screen, maxWidth: 1440)
                    } catch {
                        result["error"] = "该屏幕抓取失败，请检查连接和录屏权限"
                    }
                } else { result["error"] = "显示器已断开或当前不可用" }
                // If the user locks or changes apps mid-group, discard the entire group.
                try await ensureCaptureAllowed(expected: expected, excluded: excluded)
                screens.append(result)
            }
            let finalWindow = frontWindow(front)
            let finalTitle = finalWindow?[kCGWindowName as String] as? String ?? ""
            guard foregroundDisplay == displayForWindow(finalWindow, displays: content.displays),
                  title == finalTitle,
                  window?[kCGWindowNumber as String] as? UInt32 == finalWindow?[kCGWindowNumber as String] as? UInt32 else {
                throw failure("采样期间前台窗口变化，等待下一次稳定采样")
            }
            guard screens.contains(where: { $0["image"] != nil }) else { throw failure("所有选中屏幕均采集失败，请检查显示器连接") }
            let activity: [String: Any] = ["app_name": front?.localizedName ?? "未知应用",
                "bundle_id": front?.bundleIdentifier ?? "", "window_title": String(title.prefix(300)),
                "foreground_display_id": foregroundDisplay as Any? ?? NSNull(),
                "idle_seconds": CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .null),
                "switches": events, "observed_since": since]
            events = []; since = Int64(Date().timeIntervalSince1970 * 1000)
            return ["ok": true, "captured_at": capturedAt, "screens": screens,
                "activity": activity, "capture_source": "多屏采样 · 选中 \(selected.count) 块屏幕"]

        default: throw failure("不支持的采集命令")
        }
    }
}

@main
enum Main {
    static func main() {
        // Rust owns the lifetime. EOF or termination releases every native device.
        Task { @MainActor in
            let worker = CaptureWorker()
            DispatchQueue.global().async {
                while let line = readLine() {
                    let done = DispatchSemaphore(value: 0)
                    Task { @MainActor in
                        let result: [String: Any]
                        do {
                            guard let data = line.data(using: .utf8), let request = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw failure("采集命令格式无效") }
                            result = try await worker.request(request)
                        } catch let error as CaptureFailure { await worker.camera.stop(); result = ["ok": false, "error": error.message] }
                        catch { await worker.camera.stop(); result = ["ok": false, "error": "macOS 采样失败，请检查录屏权限、显示器及摄像头状态"] }
                        if let data = try? JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]) {
                            FileHandle.standardOutput.write(data); FileHandle.standardOutput.write(Data([10]))
                        }
                        done.signal()
                    }
                    done.wait()
                }
                exit(0)
            }
        }
        NSApplication.shared.run()
    }
}
