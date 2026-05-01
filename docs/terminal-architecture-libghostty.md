# Terminal Architecture and libghostty Feasibility

Date: 2026-05-01
Issue: borabr-m0z.1
Status: Proposed

## Decision

BoraBR should build the terminal feature around a stable terminal session boundary first:

- Native Rust PTY/session manager in `src-tauri`, using a cross-platform PTY crate such as `portable-pty`.
- Vue/Nuxt terminal panel that talks to the Rust session manager through Tauri commands and events.
- Renderer adapter boundary in the frontend/native layer so `xterm.js` can be the first integrated renderer while a libghostty-backed renderer is developed and validated behind the same session API.
- libghostty remains the primary renderer target for the terminal epic, but it should not block the first PTY/session backend or UI shell.

This keeps the risky native-renderer work isolated from the core lifecycle work: create, focus, resize, write input, stream output, restart, close, and refresh Beads data after CLI mutations.

## Implementation Update

`borabr-m0z.6` adds task-scoped terminal slots directly under issue rows. Those slots request the `libghostty` renderer target through a renderer adapter boundary.

`borabr-m0z.10` replaces the temporary DOM `<pre>` scrollback with an xterm.js terminal emulator fallback. The fallback is intentionally explicit in code via `resolveTerminalRenderer`: the target remains `libghostty`, while the active renderer is `xterm` until a macOS native surface bridge can prove focus, input, resize, packaging, and signing. This keeps the task-terminal UX and PTY lifecycle shippable without hiding the fact that GPU rendering is still blocked on native integration work, and avoids masking PTY output with search-and-replace sanitization.

`borabr-m0z.13` adds the first native renderer bridge boundary. Tauri now exposes Ghostty bridge capabilities and a native renderer launch command. The frontend renderer adapter can select a `ghostty-external` bridge when a launchable Ghostty app/CLI is available, and it keeps the xterm fallback when it is not. On macOS this intentionally requires a launchable `Ghostty.app`; the `ghostty` helper bundled inside cmux can report a version, but Ghostty itself says macOS terminal-emulator launch from that CLI is not supported, so BoraBR must not switch away from xterm unless the native bridge can actually open a renderer.

## Current App Fit

The app is already a Tauri 2 + Nuxt 4 desktop application. The Rust backend exposes commands through `#[tauri::command]`, uses `tauri::Emitter` to notify the frontend, and already watches `.beads/` for changes. The frontend already consumes Tauri commands from `app/utils/bd-api.ts` and listens to native events in `app/composables/useChangeDetection.ts`.

That makes the lowest-friction terminal transport:

1. Add Rust commands for terminal lifecycle:
   - `terminal_create(project_path, issue_id?) -> session_id`
   - `terminal_write(session_id, data)`
   - `terminal_resize(session_id, cols, rows, width_px, height_px)`
   - `terminal_restart(session_id)`
   - `terminal_close(session_id)`
2. Emit events:
   - `terminal:data` with `{ sessionId, chunk }`
   - `terminal:exit` with `{ sessionId, code }`
   - `terminal:error` with `{ sessionId, message }`
3. Keep terminal sessions in managed Rust state keyed by session id.
4. Start shells in the selected project cwd with a controlled environment that preserves `PATH` for `br`/`bd`.
5. Reuse the existing `.beads/` watcher so running `br` or `bd` inside a terminal refreshes the issue list.

## Option A: libghostty-Backed Native Renderer

libghostty is attractive because Ghostty is fast, native, feature rich, and exposes embeddable C/Zig APIs. The current public status is not risk-free:

- Ghostty documents `libghostty` as embeddable, but the project is being split into libraries starting with `libghostty-vt`.
- `libghostty-vt` is usable today for Zig and C across macOS, Linux, Windows, and WebAssembly.
- The API is still in flux and has no tagged stable libghostty release.
- `libghostty-vt` covers terminal parsing/state/render-state concerns; consumers still need to provide integration glue, windowing, rendering surface ownership, input routing, and lifecycle management.

For Tauri/macOS, a true libghostty native path likely needs:

- C ABI headers plus Rust FFI, or a small C/Swift/Objective-C bridge wrapped by Rust.
- A native view/surface integration strategy for the Tauri window. On macOS that means coordinating an `NSView` or Metal-backed layer with the webview layout, focus, resize, and z-order.
- A message bridge between Vue layout state and native surface state.
- A shell/session lifecycle boundary separate from renderer code unless the final libghostty surface API owns the child process end to end.
- Dedicated packaging work for bundled native libraries and architecture-specific macOS artifacts.

Recommendation: implement a small libghostty spike before committing the production renderer. The spike should prove:

- A native surface can be embedded in or aligned with the Tauri window on macOS.
- Keyboard, paste, selection, focus, resize, and scroll behavior can be routed without fighting the webview.
- The library can be built, linked, notarized/signed, and shipped for Apple Silicon and Intel.
- The adapter can expose the same `create/write/resize/restart/close` semantics as the web renderer path.

## Option B: xterm.js + Native Rust PTY

This is the pragmatic first integrated path. `xterm.js` already has a browser-oriented API for rendering in an HTML element, receiving user input, resizing, selection, themes, and terminal events. A Rust PTY backend can own the real shell and stream bytes to the frontend.

For the backend, `portable-pty` is a good fit because it provides a cross-platform PTY abstraction, can open a native PTY, spawn a shell, clone a reader, write to the master, and resize through a stable Rust API.

Advantages:

- Fits Tauri's existing command/event model.
- Keeps all UI layout in Vue, which is important for "terminal below each task" and tabbed panel UX.
- Lets the team ship session lifecycle, Beads refresh behavior, task context insertion, and multiple sessions without waiting on native surface embedding.
- Lower packaging risk than bundling a libghostty native renderer immediately.

Tradeoffs:

- It is not the final GPU-native target.
- Webview terminal security must be treated seriously because any JavaScript in the same context can observe or inject terminal I/O.
- Output performance and terminal correctness may lag Ghostty/libghostty for heavy TUI workloads.

## Security Model

An embedded terminal is equivalent to giving the app shell access as the current OS user.

Rules for BoraBR:

- Never run terminal processes with elevated privileges.
- Scope every session to a user-selected project cwd.
- Show enough UI context for the user to know which project and issue a terminal belongs to.
- Preserve user `PATH`, but add `br`/`bd` detection diagnostics when the command is missing.
- Treat all terminal output as untrusted. Do not inject terminal text into the DOM with `innerHTML`.
- Do not load remote scripts, ads, or dynamic JavaScript near the terminal surface.
- Keep Tauri shell permissions narrow. Do not use broad shell execution from the frontend for terminal sessions; route terminal lifecycle through explicit Rust commands.
- On close/restart/app exit, terminate or detach child processes intentionally and make the UI state explicit.
- Do not automatically run generated commands. Helper affordances should insert or stage commands unless the user explicitly executes them.

## First Implementation Path

1. Add a Rust `terminal` module with a `TerminalManager` managed by Tauri state.
2. Use `portable-pty` for shell creation, input writes, output reader threads, resizing, restart, and cleanup.
3. Emit PTY output and lifecycle events through Tauri.
4. Add a Vue composable such as `useTerminalSessions` for command calls, event listeners, and cleanup.
5. Add the docked and task-inline panel UI using a `TerminalRenderer` adapter interface.
6. Implement the first adapter with xterm.js as the bounded terminal-emulator fallback while the libghostty native bridge is developed.
7. Add the Ghostty-compatible native bridge behind the primary adapter target:
   - detect whether an embedded libghostty bridge or launchable Ghostty external bridge is available,
   - open the external Ghostty bridge with the selected project cwd and `BORABR_ISSUE_ID` context,
   - keep xterm active when the native bridge is unavailable.
8. Replace the external bridge with an embedded libghostty/NSView or Metal-backed surface once focus, input, resize, selection, scrollback, packaging, signing, and Beads dogfood workflows pass.

## Source Notes

- Ghostty README: `libghostty` is C/Zig embeddable; `libghostty-vt` is usable today but API signatures are still in flux and no stable version has been tagged.
- libghostty Doxygen: `libghostty-vt` handles escape parsing, terminal state, input encoding, scrollback, line wrapping, reflow, and related core terminal behavior.
- Ghostling README: a complete minimal terminal can be built on libghostty, but the demo provides its own windowing/rendering and is not intended as a daily-use terminal.
- Tauri 2 docs: commands provide the type-safe frontend-to-Rust call boundary; events are the natural async stream mechanism for backend-to-frontend updates.
- `portable-pty` docs: the crate provides a cross-platform native PTY API for opening PTYs, spawning shells, reading output, writing input, and resizing.
- xterm.js docs: frontend renderers expose input, resize, render, selection, title, and buffer events; xterm.js security docs warn that terminal integration raises the security bar because JavaScript in the same context can observe or manipulate terminal I/O.

## References

- https://github.com/ghostty-org/ghostty
- https://libghostty.tip.ghostty.org/index.html
- https://github.com/ghostty-org/ghostling
- https://v2.tauri.app/develop/calling-rust/
- https://tauri.app/develop/sidecar/
- https://docs.rs/portable-pty/latest/portable_pty/
- https://xtermjs.org/docs/
- https://xtermjs.org/docs/guides/security/
