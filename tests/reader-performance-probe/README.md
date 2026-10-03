# Windows mail reader resource comparison

Independent release executable with two modes: one reused WebView2 control, or
the current Blitz parse/layout → whole-page Vello CPU raster → PNG encode/decode
pipeline. Both use the same Win32 host and twenty existing local HTML files.
Neither mode creates a mail service, opens an account database, or reads credentials.
This workspace does not add WebView2 to the product dependency tree.

```powershell
./scripts/compare-windows-readers.ps1 `
  -PrivateDirectory build/html-reader-private-20260930 `
  -OutputDirectory build/reader-performance-my-run `
  -Rounds 3 -CaptureChecks
```

The script verifies Blitz revision
`ed03fe183a9f129965ead6435c0caa8e9c49c401` in `.tooling/blitz-research`, cloning it
if absent. Cargo locks the independent dependencies. Requires MSVC Rust and an
installed WebView2 Runtime; the probe does not install or update that runtime.
Run on a monitor at 100% scaling. Both modes use 720×640 pixels and a light theme.

Runs alternate order, use separate processes and fresh WebView2 user data folders,
and process the same first twenty HTML files in sorted filename order. A single
WebView2 controller is reused within each run. The synthetic initial document
separates reader initialization from subsequent mail navigation. There is no OS
file-cache flush, so initialization timings are **not cold-boot measurements**.

All output stays under ignored `build/`. `report.json` and `summary.json` contain
numeric metrics and anonymous sample indices. `-CaptureChecks` runs two additional
visual checks, writes private previews for samples 1 and 20, and excludes those
runs from aggregation. Do not commit HTML, previews, runtime profiles or raw output.

Measurement boundaries:

- Private MiB means process private committed bytes, not resident RAM. Working
  sets are also reported, but summing shared pages can count them more than once.
  Both include the host and descendants; dedicated GPU memory is excluded.
- Peaks come from 100ms process-tree polling plus observations while native
  raster/PNG buffers are live. These are lower bounds, not enforced resource caps.
- WebView2 latency ends at `NavigationCompleted`. Blitz latency includes parsing,
  three resolves, full-page CPU drawing, PNG encoding/decoding and GDI painting.
  These are different completion boundaries; they are not matched presentation
  timestamps. `CapturePreview` verifies eventual WebView2 display separately.
- Private memory is recorded after twenty reads, after three seconds visible idle,
  after three seconds hidden (`TrySuspend` for WebView2, bitmap release for Blitz),
  and after controller release. The script checks only the host remains at the
  last stage; it never terminates installed Lightmail or other applications.
- The probe blocks document scripts, external images, downloaded CSS/fonts and
  frames. It does not measure opt-in image loading, CID attachment display,
  scrolling, selection, accessibility, GPUI integration, total app resources,
  Electron or Tauri performance. Browser-runtime internal network activity is
  outside the document request policy.
- CPU counters include the probe's message pumping and process sampling. They
  cannot establish idle CPU consumption or battery cost.

See [the comparison and design decision](../../docs/windows-reader-performance-comparison.md).
