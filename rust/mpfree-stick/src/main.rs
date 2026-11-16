//! `mpfree-stick`: the MPFree USB creator's command line ([`mpfree_stick::cli`]).

fn main() {
    std::process::exit(mpfree_stick::cli::main(std::env::args().skip(1).collect()));
}
