//! Installs release zips into a plugin folder as MPFree's Plugins page
//! will, checking each against the catalog when it is listed there:
//!
//!     cargo run -p mpfree-mpcstore --example install -- <catalog.json> <dest folder> <zip>...
//!
//! With no zips it prints what the catalog says MPFree can run.

use mpfree_mpcstore::{catalog, install, sha256};
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: install <catalog.json> <dest folder> [zip...]");
        std::process::exit(2);
    }
    let text = std::fs::read_to_string(&a[1]).expect("catalog");
    let cat = catalog::parse(&text).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let mut why = std::collections::BTreeMap::<&str, usize>::new();
    for p in &cat.plugins {
        *why.entry(if p.fit == catalog::Fit::Yes {
            "runs"
        } else {
            p.fit.why_not()
        })
        .or_default() += 1;
    }
    println!(
        "catalog of {}: {} plugins",
        cat.generated,
        cat.plugins.len()
    );
    for (w, n) in why {
        println!("  {n:4}  {w}");
    }
    let dest = Path::new(&a[2]);
    for z in &a[3..] {
        let bytes = std::fs::read(z).expect("zip");
        let sum = sha256::hex(&bytes);
        let known = cat.by_sha256(&sum);
        match known {
            Some((p, r)) => println!(
                "{z}: {} {} by {} ({})",
                p.name, r.version, p.author, p.license
            ),
            None => println!("{z}: not in the catalog (SHA-256 {sum})"),
        }
        let from = known.map_or(install::Origin::default(), |(p, r)| install::Origin {
            id: &p.id,
            version: &r.version,
        });
        match install::install(&bytes, dest, known.map(|(_, r)| r.sha256.as_str()), from) {
            Ok(done) => {
                for d in done {
                    println!(
                        "  -> {} ({} files, {} KB{}{})",
                        d.folder.display(),
                        d.files,
                        d.bytes / 1024,
                        if d.replaced { ", replaced" } else { "" },
                        if d.links_skipped > 0 {
                            ", links skipped"
                        } else {
                            ""
                        }
                    );
                }
            }
            Err(e) => println!("  not installed: {e}"),
        }
    }
}
