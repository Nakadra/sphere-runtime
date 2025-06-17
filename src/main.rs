// --- Imports ---
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

// --- CLI Definition ---
/// A next-generation, sandboxed command runner
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// The .sphere file to execute
    #[arg(required = true)]
    file_path: PathBuf,

    /// Run in quiet mode, suppressing status messages
    #[arg(short, long)]
    quiet: bool,
}

// --- Data Structures ---
#[derive(Deserialize, Debug)]
struct SphereProcess {
    entrypoint: String,
    dependencies: Option<HashMap<String, String>>,
}

struct Dependency {
    alias: String,
    process: SphereProcess,
}

// --- Main Application Logic ---
fn run_sphere(file_path: &Path, quiet_mode: bool) -> Result<(), Box<dyn Error>> {
    // --- PARSING LOGIC ---
    let content = fs::read_to_string(file_path)?;
    let sphere_process: SphereProcess = toml::from_str(&content)?;
    if !quiet_mode {
        println!("-> Parsed entrypoint: '{}'", &sphere_process.entrypoint);
    }

    // --- DEPENDENCY RESOLUTION ---
    let mut resolved_deps: Vec<Dependency> = Vec::new();
    if let Some(deps) = &sphere_process.dependencies {
        if !quiet_mode {
            println!("-> Resolving dependencies...");
        }
        
        let home = std::env::var("HOME")?;
        let cache_dir = format!("{}/.sphere/cache", home);
        let index_path = format!("{}/index.json", cache_dir);
        let index_content = fs::read_to_string(index_path)?;
        let index: HashMap<String, String> = serde_json::from_str(&index_content)?;
        if !quiet_mode {
            println!("   - Loaded cache index.");
        }

        for (alias, sphere_id) in deps {
            let dep_filename = index.get(sphere_id).ok_or_else(|| {
                format!("Dependency '{}' not found in cache index!", sphere_id)
            })?;
            
            let dep_path = format!("{}/{}", cache_dir, dep_filename);
            
            if !quiet_mode {
                println!("   - Loading '{}' as '{}' from {}", sphere_id, alias, dep_path);
            }

            let dep_content = fs::read_to_string(&dep_path)?;
            let dep_process: SphereProcess = toml::from_str(&dep_content)?;

            resolved_deps.push(Dependency {
                alias: alias.clone(),
                process: dep_process,
            });
        }
    }

    // --- SANDBOX & EXECUTION ---
    let temp_dir = tempdir()?;
    if !quiet_mode {
        println!("-> Created secure sandbox at: {:?}", temp_dir.path());
    }
    let bin_path = temp_dir.path().join("bin");
    fs::create_dir(&bin_path)?;

    for dep in &resolved_deps {
        let script_path = bin_path.join(&dep.alias);
        let mut script_file = fs::File::create(&script_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = script_file.metadata()?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms)?;
        }
        writeln!(script_file, "#!/bin/sh")?;
        writeln!(script_file, "{}", dep.process.entrypoint)?;
    }
    
    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_path.to_str().unwrap(), original_path);
    
    if !quiet_mode {
        println!("-> Executing entrypoint inside sandbox...");
    }
    let output = Command::new("sh")
        .arg("-c")
        .arg(&sphere_process.entrypoint)
        .current_dir(temp_dir.path())
        .env("PATH", new_path)
        .output()?;
    if !quiet_mode {
        println!("-> Execution finished.\n");
    }
    
    // --- OUTPUT HANDLING (Always print STDOUT/STDERR from the Sphere itself) ---
    // Check if STDOUT has content before printing headers to avoid empty blocks
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    if !stdout_str.trim().is_empty() {
        println!("--- Command STDOUT ---");
        println!("{}", stdout_str.trim());
        println!("----------------------");
    }

    // Check if STDERR has content
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    if !stderr_str.trim().is_empty() {
        println!("\n--- Command STDERR ---");
        println!("{}", stderr_str.trim());
        println!("----------------------");
    }

    Ok(())
}


// --- Main function: Parser, error handler, and now passes quiet flag ---
fn main() {
    let cli = Cli::parse();
    
    if let Err(e) = run_sphere(&cli.file_path, cli.quiet) {
        if let Some(toml_error) = e.downcast_ref::<toml::de::Error>() {
            if toml_error.message().contains("missing field `entrypoint`") {
                eprintln!("\nError: The file '{}' is missing the required 'entrypoint' field.", cli.file_path.display());
            } else {
                eprintln!("\nError: Failed to parse '{}'.", cli.file_path.display());
                eprintln!("Reason: {}", toml_error);
            }
        } else {
            eprintln!("\nApplication error: {}", e);
        }
        
        std::process::exit(1);
    }
}
