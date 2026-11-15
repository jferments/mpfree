//! Writes the parts of each MPFree plugin folder that come from our code:
//! `<out>/<vendor> - VST - <name>/plugin.json` and `version.xml`, and
//! `<out>/build/<lib>/ui.json` for the SDK's gen_ui.py. Run by
//! `tools/mpc-plugins/make-folders.sh`, which adds the libraries and skins.
//!
//! Usage: make_folders OUT_DIR

#[path = "common/plugins.rs"]
mod plugins;

use std::fs;
use std::path::PathBuf;

/// The characters the MPC strips when it builds a folder name.
fn folder_name(vendor: &str, name: &str) -> String {
    let strip = |s: &str| {
        s.chars()
            .filter(|c| !"\"#@,;:<>*^|?\\/".contains(*c))
            .collect::<String>()
    };
    format!("{} - VST - {}", strip(vendor), strip(name))
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: make_folders OUT_DIR"),
    );
    for (spec, lib, category) in plugins::ALL {
        let folder = out.join(folder_name(spec.vendor, spec.name));
        fs::create_dir_all(folder.join("VST")).unwrap();
        let uid = String::from_utf8(spec.uid.to_vec()).unwrap();
        let (kind, inputs) = if spec.instrument {
            ("instrument", 0)
        } else {
            ("effect", 2)
        };
        let slug = lib.replace('_', "-");
        let json = format!(
            "{{\n  \"name\": \"{}\",\n  \"manufacturer\": \"{}\",\n  \"version\": \"0.1.{}\",\n  \"format\": \"vst2\",\n  \"kind\": \"{kind}\",\n  \"uid\": \"{uid}\",\n  \"category\": \"{category}\",\n  \"numInputs\": {inputs},\n  \"numOutputs\": 2,\n  \"file32\": \"VST/{slug}-arm32.so\",\n  \"file64\": \"VST/{slug}-arm64.so\",\n  \"license\": \"MIT\",\n  \"url\": \"https://github.com/jferments/mpfree\"\n}}\n",
            spec.name, spec.vendor, spec.version
        );
        fs::write(folder.join("plugin.json"), json).unwrap();
        fs::write(
            folder.join("version.xml"),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<plugincontent version=\"1.0\">\n    <version>0.1.0.0</version>\n    <uiversion>0.1.0.0</uiversion>\n</plugincontent>\n",
        )
        .unwrap();
        let build = out.join("build").join(lib);
        fs::create_dir_all(&build).unwrap();
        let node = (spec.make)();
        fs::write(
            build.join("ui.json"),
            mpfree_mpcplug::ui_json(node.as_ref()),
        )
        .unwrap();
        println!("{}\t{lib}\t{slug}", folder.display());
    }
}
