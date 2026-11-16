# yoobi-cli

A native Rust CLI for Yoobi's classic timesheet. Browse booked hours, activities and locked dates in your terminal, or read them as JSON for scripts. Preview exact booking edits before saving, and store your login securely in your OS credential store.

Build and install locally:

```sh
cargo install --path . --locked
```

Log in once. The command prompts for your URL, username and hidden password, verifies the login with Yoobi, then saves the password in the OS credential store (macOS Keychain, Windows Credential Manager or Linux Secret Service):

```sh
yoobi-cli auth login
# Or supply the non-secret fields and enter only your password:
yoobi-cli auth login --base-url https://your-company.yoobi.nl --username your-login
yoobi-cli auth status
yoobi-cli tui --month 2026-11
```

Every later command reuses that login automatically. Only identity metadata is written to the platform's user config directory under `yoobi-cli/login.json`; the password stays in the OS credential store. `YOOBI_CONFIG_DIR` overrides that directory. `auth status` inspects local configuration without reading the keychain or contacting Yoobi; it does not verify that the saved password still exists or is accepted. `timesheet` verifies access. SSO/MFA-only login is not supported.

```sh
yoobi-cli auth logout  # removes the saved password and login metadata
```

For scripts/CI, explicit `YOOBI_BASE_URL`, `YOOBI_USERNAME` and `YOOBI_PASSWORD` or `YOOBI_PASSWORD_FILE` remain supported. Set only one password source. Password files must be regular files with no group/world permissions (`chmod 600`). Identity fields default to the saved profile, but a saved password is never reused for a different URL or username. Logout leaves environment variables untouched. With no OS credential store available, keep using explicit environment credentials; the CLI does not silently write a plaintext password file.

To save credentials noninteractively, set those environment variables and run `auth login`, or pipe a password manager's output to `auth login --base-url URL --username USER --password-stdin`. Password input is bounded to 16 KiB; `--password-stdin` conflicts with environment password sources. There is no password command-line argument. Prompts and errors stay on stderr; passwords are never printed.

```sh
yoobi-cli projects --month 2026-11
yoobi-cli timesheet --month 2026-11
```

## Edit one booking

Choose an activity ID from `projects`. Preview the exact daily total without changing Yoobi:

```sh
yoobi-cli booking preview --activity ACTIVITY_ID --date 2026-11-17 --duration 7h30 --output json > booking.json
```

Review the JSON, then explicitly save that preview:

```sh
yoobi-cli booking save booking.json --yes
```

Durations accept `7h30`, `7:30`, or `450m`, with exact minute precision from zero to 24 hours. `0m` clears the selected activity's booking for that day. This sets the daily total; it does not add hours. Preview files contain account identifiers, so keep them private.

A save is bound to the preview's origin, employee, activity, date and previous duration. The CLI reads fresh data before saving, refuses stale or locked changes, preserves existing notes and references, and reads again to verify all bookings in the month. Already matching bookings are unchanged. Activities requiring mandatory notes, overtime or alternative booking types must be edited in Yoobi.

Yoobi's classic protocol has no atomic compare-and-save: a concurrent edit between the final read and save can race. Avoid editing the same timesheet elsewhere during a save. Failed or uncertain writes are never retried automatically; inspect the timesheet before trying again. Exit code 7 means a save may have happened but could not be verified.

## Interactive review

```sh
yoobi-cli tui --demo                    # offline sample timesheet
yoobi-cli tui --month 2026-11           # connected timesheet
yoobi-cli tui --demo --snapshot --month 2026-11 --width 120 --height 36
```

The TUI shows activities, daily booked hours and locked dates. Press **e** to edit the selected booking, enter an exact duration, and press **Enter** to preview. In the preview, **s** explicitly saves; **Enter** returns to editing and **Esc** cancels. **Ctrl-U** clears the duration input. Demo saves update sample data only. It adapts from 40-column terminals to a three-pane workspace. The demo is always labelled and does not read credentials or send requests. Connected reads happen in a background worker. If a refresh fails, its error stays visible and can be retried.

Use **↑/↓** or **j/k** for days, **Tab/Shift-Tab** for activities, **Enter** for day detail/back, **[ / ]** for months, **/** to find a date, **r** to refresh, **?** for help, and **q** or **Ctrl-C** to quit. Home/End and Page Up/Down work too. In day detail, movement keys scroll the details in short terminals. Press **!** to expand an error and read its recovery instructions. Browsing and previews send no booking writes. Saving requires the explicit **s** key in the review screen; navigation pauses until verification finishes.

Interactive use requires terminal stdin and stdout. Piped calls fail before authentication. The bounded, plain-text snapshot mode requires `--demo`; JSON output is available through the regular commands. Human text output uses tables rather than raw JSON. Error code **6** means a terminal is required.

Output is JSON when piped and readable text on a terminal. Override with `--output json|text`. `schema` prints the clispec v0.3 contract; `completions zsh` generates shell completion definitions. Timesheet output contains account identifiers and should be kept private.

Exit codes: 0 success; 1 input/network/protocol/I/O error; 3 usage error; 4 authentication error; 5 conflict; 7 save uncertain. Errors are JSON envelopes on stderr.

Requires Rust 1.100 or later. Development: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`. `uv run --no-project --with pyte scripts/check_tui.py` exercises help, filtering, month navigation, day detail, editing, cancellation, demo saves, Ctrl-C and terminal restoration in real pseudo-terminals at 120×36 and 40×16. CI runs this check too. HTTP tests use synthetic fixtures and a local mock server; no real timesheets are changed by tests.

MIT licensed.
