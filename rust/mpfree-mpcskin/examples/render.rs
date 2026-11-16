//! Renders each tab of an MPC plugin folder's skin to a PNG, with every
//! parameter at the middle of its range (no plugin is loaded):
//!
//!     cargo run -p mpfree-mpcskin --example render -- <plugin folder> <out dir> [value]
//!
//! For checking the drawing against the screenshots the plugin's authors
//! publish. Prints each file with the skin's warnings.

use mpfree_mpcskin::handle::Params;
use mpfree_mpcskin::view::{guess_params, load, View, MPC_ONE, TAB_H, TAB_W};
use mpfree_ui::{Rect, SoftCanvas};
use std::path::Path;

struct Middle {
    n: usize,
    v: f32,
}

impl Params for Middle {
    fn count(&self) -> usize {
        self.n
    }
    fn value(&self, _: usize) -> f32 {
        self.v
    }
    fn name(&self, i: usize) -> String {
        format!("Param {}", i + 1)
    }
    fn text(&self, _: usize) -> String {
        format!("{:.2}", self.v)
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: render <plugin folder> <out dir> [value 0..1]");
        std::process::exit(2);
    }
    let plugin = Path::new(&a[1]);
    let out = Path::new(&a[2]);
    let v: f32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.5);
    std::fs::create_dir_all(out).expect("out dir");
    let params = Middle {
        n: guess_params(plugin),
        v,
    };
    let (skin, qlinks) = match load(plugin, &MPC_ONE, params.n) {
        Ok(Some(s)) => s,
        Ok(None) => {
            println!("no skin: the MPC shows its default pages");
            return;
        }
        Err(e) => {
            println!("skin doesn't read ({e}): the MPC shows no tabs");
            std::process::exit(1);
        }
    };
    for w in &skin.warnings {
        println!("warning: {w}");
    }
    let mut view = View::new(skin, qlinks);
    let tabs = view.tabs();
    for (i, (name, key, sub)) in tabs.iter().enumerate() {
        view.set_tab(i);
        let mut c = SoftCanvas::new(TAB_W as i32, TAB_H as i32);
        let t = std::time::Instant::now();
        view.draw(&params, &mut c, Rect::new(0, 0, TAB_W as i32, TAB_H as i32));
        let first = t.elapsed();
        let t = std::time::Instant::now();
        view.draw(&params, &mut c, Rect::new(0, 0, TAB_W as i32, TAB_H as i32));
        let again = t.elapsed();
        let safe: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let file = out.join(format!("tab{}-{safe}.png", i + 1));
        let png = mpfree_ui::png::encode_rgb(TAB_W as u32, TAB_H as u32, &c.to_rgb());
        std::fs::write(&file, png).expect("write png");
        println!(
            "{}  F{} sub {}  first {:.1} ms, again {:.1} ms, images {} KB",
            file.display(),
            key + 1,
            sub + 1,
            first.as_secs_f64() * 1e3,
            again.as_secs_f64() * 1e3,
            view.images.bytes() / 1024
        );
    }
}
