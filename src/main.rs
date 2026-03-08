mod app;
mod config;
mod mock_slurm;
pub mod param_catalog;
mod sbatch_parser;
mod slurm_api;
mod tabs;
mod templates;
pub mod theme;
mod validators;

use app::App;
use config::load_config;
use slurm_api::SlurmController;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("slurm-term {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("slurm-term {} — Terminal UI for the Slurm workload manager", env!("CARGO_PKG_VERSION"));
        println!();
        println!("Usage: slurm-term [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --demo              Run with simulated cluster data");
        println!("  --file <PATH>       Load a .sbatch file into the Composer");
        println!("  --since <WINDOW>    Set initial history window (e.g. now-7days)");
        println!("  -V, --version       Print version");
        println!("  -h, --help          Print this help");
        return;
    }

    let demo = args.iter().any(|a| a == "--demo");

    let mut since: Option<String> = None;
    let mut load_file: Option<String> = None;
    for (i, a) in args.iter().enumerate() {
        if a == "--since" {
            if let Some(val) = args.get(i + 1) {
                since = Some(val.clone());
            }
        } else if let Some(val) = a.strip_prefix("--since=") {
            since = Some(val.to_string());
        } else if a == "--file" {
            if let Some(val) = args.get(i + 1) {
                load_file = Some(val.clone());
            }
        } else if let Some(val) = a.strip_prefix("--file=") {
            load_file = Some(val.to_string());
        }
    }

    let mut cfg = load_config(None);
    if let Some(s) = since {
        cfg.history_window = s;
    }

    templates::ensure_default_templates();

    let slurm: Box<dyn SlurmController + Send> = if demo {
        Box::new(mock_slurm::MockSlurmController::new(8, Some(42)))
    } else {
        Box::new(slurm_api::RealSlurmController)
    };

    if let Err(e) = App::run(slurm, cfg, load_file) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
