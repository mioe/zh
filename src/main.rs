//! zh — "zed helpers": stdin → transform → stdout.
//!
//! Designed to be used as a vim filter in Zed:
//!   select lines → `:` → `'<,'>!zh` → Enter
//!
//! Usage:
//!   zh              apply ALL helpers (sort excluded — it is opt-in)
//!   zh px           only px → rem
//!   zh hex          only hex → oklch
//!   zh now          refresh a timestamp to the current local time
//!   zh sort         sort the selected lines alphabetically
//!   zh --list       list available helpers
//!
//! Env:
//!   ZH_REM_BASE     root font-size for px→rem (default: 16)

use regex::{Captures, Regex};
use std::env;
use std::io::{self, Read, Write};

type Transform = fn(&str) -> String;

struct Helper {
    name: &'static str,
    aliases: &'static [&'static str],
    about: &'static str,
    /// Whether bare `zh` (no args) runs this helper. The value-conversion
    /// helpers compose safely over a line and default to `true`; structural
    /// helpers like `sort`, which *reorder* lines, are opt-in (`false`) so a
    /// plain `zh` never shuffles a selection unexpectedly — you must name it.
    in_all: bool,
    /// Priority over markdown link targets. A `](…)` region belongs to
    /// `mdlink`: helpers with `false` here never see it, so a filename like
    /// `Screenshot 2026-06-11 at 1.44.05 PM.webp` can't be mistaken for a
    /// timestamp / px value / hex color to convert. `mdlink` (which owns the
    /// region) and `sort` (which reorders whole lines and never rewrites
    /// characters) run over the full text.
    sees_link_targets: bool,
    run: Transform,
}

/// To add a new helper: write a `fn(&str) -> String` and register it here.
/// Bare `zh` applies helpers in this order.
const HELPERS: &[Helper] = &[
    Helper {
        name: "px2rem",
        aliases: &["px", "rem"],
        about: "padding: 16px 8px; -> padding: 1rem 0.5rem; /* 16px 8px */",
        in_all: true,
        sees_link_targets: false,
        run: px2rem,
    },
    Helper {
        name: "hex2oklch",
        aliases: &["hex", "oklch"],
        about: "color: #fff; -> color: oklch(100% 0 0); /* #fff */",
        in_all: true,
        sees_link_targets: false,
        run: hex2oklch,
    },
    Helper {
        name: "now",
        aliases: &["date", "time"],
        about: "2026-06-11 at 01.50.48 PM -> current local time",
        in_all: true,
        sees_link_targets: false,
        run: now,
    },
    Helper {
        name: "mdlink",
        aliases: &["link", "links"],
        about: "[a](b c.md) -> [a](b%20c.md)  (escape spaces in md link paths)",
        in_all: true,
        sees_link_targets: true,
        run: mdlink,
    },
    Helper {
        name: "sort",
        aliases: &["asc"],
        about: "sort the selected lines alphabetically (opt-in; visual mode)",
        in_all: false,
        sees_link_targets: true,
        run: sort,
    },
];

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args
        .iter()
        .any(|a| a == "--list" || a == "-l" || a == "--help")
    {
        for h in HELPERS {
            eprintln!("{:<12} ({})  {}", h.name, h.aliases.join(", "), h.about);
        }
        return;
    }

    let selected: Vec<&Helper> = if args.is_empty() {
        HELPERS.iter().filter(|h| h.in_all).collect()
    } else {
        args.iter()
            .filter_map(|a| {
                HELPERS
                    .iter()
                    .find(|h| h.name == a || h.aliases.contains(&a.as_str()))
            })
            .collect()
    };

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .expect("zh: failed to read stdin");

    // No extra newline: a vim filter must return exactly what it should paste back.
    io::stdout()
        .write_all(apply(&selected, input).as_bytes())
        .expect("zh: failed to write stdout");
}

/// Fold the helpers over the input, enforcing the link-target priority:
/// helpers without `sees_link_targets` are run only on the text *between*
/// markdown link targets.
fn apply(selected: &[&Helper], input: String) -> String {
    selected.iter().fold(input, |text, helper| {
        if helper.sees_link_targets {
            (helper.run)(&text)
        } else {
            outside_link_targets(&text, helper.run)
        }
    })
}

/// Run `f` on the chunks between markdown link targets `](…)`; the targets
/// themselves pass through verbatim.
fn outside_link_targets(input: &str, f: Transform) -> String {
    let link_re = Regex::new(r"\]\([^)]*\)").unwrap();
    let mut out = String::new();
    let mut last = 0;
    for m in link_re.find_iter(input) {
        out.push_str(&f(&input[last..m.start()]));
        out.push_str(m.as_str());
        last = m.end();
    }
    out.push_str(&f(&input[last..]));
    out
}

// ---------------------------------------------------------------------------
// css value engine — shared by px2rem and hex2oklch
// ---------------------------------------------------------------------------

/// Replace value matches line by line, adapting the comment to the syntax the
/// value lives in:
///
///   foo: #fff;          ->  foo: oklch(100% 0 0); /* #fff */
///   padding: 16px 8px;  ->  padding: 1rem 0.5rem; /* 16px 8px */
///   px-[16px]           ->  px-[1rem]                 (tailwind: no comment)
///   6px                 ->  0.375rem /* 6px */        (no `;`: end of line)
///
/// - values inside `/* … */` are skipped, so re-running is a no-op;
/// - inside `[…]` (a tailwind arbitrary value) there is no room for a
///   comment, and spaces become `_` (`text-[oklch(100%_0_0)]`);
/// - otherwise a declaration's originals are collected into ONE comment after
///   its closing `;`, merging into a comment already sitting there — so
///   px2rem and hex2oklch on the same declaration share it;
/// - `convert` returning `None` leaves the match untouched.
fn convert_values(input: &str, re: &Regex, convert: &dyn Fn(&Captures) -> Option<String>) -> String {
    let comment_re = Regex::new(r"/\*.*?\*/").unwrap();
    let bracket_re = Regex::new(r"\[[^\]]*\]").unwrap();
    input
        .split_inclusive('\n')
        .map(|line| convert_line(line, re, convert, &comment_re, &bracket_re))
        .collect()
}

fn convert_line(
    full: &str,
    re: &Regex,
    convert: &dyn Fn(&Captures) -> Option<String>,
    comment_re: &Regex,
    bracket_re: &Regex,
) -> String {
    let (line, eol) = match full.strip_suffix('\n') {
        Some(l) => (l, "\n"),
        None => (full, ""),
    };

    let spans = |re: &Regex| -> Vec<(usize, usize)> {
        re.find_iter(line).map(|m| (m.start(), m.end())).collect()
    };
    let comments = spans(comment_re);
    let brackets = spans(bracket_re);
    let within =
        |spans: &[(usize, usize)], s: usize, e: usize| spans.iter().any(|&(a, b)| s >= a && e <= b);

    // Replacements: (start, end, converted text, original to echo in a comment).
    let mut reps: Vec<(usize, usize, String, Option<&str>)> = Vec::new();
    for c in re.captures_iter(line) {
        let m = c.get(0).unwrap();
        if within(&comments, m.start(), m.end()) {
            continue; // already converted on an earlier run — the original lives here
        }
        let Some(converted) = convert(&c) else { continue };
        if within(&brackets, m.start(), m.end()) {
            reps.push((m.start(), m.end(), converted.replace(' ', "_"), None));
        } else {
            reps.push((m.start(), m.end(), converted, Some(m.as_str())));
        }
    }
    if reps.is_empty() {
        return full.to_string();
    }

    let mut out = String::new();
    let mut pending: Vec<&str> = Vec::new();
    let mut reps = reps.into_iter().peekable();
    let mut i = 0;

    while i < line.len() {
        // Next event: a replacement to emit, or — once originals are pending —
        // the `;` that closes their declaration.
        let next_rep = reps.peek().map(|r| r.0);
        let next_semi = if pending.is_empty() {
            None
        } else {
            line.as_bytes()[i..]
                .iter()
                .enumerate()
                .map(|(off, &b)| (i + off, b))
                .find(|&(p, b)| b == b';' && !within(&comments, p, p + 1) && !within(&brackets, p, p + 1))
                .map(|(p, _)| p)
        };
        let stop = [next_rep, next_semi, Some(line.len())]
            .into_iter()
            .flatten()
            .min()
            .unwrap();
        out.push_str(&line[i..stop]);
        i = stop;
        if i >= line.len() {
            break;
        }

        if next_rep == Some(i) {
            let (_, end, text, original) = reps.next().unwrap();
            out.push_str(&text);
            if let Some(o) = original {
                pending.push(o);
            }
            i = end;
        } else {
            out.push(';');
            i += 1;
            // Merge into a comment already sitting right after the `;` (left
            // by a previous helper for this same declaration) instead of
            // stacking a second one.
            let ws_end = line[i..].find(|ch| ch != ' ').map_or(line.len(), |n| i + n);
            match comments.iter().find(|&&(cs, _)| cs == ws_end) {
                Some(&(cs, ce)) => {
                    let existing = line[cs + 2..ce - 2].trim();
                    out.push_str(&line[i..cs]);
                    out.push_str(&format!("/* {} {} */", existing, pending.join(" ")));
                    i = ce;
                }
                None => out.push_str(&format!(" /* {} */", pending.join(" "))),
            }
            pending.clear();
        }
    }

    // No `;` after the last converted value: the comment lands at the end of
    // the line, again merging into a trailing comment if one is there.
    if !pending.is_empty() {
        match out.ends_with("*/").then(|| out.rfind("/*")).flatten() {
            Some(open) => {
                let existing = out[open + 2..out.len() - 2].trim().to_string();
                out.truncate(open);
                out.push_str(&format!("/* {} {} */", existing, pending.join(" ")));
            }
            None => out.push_str(&format!(" /* {} */", pending.join(" "))),
        }
    }
    out.push_str(eol);
    out
}

// ---------------------------------------------------------------------------
// px → rem
// ---------------------------------------------------------------------------

fn px2rem(input: &str) -> String {
    let base: f64 = env::var("ZH_REM_BASE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16.0);

    let re = Regex::new(r"-?\d+(?:\.\d+)?px\b").unwrap();
    convert_values(input, &re, &|c: &Captures| {
        let px: f64 = c[0].strip_suffix("px").unwrap().parse().unwrap();
        Some(format!("{}rem", fmt(px / base, 4)))
    })
}

// ---------------------------------------------------------------------------
// hex → oklch
// ---------------------------------------------------------------------------

fn hex2oklch(input: &str) -> String {
    let re = Regex::new(r"#[0-9a-fA-F]{3,8}\b").unwrap();
    convert_values(input, &re, &|c: &Captures| {
        // None (e.g. a 5-digit "hex") leaves the match untouched.
        parse_hex(&c[0][1..])
            .map(|(r, g, b, alpha)| format_oklch(srgb_to_oklch(r, g, b), alpha))
    })
}

/// Returns (r, g, b) in 0..=255 and optional alpha in 0.0..=1.0.
fn parse_hex(hex: &str) -> Option<(u8, u8, u8, Option<f64>)> {
    let expanded: String = match hex.len() {
        3 | 4 => hex.chars().flat_map(|ch| [ch, ch]).collect(),
        6 | 8 => hex.to_string(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(&expanded[i..i + 2], 16).ok();
    let (r, g, b) = (byte(0)?, byte(2)?, byte(4)?);
    let alpha = if expanded.len() == 8 {
        Some(byte(6)? as f64 / 255.0)
    } else {
        None
    };
    Some((r, g, b, alpha))
}

/// sRGB (0..=255) → OKLCH (L: 0..1, C, H: degrees).
/// Matrices from Björn Ottosson's OKLab reference implementation.
fn srgb_to_oklch(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    fn linearize(c: u8) -> f64 {
        let c = c as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    let (r, g, b) = (linearize(r), linearize(g), linearize(b));

    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();

    let lightness = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let b2 = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;

    let chroma = (a * a + b2 * b2).sqrt();
    let mut hue = b2.atan2(a).to_degrees();
    if hue < 0.0 {
        hue += 360.0;
    }
    (lightness, chroma, hue)
}

fn format_oklch((l, c, h): (f64, f64, f64), alpha: Option<f64>) -> String {
    // Achromatic colors: hue is numerically meaningless noise, print 0.
    let (c_str, h_str) = if c < 1e-4 {
        ("0".to_string(), "0".to_string())
    } else {
        (fmt(c, 4), fmt(h, 2))
    };
    let base = format!("oklch({}% {} {}", fmt(l * 100.0, 2), c_str, h_str);
    match alpha {
        Some(a) => format!("{} / {}%)", base, fmt(a * 100.0, 1)),
        None => format!("{})", base),
    }
}

/// Format with up to `decimals` places, trailing zeros trimmed.
fn fmt(v: f64, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// now — refresh a timestamp to the current local time
// ---------------------------------------------------------------------------

/// Matches the timestamp shape produced by `date "+%Y-%m-%d at %I.%M.%S %p"`,
/// e.g. `2026-06-11 at 01.50.48 PM`, and replaces every occurrence with the
/// current local time in the same format. Lines without such a stamp are
/// returned untouched. Dates inside markdown link targets never reach this
/// helper — `apply` masks them out (see `Helper::sees_link_targets`).
fn now(input: &str) -> String {
    let re = Regex::new(r"\d{4}-\d{2}-\d{2} at \d{2}\.\d{2}\.\d{2} (?:AM|PM)").unwrap();

    // Bail out (and skip the subprocess) when there's nothing to refresh.
    if !re.is_match(input) {
        return input.to_string();
    }

    match current_timestamp() {
        Some(stamp) => re.replace_all(input, stamp.as_str()).into_owned(),
        None => input.to_string(),
    }
}

/// Shell out to `date` for the local time — it owns the timezone and the
/// 12-hour/AM-PM formatting, so we don't reimplement either. Returns `None`
/// (leaving the input untouched) if `date` is missing or fails.
fn current_timestamp() -> Option<String> {
    let out = std::process::Command::new("date")
        .arg("+%Y-%m-%d at %I.%M.%S %p")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stamp = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if stamp.is_empty() {
        None
    } else {
        Some(stamp)
    }
}

// ---------------------------------------------------------------------------
// mdlink — escape spaces in markdown link paths
// ---------------------------------------------------------------------------

/// Fixes broken relative links to static files whose path contains spaces, e.g.
/// `[doc](my notes.md)` -> `[doc](my%20notes.md)`. Only the path inside a
/// markdown `](...)` is touched, and only when it is a local path — absolute
/// URLs (`scheme://…`) and `mailto:` / `tel:` / `#anchor` targets are left
/// alone so we never mangle a real address.
///
/// Two whitespace characters are encoded: the regular space (` ` -> `%20`) and
/// the narrow no-break space U+202F (-> `%E2%80%AF`, its percent-encoded UTF-8
/// bytes), which sneaks in from macOS date/Finder strings. Re-running is a
/// no-op: once a space is `%20` there is nothing left to encode.
fn mdlink(input: &str) -> String {
    // Group 2 is the link target between `](` and the closing `)`. Paths that
    // themselves contain `)` are out of scope (same limitation as a one-liner).
    let re = Regex::new(r"(\]\()([^)]+)(\))").unwrap();
    // A path is "remote" — and therefore left untouched — when it starts with a
    // URL scheme like `https://`, or with `mailto:` / `tel:` / `#`.
    let remote = Regex::new(r"(?i)^(?:[a-z][a-z0-9+.-]*://|mailto:|tel:|#)").unwrap();

    re.replace_all(input, |c: &Captures| {
        let path = &c[2];
        if remote.is_match(path) {
            return c[0].to_string();
        }
        let fixed = path.replace(' ', "%20").replace('\u{202F}', "%E2%80%AF");
        format!("{}{}{}", &c[1], fixed, &c[3])
    })
    .into_owned()
}

// ---------------------------------------------------------------------------
// sort — sort the selected lines alphabetically
// ---------------------------------------------------------------------------

/// Sort the input's lines in ascending lexicographic order. This is meant for a
/// *visual* selection of multiple lines; sorting a single line is a no-op, which
/// is why the README binds it in visual mode only. A trailing newline (if the
/// input had one) is preserved so the filter pastes back cleanly, and sorting is
/// naturally idempotent — re-running over already-sorted lines changes nothing.
fn sort(input: &str) -> String {
    let trailing_newline = input.ends_with('\n');
    let mut lines: Vec<&str> = input.lines().collect();
    lines.sort();
    let mut output = lines.join("\n");
    if trailing_newline {
        output.push('\n');
    }
    output
}

// ---------------------------------------------------------------------------
// Tests (oklch reference values cross-checked against oklch.com)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<&'static Helper> {
        HELPERS.iter().filter(|h| h.in_all).collect()
    }

    // --- px2rem -----------------------------------------------------------

    #[test]
    fn px_css_declaration_comments_after_semicolon() {
        assert_eq!(
            px2rem("padding: 16px 8px;"),
            "padding: 1rem 0.5rem; /* 16px 8px */"
        );
    }

    #[test]
    fn px_tailwind_bracket_has_no_comment() {
        assert_eq!(px2rem("px-[16px]"), "px-[1rem]");
    }

    #[test]
    fn px_bare_value_comments_inline() {
        assert_eq!(px2rem("6px"), "0.375rem /* 6px */");
    }

    #[test]
    fn px_idempotent() {
        let once = px2rem("padding: 16px 8px;");
        assert_eq!(px2rem(&once), once);
    }

    // --- hex2oklch --------------------------------------------------------

    #[test]
    fn hex_css_declaration_comments_after_semicolon() {
        assert_eq!(hex2oklch("foo: #fff;"), "foo: oklch(100% 0 0); /* #fff */");
    }

    #[test]
    fn hex_tailwind_bracket_uses_underscores_no_comment() {
        assert_eq!(hex2oklch("text-[#fff]"), "text-[oklch(100%_0_0)]");
    }

    #[test]
    fn hex_alpha() {
        assert_eq!(
            hex2oklch("#ff000080"),
            "oklch(62.8% 0.2577 29.23 / 50.2%) /* #ff000080 */"
        );
    }

    #[test]
    fn hex_idempotent() {
        let once = hex2oklch("color: #ff0000;");
        assert_eq!(hex2oklch(&once), once);
    }

    // --- composition ------------------------------------------------------

    #[test]
    fn mixed_declaration_shares_one_comment() {
        let out = hex2oklch(&px2rem("border: 1px solid #3b3b3b;"));
        assert_eq!(
            out,
            "border: 0.0625rem solid oklch(35.23% 0 0); /* 1px #3b3b3b */"
        );
    }

    // --- now --------------------------------------------------------------

    #[test]
    fn now_replaces_the_stamp_only() {
        let stamp = Regex::new(r"^\d{4}-\d{2}-\d{2} at \d{2}\.\d{2}\.\d{2} (?:AM|PM)$").unwrap();
        let out = now("createdAt: 2026-06-11 at 01.50.48 PM");
        let value = out.strip_prefix("createdAt: ").unwrap();
        assert!(stamp.is_match(value), "got {out:?}");
    }

    // --- priorities -------------------------------------------------------

    #[test]
    fn link_targets_belong_to_mdlink() {
        // A screenshot filename holds a date, a "px value" and a "hex color" —
        // none of them may be converted; only mdlink escapes the spaces.
        let line = "![](../Resources/Screenshot 2026-06-11 at 11.44.05 AM 16px #fff.webp)";
        assert_eq!(
            apply(&all(), line.to_string()),
            "![](../Resources/Screenshot%202026-06-11%20at%2011.44.05%20AM%2016px%20#fff.webp)"
        );
    }

    #[test]
    fn now_still_refreshes_outside_a_link() {
        let out = apply(
            &all(),
            "updated: 2020-01-01 at 09.00.00 AM ![](a 2026-06-17 at 12.54.47 PM.webp)".to_string(),
        );
        assert!(!out.contains("2020-01-01"), "outside date refreshed: {out:?}");
        assert!(
            out.contains("![](a%202026-06-17%20at%2012.54.47%20PM.webp)"),
            "link only escaped, date kept: {out:?}"
        );
    }

    // --- mdlink -----------------------------------------------------------

    #[test]
    fn mdlink_escapes_local_paths_only() {
        assert_eq!(mdlink("[doc](my notes.md)"), "[doc](my%20notes.md)");
        assert_eq!(
            mdlink("[site](https://example.com/a b)"),
            "[site](https://example.com/a b)"
        );
    }

    #[test]
    fn mdlink_idempotent() {
        let once = mdlink("[doc](my notes.md)");
        assert_eq!(mdlink(&once), once);
    }

    // --- sort -------------------------------------------------------------

    #[test]
    fn sort_orders_lines_and_keeps_trailing_newline() {
        assert_eq!(sort("banana\napple\ncherry"), "apple\nbanana\ncherry");
        assert_eq!(sort("b\na\n"), "a\nb\n");
    }
}
