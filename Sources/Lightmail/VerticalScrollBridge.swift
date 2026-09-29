import AppKit
import SwiftUI

// Keep horizontal tables scrollable, while vertical gestures anywhere in the
// native document go to its single outer scroll view. The monitor is scoped to
// this visible view and window and is removed when the reader is dismantled.
struct VerticalScrollBridge: NSViewRepresentable {
  final class Anchor: NSView {
    var monitor: Any?
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      stop()
      guard window != nil else { return }
      monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
        guard let self, event.window === self.window,
          abs(event.scrollingDeltaY) > abs(event.scrollingDeltaX),
          self.visibleRect.contains(self.convert(event.locationInWindow, from: nil)),
          let scroll = self.enclosingScrollView else { return event }
        scroll.scrollWheel(with: event)
        return nil
      }
    }
    func stop() {
      if let monitor { NSEvent.removeMonitor(monitor) }
      monitor = nil
    }
    deinit { if let monitor { NSEvent.removeMonitor(monitor) } }
  }
  func makeNSView(context: Context) -> Anchor { Anchor() }
  func updateNSView(_ view: Anchor, context: Context) {}
  static func dismantleNSView(_ view: Anchor, coordinator: ()) { view.stop() }
}
