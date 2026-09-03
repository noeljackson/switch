use crate::ctx::Ctx;
use crate::error::Result;
use crate::{out, outln};

/// Asks for a line of text, falling back to `default_val` when the answer is
/// blank. Port of `promptString`.
pub fn prompt_string(ctx: &mut Ctx, label: &str, default_val: &str) -> Result<String> {
    if !default_val.is_empty() {
        out!(ctx, "{label} ({default_val}): ");
    } else {
        out!(ctx, "{label}: ");
    }

    let input = ctx.read_line()?;
    let input = input.trim();
    if input.is_empty() {
        Ok(default_val.to_string())
    } else {
        Ok(input.to_string())
    }
}

/// Asks a yes/no question. Port of `promptYesNo`.
pub fn prompt_yes_no(ctx: &mut Ctx, label: &str, default_yes: bool) -> Result<bool> {
    let def = if default_yes { "Y/n" } else { "y/N" };
    out!(ctx, "{label} ({def}): ");

    let input = ctx.read_line()?;
    let input = input.trim().to_lowercase();
    if input.is_empty() {
        return Ok(default_yes);
    }
    Ok(input == "y" || input == "yes")
}

/// Offers a numbered menu, re-prompting until the answer is valid.
///
/// Port of `promptChoice`. `Ok(None)` is the empty-line "cancel" that Go
/// signalled by returning `-1`.
pub fn prompt_choice(ctx: &mut Ctx, title: &str, options: &[String]) -> Result<Option<usize>> {
    outln!(ctx, "{title}");
    for (i, opt) in options.iter().enumerate() {
        outln!(ctx, "  {}. {}", i + 1, opt);
    }

    loop {
        out!(ctx, "Choose (1-{}): ", options.len());
        let line = ctx.read_line()?;
        let line = line.trim();
        if line.is_empty() {
            return Ok(None);
        }

        if let Some(idx) = scan_leading_int(line) {
            if idx >= 1 && idx <= options.len() as i64 {
                return Ok(Some((idx - 1) as usize));
            }
        }
        outln!(ctx, "Invalid choice, try again.");
    }
}

/// Parses a leading integer, ignoring anything after it.
///
/// Matches `fmt.Sscanf(line, "%d", &idx)`, which accepts a sign, stops at the
/// first non-digit, and only fails when there is no digit at all — so "2abc"
/// scans as 2 while "abc" fails.
fn scan_leading_int(s: &str) -> Option<i64> {
    let mut chars = s.trim_start().chars().peekable();

    let mut negative = false;
    match chars.peek() {
        Some('+') => {
            chars.next();
        }
        Some('-') => {
            negative = true;
            chars.next();
        }
        _ => {}
    }

    let mut digits = String::new();
    while let Some(c) = chars.peek() {
        if c.is_ascii_digit() {
            digits.push(*c);
            chars.next();
        } else {
            break;
        }
    }

    if digits.is_empty() {
        return None;
    }
    let value: i64 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::{failing_stdin_ctx, test_ctx};

    fn opts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // Port of TestPrompts.
    #[test]
    fn prompts() {
        let (mut ctx, out, _) = test_ctx("/home/tester", "\nvalue\ny\n\n2\n");

        // A blank answer takes the default.
        assert_eq!(prompt_string(&mut ctx, "Name", "def").unwrap(), "def");
        // A typed answer wins over the default.
        assert_eq!(prompt_string(&mut ctx, "Name", "def").unwrap(), "value");
        // "y" is yes.
        assert!(prompt_yes_no(&mut ctx, "OK?", false).unwrap());
        // A blank answer takes the default.
        assert!(prompt_yes_no(&mut ctx, "OK?", true).unwrap());
        // Menu selection is 1-based on the wire, 0-based in the result.
        assert_eq!(
            prompt_choice(&mut ctx, "Pick:", &opts(&["a", "b"])).unwrap(),
            Some(1)
        );

        let text = out.contents();
        assert!(text.contains("Name (def): "), "{text}");
        assert!(text.contains("OK? (y/N): "), "{text}");
        assert!(text.contains("OK? (Y/n): "), "{text}");
        assert!(text.contains("  1. a"), "{text}");
        assert!(text.contains("Choose (1-2): "), "{text}");
    }

    // Port of TestPromptChoice_InvalidThenValid.
    #[test]
    fn prompt_choice_invalid_then_valid() {
        let (mut ctx, out, _) = test_ctx("/home/tester", "99\n2\n");
        let idx = prompt_choice(&mut ctx, "Title:", &opts(&["a", "b"])).unwrap();
        assert_eq!(idx, Some(1));
        assert!(out.contents().contains("Invalid choice, try again."));
    }

    // Port of TestPrompts_ErrorPaths.
    #[test]
    fn prompts_error_paths() {
        let (mut ctx, _, _) = failing_stdin_ctx("/home/tester");
        assert!(prompt_string(&mut ctx, "L", "").is_err());
        assert!(prompt_yes_no(&mut ctx, "L", true).is_err());
        assert!(prompt_choice(&mut ctx, "T", &opts(&["a"])).is_err());
    }

    #[test]
    fn prompt_choice_cancels_on_blank_line() {
        let (mut ctx, _, _) = test_ctx("/home/tester", "\n");
        assert_eq!(prompt_choice(&mut ctx, "T", &opts(&["a"])).unwrap(), None);
    }

    #[test]
    fn prompt_yes_no_variants() {
        let (mut ctx, _, _) = test_ctx("/home/tester", "YES\nNo\nmaybe\n");
        assert!(prompt_yes_no(&mut ctx, "?", false).unwrap());
        assert!(!prompt_yes_no(&mut ctx, "?", true).unwrap());
        // Anything that is not y/yes counts as no.
        assert!(!prompt_yes_no(&mut ctx, "?", true).unwrap());
    }

    #[test]
    fn scan_leading_int_matches_go_sscanf() {
        assert_eq!(scan_leading_int("2"), Some(2));
        assert_eq!(scan_leading_int("2abc"), Some(2));
        assert_eq!(scan_leading_int("abc"), None);
        assert_eq!(scan_leading_int("+2"), Some(2));
        assert_eq!(scan_leading_int("-1"), Some(-1));
        assert_eq!(scan_leading_int(" 2"), Some(2));
        assert_eq!(scan_leading_int("2 3"), Some(2));
        assert_eq!(scan_leading_int("02"), Some(2));
        assert_eq!(scan_leading_int("2.5"), Some(2));
        assert_eq!(scan_leading_int(""), None);
    }

    #[test]
    fn ctx_read_line_reports_eof_like_go() {
        // A final line without a newline is an error, as ReadString('\n') was.
        let (mut ctx, _, _) = test_ctx("/home/tester", "abc");
        assert!(prompt_string(&mut ctx, "L", "").is_err());
    }
}
