//! Print a built-in theme as a theme file, to start your own from:
//!
//! ```bash
//! cargo run -p mpfree-ui --example theme_file -- synthwave > mine.theme
//! ```

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "default".into());
    match mpfree_ui::Theme::builtin(&name) {
        Some(t) => print!("{}", t.to_text()),
        None => {
            eprintln!(
                "no built-in theme {name:?}; try {}",
                mpfree_ui::theme::BUILTIN.join(", ")
            );
            std::process::exit(2);
        }
    }
}
