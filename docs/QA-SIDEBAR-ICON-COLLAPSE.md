# QA: Sidebar icon-collapse mode on desktop

Closes the verification ask in [#5676]. The root shell's sidebar moved from
`collapsible="offcanvas"` (column unmounts when collapsed) to
`collapsible="icon"` (a real ~56px column that stays mounted), and `AppSidebar`
now renders its own icon-only body by reading `useSidebar().state`. Three of the
properties that changed are invisible to jsdom, and one is invisible to any
browser: they need eyes on a real desktop build.

Verification is split into two layers:

| Layer                                                                                              | Covers                                                                                                                            | Where                                                     |
| -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| **Automated, real Chromium** against the web E2E build (real core + mock backend, same as CI Full) | Collapsed-state DOM contract, drag-strip geometry, resolved seam colours across modes and presets, resize + persistence mechanics | `app/test/playwright/specs/sidebar-icon-collapse.spec.ts` |
| **Manual, desktop app**                                                                            | Native-layer punch-through, macOS traffic-light clickability, persistence across a true process restart                           | This checklist                                            |

The automated layer already proves the following, so the manual pass does not
need to re-derive them (it should still notice anything odd):

- The collapsed column stays mounted at exactly 56px (`SIDEBAR_ICON_WIDTH`)
  through three expand/collapse toggles and a window resize; the resize rail is
  absent while collapsed; the reopen trigger and icon nav render inside the
  column.
- The collapsed rail's first element is a full-width, 28px (`h-7`) strip at the
  very top of the column carrying `data-tauri-drag-region`, and every rail item
  starts below it.
- The resize seam indicator's rendered background resolves to exactly the live
  `--line-chrome` token on hover and on focus, never the plain `--line` token,
  in light and dark across Classic, Ocean, Matrix (light) and HAL 9000.
- Pointer drag resizes the column (+60px drag lands as +60px width, clamped to
  188..420), arrow keys step by 16px both directions, and the committed width
  reaches the per-user persisted layout blob and survives a page reload.

Run the automated layer with:

```bash
pnpm test:e2e:web -- test/playwright/specs/sidebar-icon-collapse.spec.ts
# or against an existing build:
bash app/scripts/e2e-web-session.sh test/playwright/specs/sidebar-icon-collapse.spec.ts
```

Set `PW_SIDEBAR_SHOTS=1` to also drop evidence screenshots into
`app/test-results/sidebar-shots/`.

---

## How to run the manual pass

1. Build or run the desktop app for your platform (`pnpm dev:app`, or an
   installed build). Sign in far enough to see the main shell with the sidebar.
2. Walk the checklist below, ticking each box only after verifying the expected
   outcome with your own eyes.
3. Record results in the sign-off block and paste it into #5676.
4. A defect found here is fixed separately: file it and link it from #5676.
   Do not widen this issue.

### macOS (required)

- [ ] **No punch-through while collapsed** — Collapse the sidebar (header
      button or mod+B). Look at the narrowed icon column: no content from
      outside the app window bleeds through, no stale frame ghosts inside it,
      and the chrome background reads continuous behind the rail icons. Then
      toggle expanded <-> collapsed five times fast, and drag-resize the window
      by its edges while collapsed. Expected: the rail repaints cleanly every
      time; no flash of wrong content, no black rectangle, no torn frame.
- [ ] **Traffic lights stay clear while collapsed** — With the sidebar
      collapsed, the macOS window controls must sit on bare draggable chrome,
      fully visible and clickable, not overlapping the first rail icon. Click
      each traffic light (close last). Expected: every click lands on the
      control, none gets swallowed by a rail button; dragging the window by the
      strip above the icons moves the window.
- [ ] **Traffic lights stay clear while expanded** — Same check with the
      sidebar expanded: the controls sit over the sidebar header's empty left
      edge and remain clickable.
- [ ] **Seam colour under real compositing** — Expand the sidebar, hover the
      1px resize seam at the sidebar's right edge, then focus it with Tab.
      Expected: a hairline appears on hover/focus, visibly matching the chrome
      hairline tone, in both Appearance modes and under at least two theme
      presets (Settings > Appearance / Theme Studio). It must not read as
      nearly invisible (the old plain `line` token failure) nor as a hard black
      line.
- [ ] **Drag resize and true-restart persistence** — Drag the seam; the column
      tracks the pointer and clamps at its narrowest/widest. Focus the seam and
      press Left/Right; expected: 16px steps. Quit the app entirely (mod+Q) and
      relaunch. Expected: the sidebar reopens at the dragged width. Also
      collapse, quit, relaunch: expected: still collapsed.

### Windows (recommended)

- [ ] **No punch-through while collapsed** — Same steps as the macOS
      punch-through item. The compositing concern is not macOS-specific; only
      the traffic-light one is.
- [ ] **Drag resize and true-restart persistence** — Same steps as the macOS
      resize item. The title bar here is native, so skip the traffic-light row.

### Linux (recommended)

- [ ] **No punch-through while collapsed** — Same steps as the macOS
      punch-through item, on the WebKitGTK build.
- [ ] **Drag resize and true-restart persistence** — Same steps as the macOS
      resize item. The title bar here is native, so skip the traffic-light row.

---

## Why the punch-through precondition was believed gone (context)

The original `offcanvas` choice recorded: "the native webview glued to the
content bounds has historically punched through a zero-width-but-present
column." That mechanism belonged to CEF's per-provider child webviews
(`webview_accounts` / the CDP scanners), which positioned a separate native
surface by tracking a DOM placeholder's bounds. Both halves are gone: the CDP
scanners and `webview_accounts` in #5478, and CEF itself in #5456 (Wry now;
one webview total). `icon` mode additionally keeps the column non-zero-width in
every state, which was the specific trigger. This pass exists because static
analysis is the wrong instrument for a compositing question, not because the
old failure was imaginary.

## Sign-off

```text
Issue: #5676
Tester: @<github-handle>
Date: YYYY-MM-DD
Platform(s) tested: [macOS arm64] [macOS x64] [Windows] [Linux]
Automated suite commit/sha: <sha>
Notes:
```
