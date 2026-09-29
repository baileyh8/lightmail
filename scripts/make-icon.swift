import AppKit
import Foundation

let root = URL(fileURLWithPath: CommandLine.arguments[1])
let iconset = root.appendingPathComponent("build/AppIcon.iconset")
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for size in [16, 32, 128, 256, 512] {
  for scale in [1, 2] {
    let pixels = size * scale
    let image = NSImage(size: NSSize(width: pixels, height: pixels))
    image.lockFocus()
    let transform = NSAffineTransform()
    transform.scale(by: CGFloat(pixels) / 1024)
    transform.concat()
    NSColor(srgbRed: 0.94, green: 0.96, blue: 0.94, alpha: 1).setFill()
    NSBezierPath(roundedRect: NSRect(x: 56, y: 56, width: 912, height: 912), xRadius: 205, yRadius: 205).fill()
    NSColor(srgbRed: 0.15, green: 0.37, blue: 0.30, alpha: 1).setFill()
    NSBezierPath(roundedRect: NSRect(x: 238, y: 319, width: 548, height: 385), xRadius: 65, yRadius: 65).fill()
    NSColor.white.setStroke()
    let fold = NSBezierPath()
    fold.lineWidth = 26
    fold.lineCapStyle = .round
    fold.lineJoinStyle = .round
    fold.move(to: NSPoint(x: 284, y: 640))
    fold.line(to: NSPoint(x: 512, y: 479))
    fold.line(to: NSPoint(x: 740, y: 640))
    fold.stroke()
    image.unlockFocus()
    let data = NSBitmapImageRep(data: image.tiffRepresentation!)!.representation(using: .png, properties: [:])!
    let name = "icon_\(size)x\(size)\(scale == 2 ? "@2x" : "").png"
    try data.write(to: iconset.appendingPathComponent(name))
  }
}
