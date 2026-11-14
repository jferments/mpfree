//! CLAP plugins in the app ([plugins design](../../../docs/dev/design/plugins.md)).
//!
//! Every plugin runs in a plugin process of its own (the sandbox,
//! `mpfree-plughost`), never in the app's: a plugin that crashes, hangs or
//! blocks costs its own sound, never the music. The rack that finds,
//! starts, watches and restarts them is `mpfree_plughost::rack`; this is
//! the app's side of it.
//!
//! - **The plugin process** is `mpfree-plughost` beside the app if it is
//!   there (or `MPFREE_PLUGHOST`), else the app itself as `mpfree plughost
//!   ...` ([`program`], [`serve`]), so a stick with only the app on it still
//!   hosts plugins.
//! - **Their own files.** Each plugin process runs with its home (and its
//!   XDG folders) in `files/plugin-home/`, which the store makes
//!   ([`home`]), so what a plugin keeps for itself lands on the data
//!   partition and nowhere else. Without a data folder, a fresh folder in
//!   the system's temp folder, never the real home.
//! - **Finding them.** At start, `files/plugins/*.clap` in the data folder
//!   ([`scan`]). Effects go in the chain editor's picker after the built-in
//!   ones ([`picks`]); instruments wait for the track host.
//! - **Adding one.** The chain editor asks the builder thread (`control.rs`)
//!   for `clap:<plugin id>`; it calls `Rack::make`, which starts the
//!   process. The builder thread also calls `Rack::watch`, which restarts a
//!   crashed plugin from its last state.
//! - **Never hidden** (Jay's fallback rule, 12 Nov). Each start, crash,
//!   restart and give-up goes in the report as a `plugins:` line, and a
//!   `status: plugins:` line every 30 s gives the totals.

use std::path::{Path, PathBuf};

use mpfree_chain_ui::Pick;
use mpfree_plughost::rack::Rack;
use mpfree_plughost::Program;

/// The rack, named for the app.
pub type Plugins = Rack;

/// The data folder's plugins folder, under `files/` like the others.
pub const PLUGINS_FOLDER: &str = "plugins";
/// The plugin process's own binary, looked for beside the app.
pub const PLUGHOST: &str = "mpfree-plughost";
/// The app's subcommand that serves as the plugin process.
pub const SUBCOMMAND: &str = "plughost";
/// The plugin processes' home folder, under `files/` (storage-spec.md).
pub const HOME_FOLDER: &str = "plugin-home";

/// The plugin processes' home folder: `files/plugin-home/` in the data
/// folder, made by the store, or without one (or if it can't be made) a
/// folder of this run's own in the system's temp folder, which nothing
/// makes until a plugin does.
pub fn home(store: &mpfree_store::Store) -> PathBuf {
    store.dir(HOME_FOLDER).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("mpfree-plugin-home-{}", std::process::id()))
    })
}

/// The plugin process to start: `MPFREE_PLUGHOST`, else `mpfree-plughost`
/// beside `app`, else `app plughost`.
pub fn program(app: &Path) -> Program {
    if let Some(p) = std::env::var_os("MPFREE_PLUGHOST") {
        return Program::new(Path::new(&p));
    }
    let beside = app.with_file_name(PLUGHOST);
    if beside.is_file() {
        return Program::new(&beside);
    }
    Program::with_lead(app, &[SUBCOMMAND])
}

/// The plugins in `data/files/plugins/`, listed by `program`. None found
/// (or no data folder): an empty rack and no lines.
pub fn scan(program: Program, data: Option<&Path>) -> (Plugins, Vec<String>) {
    match data {
        Some(d) => Rack::scan(
            program,
            &d.join("files").join(PLUGINS_FOLDER),
            &format!("files/{PLUGINS_FOLDER}/"),
        ),
        None => (Rack::none(), Vec::new()),
    }
}

/// The running app's own path, for [`program`].
pub fn app_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mpfree"))
}

/// The chain editor picker's plugin cards: the effects, after the
/// built-in ones, under the Plugins heading.
pub fn picks(rack: &Plugins) -> Vec<Pick> {
    rack.effects()
        .map(|e| Pick {
            id: e.node_id,
            name: e.name,
            about: e.about,
            category: mpfree_fx::Category::Plugins,
        })
        .collect()
}

/// `mpfree plughost ...`: the app serving as one plugin process. Returns
/// its exit status.
pub fn serve(args: impl IntoIterator<Item = std::ffi::OsString>) -> i32 {
    mpfree_plughost::child::child_main(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_process_is_the_binary_beside_the_app_or_the_app_itself() {
        let d = std::env::temp_dir().join(format!("mpfree-plughost-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let app = d.join("mpfree");
        std::fs::write(&app, b"").unwrap();
        // MPFREE_PLUGHOST is not set in the tests' environment.
        assert!(std::env::var_os("MPFREE_PLUGHOST").is_none());
        assert_eq!(program(&app), Program::with_lead(&app, &["plughost"]));
        std::fs::write(d.join(PLUGHOST), b"").unwrap();
        assert_eq!(program(&app), Program::new(&d.join(PLUGHOST)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn plugins_live_in_the_data_folder_or_else_a_temp_folder_never_the_real_home() {
        let d =
            std::env::temp_dir().join(format!("mpfree-plugin-home-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let store = mpfree_store::Store::at(&d, |_| {});
        let h = home(&store);
        assert_eq!(h, d.join("files").join(HOME_FOLDER));
        assert!(h.is_dir(), "the store made it");
        let none = home(&mpfree_store::Store::none());
        assert!(none.starts_with(std::env::temp_dir()), "{}", none.display());
        assert!(!none.exists(), "nothing made");
        assert_eq!(
            program(&d.join("mpfree")).with_home(&h).home.as_deref(),
            Some(h.as_path())
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_data_folder_or_no_plugins_folder_means_no_plugins_and_no_lines() {
        let (r, notes) = scan(Program::new(Path::new("/nonexistent")), None);
        assert!(r.entries().is_empty() && notes.is_empty());
        let d = std::env::temp_dir().join(format!("mpfree-plugins-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("files/plugins")).unwrap();
        std::fs::write(d.join("files/plugins/readme.txt"), b"not a plugin").unwrap();
        let (r, notes) = scan(Program::new(Path::new("/nonexistent")), Some(&d));
        assert!(r.entries().is_empty() && notes.is_empty(), "{notes:?}");
        assert!(picks(&r).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
