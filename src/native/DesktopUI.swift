import AppKit
import SwiftUI

struct ReminderCard: View {
    let title: String
    let message: String
    let preview: Bool
    let review: () -> Void
    let snooze: () -> Void
    let dismiss: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 17) {
            HStack {
                Image(systemName: "waveform.path").foregroundColor(Color(red:0.91,green:0.32,blue:0.16))
                Text("FLOW INSIGHT").font(.system(size:11,weight:.bold,design:.rounded)).tracking(2)
                Spacer()
                Button(action:dismiss) { Image(systemName:"xmark").font(.system(size:12)) }.buttonStyle(.plain).accessibilityLabel("关闭提醒")
            }.foregroundColor(Color(red:0.35,green:0.36,blue:0.31))
            Text(title).font(.system(size:23,weight:.semibold)).foregroundColor(Color(red:0.17,green:0.23,blue:0.18))
            Text(message).font(.system(size:14)).lineSpacing(5).foregroundColor(.secondary).fixedSize(horizontal:false,vertical:true)
            HStack(spacing:12) {
                Button(action:review) { Text(preview ? "打开工作区" : "查看这段记录 ↗").font(.system(size:13,weight:.semibold)).padding(.horizontal,15).padding(.vertical,11).background(Color(red:0.19,green:0.30,blue:0.23)).foregroundColor(.white).cornerRadius(10) }.buttonStyle(.plain)
                Button("15 分钟后再提醒",action:snooze).font(.system(size:12)).buttonStyle(.plain).foregroundColor(.secondary)
            }
            Text(preview ? "预览卡片 · 不是实际状态判断" : "依据连续采样估计，可以忽略本次提醒").font(.system(size:10)).foregroundColor(.secondary)
        }.padding(24).frame(width:368).background(Color(red:1,green:0.97,blue:0.91)).cornerRadius(22).overlay(RoundedRectangle(cornerRadius:22).stroke(Color.white.opacity(0.8),lineWidth:1)).padding(12)
    }
}
@MainActor final class DesktopUI: NSObject {
    var panel: NSPanel?
    var statusItem: NSStatusItem?
    var port=Int(ProcessInfo.processInfo.environment["FLOW_INSIGHT_PORT"] ?? "17901") ?? 17901
    override init() {
        super.init()
        NSApplication.shared.setActivationPolicy(.accessory)
        statusItem=NSStatusBar.system.statusItem(withLength:NSStatusItem.variableLength)
        statusItem?.button?.image=NSImage(systemSymbolName:"waveform.path",accessibilityDescription:"Flow Insight")
        let menu=NSMenu()
        menu.addItem(withTitle:"Flow Insight · 心流洞察",action:nil,keyEquivalent:"")
        let open=menu.addItem(withTitle:"打开工作区",action:#selector(openWorkspace),keyEquivalent:""); open.target=self
        let stop=menu.addItem(withTitle:"暂停记录",action:#selector(pauseRecording),keyEquivalent:"");stop.target=self
        menu.addItem(NSMenuItem.separator())
        let quit=menu.addItem(withTitle:"退出 Flow Insight",action:#selector(quitApp),keyEquivalent:"");quit.target=self
        statusItem?.menu=menu
    }
    @objc func openWorkspace() { if let url=URL(string:"http://127.0.0.1:\(port)") {NSWorkspace.shared.open(url)} }
    @objc func pauseRecording() {post("/api/recorder/stop",body:[:]);dismiss()}
    @objc func quitApp() { kill(getppid(), SIGTERM) }
    func post(_ path:String,body:[String:Any]) {
        guard let url=URL(string:"http://127.0.0.1:\(port)\(path)") else{return}
        var request=URLRequest(url:url);request.httpMethod="POST"
        request.setValue("application/json",forHTTPHeaderField:"Content-Type")
        request.setValue("web",forHTTPHeaderField:"x-flow-insight-client")
        request.httpBody=try? JSONSerialization.data(withJSONObject:body)
        URLSession.shared.dataTask(with:request).resume()
    }
    func dismiss() {panel?.orderOut(nil);panel=nil}
    func show(_ command:[String:Any]) {
        dismiss()
        port=command["port"] as? Int ?? port
        let id=command["sample_id"] as? String ?? ""
        let card=ReminderCard(title:command["title"] as? String ?? "工作状态提醒",message:command["message"] as? String ?? "",preview:command["preview"] as? Bool ?? false,review:{[weak self] in
            guard let self else{return}
            let suffix=UUID(uuidString:id) == nil ? "" : "#review/\(id)"
            if let url=URL(string:"http://127.0.0.1:\(self.port)/\(suffix)"){NSWorkspace.shared.open(url)}
            self.dismiss()
        },snooze:{[weak self] in self?.post("/api/reminders/snooze",body:["minutes":15]);self?.dismiss()},dismiss:{[weak self] in self?.dismiss()})
        let host=NSHostingView(rootView:card)
        let size=host.fittingSize
        let screen=NSScreen.main?.visibleFrame ?? NSRect(x:0,y:0,width:1440,height:900)
        let window=NSPanel(contentRect:NSRect(x:screen.maxX-size.width-22,y:screen.maxY-size.height-22,width:size.width,height:size.height),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
        window.isOpaque=false;window.backgroundColor = .clear;window.hasShadow=true
        window.level = .floating;window.collectionBehavior=[.canJoinAllSpaces,.fullScreenAuxiliary]
        window.isMovableByWindowBackground=true;window.hidesOnDeactivate=false
        window.contentView=host;window.orderFrontRegardless();panel=window
        DispatchQueue.main.asyncAfter(deadline:.now()+25){[weak self,weak window] in if self?.panel === window {self?.dismiss()} }
    }
}
