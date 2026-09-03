//! Terminal colours.
//!
//! The Go original emitted escape sequences unconditionally, so piping output
//! to a file embedded them. Colours are now chosen per run and carried on
//! [`Ctx`](crate::ctx::Ctx), so a redirected or `NO_COLOR` run is plain text.

/// The escape sequences to use, or empty strings when colour is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub reset: &'static str,
    pub red: &'static str,
    pub green: &'static str,
    pub yellow: &'static str,
    pub cyan: &'static str,
}

impl Palette {
    pub const ANSI: Palette = Palette {
        reset: "\x1b[0m",
        red: "\x1b[31m",
        green: "\x1b[32m",
        yellow: "\x1b[33m",
        cyan: "\x1b[36m",
    };

    pub const PLAIN: Palette = Palette {
        reset: "",
        red: "",
        green: "",
        yellow: "",
        cyan: "",
    };

    /// Picks a palette from the environment and whether output is a terminal.
    ///
    /// `NO_COLOR` (set and non-empty) wins, per <https://no-color.org>.
    /// `CLICOLOR_FORCE` keeps colour on when output is redirected, which is
    /// how CI logs stay readable.
    pub fn detect(is_terminal: bool) -> Palette {
        Palette::choose(
            is_terminal,
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("CLICOLOR_FORCE").ok().as_deref(),
        )
    }

    pub fn choose(is_terminal: bool, no_color: Option<&str>, force: Option<&str>) -> Palette {
        if matches!(no_color, Some(v) if !v.is_empty()) {
            return Palette::PLAIN;
        }
        if matches!(force, Some(v) if !v.is_empty() && v != "0") {
            return Palette::ANSI;
        }
        if is_terminal {
            Palette::ANSI
        } else {
            Palette::PLAIN
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_gets_colour_and_a_pipe_does_not() {
        assert_eq!(Palette::choose(true, None, None), Palette::ANSI);
        assert_eq!(Palette::choose(false, None, None), Palette::PLAIN);
    }

    #[test]
    fn no_color_wins_over_everything() {
        assert_eq!(Palette::choose(true, Some("1"), None), Palette::PLAIN);
        assert_eq!(Palette::choose(true, Some("1"), Some("1")), Palette::PLAIN);
        // An empty NO_COLOR does not count as set.
        assert_eq!(Palette::choose(true, Some(""), None), Palette::ANSI);
    }

    #[test]
    fn clicolor_force_keeps_colour_when_redirected() {
        assert_eq!(Palette::choose(false, None, Some("1")), Palette::ANSI);
        assert_eq!(Palette::choose(false, None, Some("0")), Palette::PLAIN);
        assert_eq!(Palette::choose(false, None, Some("")), Palette::PLAIN);
    }
}
