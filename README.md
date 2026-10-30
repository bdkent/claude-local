# Claude Local

**A buddy list for your local Claude Code sessions.** A small native tray app
that lists the Claude Code sessions running on your machine, keeps background
sessions from idling out, and opens any of them in your terminal with one
double-click.

[**Website**](https://bdkent.github.io/claude-local/) ·
[**Download**](https://github.com/bdkent/claude-local/releases/latest) ·
macOS and Windows

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/overview-dark.png" />
  <img
    src="docs/screenshots/overview-light.png"
    alt="Three views of Claude Local: the session list grouped by project with a tooltip on the keep-alive link icon; the New session dialog with a project folder, name and first prompt; and the delete confirmation asking before discarding unpushed work."
  />
</picture>

<sub>The session list, starting a new session, and the second confirmation
before discarding unpushed work. Shown with sample data.</sub>

Built for running `claude --bg` sessions locally (for example projects that
need a real GPU, a slow-to-start editor or a local toolchain) and driving them
through Remote Control.

- **Buddy list.** Click the tray icon for a compact list grouped by project.
  Filter by background or interactive, by project, or by name. The dot shows
  the state shown under the name: green is running, grey-blue is done or idle,
  pulsing red is blocked (waiting for you, e.g. a permission prompt), and a
  hollow ring is stopped.
- **Keep-alive.** Claude Code idles a background session after 8 hours with no
  client attached. Claude Local holds a hidden `claude attach <id>` on every
  background session (marked with a link icon) so that never happens.
- **Open in terminal.** Double-click a background session, or hit **Open**, to
  run `claude attach <id>` in Terminal, iTerm2, Ghostty, WezTerm, kitty,
  Alacritty, Windows Terminal, PowerShell, or a custom command. The hidden
  keep-alive stays attached alongside it.
- **Delete.** Hover a background session and click the trash icon (or select
  it and press Delete). After you confirm, it runs `claude rm <id>`, which
  deletes the session and its worktree. If the worktree has unpushed commits,
  Claude refuses and the dialog shows its message with a second, explicit
  **Discard unpushed work and delete** button.
- **New session.** Hit **＋**, pick a project folder (recent and active ones
  are listed, or choose any folder), give it a name and a first prompt.
  **Start in background** runs `claude --bg --name … "prompt"` there.
  Unchecked, it opens your terminal running `claude --name … "prompt"`. The
  checkbox default is a setting. The folder must already be trusted by Claude
  Code (run `claude` there once); otherwise Claude's error is shown in the
  dialog.

## Install

Download the latest build from
[Releases](https://github.com/bdkent/claude-local/releases):

- **macOS:** the `.dmg` (universal: Apple Silicon and Intel). Keep-alive needs
  tmux: `brew install tmux`. Builds aren't notarized yet, so macOS blocks the
  first launch. Either run

  ```sh
  xattr -dr com.apple.quarantine "/Applications/Claude Local.app"
  ```

  or try to open the app, click **Done**, then go to **System Settings →
  Privacy & Security** and click **Open Anyway** next to Claude Local. (On
  macOS 15 and later, right-click → Open no longer skips the check.)
- **Windows:** the `-setup.exe` or `.msi`.

Turn on **Launch at login** in Settings so keep-alive is always running.

On startup the app checks for `claude`, `tmux` (macOS, when keep-alive is on)
and your chosen terminal. If anything is missing, the list opens with a
**Setup needed** card that says what's missing and how to install it (for
example `brew install tmux`). The card clears by itself once it's fixed.

## How it works

Every few seconds (every 3 s while the list is open, every 30 s while it's
hidden) the app runs `claude agents --json`. That lists interactive and
background sessions with their `cwd`, `kind`, `status` and, for background
sessions, the short `id` that `claude attach` takes.

Keep-alive then reconciles: background sessions without a hidden client get
one, and clients whose session has gone away are stopped.

| | macOS / Linux | Windows |
|---|---|---|
| Hidden client lives in | a detached tmux session on a private server (`tmux -L claude-local`) | a ConPTY owned by the app |
| Survives quitting the app | yes | no; run the app at login |
| Inspect | `tmux -L claude-local ls` | — |

Apps started from Finder get a minimal `PATH`, so on macOS the app asks your
login shell for its `PATH` once and uses that to find `claude` and `tmux`. If
that fails, set the path to `claude` in Settings.

Settings are stored in `~/Library/Application Support/claude-local/config.json`
on macOS and `%APPDATA%\claude-local\config.json` on Windows. The custom
terminal command accepts `{args}` (the whole `claude …` command, as its own
word), `{cwd}`, `{name}`, and `{command}` (a full `cd … && claude …` line for
a shell), for example:

```
open -na Ghostty --args --working-directory={cwd} -e {args}
```

## Development

Requires Rust (stable) and Node 22. On Linux you also need the
[Tauri system dependencies](https://tauri.app/start/prerequisites/).

```sh
npm ci
npm run tauri dev       # run the app with hot-reloading UI
cargo test --workspace  # core logic tests (includes a real-tmux test if tmux is installed)
```

Layout:

```
crates/core/   session parsing, PATH discovery, keep-alive backends, terminal launchers (no UI)
src-tauri/     the tray app: polling loop, tray icon, window, commands
ui/            the list window (TypeScript + Vite, no framework)
site/          the GitHub Pages website
docs/          README screenshots
scripts/       icon generator, screenshot compositor
```

The project website lives in `site/` (plain HTML, CSS and JS, no build step)
and is published to GitHub Pages by the **Pages** workflow whenever `site/`
changes.

To regenerate icons: `python3 scripts/make-icons.py && npx tauri icon src-tauri/icons/app-icon.png -o src-tauri/icons`
(then delete the `android/`, `ios/` and `Square*` outputs).

## Releasing

1. Bump `version` in `package.json` (the app version is read from it).
2. Commit, then either tag and push (`git tag v0.2.0 && git push origin v0.2.0`)
   or open Actions → **Release** → **Run workflow** on `master`, which creates
   the tag from the `package.json` version.
3. The **Release** workflow builds the macOS universal `.dmg` and the Windows
   installers, and attaches them to a **draft** GitHub Release. Review it and
   publish.

macOS builds are ad-hoc signed. To ship notarized builds, add an Apple
Developer ID certificate and the `APPLE_*` secrets described in
[Tauri's macOS signing guide](https://tauri.app/distribute/sign/macos/) to the
workflow.

## License

[MIT](LICENSE)
