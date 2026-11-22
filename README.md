# Exo for HEY

A fast, local-first, keyboard-driven native desktop client for [HEY](https://www.hey.com), built on Exo's egui/eframe + fastframe + Blitz stack.

Exo talks directly to HEY through the official [`hey-sdk`](https://github.com/basecamp/hey-sdk) — no Google APIs, no Gmail abstractions, no third-party mail servers. It runs on macOS, Linux and Windows.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/screenshot-dark.png">
  <img alt="Exo's inbox" src=".github/screenshot-light.png">
</picture>

> The screenshots above are from the original Gmail-based Exo and are kept for historical context. The HEY client uses a refreshed three-pane layout with the same design language.

## What's in the box

- **HEY's workflow, natively**: The Imbox, The Feed, Paper Trail, Set Aside, Reply Later, Bubble Up and Bubble Up scheduling, labels/folders, and The Screener.
- **Local-first**: a per-account SQLite cache means the thread list opens instantly. HEY's incremental posting-changes feed keeps it current in the background.
- **Keyboard first**: `j`/`k` to move, `Enter` to open, `e` to Set Aside, `r` to reply, `c` to compose, `s` to mark spam, `b` to bubble up, `/` to search, and a `⌘K`/`Ctrl+K` command palette. `?` opens the shortcut sheet.
- **Real HTML mail**: messages are sanitized and laid out with the Blitz renderer from the original Exo, then painted as egui textures. Remote images, trackers and quoted mail are handled by the same pipeline.
- **Native feel**: eframe/winit window, fastframe app directories, logging and desktop notifications, with cross-platform credential storage (Keychain / Credential Manager / Secret Service, private-file fallback).
- **Cross-platform**: no `objc2` or `security-framework` in the build. The macOS-only bindings that chained Exo to one platform are gone.

Calendar, Contacts, Journal, Habits and Time Tracking are deliberately out of the core mail experience; the SDK surfaces remain available to build on without bloating the client.

## Build & run

```sh
cargo build --release -p exo
cargo run -p exo
```

On first launch, click **Sign in with HEY**. Your browser opens HEY's OAuth page; the app catches the loopback redirect, exchanges the code with PKCE, and stores the tokens on this device.

## Verify

```sh
cargo check --workspace
cargo test --workspace
```

`crates/exo-store/tests/sync.rs` runs the store end-to-end against a local HEY-shaped HTTP server: it checks box sync, posting-to-thread resolution, label loading, thread hydration and the SQLite cache. No account or network access is needed.

Cross-platform builds were checked with `cargo check --target x86_64-pc-windows-gnu -p exo` (passes) and by resolving the Linux dependency graph with `cargo tree --target x86_64-unknown-linux-gnu` — it contains no `objc2`, `security-framework` or Gmail crates. A full Linux cross-check needs an `x86_64-linux-gnu-gcc` toolchain for the TLS dependency's C code, which isn't installed on every build host.

## Architecture

| Crate | Role |
| --- | --- |
| `exo` | eframe app: three-pane HEY UI, command palette, async backend worker |
| `exo-store` | SQLite cache + HEY SDK sync, reads, actions, outbox |
| `exo-auth` | HEY OAuth PKCE, token refresh, cross-platform credential storage |
| `exo-mail` | Blitz HTML email render pipeline and painter |
| `exo-ui` / `exo-commands` | Exo's design system and keyboard/command layer |
| `exo-editor` | Rich compose editor (available as a component) |
| `exo-fonts` | Bundled fonts |

The original `gmail`, `gmail-mock`, Google OAuth, Calendar, and Gmail-store crates and modules have been removed.

## License

MIT — see [LICENSE](LICENSE).
