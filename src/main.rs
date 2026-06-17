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
    run: Transform,
}

/// To add a new helper: write a `fn(&str) -> String` and register it here.
const HELPERS: &[Helper] = &[
    Helper {
        name: "px2rem",
        aliases: &["px", "rem"],
        about: "6px -> 0.375rem /* 6px */",
        in_all: true,
        run: px2rem,
    },
    Helper {
        name: "hex2oklch",
        aliases: &["hex", "oklch"],
        about: "#ff0000 -> oklch(62.8% 0.2577 29.23) /* #ff0000 */",
        in_all: true,
        run: hex2oklch,
    },
    Helper {
        name: "now",
        aliases: &["date", "time"],
        about: "2026-06-11 at 01.50.48 PM -> current local time",
        in_all: true,
        run: now,
    },
    Helper {
        name: "mdlink",
        aliases: &["link", "links"],
        about: "[a](b c.md) -> [a](b%20c.md)  (escape spaces in md link paths)",
        in_all: true,
        run: mdlink,
    },
    Helper {
        name: "sort",
        aliases: &["asc"],
        about: "sort the selected lines alphabetically (opt-in; visual mode)",
        in_all: false,
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

    let output = selected
        .iter()
        .fold(input, |text, helper| (helper.run)(&text));

    // No extra newline: a vim filter must return exactly what it should paste back.
    io::stdout()
        .write_all(output.as_bytes())
        .expect("zh: failed to write stdout");
}

// ---------------------------------------------------------------------------
// px → rem
// ---------------------------------------------------------------------------

fn px2rem(input: &str) -> String {
    let base: f64 = env::var("ZH_REM_BASE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16.0);

    // Group 1 catches values already living inside a comment (`/* 6px */`)
    // so re-running zh on an already converted line is a no-op for them.
    let re = Regex::new(r"(/\*\s*)?(-?\d+(?:\.\d+)?)px\b").unwrap();

    re.replace_all(input, |c: &Captures| {
        if c.get(1).is_some() {
            return c[0].to_string();
        }
        let px: f64 = c[2].parse().unwrap();
        format!("{}rem /* {}px */", fmt(px / base, 4), &c[2])
    })
    .into_owned()
}

// ---------------------------------------------------------------------------
// hex → oklch
// ---------------------------------------------------------------------------

fn hex2oklch(input: &str) -> String {
    // Group 1 catches a hex already living inside a comment (`/* #ff0000 */`)
    // so re-running zh on an already converted line is a no-op for it.
    let re = Regex::new(r"(/\*\s*)?#([0-9a-fA-F]{3,8})\b").unwrap();

    re.replace_all(input, |c: &Captures| {
        if c.get(1).is_some() {
            return c[0].to_string();
        }
        match parse_hex(&c[2]) {
            // Echo the original hex as a trailing comment, lowercased (`#FF0000` -> `#ff0000`).
            Some((r, g, b, alpha)) => format!(
                "{} /* #{} */",
                format_oklch(srgb_to_oklch(r, g, b), alpha),
                c[2].to_lowercase()
            ),
            None => c[0].to_string(), // not a valid color length (e.g. 5 digits)
        }
    })
    .into_owned()
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
/// returned untouched, so running over a whole selection only rewrites dates.
fn now(input: &str) -> String {
    let re = Regex::new(r"\d{4}-\d{2}-\d{2} at \d{2}\.\d{2}\.\d{2} (?:AM|PM)").unwrap();

    // Bail out (and skip the subprocess) when there's nothing to refresh.
    if !re.is_match(input) {
        return input.to_string();
    }

    match current_timestamp() {
        Some(stamp) => re
            .replace_all(input, |_: &Captures| stamp.clone())
            .into_owned(),
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
// Tests (reference values cross-checked against oklch.com)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_basic() {
        assert_eq!(px2rem("margin: 6px;"), "margin: 0.375rem /* 6px */;");
    }

    #[test]
    fn px_idempotent() {
        let once = px2rem("padding: 12px 6px;");
        assert_eq!(px2rem(&once), once);
    }

    #[test]
    fn hex_red() {
        assert_eq!(
            hex2oklch("#ff0000"),
            "oklch(62.8% 0.2577 29.23) /* #ff0000 */"
        );
    }

    #[test]
    fn hex_uppercase_is_lowercased_in_comment() {
        assert_eq!(
            hex2oklch("#FF0000"),
            "oklch(62.8% 0.2577 29.23) /* #ff0000 */"
        );
    }

    #[test]
    fn hex_idempotent() {
        let once = hex2oklch("color: #FF0000;");
        assert_eq!(hex2oklch(&once), once);
    }

    #[test]
    fn hex_gray_is_achromatic() {
        assert_eq!(hex2oklch("#808080"), "oklch(59.99% 0 0) /* #808080 */");
    }

    #[test]
    fn hex_shorthand_and_alpha() {
        assert_eq!(hex2oklch("#fff"), "oklch(100% 0 0) /* #fff */");
        assert_eq!(
            hex2oklch("#ff000080"),
            "oklch(62.8% 0.2577 29.23 / 50.2%) /* #ff000080 */"
        );
    }

    #[test]
    fn now_replaces_the_stamp_only() {
        let stamp = Regex::new(r"^\d{4}-\d{2}-\d{2} at \d{2}\.\d{2}\.\d{2} (?:AM|PM)$").unwrap();
        let out = now("createdAt: 2026-06-11 at 01.50.48 PM");
        let value = out.strip_prefix("createdAt: ").unwrap();
        assert!(stamp.is_match(value), "got {out:?}");
        // The prefix (key) is preserved untouched.
        assert!(out.starts_with("createdAt: "));
    }

    #[test]
    fn now_leaves_non_dates_untouched() {
        assert_eq!(now("author: mioe"), "author: mioe");
        assert_eq!(now("margin: 6px;"), "margin: 6px;");
    }

    #[test]
    fn now_is_idempotent_in_shape() {
        // Re-running keeps producing a valid stamp (a fresh "now", same format).
        let stamp = Regex::new(r"^\d{4}-\d{2}-\d{2} at \d{2}\.\d{2}\.\d{2} (?:AM|PM)$").unwrap();
        let once = now("2026-06-11 at 01.50.48 PM");
        assert!(stamp.is_match(now(&once).trim()));
    }

    #[test]
    fn mdlink_escapes_spaces_in_path() {
        assert_eq!(mdlink("[doc](my notes.md)"), "[doc](my%20notes.md)");
    }

    #[test]
    fn mdlink_escapes_narrow_no_break_space() {
        assert_eq!(
            mdlink("[doc](a\u{202F}b.md)"),
            "[doc](a%E2%80%AFb.md)"
        );
    }

    #[test]
    fn mdlink_leaves_remote_urls_untouched() {
        // The query string has a space, but it's a real URL — don't touch it.
        assert_eq!(
            mdlink("[site](https://example.com/a b)"),
            "[site](https://example.com/a b)"
        );
        assert_eq!(mdlink("[mail](mailto:a b@x.com)"), "[mail](mailto:a b@x.com)");
        assert_eq!(mdlink("[top](#a b)"), "[top](#a b)");
    }

    #[test]
    fn mdlink_handles_image_and_multiple_links() {
        assert_eq!(
            mdlink("![alt](a b.png) and [x](c d.md)"),
            "![alt](a%20b.png) and [x](c%20d.md)"
        );
    }

    #[test]
    fn mdlink_idempotent() {
        let once = mdlink("[doc](my notes.md)");
        assert_eq!(mdlink(&once), once);
    }

    #[test]
    fn mdlink_leaves_non_links_untouched() {
        assert_eq!(mdlink("margin: 6px;"), "margin: 6px;");
    }

    #[test]
    fn sort_orders_lines() {
        assert_eq!(sort("banana\napple\ncherry"), "apple\nbanana\ncherry");
    }

    #[test]
    fn sort_preserves_trailing_newline() {
        assert_eq!(sort("b\na\n"), "a\nb\n");
        assert_eq!(sort("b\na"), "a\nb");
    }

    #[test]
    fn sort_is_idempotent() {
        let once = sort("c\na\nb");
        assert_eq!(sort(&once), once);
    }

    #[test]
    fn sort_single_line_is_noop() {
        assert_eq!(sort("only one line"), "only one line");
    }

    #[test]
    fn mixed_line() {
        let line = "border: 1px solid #3b3b3b;";
        let out = hex2oklch(&px2rem(line));
        assert_eq!(
            out,
            "border: 0.0625rem /* 1px */ solid oklch(35.23% 0 0) /* #3b3b3b */;"
        );
    }
}
