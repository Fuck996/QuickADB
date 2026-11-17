// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit
import CoreServices
import SwiftUI

// Use SwiftUI's property wrapper with Command Line Tools, which ship without the State macro plugin.
typealias ViewState<Value> = SwiftUI.State<Value>

@main
@MainActor
enum QuickADBApp {
    static func main() {
        let application = NSApplication.shared
        let delegate = AppDelegate()
        application.delegate = delegate
        application.setActivationPolicy(.regular)
        withExtendedLifetime(delegate) { application.run() }
    }
}

@MainActor
private final class AppDelegate: NSObject, NSApplicationDelegate, NSToolbarDelegate {
    private enum Toolbar {
        static let connect = NSToolbarItem.Identifier("connect")
        static let open = NSToolbarItem.Identifier("open")
    }

    private var model: BackendModel?
    private let routes = WorkspaceRoutes()
    private let drawerRoutes = WorkspaceRoutes()
    private var statusItem: NSStatusItem?
    private let popover = NSPopover()
    private var workspace: NSWindow?
    private var drawer: NSPanel?
    private var preferences: NSWindow?
    private var refresh: Timer?
    private var updateTimer: Timer?
    private var updateTask: Task<Void, Never>?
    private var pendingUpdate: MacUpdate.Staged?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let event = NSAppleEventManager.shared().currentAppleEvent
        let login = event?.eventID == kAEOpenApplication
            && event?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
        let appearance = UserDefaults.standard.string(forKey: "appearance").flatMap(Appearance.init(rawValue:))
        (appearance ?? .system).apply()
        do {
            model = try BackendModel()
        } catch {
            let alert = NSAlert()
            alert.alertStyle = .critical
            alert.messageText = "无法启动 QuickADB"
            alert.informativeText = error.localizedDescription
            alert.runModal()
            NSApp.terminate(nil)
            return
        }

        makeMenus()
        makeStatusItem()
        if !login { showWorkspace() }
        let timer = Timer(timeInterval: 0.2, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.workspace?.isVisible == true || self.drawer?.isVisible == true
                    || self.popover.isShown || self.preferences?.isVisible == true else { return }
                self.model?.refresh()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        refresh = timer
        MacUpdate.finishPreviousUpdate(installed: Bundle.main.bundleURL.resolvingSymlinksInPath(),
                                       arguments: CommandLine.arguments)
        let updater = Timer(timeInterval: 24 * 60 * 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.startUpdateCheck(manual: false) }
        }
        RunLoop.main.add(updater, forMode: .common)
        updateTimer = updater
        startUpdateCheck(manual: false)
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showWorkspace()
        return false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard model?.restartingForUpdate != true else { return .terminateCancel }
        model?.refresh()
        guard model?.snapshot.jobs.contains(where: \.active) == true else { return .terminateNow }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "仍有安装任务正在运行"
        alert.informativeText = "退出 QuickADB 会中断队列。已交给 Android 的安装可能继续，请在设备上确认结果。"
        alert.addButton(withTitle: "继续运行")
        alert.addButton(withTitle: "退出 QuickADB")
        return alert.runModal() == .alertSecondButtonReturn ? .terminateNow : .terminateCancel
    }

    func applicationWillTerminate(_ notification: Notification) {
        refresh?.invalidate()
        updateTimer?.invalidate()
        updateTask?.cancel()
        pendingUpdate?.discard()
    }

    @objc private func showWorkspace() {
        guard let model else { return }
        popover.performClose(nil)
        model.refresh()
        if workspace == nil {
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 940, height: 650),
                styleMask: [.titled, .closable, .miniaturizable, .resizable],
                backing: .buffered, defer: false
            )
            window.title = "QuickADB"
            window.autorecalculatesKeyViewLoop = true
            window.contentViewController = NSHostingController(rootView: content(model, drawer: false))
            window.contentMinSize = NSSize(width: 690, height: 510)
            window.setContentSize(NSSize(width: 940, height: 650))
            window.isReleasedWhenClosed = false
            let toolbar = NSToolbar(identifier: "QuickADB.Workspace")
            toolbar.delegate = self
            toolbar.displayMode = .iconOnly
            window.toolbar = toolbar
            window.toolbarStyle = .unified
            if !window.setFrameUsingName("QuickADB.Workspace") { window.center() }
            window.setFrameAutosaveName("QuickADB.Workspace")
            workspace = window
        }
        NSApp.activate()
        workspace?.deminiaturize(nil)
        workspace?.makeKeyAndOrderFront(nil)
    }

    @objc private func showSettings() {
        guard let model else { return }
        popover.performClose(nil)
        if preferences == nil {
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 460, height: 410),
                styleMask: [.titled, .closable],
                backing: .buffered, defer: false
            )
            window.title = "QuickADB 设置"
            window.contentViewController = NSHostingController(rootView: Preferences(model: model))
            window.isReleasedWhenClosed = false
            window.center()
            preferences = window
        }
        NSApp.activate()
        preferences?.makeKeyAndOrderFront(nil)
    }

    private func content(_ model: BackendModel, drawer: Bool) -> Workspace {
        Workspace(
            model: model, routes: drawer ? drawerRoutes : routes, drawer: drawer,
            openWindow: { [weak self] in self?.showWorkspace() },
            togglePin: { [weak self] in self?.togglePin() },
            openSettings: { [weak self] in self?.showSettings() },
            drawerSheet: drawer ? { [weak self] sheet in
                self?.showWorkspace()
                self?.routes.sheet = sheet
            } : nil
        )
    }

    private func makeStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = item.button {
            button.image = StatusIcon.quickADBMark()
            button.toolTip = "QuickADB 安装抽屉"
            button.setAccessibilityLabel("QuickADB 安装抽屉")
            button.target = self
            button.action = #selector(toggleDrawer)
        }
        statusItem = item
        popover.behavior = .transient
        popover.contentSize = NSSize(width: 400, height: 640)
    }

    @objc private func toggleDrawer() {
        guard let model, let button = statusItem?.button else { return }
        model.refresh()
        if model.snapshot.pinned {
            if drawer?.isVisible == true {
                drawer?.orderOut(nil)
            } else {
                showPinnedDrawer(model, anchor: button)
            }
        } else if popover.isShown {
            popover.performClose(nil)
        } else {
            if popover.contentViewController == nil {
                popover.contentViewController = NSHostingController(rootView: content(model, drawer: true))
            }
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        }
    }

    private func togglePin() {
        guard let model, let button = statusItem?.button else { return }
        let pinned = !model.snapshot.pinned
        guard model.send(Command(.preferences, pinned: pinned)) else { return }
        if pinned {
            popover.performClose(nil)
            showPinnedDrawer(model, anchor: button)
        } else {
            drawer?.orderOut(nil)
            toggleDrawer()
        }
    }

    private func showPinnedDrawer(_ model: BackendModel, anchor: NSStatusBarButton) {
        if drawer == nil {
            let panel = NSPanel(
                contentRect: NSRect(x: 0, y: 0, width: 400, height: 640),
                styleMask: [.titled, .closable, .utilityWindow],
                backing: .buffered, defer: false
            )
            panel.title = "QuickADB 安装抽屉"
            panel.contentViewController = NSHostingController(rootView: content(model, drawer: true))
            panel.hidesOnDeactivate = false
            panel.autorecalculatesKeyViewLoop = true
            panel.isReleasedWhenClosed = false
            panel.level = .floating
            if !panel.setFrameUsingName("QuickADB.Drawer") {
                if let anchorWindow = anchor.window, let screen = anchorWindow.screen {
                    let buttonFrame = anchorWindow.convertToScreen(anchor.convert(anchor.bounds, to: nil))
                    let visible = screen.visibleFrame
                    let x = min(max(buttonFrame.midX - panel.frame.width / 2, visible.minX),
                                visible.maxX - panel.frame.width)
                    panel.setFrameTopLeftPoint(NSPoint(x: x, y: visible.maxY - 6))
                } else {
                    panel.center()
                }
            }
            panel.setFrameAutosaveName("QuickADB.Drawer")
            drawer = panel
        }
        NSApp.activate()
        drawer?.makeKeyAndOrderFront(nil)
    }

    @objc private func openAPKs() {
        guard let model else { return }
        showWorkspace()
        pickAPKs(model, split: false)
    }

    @objc private func connectDevice() {
        showWorkspace()
        routes.sheet = .connection
    }

    @objc private func checkForUpdates() {
        startUpdateCheck(manual: true)
    }

    private func startUpdateCheck(manual: Bool) {
        if updateTask != nil {
            if manual { showUpdateMessage("更新正在处理", "已确认的更新会等待当前安装和连接操作结束。") }
            return
        }
        updateTask = Task {
            defer { updateTask = nil }
            var confirmed = false
            do {
                let current = try MacUpdate.currentVersion(.main)
                guard current.beta > 0 else {
                    if manual { showUpdateMessage("当前构建未启用在线更新", "请使用 GitHub 发布的 macOS 测试版。") }
                    return
                }
                guard let release = try await MacUpdate.check(current: current) else {
                    if manual { showUpdateMessage("已是最新 Mac 测试版", current.tag) }
                    return
                }
                let alert = NSAlert()
                alert.messageText = "发现 QuickADB \(release.version.tag)"
                alert.informativeText = "从 GitHub 下载并校验后，QuickADB 会等待当前任务结束，并安装更新、重新打开。"
                alert.addButton(withTitle: "下载并更新")
                alert.addButton(withTitle: "稍后")
                alert.addButton(withTitle: "发布说明")
                NSApp.activate()
                switch alert.runModal() {
                case .alertFirstButtonReturn:
                    confirmed = true
                case .alertThirdButtonReturn:
                    NSWorkspace.shared.open(release.page)
                    return
                default:
                    return
                }

                let installed = Bundle.main.bundleURL.resolvingSymlinksInPath()
                let staged = try await MacUpdate.prepare(release, installed: installed)
                pendingUpdate = staged
                guard let model else { staged.discard(); pendingUpdate = nil; return }
                while updateIsBlocked(model) {
                    try await Task.sleep(for: .seconds(1))
                }
                try Task.checkCancellation()
                try await installMacUpdate(staged, model: model)
            } catch is CancellationError {
                pendingUpdate?.discard()
                pendingUpdate = nil
            } catch {
                pendingUpdate?.discard()
                pendingUpdate = nil
                if manual || confirmed { showUpdateMessage("无法完成更新", error.localizedDescription) }
            }
        }
    }

    private func updateIsBlocked(_ model: BackendModel) -> Bool {
        model.refresh()
        let snapshot = model.snapshot
        return snapshot.connecting || snapshot.pairing || snapshot.jobs.contains(where: \.active)
            || snapshot.devices.contains(where: \.managing)
    }

    private func installMacUpdate(_ staged: MacUpdate.Staged, model: BackendModel) async throws {
        model.restartingForUpdate = true
        defer { model.restartingForUpdate = false }
        try staged.swap()
        pendingUpdate = nil
        do {
            _ = try await MacUpdate.relaunch(staged)
        } catch {
            do {
                try staged.swap()
                staged.discard()
            } catch {
                throw MacUpdate.Failure("新版无法重新打开。旧版仍保存在 \(staged.candidate.path)。")
            }
            throw MacUpdate.Failure("新版无法重新打开，已还原旧版。\(error.localizedDescription)")
        }
        model.restartingForUpdate = false
        NSApp.terminate(nil)
    }

    private func showUpdateMessage(_ title: String, _ message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        NSApp.activate()
        alert.runModal()
    }

    private func makeMenus() {
        let main = NSMenu()
        let root = NSMenuItem()
        let app = NSMenu(title: "QuickADB")
        root.submenu = app
        main.addItem(root)
        app.addItem(withTitle: "关于 QuickADB", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        app.addItem(withTitle: "检查更新…", action: #selector(checkForUpdates), keyEquivalent: "").target = self
        app.addItem(.separator())
        app.addItem(withTitle: "设置…", action: #selector(showSettings), keyEquivalent: ",").target = self
        app.addItem(.separator())
        app.addItem(withTitle: "隐藏 QuickADB", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        app.addItem(withTitle: "退出 QuickADB", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")

        let file = NSMenu(title: "文件")
        let fileRoot = NSMenuItem()
        fileRoot.submenu = file
        main.addItem(fileRoot)
        file.addItem(withTitle: "选择 APK…", action: #selector(openAPKs), keyEquivalent: "o").target = self
        file.addItem(withTitle: "连接设备…", action: #selector(connectDevice), keyEquivalent: "n").target = self
        file.addItem(withTitle: "关闭窗口", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")

        let edit = NSMenu(title: "编辑")
        let editRoot = NSMenuItem()
        editRoot.submenu = edit
        main.addItem(editRoot)
        edit.addItem(withTitle: "撤销", action: Selector(("undo:")), keyEquivalent: "z")
        edit.addItem(withTitle: "重做", action: Selector(("redo:")), keyEquivalent: "Z")
        edit.addItem(.separator())
        edit.addItem(withTitle: "剪切", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "复制", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "粘贴", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "全选", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")

        let windows = NSMenu(title: "窗口")
        let windowsRoot = NSMenuItem()
        windowsRoot.submenu = windows
        main.addItem(windowsRoot)
        windows.addItem(withTitle: "打开主窗口", action: #selector(showWorkspace), keyEquivalent: "1").target = self
        windows.addItem(withTitle: "最小化", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        NSApp.windowsMenu = windows
        NSApp.mainMenu = main
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.flexibleSpace, Toolbar.open, Toolbar.connect]
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarAllowedItemIdentifiers(toolbar)
    }

    func toolbar(
        _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.target = self
        item.isBordered = true
        if identifier == Toolbar.open {
            item.label = "选择 APK"
            item.image = NSImage(systemSymbolName: "folder", accessibilityDescription: item.label)
            item.action = #selector(openAPKs)
        } else if identifier == Toolbar.connect {
            item.label = "连接设备"
            item.image = NSImage(systemSymbolName: "plus", accessibilityDescription: item.label)
            item.action = #selector(connectDevice)
        } else {
            return nil
        }
        item.toolTip = item.label
        return item
    }
}
