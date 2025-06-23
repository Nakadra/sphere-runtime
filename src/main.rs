// --- Imports ---
use clap::{Parser, Subcommand};
use serde::Deserialize;
// serde_json is used via its full path like serde_json::from_str, so top-level import removed by clippy
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write}; 
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;
use sha2::{Digest, Sha256};
use reqwest::blocking::Client;

// --- Constants ---
const SPHEREHUB_REGISTRY_URL: &str = "https://raw.githubusercontent.com/Nakadra/sphere-hub-registry/main/registry/";


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
    /// Publish a .sphere file to the SphereHub (generates PR instructions)
    Publish {
        /// The .sphere file to prepare for publishing
        #[arg(required = true)]
        file_path: PathBuf,
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
        sphere_file_path: PathBuf,
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
    id: Option<String>,
    entrypoint: String,
    dependencies: Option<HashMap<String, String>>,
}

struct Dependency {
    alias: String,
    process: SphereProcess,
}

#[derive(Deserialize, Debug, Clone)] 
struct HubSphereInfo {
    filename: String,
    description: String,
    author: String,
    hash_sha256: String,
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
        let mut cached_file_name = id.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "_");
        if !cached_file_name.ends_with(".sphere") {
            cached_file_name.push_str(".sphere");
        }
        if cached_file_name == ".sphere" || cached_file_name.is_empty() {
             cached_file_name = format!("sphere_{}.sphere", id.chars().filter(|c| c.is_alphanumeric()).collect::<String>());
             if cached_file_name == "sphere_.sphere" {
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
        sphere_filename_in_index = cached_file_name;
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

    if !index.contains_key(id) {
        return Err(format!("Sphere ID '{}' not found in the cache index. Nothing to remove.", id).into());
    }
    let removed_file_path = index.remove(id);
    save_cache_index(&index_path, &index)?;

    if !quiet {
        println!("   Successfully removed Sphere ID '{}' from the index.", id);
        if let Some(path_str) = removed_file_path {
            let path_obj = Path::new(&path_str);
            if path_obj.is_relative() && !path_str.starts_with('/') && !path_str.starts_with('~') {
                 println!("   Note: The associated file '{}' in the cache directory was NOT deleted.", path_str);
                 println!("   If it was copied to cache, you may want to remove it manually from: {}/{}", _cache_dir.display(), path_str);
            } else {
                 println!("   Note: The index entry pointed to an external file at '{}'. This file was NOT deleted.", path_str);
            }
        }
    }
    Ok(())
}

// --- Publish Command Handler ---
fn handle_sphere_publish(file_path: &PathBuf, quiet: bool) -> Result<(), Box<dyn Error>> {
    if !quiet {
        println!("-> Preparing to publish Sphere from: {}", file_path.display());
        println!("   (This command will guide you to create a Pull Request to the SphereHub registry)");
        println!("---");
    }

    if !file_path.exists() {
        return Err(format!("Sphere file '{}' not found.", file_path.display()).into());
    }
    if !file_path.is_file() {
        return Err(format!("Path '{}' is not a file.", file_path.display()).into());
    }

    let content_string = fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read sphere file '{}': {}", file_path.display(), e))?;
    let sphere_process: SphereProcess = toml::from_str(&content_string)
        .map_err(|e| format!("Failed to parse TOML from '{}': {}", file_path.display(), e))?;

    let sphere_id = match &sphere_process.id {
        Some(id_val) if !id_val.trim().is_empty() => id_val.trim().to_string(),
        _ => return Err(format!(
            "The .sphere file '{}' must contain a valid, non-empty 'id' field for publishing.",
            file_path.display()
        ).into()),
    };
    
    if !quiet {
        println!("   Successfully parsed Sphere. ID: {}", sphere_id);
    }

    let author = {
        print!("   Enter your GitHub username or author name for this Sphere: ");
        io::stdout().flush()?; 
        let mut buffer = String::new();
        io::stdin().lock().read_line(&mut buffer)?;
        let name = buffer.trim();
        if name.is_empty() { "UnknownAuthor".to_string() } else { name.to_string() }
    };

    let description = {
        print!("   Enter a short, one-line description for this Sphere: ");
        io::stdout().flush()?;
        let mut buffer = String::new();
        io::stdin().lock().read_line(&mut buffer)?;
        let desc = buffer.trim();
        if desc.is_empty() { "No description provided.".to_string() } else { desc.to_string() }
    };
    
    if !quiet {
        println!("---");
    }

    let mut hasher = Sha256::new();
    hasher.update(content_string.as_bytes());
    let hash_bytes = hasher.finalize();
    let hash_hex = format!("{:x}", hash_bytes);

    let mut derived_filename = sphere_id.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "_");
    if !derived_filename.ends_with(".sphere") {
        derived_filename.push_str(".sphere");
    }
    if derived_filename == ".sphere" || derived_filename.is_empty() {
         derived_filename = format!("sphere_{}.sphere", sphere_id.chars().filter(|c| c.is_alphanumeric()).collect::<String>());
         if derived_filename == "sphere_.sphere" {
             return Err("Cannot derive a valid filename for SphereHub from the provided ID. Please use an ID with alphanumeric characters.".into());
         }
    }

    println!("\n--- How to Publish '{}' to SphereHub ---", sphere_id);
    println!("SphereHub Registry: https://github.com/Nakadra/sphere-hub-registry\n");
    println!("1. Fork the SphereHub Registry repository to your GitHub account.");
    println!("2. Clone your fork locally: `git clone https://github.com/YOUR_USERNAME/sphere-hub-registry.git`");
    println!("3. Create a new branch: `git checkout -b add-sphere-{}`", sphere_id.chars().take(15).filter(|c| c.is_alphanumeric()).collect::<String>());
    println!("\n4. Create/Update the Sphere file in your fork:");
    println!("   - Path: `registry/spheres/{}`", derived_filename);
    println!("   - Content: (Copy the exact content of your local '{}' file into this new file)\n", file_path.display());
    println!("5. Add/Update the entry in `registry/index.json` in your fork:");
    println!("   Ensure the JSON is valid. Add your Sphere entry like this (add a comma if needed):");
    println!("   ```json");
    println!("   \"{}\": {{", sphere_id);
    println!("     \"filename\": \"{}\",", derived_filename);
    println!("     \"description\": \"{}\",", description);
    println!("     \"author\": \"{}\",", author);
    println!("     \"hash_sha256\": \"{}\"", hash_hex);
    println!("   }}");
    println!("   ```\n");
    println!("6. Commit your changes: `git add . && git commit -m \"feat: Add Sphere {} \"`", sphere_id);
    println!("7. Push to your fork: `git push origin add-sphere-{}`", sphere_id.chars().take(15).filter(|c| c.is_alphanumeric()).collect::<String>());
    println!("8. Go to `https://github.com/Nakadra/sphere-hub-registry` and click 'New Pull Request'. Choose your fork and branch.\n");
    println!("The Sphere maintainers (Clein, Kelly, Ronald) will review your PR. Thank you for contributing!");
    println!("---");

    if !quiet {
        println!("-> Publish preparation complete. Please follow the instructions above.");
    }
    Ok(())
}

// --- SphereHub Fetching Logic ---
fn fetch_sphere_from_hub(
    sphere_id: &str,
    local_cache_dir: &Path,
    local_index_path: &Path,
    local_index: &mut HashMap<String, String>, 
    http_client: &Client,
    quiet: bool,
) -> Result<PathBuf, Box<dyn Error>> {
    if !quiet {
        println!("   -> Dependency '{}' not in local cache. Attempting to fetch from SphereHub...", sphere_id);
    }

    let master_index_url = format!("{}index.json", SPHEREHUB_REGISTRY_URL);
    let response = http_client.get(&master_index_url).send()?;
    if !response.status().is_success() {
        return Err(format!("Failed to fetch SphereHub master index from '{}': HTTP {}", master_index_url, response.status()).into());
    }
    let response_text = response.text()?;
    
    let master_index: HashMap<String, HubSphereInfo> = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse SphereHub master index: {}. Content: '{}'", e, response_text))?;

    let hub_info = master_index.get(sphere_id).ok_or_else(|| {
        format!("Sphere ID '{}' not found in the public SphereHub registry at {}.", sphere_id, master_index_url)
    })?;

    if !quiet {
        println!("   -> Found '{}' in SphereHub. Filename: {}, Author: {}, Desc: {}, Hash: {}...", 
                 sphere_id, hub_info.filename, hub_info.author, hub_info.description, &hub_info.hash_sha256[..8]);
    }

    let sphere_file_url = format!("{}spheres/{}", SPHEREHUB_REGISTRY_URL, hub_info.filename);
    let sphere_file_response = http_client.get(&sphere_file_url).send()?;
     if !sphere_file_response.status().is_success() {
        return Err(format!("Failed to fetch Sphere file '{}' from '{}': HTTP {}", hub_info.filename, sphere_file_url, sphere_file_response.status()).into());
    }
    let sphere_file_content_bytes = sphere_file_response.bytes()?;


    let mut hasher = Sha256::new();
    hasher.update(&sphere_file_content_bytes);
    let calculated_hash_bytes = hasher.finalize();
    let calculated_hash_hex = format!("{:x}", calculated_hash_bytes);

    if calculated_hash_hex != hub_info.hash_sha256 {
        return Err(format!(
            "Hash mismatch for Sphere '{}' (file '{}')! Expected: {}, Got: {}. File may be corrupted or tampered.",
            sphere_id, hub_info.filename, hub_info.hash_sha256, calculated_hash_hex
        ).into());
    }
    if !quiet {
        println!("   -> Hash verification successful for '{}'.", sphere_id);
    }

    let local_sphere_file_path = local_cache_dir.join(&hub_info.filename);
    fs::write(&local_sphere_file_path, &sphere_file_content_bytes)
        .map_err(|e| format!("Failed to save downloaded Sphere '{}' to local cache ('{}'): {}", sphere_id, local_sphere_file_path.display(), e))?;
    
    local_index.insert(sphere_id.to_string(), hub_info.filename.clone()); 
    save_cache_index(local_index_path, local_index)?; 

    if !quiet {
        println!("   -> Successfully downloaded, verified, and cached '{}' to '{}'.", sphere_id, local_sphere_file_path.display());
    }
    
    Ok(local_sphere_file_path)
}


// --- Main Application Logic for 'sphere run' ---
fn run_sphere(file_path: &Path, quiet: bool) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read sphere file '{}': {}", file_path.display(), e))?;
    let sphere_process: SphereProcess = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse TOML from '{}': {}", file_path.display(), e))?;
    
    if !quiet {
        println!("-> Parsed entrypoint: '{}' from '{}'", &sphere_process.entrypoint, file_path.display());
        if let Some(id) = &sphere_process.id {
            println!("   Sphere ID: {}", id);
        }
    }

    let mut resolved_deps: Vec<Dependency> = Vec::new();
    if let Some(deps) = &sphere_process.dependencies {
        if !quiet {
            println!("-> Resolving dependencies...");
        }
        let (cache_dir, local_index_path) = get_cache_paths()?;
        let mut local_index = load_cache_index(&local_index_path)?;
        
        if !quiet && !local_index.is_empty() {
             println!("   - Loaded local cache index from '{}'.", local_index_path.display());
        } else if !quiet && local_index.is_empty() {
             println!("   - Local cache index at '{}' is empty or not found.", local_index_path.display());
        }
        
        let http_client = Client::builder()
            .user_agent(format!("sphere-cli/{}", env!("CARGO_PKG_VERSION")))
            .build()?;

        for (alias, sphere_id) in deps {
            let dep_path: PathBuf; // Removed 'mut' as per clippy

            if let Some(dep_filename_in_local_cache) = local_index.get(sphere_id) {
                let current_dep_path = if Path::new(dep_filename_in_local_cache).is_absolute() {
                    PathBuf::from(dep_filename_in_local_cache)
                } else {
                    cache_dir.join(dep_filename_in_local_cache)
                };

                if !current_dep_path.exists() {
                    if !quiet {
                        println!("   - Dependency '{}' (Sphere ID: '{}') found in local index but file missing at '{}'. Attempting Hub fetch.", alias, sphere_id, current_dep_path.display());
                    }
                    dep_path = fetch_sphere_from_hub(sphere_id, &cache_dir, &local_index_path, &mut local_index, &http_client, quiet)?;
                } else {
                    if !quiet {
                        println!("   - Using locally cached dependency '{}' (Sphere ID: '{}') from '{}'", alias, sphere_id, current_dep_path.display());
                    }
                    dep_path = current_dep_path;
                }
            } else {
                dep_path = fetch_sphere_from_hub(sphere_id, &cache_dir, &local_index_path, &mut local_index, &http_client, quiet)?;
            }
            
            if !quiet && dep_path.exists() {
                println!("   - Loading dependency definition for '{}' from '{}'", alias, dep_path.display());
            } else if !dep_path.exists() {
                 return Err(format!("Failed to obtain dependency '{}' (Sphere ID: '{}'). Expected at '{}' after attempting local cache and Hub fetch.", alias, sphere_id, dep_path.display()).into());
            }

            let dep_content = fs::read_to_string(&dep_path)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Box::new(std::io::Error::other(format!( // Used Error::other
                            "Dependency file for '{}' (Sphere ID: '{}', alias: '{}') not found at expected path '{}' even after cache/Hub check. This indicates an inconsistency.",
                            dep_path.display(), sphere_id, alias, dep_path.display()
                        ))) as Box<dyn Error>
                    } else {
                        Box::new(std::io::Error::other(format!( // Used Error::other
                            "Failed to read dependency file '{}' (Sphere ID: '{}', alias: '{}'): {}", 
                            dep_path.display(), sphere_id, alias, e
                        ))) as Box<dyn Error>
                    }
                })?;
            let dep_process: SphereProcess = toml::from_str(&dep_content)
                .map_err(|e| format!("Failed to parse TOML for dependency '{}' (file: {}): {}", sphere_id, dep_path.display(), e))?;

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
            CacheAction::Add { id, sphere_file_path, copy_to_cache } => {
                handle_cache_add(id, sphere_file_path, *copy_to_cache, cli.quiet)
            }
            CacheAction::Remove { id } => {
                handle_cache_remove(id, cli.quiet)
            }
        },
        Commands::Publish { file_path } => { 
            handle_sphere_publish(file_path, cli.quiet)
        }
    };

    if let Err(e) = result {
        let mut error_message = format!("{}", e);
        let mut specific_error_handled = false;
        let mut file_path_for_error: Option<String> = None;

        match &cli.command {
            Commands::Run { file_path } => {
                file_path_for_error = Some(file_path.display().to_string());
            }
            Commands::Publish { file_path } => {
                 file_path_for_error = Some(file_path.display().to_string());
            }
            Commands::Cache { action } => {
                if let CacheAction::Add { sphere_file_path, .. } = action {
                    file_path_for_error = Some(sphere_file_path.display().to_string());
                }
            }
        }

        if let Some(toml_error) = e.downcast_ref::<toml::de::Error>() {
            let path_str = file_path_for_error.as_deref().unwrap_or("the specified .sphere file");
            if toml_error.message().contains("missing field `entrypoint`") {
                error_message = format!("The file '{}' is missing the required 'entrypoint' field.", path_str);
                specific_error_handled = true;
            } else { 
                 error_message = format!("Failed to parse TOML from '{}'. Reason: {}", path_str, toml_error);
                 specific_error_handled = true;
            }
        }
        
        if !specific_error_handled {
            let custom_prefixes = [
                "Dependency", "Failed to read sphere file", "Failed to parse TOML from",
                "Sphere ID", "Source file", "A file named", "Failed to copy",
                "Failed to get absolute path", "Failed to save cache index",
                "Failed to parse cache index", "Could not determine home directory",
                "Cannot derive a valid cache filename", "Failed to fetch SphereHub master index",
                /* "Sphere ID" is too generic, use more specific part of the error message */
                "not found in the public SphereHub registry", "Failed to fetch Sphere file",
                "Hash mismatch for Sphere", "Failed to save downloaded Sphere"
            ];
            if !custom_prefixes.iter().any(|p| e.to_string().contains(p)) { // Changed to .contains() for broader matching
                error_message = format!("Application error: {}", e);
            }
        }
        
        eprintln!("\nError: {}", error_message.trim());
        std::process::exit(1);
    }
}
