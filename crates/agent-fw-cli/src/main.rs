use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agent-fw", about = "agent-fw framework utilities")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a small Rust framework project skeleton.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init { path } => scaffold_project(&path),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn scaffold_project(path: &Path) -> anyhow::Result<()> {
    let project_dir = if path == Path::new(".") {
        std::env::current_dir()?
    } else {
        fs::create_dir_all(path)?;
        path.canonicalize()?
    };

    let project_name = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("my-agent");

    let cargo_toml = format!(
        r#"[package]
name = "{project_name}"
version = "0.1.0"
edition = "2021"

[dependencies]
agent-fw-agent = {{ path = "path/to/agent-fw/crates/agent-fw-agent" }}
agent-fw-core = {{ path = "path/to/agent-fw/crates/agent-fw-core" }}
agent-fw-tool = {{ path = "path/to/agent-fw/crates/agent-fw-tool" }}
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}
"#,
    );
    write_if_missing(&project_dir.join("Cargo.toml"), &cargo_toml)?;

    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)?;
    let main_rs = r#"fn main() {
    println!("agent-fw project skeleton");
}
"#;
    write_if_missing(&src_dir.join("main.rs"), main_rs)?;

    let readme = format!(
        r#"# {project_name}

This skeleton is intentionally headless. The old Rust Studio/Ops launcher has
been removed from `agent-fw`; wire framework crates directly or use the current
Flow AI harness runtime path when it is available on your branch.
"#,
    );
    write_if_missing(&project_dir.join("README.md"), &readme)?;

    println!(
        "Initialized headless agent-fw project at {}",
        project_dir.display()
    );
    Ok(())
}

fn write_if_missing(path: &Path, content: &str) -> anyhow::Result<()> {
    if path.exists() {
        println!("  skip  {}", path.display());
    } else {
        fs::write(path, content)?;
        println!("  create {}", path.display());
    }
    Ok(())
}
