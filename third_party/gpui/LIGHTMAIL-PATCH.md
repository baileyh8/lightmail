# Lightmail GPUI compatibility patch

Source: crates.io `gpui` 0.2.2, licensed Apache-2.0. All upstream source files are retained except packaging metadata and its standalone lockfile.

One functional change: `src/platform/windows/directx_renderer.rs` creates a non-topmost DirectComposition target. The upstream topmost target paints above native child HWNDs, covering the WebView2 mail reader even when navigation and DOM checks succeed. Child windows must paint above the parent composition surface; Lightmail hides its reader during settings and compose modals.

The ordering follows Microsoft's [CreateTargetForHwnd documentation](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositiondevice-createtargetforhwnd).

Acceptance: the Windows CI must verify actual dark pixels within the reader bounds, in addition to DOM text, links, CSP, and scrolling. Keep this patch until an upstream native-child composition option is available.
