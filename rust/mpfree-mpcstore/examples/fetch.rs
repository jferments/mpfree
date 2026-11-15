//! Downloads one address from the catalog's sites, as MPFree's Plugins
//! page does, to check libcurl and the network on a machine:
//!
//!     cargo run -p mpfree-mpcstore --example fetch -- <https url> <out file>

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 {
        eprintln!("usage: fetch <https url> <out file>");
        std::process::exit(2);
    }
    let t = std::time::Instant::now();
    let mut shown = 0;
    let r = mpfree_mpcstore::fetch::fetch(
        &a[1],
        std::path::Path::new(&a[2]),
        512 << 20,
        &mut |d, total| {
            if d >= shown + (1 << 20) || (total > 0 && d == total && d != shown) {
                shown = d;
                eprintln!("{} of {} KB", d / 1024, total / 1024);
            }
            true
        },
    );
    match r {
        Ok(n) => println!("{n} bytes in {:.1} s", t.elapsed().as_secs_f64()),
        Err(e) => {
            println!("failed: {e}");
            std::process::exit(1);
        }
    }
}
