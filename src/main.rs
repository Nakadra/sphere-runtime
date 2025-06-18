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
        sphere_file_path: PathBuf, // Renamed for clarity in this context
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
    let home_dir = std::env::var("HOME")
        .map_err(|_| "Could not determine home directory. Is HOME environment variable set?")?;
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
    let index: HashMap<String, String> = serde_json::from_str(&index_content)
        .map_err(|e| format!("Failed to parse cache index '{}': {}. Ensure it is valid JSON.", index_path.display(), e))?;
    Ok(index)
}

fn save_cache_index(index_path: &Path, index: &HashMap<String, String>) -> Result<(), Box<dyn Error>> {
    let index_content = serde_json::to_string_pretty(index)?;
    fs::write(index_path, index_content)
        .map_err(|e| format!("Failed to save cache index to '{}': {}", index_path.display(), e))?;
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
        println!("   Sphere ID                             | Filename/Path in Cache");
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

fn handle_cache_add(id: &str, sphere_file_path_arg: &PathBuf, copy_to_cache: bool, quiet: bool) -> Result<(), Box<dyn Error>> {
    if !quiet {
        println!("-> Adding Sphere ID '{}' to local cache index...", id);
        println!("   Source file: {}", sphere_file_path_arg.display());
        println!("   Copy to cache option: {}", copy_to_cache);
    }

    let (cache_dir, index_path) = get_cache_paths()?;
    let mut index = load_cache_index(&index_path)?;

    if id.trim().is_empty() {
        return Err("Sphere ID cannot be empty.".into());
    }

    if index.contains_key(id) {
        return Err(format!("Sphere ID '{}' already exists in the cache index. Use 'sphere cache remove {}' first or choose a different ID.", id, id).into());
    }

    if !sphere_file_path_arg.exists() {
        return Err(format!("Source file '{}' does not exist.", sphere_file_path_arg.display()).into());
    }
    if !sphere_file_path_arg.is_file() {
        return Err(format!("Source path '{}' is not a file.", sphere_file_path_arg.display()).into());
    }

    let sphere_filename_in_index: String;

    if copy_to_cache {
        // Derive a safe filename for the cache from the ID
        let mut cached_file_name = id.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "_");
        if !cached_file_name.ends_with(".sphere") {
            cached_file_name.push_str(".sphere");
        }
        // Ensure it's not empty after sanitizing if ID was all special chars
        if cached_file_name == ".sphere" || cached_file_name.is_empty() {
             cached_file_name = format!("sphere_{}.sphere", id.chars().filter(|c| c.is_alphanumeric()).collect::<String>());
             if cached_file_name == "sphere_.sphere" { // if ID had no alphanumerics
                return Err("Cannot derive a valid cache filename from the provided ID. Please use an ID with alphanumeric characters.".into());
             }
        }


        let target_cache_path = cache_dir.join(&cached_file_name);

        if target_cache_path.exists() {
            return Err(format!(
                "A file named '{}' (derived from ID '{}') already exists in the cache directory '{}'. \
                Please choose a different ID or clean up the cache: 'sphere cache remove {}' then try again, or ensure the target file is removed manually.",
                cached_file_name, id, cache_dir.display(), id
            ).into());
        }

        fs::copy(sphere_file_path_arg, &target_cache_path)
            .map_err(|e| format!("Failed to copy '{}' to '{}': {}", sphere_file_path_arg.display(), target_cache_path.display(), e))?;
        
        sphere_filename_in_index = cached_file_name; // Store relative filename for copied files
        if !quiet {
            println!("   Successfully copied '{}' to '{}'", sphere_file_path_arg.display(), target_cache_path.display());
        }
    } else {
        let absolute_sphere_file_path = fs::canonicalize(sphere_file_path_arg)
            .map_err(|e| format!("Failed to get absolute path for '{}': {}", sphere_file_path_arg.display(), e))?;
        sphere_filename_in_index = absolute_sphere_file_path.to_string_lossy().into_owned();
        if !quiet {
            println!("   Will reference original file at '{}'", sphere_filename_in_index);
        }
    }

    index.insert(id.to_string(), sphere_filename_in_index.clone());
    save_cache_index(&index_path, &index)?;

    if !quiet {
        println!("   Successfully added Sphere ID '{}' pointing to '{}' in the index.", id, sphere_filename_in_index);
    }
    Ok(())
}

fn handle_cache_remove(id: &str, quiet: bool) -> Result<(), Box<dyn Error>> {
    if !quiet {
        println!("-> Removing Sphere ID '{}' from local cache index...", id);
    }

    let (_cache_dir, index_path) = get_cache_paths()?;
    let mut index = load_cache_index(&index_path)?;

    // Validation: Check if ID exists
    if !index.contains_key(id) {
        return Err(format!("Sphere ID '{}' not found in the cache index. Nothing to remove.", id).into());
    }

    // Remove the entry
    let removed_file_path = index.remove(id); // .remove() returns the value if the key existed

    save_cache_index(&index_path, &index)?;

    if !quiet {
        println!("   Successfully removed Sphere ID '{}' from the index.", id);
        if let Some(path) = removed_file_path {
            // Check if the path was likely a cached file (relative) vs an absolute path
            if Path::new(&path).is_relative() && !path.starts_with('/') && !path.starts_with("~") {
                 println!("   Note: The associated file '{}' in the cache directory was NOT deleted.", path);
                 println!("   If it was copied to cache, you may want to remove it manually from: {}/{}", _cache_dir.display(), path);
            } else {
                 println!("   Note: The index entry pointed to an external file at '{}'. This file was NOT deleted.", path);
            }
        }
    }
    Ok(())
}


// --- Main Application Logic for 'sphere run' ---
fn run_sphere(file_path: &Path, quiet: bool) -> Result<(), Box<dyn Error>> {
    // ... (run_sphere function remains largely the same as the previous full version) ...
    // ... (ensure all its println! calls are wrapped in `if !quiet`) ...
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
            let dep_path = if Path::new(dep_filename).is_absolute() {
                PathBuf::from(dep_filename) // It's already an absolute path
            } else {
                cache_dir.join(dep_filename) // It's a relative filename in the cache_dir
            };
            
            if !quiet {
                println!("   - Loading dependency '{}' (Sphere ID: '{}') from '{}'", alias, sphere_id, dep_path.display());
            }

            let dep_content = fs::read_to_string(&dep_path)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Box::new(std::io::Error::new(std::io::ErrorKind::Other, format!(
                            "Dependency file '{}' (for Sphere ID '{}', aliased as '{}') not found at expected path '{}'. Please ensure it is installed correctly or the path in index.json is correct.",
                            dep_filename, sphere_id, alias, dep_path.display()
                        ))) as Box<dyn Error>
                    } else {
                        Box::new(e) as Box<dyn Error>
                    }
                })?;
            let dep_process: SphereProcess = toml::from_str(&dep_content)
                .map_err(|e| format!("Failed to parse TOML for dependency '{}' (file: {}): {}", sphere_id, dep_filename, e))?;

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
    
    if !quiet { 
        println!("--- Command STDOUT ---");
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        println!("{}", stdout);
    } else if !quiet { 
        println!("(empty)");
    }
    if !quiet {
        println!("----------------------");
    }


    if !output.stderr.is_empty() {
        if !quiet {
            println!("\n--- Command STDERR ---");
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        println!("{}", stderr); 
        if !quiet {
            println!("----------------------");
        }
    }
    Ok(())
}

// --- Main function: Parses CLI args and dispatches to handlers ---
fn main() {
    let cli = Cli::parse();

    let result = match &cli.command { 
        Commands::Run { file_path } => {
            run_sphere(file_path, cli.quiet)
        }
        Commands::Cache { action } => match action { 
            CacheAction::List => {
                handle_cache_list(cli.quiet)
            }
            CacheAction::Add { id, sphere_file_path, copy_to_cache } => { // Renamed here to match struct
                handle_cache_add(id, sphere_file_path, *copy_to_cache, cli.quiet)
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
        } else if let Commands::Cache { action: CacheAction::Add { ref sphere_file_path, .. } } = cli.command {
            // For cache add, the relevant path might be the one being added
             file_path_for_error = Some(sphere_file_path.display().to_string());
        }


        if let Some(toml_error) = e.downcast_ref::<toml::de::Error>() {
            let path_str = file_path_for_error.as_deref().unwrap_or("the specified .sphere file");
            if toml_error.message().contains("missing field `entrypoint`") {
                error_message = format!("The file '{}' is missing the required 'entrypoint' field.", path_str);
                specific_error_handled = true;
            } else { // Generic TOML error
                 error_message = format!("Failed to parse TOML from '{}'. Reason: {}", path_str, toml_error);
                 specific_error_handled = true;
            }
        }
        
        if !specific_error_handled {
            // Avoid prepending "Application error:" if it's already one of our custom, detailed messages
            if !e.to_string().starts_with("Dependency") && 
               !e.to_string().starts_with("Failed to read sphere file") && 
               !e.to_string().starts_with("Failed to parse TOML from") &&
               !e.to_string().starts_with("Sphere ID") &&
               !e.to_string().starts_with("Source file") &&
               !e.to_string().starts_with("A file named") &&
               !e.to_string().starts_with("Failed to copy") &&
               !e.to_string().starts_with("Failed to get absolute path") &&
               !e.to_string().starts_with("Failed to save cache index") &&
               !e.to_string().starts_with("Failed to parse cache index") &&
               !e.to_string().starts_with("Could not determine home directory") &&
               !e.to_string().starts_with("Cannot derive a valid cache filename") {
                 error_message = format!("Application error: {}", e);
            }
        }
        
        eprintln!("\nError: {}", error_message.trim());
        std::process::exit(1);
    }
}
