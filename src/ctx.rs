use std::cell::RefCell;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::colors::Palette;
use crate::paths::home_from_env;

/// Everything the program needs from the outside world: where the user's home
/// directory is, and the three standard streams.
///
/// The Go original reached for the process globals directly (`os.Stdout`, and a
/// package-level `stdinReader`) and its tests monkeypatched them. Rust has no
/// safe equivalent, so the same seams are expressed as injected values here.
/// This is the one structural difference between the two implementations.
pub struct Ctx {
    pub input: Box<dyn BufRead>,
    pub out: Box<dyn Write>,
    pub err: Box<dyn Write>,
    pub home: PathBuf,
    /// `$EDITOR`; `None` when unset.
    pub editor: Option<String>,
    /// The search path used to locate an editor. `None` means "read `$PATH`
    /// when needed", which is what the real binary does.
    pub path_env: Option<String>,
    /// Escape sequences for output, or blanks when colour is off.
    pub colors: Palette,
}

impl Ctx {
    /// The context used by the real binary: actual stdio and `$HOME`.
    ///
    /// A home directory that cannot be resolved is left empty rather than
    /// failing here; `Switcher::new` reports it, matching `getHomeDir`'s error
    /// being surfaced by `NewSwitcher` while `expandPath` silently ignored it.
    pub fn real() -> Ctx {
        Ctx {
            input: Box::new(BufReader::new(io::stdin())),
            out: Box::new(io::stdout()),
            err: Box::new(io::stderr()),
            home: home_from_env().unwrap_or_default(),
            editor: std::env::var("EDITOR").ok(),
            path_env: None,
            colors: Palette::detect(io::stdout().is_terminal()),
        }
    }

    pub fn new(
        home: impl Into<PathBuf>,
        input: Box<dyn BufRead>,
        out: Box<dyn Write>,
        err: Box<dyn Write>,
    ) -> Ctx {
        Ctx {
            input,
            out,
            err,
            home: home.into(),
            editor: None,
            path_env: None,
            // Tests read plain text; the real binary decides from the terminal.
            colors: Palette::PLAIN,
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Reads one line, reproducing Go's `bufio.Reader.ReadString('\n')`:
    /// the newline is kept, and hitting EOF before one is found is an error
    /// even when some bytes were read.
    ///
    /// Pending output is flushed first so that a prompt written without a
    /// trailing newline reaches the terminal before we block on input. Go's
    /// `fmt.Printf` writes straight to the fd, so this had no counterpart.
    pub fn read_line(&mut self) -> io::Result<String> {
        let _ = self.out.flush();
        let mut s = String::new();
        let n = self.input.read_line(&mut s)?;
        if n == 0 || !s.ends_with('\n') {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"));
        }
        Ok(s)
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
        let _ = self.err.flush();
    }
}

/// Writes to `ctx.out`. Write errors are dropped, as Go's `fmt.Printf` did.
#[macro_export]
macro_rules! out {
    ($ctx:expr, $($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = write!($ctx.out, $($arg)*);
    }};
}

/// Writes a line to `ctx.out`.
#[macro_export]
macro_rules! outln {
    ($ctx:expr) => {{
        use std::io::Write as _;
        let _ = writeln!($ctx.out);
    }};
    ($ctx:expr, $($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!($ctx.out, $($arg)*);
    }};
}

/// Writes a line to `ctx.err`.
#[macro_export]
macro_rules! errln {
    ($ctx:expr, $($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!($ctx.err, $($arg)*);
    }};
}

/// A `Write` sink whose contents can still be read after the writer has been
/// handed away, standing in for the `os.Pipe` plumbing in `captureOutput`.
#[derive(Clone, Default)]
pub struct SharedBuf(Rc<RefCell<Vec<u8>>>);

impl SharedBuf {
    pub fn new() -> SharedBuf {
        SharedBuf::default()
    }

    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.borrow()).into_owned()
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A reader that always fails, mirroring the test-only `badReader` in
/// `switch_test.go`.
pub struct ErrReader;

impl Read for ErrReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("read error"))
    }
}

/// Builds a context wired to in-memory streams. Returns the captured stdout and
/// stderr buffers alongside it.
pub fn test_ctx(home: impl Into<PathBuf>, stdin: &str) -> (Ctx, SharedBuf, SharedBuf) {
    let out = SharedBuf::new();
    let err = SharedBuf::new();
    let ctx = Ctx::new(
        home,
        Box::new(io::Cursor::new(stdin.as_bytes().to_vec())),
        Box::new(out.clone()),
        Box::new(err.clone()),
    );
    (ctx, out, err)
}

/// Like [`test_ctx`], but stdin always fails.
pub fn failing_stdin_ctx(home: impl Into<PathBuf>) -> (Ctx, SharedBuf, SharedBuf) {
    let out = SharedBuf::new();
    let err = SharedBuf::new();
    let ctx = Ctx::new(
        home,
        Box::new(BufReader::new(ErrReader)),
        Box::new(out.clone()),
        Box::new(err.clone()),
    );
    (ctx, out, err)
}
