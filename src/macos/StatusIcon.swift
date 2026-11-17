// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit

enum StatusIcon {
    /// Small template version of the Android head, APK sheet and motion lines in AppIcon.png.
    static func quickADBMark() -> NSImage {
        let image = NSImage(size: NSSize(width: 21, height: 18), flipped: false) { _ in
            NSColor.black.setFill()
            NSColor.black.setStroke()

            let antennae = NSBezierPath()
            antennae.lineWidth = 1.2
            antennae.lineCapStyle = .round
            antennae.move(to: NSPoint(x: 7.2, y: 14.1))
            antennae.line(to: NSPoint(x: 5.6, y: 16.6))
            antennae.move(to: NSPoint(x: 13.4, y: 14.1))
            antennae.line(to: NSPoint(x: 15, y: 16.6))
            antennae.stroke()

            // Leave a gap where the sheet overlaps the head so both read in one color.
            let head = NSBezierPath()
            head.windingRule = .evenOdd
            head.move(to: NSPoint(x: 3.8, y: 8.9))
            head.curve(to: NSPoint(x: 10.3, y: 15.6),
                       controlPoint1: NSPoint(x: 3.8, y: 12.7), controlPoint2: NSPoint(x: 6.7, y: 15.6))
            head.curve(to: NSPoint(x: 16.4, y: 11.45),
                       controlPoint1: NSPoint(x: 13.1, y: 15.6), controlPoint2: NSPoint(x: 15.5, y: 13.7))
            head.line(to: NSPoint(x: 15.65, y: 12))
            head.line(to: NSPoint(x: 10.9, y: 12))
            head.curve(to: NSPoint(x: 8.45, y: 8.9),
                       controlPoint1: NSPoint(x: 9.1, y: 12), controlPoint2: NSPoint(x: 8.45, y: 10.7))
            head.close()
            for x in [7.45, 12.9] {
                head.appendOval(in: NSRect(x: x - 0.72, y: 12.45, width: 1.44, height: 1.44))
            }
            head.fill()

            let sheet = NSBezierPath()
            sheet.lineWidth = 1.2
            sheet.lineJoinStyle = .round
            sheet.move(to: NSPoint(x: 11.2, y: 10.5))
            sheet.line(to: NSPoint(x: 15.1, y: 10.5))
            sheet.line(to: NSPoint(x: 18.7, y: 6.9))
            sheet.line(to: NSPoint(x: 18.7, y: 2.8))
            sheet.curve(to: NSPoint(x: 17.35, y: 1.45),
                        controlPoint1: NSPoint(x: 18.7, y: 2), controlPoint2: NSPoint(x: 18.15, y: 1.45))
            sheet.line(to: NSPoint(x: 11.2, y: 1.45))
            sheet.curve(to: NSPoint(x: 9.85, y: 2.8),
                        controlPoint1: NSPoint(x: 10.4, y: 1.45), controlPoint2: NSPoint(x: 9.85, y: 2))
            sheet.line(to: NSPoint(x: 9.85, y: 9.15))
            sheet.curve(to: NSPoint(x: 11.2, y: 10.5),
                        controlPoint1: NSPoint(x: 9.85, y: 9.95), controlPoint2: NSPoint(x: 10.4, y: 10.5))
            sheet.close()
            sheet.move(to: NSPoint(x: 15.1, y: 10.5))
            sheet.line(to: NSPoint(x: 15.1, y: 6.9))
            sheet.line(to: NSPoint(x: 18.7, y: 6.9))
            sheet.stroke()

            let motion = NSBezierPath()
            motion.lineWidth = 1.2
            motion.lineCapStyle = .round
            motion.move(to: NSPoint(x: 2.05, y: 6.4))
            motion.line(to: NSPoint(x: 7.4, y: 6.4))
            motion.move(to: NSPoint(x: 3.65, y: 3.75))
            motion.line(to: NSPoint(x: 7.4, y: 3.75))
            motion.stroke()
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "QuickADB 安装抽屉"
        return image
    }
}
