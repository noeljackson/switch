use std::process::exit;

use switch::cli;
use switch::ctx::Ctx;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut ctx = Ctx::real();
    let code = cli::run(&args, &mut ctx);
    // `exit` skips the runtime's stdout flush, so do it here.
    ctx.flush();
    exit(code);
}
