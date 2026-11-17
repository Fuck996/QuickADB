// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit
import CoreServices
import SwiftUI

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
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showWorkspace()
        return false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
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
            if let path = Bundle.main.url(forResource: "TrayIcon", withExtension: "png"),
               let icon = NSImage(contentsOf: path) {
                icon.size = NSSize(width: 18, height: 18)
                button.image = icon
            } else {
                button.image = NSImage(systemSymbolName: "arrow.down.to.line", accessibilityDescription: "QuickADB")
            }
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

    private func makeMenus() {
        let main = NSMenu()
        let root = NSMenuItem()
        let app = NSMenu(title: "QuickADB")
        root.submenu = app
        main.addItem(root)
        app.addItem(withTitle: "关于 QuickADB", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
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
