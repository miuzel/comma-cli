# AGENTS.md

Guidance for AI coding agents working on this repository. Assumes no prior knowledge of the project.

## Project overview

**comma-cli** (binary name: `comma`, installed as `,`) is a tiny, stateless CLI that translates natural-language intent into a single shell command using an LLM, then optionally executes it after user confirmation. It is a command *generator*, not an agent: one intent → one command → execute → done.

- Language: Rust (edition 2021), single crate, single binary.
- Current version: `0.24.0` (see `Cargo.toml`; the binary reports `env!("CARGO_PKG_VERSION")`).
- Repository: https://github.com/miuzel/comma-cli
- License: MIT.
- Platforms: Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64). On Unix the binary is installed as `,`; on Windows as `comma.exe` (PowerShell reserves `,`).

## Repository layout

The application is a single binary crate split into modules under `src/` (no `lib.rs`). Major areas inside each module are marked with `// ── ... ──` banner comments.

- `src/main.rs` — entry point and dispatch: CLI parsing (only *leading* args are flags; `--` ends flag parsing), `--version`/`--update`/`-h`/`--test`/`-v`/`-vv`, one-shot mode (args joined as intent, trailing `!` = auto-confirm) vs interactive REPL mode, piped stdin → one-shot with line-based confirmation, execute/edit/refine loop. `execute()` runs the confirmed command via `sh -c` on Unix; on Windows via `sh -c` when `SHELL` is set (Git Bash/MSYS) and `cmd /C` otherwise — the same SHELL/platform fallback rule `get_shell()` uses (`shell_interp()` in main.rs), so execution matches the shell the model generated for. Shell-integration eval mode: when `COMMA_EVAL_FILE` is set (non-empty), `execute()` instead appends the comment-stripped command to that file (one per line) and returns — a wrapper shell function (see README § Shell integration) evals the file in the user's *current* shell, because a child process can never change the parent shell's cwd/environment. In child-process mode a bare `cd` (`is_bare_cd`) prints a note pointing at the wrapper.
- `src/ui.rs` — terminal UI: rustyline `FileHelper` (filename Tab-completion), candidate selector (`|||` separated alternatives), spinner, print helpers (`print_info`/`print_error`/`print_debug`/`print_cmd`), confirm/edit/refine prompts (crossterm raw mode, `atty` checks, line-based fallbacks for piped stdin), char-boundary-safe `truncate`, clipboard copy. Raw-mode key loops act only on `KeyEventKind::Press` — Windows also reports Release/Repeat events, which must be ignored (a buffered Enter release would otherwise execute without a keypress); on Unix only Press is reported, so the filter is a no-op there.
- `src/config.rs` — config loading: legacy single-model format and multi-provider `providers`/`models` format, `COMMA_*` env overrides, API-style auto-detection, `home_dir`.
- `src/context.rs` — system context gathering (distro, kernel, arch, shell, CWD, user-installed packages) and privacy placeholders (`{{USER}}`, `{{HOSTNAME}}`, `{{HOME}}`) — collected locally, substituted into LLM output only after the response. `get_shell()` reports `COMMA_EVAL_SHELL` when set and non-empty (the eval wrapper declares the dialect the model should generate for — e.g. `powershell`), then `$SHELL` when set (Git Bash/MSYS users on Windows), falling back to `/bin/sh` on Unix and `cmd.exe` on Windows.
- `src/prompt.rs` — system prompt loading (`prompt_path()` resolves XDG-first like `config_path()`: `~/.config/comma/prompt.md` preferred, legacy `~/.local/bin/,.prompt.md` fallback, `DEFAULT_PROMPT` compiled in as last resort), `prefer` tool-preference formatting. The `#SEARCH:` rules (`SEARCH_SECTION`) are appended at runtime when `search.provider` is not `off`, so users with a custom `prompt.md` get them without editing their file.
- `src/llm.rs` — LLM clients: OpenAI-compatible (`/v1/chat/completions`), OpenAI Responses API (`/v1/responses`) and Anthropic (`/v1/messages`, optional `reasoning` thinking budget), blocking `reqwest` with rustls; ordered fallback and per-model retries in `call_llm_with_retry` (which also re-labels the passed-in spinner per fallback attempt).
- `src/cache.rs` — response cache (`cache_path()` resolves `~/.cache/comma/cache.json` (`%APPDATA%\comma\cache.json` on Windows) first, then exe-adjacent `,.cache.json`, then legacy `~/.local/bin/,.cache.json`; oldest-entry eviction, atomic save only when dirty, parent dir auto-created; entries cached only after the user executes the command; checked across ALL configured model entries in fallback order before any API call; `cache_size: 0` disables caching entirely).
- `src/update.rs` — self-update (`--update`) from GitHub releases, with archive verification against the release's `sha256sums.txt` and cross-device/locked-exe handling. Both manual `--update` and the periodic `check_and_notify` show the release changelog and ask for consent BEFORE touching the binary (`install_version` is the post-consent part). When an auto-update offer is declined, the user is asked whether to disable future checks (Enter = keep; `prompt_confirm_default_no`), and `y` persists `auto_update: false` via `write_auto_update_flag` in config.rs. Non-TTY (piped) runs never prompt — the old one-line notice only.
- `src/protocol.rs` — `#CHECK:` (tool availability probe), `#EXPLORE:` (run a help command once and learn from output) and `#SEARCH:` (web search, results fed back) protocol handling, chained in `process_response`.
- `src/search.rs` — web search backends for `#SEARCH:`: DuckDuckGo (default, no key — scrapes the lite HTML endpoint; automatically falls back to Mojeek when DDG rate-limits), Mojeek (no key, scraping-friendly), Brave and Tavily (need `search.api_key`), SearXNG (needs `search.base_url`); tolerant regex-free HTML/JSON parsing, no extra dependencies.
- `src/danger.rs` — danger detection: `DANGER_PATTERNS` substring list (whitespace-normalized) plus exact-token pipe-to-shell matching (`curl x | sh` flags, `| shuf` does not), red ⚠ warning.
- `src/i18n.rs` — UI language selection: `config.lang` > `COMMA_LANG` > `LANG`/`LC_ALL` > `"en"`, with `normalize_lang` mapping locale variants (`fr_FR`, `pt-BR`, ...) to a supported code. UI strings go through the `t!` macro (rust-i18n, embedded at compile time via `i18n!("locales", fallback = "en")` in main.rs).
- `locales/` — one `<lang>.toml` per UI language (en, zh, ja, ko, fr, de, es, pt, ru); every file must contain the same keys as `en.toml`. Placeholders are rust-i18n v3 named syntax `%{name}` — `{}` is NOT substituted (this was a real bug in en/ja); match each `t!` call site's argument names. `run_tests` checks placeholder substitution for every locale.
- `src/tests.rs` — `run_tests`, the built-in self-test suite (see Testing below).
- `Cargo.toml` — dependencies: `reqwest` (blocking, json, rustls-tls), `serde`, `serde_json`, `sha2` (update checksums), `atty`, `crossterm`, `rustyline`, `rust-i18n`. Release profile: `strip = true`, `opt-level = "z"`, `lto = true` (size-optimized ~3MB binary — keep it that way).
- `config.json` — default config template (on Linux/macOS installed to `~/.config/comma/config.json`, honoring `$XDG_CONFIG_HOME`; on Windows to `%APPDATA%\comma\config.json`). `config_path()` in config.rs resolves default-first with exe-adjacent (portable) and legacy `~/.local/bin` paths as fallback for existing installs.
- `prompt.md` — default system prompt template (installed to `~/.config/comma/prompt.md` on Linux/macOS, `%APPDATA%\comma\prompt.md` on Windows). Placeholders: `{{SYSTEM_CONTEXT}}`, `{{PREFERENCES}}`.
- `build.sh` — builds with cargo and installs to `~/.local/bin` (see below).
- `install.sh` — end-user installer: downloads the latest GitHub release archive for the detected platform and verifies it against `sha256sums.txt` when available.
- `uninstall.sh` — removes `,`, `,.config.json`, `,.prompt.md` from `~/.local/bin`, `config.json`/`prompt.md` from `~/.config/comma/`, `cache.json` from `~/.cache/comma/`, and all three from `%APPDATA%\comma\` on Windows.
- `.github/workflows/release.yml` — release pipeline (see Deployment).
- `README.md` / `README.zh-CN.md` — user documentation (English / Chinese). Keep both in sync when changing user-visible behavior.

## Build and test commands

```bash
cargo build            # debug build
cargo build --release  # release build (binary: target/release/comma)
./build.sh             # release build + install to ~/.local/bin/, (also copies
                       # config.json and prompt.md → ~/.config/comma/ if missing)
```

There is no `cargo test` suite and no CI check workflow — do not assume one exists.

## Testing instructions

Testing is done via a built-in self-test flag compiled into the binary:

```bash
cargo run -- --test          # or: ./target/release/comma --test
```

`run_tests()` in `src/tests.rs` runs ~160 ad-hoc assertions (placeholder substitution, privacy leak checks on gathered context, empty-HOME guard, `#CHECK:`/`#EXPLORE:`/`#SEARCH:` parsing, DDG lite HTML parsing and redirect unwrapping, candidate parsing, char-boundary-safe `truncate`, `is_dangerous` pattern and pipe-to-shell matching, retry constants, `COMMA_EVAL_FILE` eval-file append, `COMMA_EVAL_SHELL` override in `get_shell`, `is_bare_cd` first-token detection, `%{name}` substitution for every embedded locale) and exits non-zero on failure. When you change parsing or privacy-related logic, add matching checks to `run_tests()` and make sure `--test` passes. Manual smoke test (requires a configured API key): `, list files larger than 1G`.

## Code style guidelines

- Keep the module structure; new code goes in the module it belongs to (see Repository layout). Split a new module only when a change genuinely demands it.
- Match the existing style: `// ── Section ──` banner comments separating major areas, short doc comments on non-obvious functions, plain `String`/`Result<_, String>` error handling (no `anyhow`/`thiserror`).
- Blocking, synchronous code throughout (reqwest blocking client); no async runtime — do not add tokio.
- Keep the dependency list minimal and the binary small; check `Cargo.toml` before assuming any crate is available.
- Terminal output goes through the existing helpers (`print_info`, `print_error`, `print_debug`, `print_cmd`) with crossterm colors; interactive input uses crossterm raw mode, with `atty` checks and line-based fallbacks for piped stdin.
- Comments and documentation are written in English (user docs also have a Chinese translation, `README.zh-CN.md`).

## Configuration and runtime data

- Config resolution priority: `COMMA_*` env vars (`COMMA_BASE_URL`, `COMMA_API_KEY`, `COMMA_MODEL`, `COMMA_API_STYLE`) → config file (`~/.config/comma/config.json` on Linux/macOS, `%APPDATA%\comma\config.json` on Windows → exe-adjacent `,.config.json` → legacy `~/.local/bin/,.config.json`) → `~/.claude/settings.json` env section (legacy path only) → built-in defaults. API style is auto-detected from URL (`anthropic` in URL → Anthropic Messages API, `responses` in URL → OpenAI Responses API, otherwise OpenAI chat completions).
- `COMMA_EVAL_FILE` is a runtime integration variable (not config): when set and non-empty, `execute()` in `src/main.rs` appends each confirmed command to that file instead of spawning a shell, and the shell-integration wrapper function evals the file in the user's current shell (rationale: a child process cannot change the parent shell's cwd/env).
- `COMMA_EVAL_SHELL` is a runtime integration variable (not config): when set and non-empty, `get_shell()` in `src/context.rs` reports it as the shell in the system context, so the eval wrapper can declare the dialect the model should generate (the PowerShell wrapper sets it to `powershell`; bash/zsh rely on `$SHELL`, and cmd.exe is already the SHELL-less Windows default).
- The multi-provider `providers` + `models` format in the config enables ordered fallback with per-model `retries`. In this format `COMMA_MODEL` overrides only the primary (first) entry's model.
- `cache_size` sets the response-cache capacity; `cache_size: 0` disables the cache entirely (nothing read back or written).
- `reasoning` (Anthropic only) sets the thinking budget in tokens; `max_tokens` is raised accordingly so values ≥ 1024 work.
- The `search` object configures the `#SEARCH:` web-search backend: `provider` (`off` **by default** — keyless scraping triggers aggressive anti-bot measures, so the model is only told it can search when the user explicitly picks a backend; `brave`/`tavily` need `api_key`; `searxng` needs `base_url`; `duckduckgo`/`mojeek` are keyless scraping, DDG falls back to Mojeek), `max_results` (default 5, clamped to 1–10). Scraping backends fetch via an external `curl` when available (its TLS fingerprint passes bot checks that reject reqwest/rustls); query spaces are encoded as `+`, not `%20` (the latter trips Mojeek's ALTCHA). `#SEARCH:` does NOT prompt for confirmation (read-only request, model-generated query) and is limited to ONE live search per intent (repeats get a text nudge, not another search); `#EXPLORE:` still prompts. Search failures and empty results are fed back to the model as context instead of aborting, so it can still answer from its own knowledge.
- Runtime files are resolved per-file, first match wins: platform default (`config.json`/`prompt.md` under `$XDG_CONFIG_HOME/comma/` resp. `~/.config/comma/`, `cache.json` under `$XDG_CACHE_HOME/comma/` resp. `~/.cache/comma/`; on Windows all three under `%APPDATA%\comma\`) → next to the executable (`,.config.json` / `,.prompt.md` / `,.cache.json`, for portable installs) → legacy `~/.local/bin/` → if none exists, the platform default (where new writes go). Resolution lives in `xdg_or_legacy()` / `exe_dir()` in config.rs.

## Security considerations

- **Privacy by design**: username, hostname, and home path must never be sent to the LLM API. The system context sanitizes CWD to `{{HOME}}`/`{{USER}}`, and the model is instructed to emit `{{USER}}`/`{{HOSTNAME}}`/`{{HOME}}` placeholders that are substituted locally after the response. On interactive refine, the *raw* (pre-substitution) reply is what goes back to the API as conversation history, so real paths never leak. Preserve this invariant — `run_tests` has explicit "context does not leak ..." checks.
- Config files contain API keys (`auth_token`); never commit real keys, log them, or print them in verbose/debug output.
- Generated commands are shown for confirmation before execution; dangerous patterns are flagged via `DANGER_PATTERNS` and pipe-to-shell matching. Piped stdin is no exception: `echo intent | ,` asks for a `y` line on stdin before executing, and only `echo intent | , !` (auto-confirm) skips it. The `!` auto-confirm flag and `--update` self-replacement exist by design — be careful not to weaken the confirmation flow silently.
- `#EXPLORE:` commands requested by the model are executed via `sh -c` on Unix / `cmd /C` on Windows (unconditional per-OS choice, unlike `execute()`'s SHELL-aware rule) after user confirmation (a single probe asks too) unless auto-confirm; output is truncated to 4096 chars before being fed back to the LLM.
- Self-update (`--update`) verifies the downloaded archive against the release's `sha256sums.txt` before replacing the binary.

## Deployment process

Releases are fully automated via `.github/workflows/release.yml`:

1. Push a git tag matching `v*` (after bumping `version` in `Cargo.toml`).
2. GitHub Actions builds release binaries for 5 targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` (via `cross`), `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
3. Each is packaged (`comma-<os>-<arch>.tar.gz`, or `.zip` for Windows); a `sha256sums.txt` covering all archives is generated and published to the GitHub release by `softprops/action-gh-release`, together with `install.sh`.

End users install/update via `install.sh` or the built-in `, --update`, both of which pull from GitHub releases and verify the archive against `sha256sums.txt` — so archive names in the workflow, `install.sh`, and `do_update()` in `src/update.rs` must stay consistent.

There is also a Homebrew tap at `miuzel/homebrew-tap` (install: `brew install miuzel/tap/comma-cli`). Its `Formula/comma-cli.rb` is **not** updated by the release workflow — after each release, bump `version`, update the four archive URLs, and copy the new hashes from the release's `sha256sums.txt`, then push. The formula installs the binary as `,` (alias `comma`) plus a `comma-setup` helper that interactively writes `~/.config/comma/config.json` (skipping the prompts still leaves a default config for manual editing) — the setup prompt cannot live in `post_install`, because Homebrew points HOME at a temporary build dir during install hooks and user lookups are unreliable there.

**After pushing a release tag, wait ~10 minutes before testing `, --update` or `install.sh`.** The `releases/latest/download/` redirect lags behind on GitHub's CDN: right after a release it can serve the *previous* release's `sha256sums.txt` together with the *new* archive, causing a spurious checksum mismatch (the abort is the safety check working, not a broken release). Pin-tag URLs (`releases/download/vX.Y.Z/...`) are not affected.
