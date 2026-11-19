# Lyri

Live, synced lyrics for whatever is playing on your PC. Lyrics are fetched from [LRCLIB](https://lrclib.net) and are scrolled along with the playing song. A local webview allows you to share the lyrics with other devices (webbrowsers or Lyri clients) on your network.

<p align="center">
  <img src="docs/screenshots/lyrics-dark.png" width="32%" alt="Synced lyrics in dark mode">
  <img src="docs/screenshots/lyrics-light.png" width="32%" alt="Synced lyrics in light mode">
  <img src="docs/screenshots/settings.png" width="32%" alt="Settings pane">
</p>

## Features

- Follows the current track of any player that uses the Windows media controls
- Synced lyrics from LRCLIB (including custom LRCLIB servers)
- Native platform-UI apps (currently only Windows WinUI)
- Host a web view on your network so other devices can follow along (with optional controls)
- Mirror another Lyri instance instead of the local player

<p align="center">
  <img src="docs/screenshots/web-view.png" width="80%" alt="Web view in a browser">
</p>

## Building

Requirements: Windows 11, [Rust](https://rustup.rs), [pnpm](https://pnpm.io) and the
[Windows App Runtime](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads) 2.6.4 or newer.

```sh
# The web view is embedded into the app, so build it first
pnpm -C apps/lyri-web install
pnpm -C apps/lyri-web build

cargo run -p lyri-windows --release
```

To build an MSIX package, run `cargo make msix` in `apps/lyri-windows`
(requires [cargo-make](https://github.com/sagiegurari/cargo-make) and the
`winapp` CLI).

## Project layout

| Path                    | What it is                                                                                                         |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `apps/lyri-windows`     | The Windows app (using [windows-reactor](https://github.com/microsoft/windows-rs/tree/master/crates/libs/reactor)) |
| `apps/lyri-web`         | The web view (using Svelte)                                                                                        |
| `crates/lyri-core`      | Lyrics parsing and shared types                                                                                    |
| `crates/lyri-providers` | Lyrics lookup (LRCLIB)                                                                                             |
| `crates/lyri-server`    | Web view host, client and protocol                                                                                 |

## License

[MIT](LICENSE)
