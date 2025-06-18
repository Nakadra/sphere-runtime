// --- Imports ---
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

// --- CLI Definition using clap ---
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Run in quiet mode, suppressing status messages
    #[arg(short, long, global = true)]
    quiet: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run a .sphere file
    Run {
        /// The .sphere file to execute
        #[arg(required = true)]
        file_path: PathBuf,
    },
    /// Manage the local Sphere cache
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand, Debug)]
enum CacheAction {
    /// List all Spheres in the local cache index
    List,
    /// Add a Sphere to the local cache index
    Add {
        /// The unique ID of the Sphere (e.g., com.example/my-tool/v1)
        #[arg(required = true)]
        id: String,
        /// The path to the .sphere file to add
        #[arg(required = true)]
        file_path: PathBuf,
        /// Optionally copy the file into the cache directory
        #[arg(long)]
        copy_to_cache: bool,
    },
    /// Remove a Sphere from the local cache index
    Remove {
        /// The unique ID of the Sphere to remove
        #[arg(required = true)]
        id: String,
    },
}

// --- Data Structures for Sphere ---
#[derive(Deserialize, Debug)]
struct SphereProcess {
    entrypoint: String,
    dependencies: Option<HashMap<String, String>>,
}

struct Dependency {
    alias: String,
    process: SphereProcess,
}

// --- Helper Functions for Cache Management ---
fn get_cache_paths() -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let home_dir = std::env::var("HOME")?;
    let cache_root = PathBuf::from(home_dir).join(".sphere");
    let cache_dir = cache_root.join("cache");
    fs::create_dir_all(&cache_dir)?;
    let index_path = cache_dir.join("index.json");
    Ok((cache_dir, index_path))
}

fn load_cache_index(index_path: &Path) -> Result<HashMap<String, String>, Box<dyn Error>> {
    if !index_path.exists() {
        return Ok(HashMap::new());
    }
    let index_content = fs::read_to_string(index_path)?;
    if index_content.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let index: HashMap<String, String> = serde_json::from_str(&index_content)?;
    Ok(index)
}

fn save_cache_index(index_path: &Path, index: &HashMap<String, String>) -> Result<(), Box<dyn Error>> {
    let index_content = serde_json::to_string_pretty(index)?;
    fs::write(index_path, index_content)?;
    Ok(())
}

// --- Cache Command Handlers ---
fn handle_cache_list(quiet: bool) -> Result<(), Box<dyn Error>> {
    if !quiet {
        println!("-> Listing Spheres in local cache index...");
    }
    let (_cache_dir, index_path) = get_cache_paths()?;
    let index = load_cache_index(&index_path)?;

    if index.is_empty() {
        println!("   Cache index is empty or not found at '{}'.", index_path.display());
    } else {
        if !quiet {
            println!("   Cache index location: '{}'", index_path.display());
        }
        println!("   --------------------------------------------------");
        println!("   Sphere ID                             | Filename");
        println!("   --------------------------------------------------");
        let mut sorted_index: Vec<_> = index.into_iter().collect();
        sorted_index.sort_by(|a, b| a.0.cmp(&b.0));
        for (id, filename) in sorted_index {
            println!("   {:<35} | {}", id, filename);
        }
        println!("   --------------------------------------------------");
    }
    Ok(())
}

fn handle_cache_add(id: &str, file_path: &PathBuf, copy_to_cache: bool, quiet: bool) -> Result<(), Box<dyn Error>> {
    if !quiet {
        println!("-> (Not Implemented Yet) Adding Sphere ID: {}", id);
        println!("   File path: {}", file_path.display());
        println!("   Copy to cache: {}", copy_to_cache);
    }
    // TODO: Implement actual logic
    Ok(())
}

fn handle_cache_remove(id: &str, quiet: bool) -> Result<(), Box<dyn Error>> {
    if !quiet {
        println!("-> (Not Implemented Yet) Removing Sphere ID: {}", id);
    }
    // TODO: Implement actual logic
    Ok(())
}

// --- Main Application Logic for 'sphere run' ---
fn run_sphere(file_path: &Path, quiet: bool) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read sphere file '{}': {}", file_path.display(), e))?;
    let sphere_process: SphereProcess = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse TOML from '{}': {}", file_path.display(), e))?;
    
    if !quiet {
        println!("-> Parsed entrypoint: '{}' from '{}'", &sphere_process.entrypoint, file_path.display());
    }

    let mut resolved_deps: Vec<Dependency> = Vec::new();
    if let Some(deps) = &sphere_process.dependencies {
        if !quiet {
            println!("-> Resolving dependencies...");
        }
        let (cache_dir, index_path) = get_cache_paths()?;
        let index = load_cache_index(&index_path)?;
        if !quiet && !index.is_empty() {
             println!("   - Loaded cache index from '{}'.", index_path.display());
        } else if !quiet && index.is_empty() {
             println!("   - Cache index at '{}' is empty or not found.", index_path.display());
        }


        for (alias, sphere_id) in deps {
            let dep_filename = index.get(sphere_id).ok_or_else(|| {
                format!("Dependency ID '{}' (aliased as '{}') not found in cache index!", sphere_id, alias)
            })?;
            let dep_path = cache_dir.join(dep_filename);
            if !quiet {
                println!("   - Loading dependency '{}' (Sphere ID: '{}') from '{}'", alias, sphere_id, dep_path.display());
            }

            let dep_content = fs::read_to_string(&dep_path)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Box::new(std::io::Error::new(std::io::ErrorKind::Other, format!(
                            "Dependency file '{}' (for Sphere ID '{}', aliased as '{}') not found in cache at path '{}'. Please ensure it is installed correctly.",
                            dep_filename, sphere_id, alias, dep_path.display()
                        ))) as Box<dyn Error>
                    } else {
                        Box::new(e) as Box<dyn Error>
                    }
                })?;
            let dep_process: SphereProcess = toml::from_str(&dep_content)
                .map_err(|e| format!("Failed to parse TOML for dependency '{}': {}", sphere_id, e))?;

            resolved_deps.push(Dependency {
                alias: alias.clone(),
                process: dep_process,
            });
        }
    }

    let temp_dir = tempdir()?;
    if !quiet {
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
    let new_path = format!("{}:{}", bin_path.to_string_lossy(), original_path);
    
    if !quiet {
        println!("-> Executing entrypoint inside sandbox...");
    }
    let output = Command::new("sh")
        .arg("-c")
        .arg(&sphere_process.entrypoint)
        .current_dir(temp_dir.path())
        .env("PATH", new_path)
        .output()?;
    if !quiet {
        println!("-> Execution finished.\n");
    }
    
    if !quiet { // Only print STDOUT/STDERR block headers if not in quiet mode
        println!("--- Command STDOUT ---");
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        println!("{}", stdout);
    } else if !quiet { // Only print (empty) if not quiet and stdout is empty
        println!("(empty)");
    }
    if !quiet {
        println!("----------------------");
    }


    if !output.stderr.is_empty() {
        if !quiet { // Only print STDERR block headers if not in quiet mode
            println!("\n--- Command STDERR ---");
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        println!("{}", stderr); // Always print stderr content if it exists
        if !quiet {
            println!("----------------------");
        }
    }
    Ok(())
}

// --- Main function: Parses CLI args and dispatches to handlers ---
fn main() {
    let cli = Cli::parse();

    let result = match &cli.command { // Borrow cli.command
        Commands::Run { file_path } => {
            run_sphere(file_path, cli.quiet)
        }
        Commands::Cache { action } => match action { // action is already a reference due to 'ref'
            CacheAction::List => {
                handle_cache_list(cli.quiet)
            }
            CacheAction::Add { id, file_path, copy_to_cache } => {
                handle_cache_add(id, file_path, *copy_to_cache, cli.quiet) // Dereference copy_to_cache
            }
            CacheAction::Remove { id } => {
                handle_cache_remove(id, cli.quiet)
            }
        },
    };

    if let Err(e) = result {
        let mut error_message = format!("{}", e);
        let mut specific_error_handled = false;
        let mut file_path_for_error: Option<String> = None;

        if let Commands::Run { ref file_path } = cli.command {
            file_path_for_error = Some(file_path.display().to_string());
        }

        if let Some(toml_error) = e.downcast_ref::<toml::de::Error>() {
            if toml_error.message().contains("missing field `entrypoint`") {
                let path_str = file_path_for_error.as_deref().unwrap_or("the .sphere file");
                error_message = format!("The file '{}' is missing the required 'entrypoint' field.", path_str);
                specific_error_handled = true;
            } else if file_path_for_error.is_some() {
                 error_message = format!("Failed to parse TOML from '{}'. Reason: {}", file_path_for_error.unwrap(), toml_error);
                 specific_error_handled = true;
            }
        }
        
        if !specific_error_handled {
            if !e.to_string().starts_with("Dependency file") && !e.to_string().starts_with("Failed to read sphere file") && !e.to_string().starts_with("Failed to parse TOML from") {
                 error_message = format!("Application error: {}", e);
            }
        }
        
        eprintln!("\nError: {}", error_message.trim());
        std::process::exit(1);
    }
}
