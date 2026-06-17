# zh — zed helpers

Tiny stdin → transform → stdout text filter for CSS values. It plugs into
Zed through the vim filter command (`!`), so the same binary also works in
Vim, Helix, or any shell pipeline.

```sh
echo "margin: 6px; color: #ff0000;" | zh
# margin: 0.375rem /* 6px */; color: oklch(62.8% 0.2577 29.23) /* #ff0000 */;
```

## Helpers

| Name        | Aliases        | What it does                                          |
| ----------- | -------------- | ----------------------------------------------------- |
| `px2rem`    | `px`, `rem`    | `6px` → `0.375rem /* 6px */`                          |
| `hex2oklch` | `hex`, `oklch` | `#ff0000` → `oklch(62.8% 0.2577 29.23) /* #ff0000 */` |
| `now`       | `date`, `time`  | `2026-06-11 at 01.50.48 PM` → current local time      |
| `mdlink`    | `link`, `links` | `[a](b c.md)` → `[a](b%20c.md)`                       |
| `sort`      | `asc`           | sort the selected lines alphabetically (visual mode)  |

`px2rem` and `hex2oklch` keep the original value as a trailing `/* … */`
comment. `hex2oklch` also lowercases the hex in that comment
(`#FF0000` → `#ff0000`). `mdlink` only touches the path inside a markdown
`](…)`, and only for local files — `https://…`, `mailto:`, `tel:` and
`#anchor` targets are left alone; besides a plain space it also encodes the
narrow no-break space U+202F (`%E2%80%AF`) that macOS date/Finder strings
sneak in.

`now` skips a date that sits inside a markdown link target — a screenshot path
like `![](…/Screenshot 2026-06-17 at 12.54.47 PM.webp)` is a filename, not a
timestamp to refresh — so a bare `zh` (or `zh now`) never rewrites it and
breaks the link.

```sh
zh              # apply ALL helpers (sort is excluded — it is opt-in)
zh px           # only px → rem
zh hex          # only hex → oklch
zh link         # only fix spaces in markdown link paths
zh sort         # sort the selected lines alphabetically (visual mode)
zh --list       # list available helpers (also -l, --help)
```

`sort` reorders whole lines, so it is **not** part of bare `zh` — you must
name it explicitly. It is bound in visual mode only (see below): sorting a
single current line in normal mode is pointless.

Helpers are applied sequentially (a fold over the input); the order is the
order in the `HELPERS` array.

## Installation

### Option 1: cargo install (recommended)

```sh
cargo install --path .
```

This builds a release binary and places it at `~/.cargo/bin/zh`. Make sure
`~/.cargo/bin` is in your `PATH` (the rustup installer normally adds it):

```sh
# ~/.zshrc or ~/.bashrc, if it's not there already
export PATH="$HOME/.cargo/bin:$PATH"
```

### Option 2: manual build and copy

```sh
cargo build --release
```

The binary ends up at `target/release/zh`. Copy it to any directory in your
`PATH`, for example:

```sh
mkdir -p ~/.local/bin
cp target/release/zh ~/.local/bin/   # ensure ~/.local/bin is in PATH
```

> **Important for Zed:** the binary must be reachable through `PATH` as seen
> by Zed itself. If Zed was launched from the Dock/Finder rather than a
> terminal, it may not pick up `PATH` changes from your shell profile —
> restart Zed after installing, or launch it via the `zed` CLI from a
> terminal.

### Updating

If you already have `zh` installed, pull the latest changes and rebuild from
the repo:

```sh
git pull
cargo install --path . --force   # rebuild and overwrite ~/.cargo/bin/zh
```

The `--force` flag is what makes `cargo install` overwrite the existing
binary. If you installed manually (Option 2), rebuild and copy again instead:

```sh
git pull
cargo build --release
cp target/release/zh ~/.local/bin/   # or wherever you copied it before
```

Restart Zed afterwards so it picks up the new binary.

### Verify

```sh
echo "margin: 6px; color: #ff0000;" | zh
# margin: 0.375rem /* 6px */; color: oklch(62.8% 0.2577 29.23) /* #ff0000 */;

zh --list    # prints the helper table

cargo test   # reference values are checked against oklch.com
```

## Usage in Zed

Requires `"vim_mode": true` in `settings.json`.

Manually: select line(s) (`V`), press `:` — Zed pre-fills `'<,'>`, then
type `!zh` and hit Enter. The selected lines are replaced with the output.

Keybindings — in `~/.config/zed/keymap.json`:

```json
[
  {
    "context": "vim_mode == visual",
    "bindings": {
      "space h h": ["workspace::SendKeystrokes", ": ! z h enter"],
      "space h p": ["workspace::SendKeystrokes", ": ! z h space p x enter"],
      "space h c": ["workspace::SendKeystrokes", ": ! z h space h e x enter"],
      "space h n": ["workspace::SendKeystrokes", ": ! z h space n o w enter"],
      "space h l": ["workspace::SendKeystrokes", ": ! z h space l i n k enter"],
      "space h s": ["workspace::SendKeystrokes", ": ! z h space s o r t enter"]
    }
  },
  {
    "context": "vim_mode == normal",
    "bindings": {
      "space h h": ["workspace::SendKeystrokes", "shift-v : ! z h enter"]
    }
  }
]
```

- `space h h` — run all helpers at once (in normal mode it selects the
  current line first)
- `space h p` — px → rem only
- `space h c` — hex → oklch only
- `space h n` — refresh a `… at HH.MM.SS AM/PM` timestamp to the current time
- `space h l` — escape spaces in markdown link paths (`](a b.md)` → `](a%20b.md)`)
- `space h s` — sort the selected lines alphabetically (visual mode only;
  sorting a single line in normal mode is pointless, so it is not bound there)

The same approach works in Vim/Neovim (`:'<,'>!zh`) and Helix
(select, then `|zh`).

## Configuration

- `ZH_REM_BASE` — root font-size used for the px→rem conversion
  (default: `16`). Set it in your shell profile, or per invocation:
  `ZH_REM_BASE=10 zh px`.

## Adding a new helper

1. Write a `fn my_helper(input: &str) -> String` in `src/main.rs`
2. Register it in the `HELPERS` array (name, aliases, description,
   function)
3. Reinstall: `cargo install --path .`

## Design notes

**Why regex over the whole line instead of a precise selection?**
The vim filter in Zed is line-based: the entire line is replaced, and you
can't pass just the `6px` fragment inside a line. So zh finds px values and
hex colors in the line itself and converts them in place — selecting whole
lines turns out to be faster than making a precise selection.

**Idempotency.** Values inside comments (`/* 6px */`) are left untouched,
so running the filter twice over the same text is safe.

**Exact output.** zh writes back exactly what it transformed — no trailing
newline is added, because a vim filter must return precisely the text that
gets pasted back into the buffer.
