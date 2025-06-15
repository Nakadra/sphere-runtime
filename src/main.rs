// --- Imports ---
use clap::Parser; // The main clap import
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Write; // Needed for creating files
use std::error::Error;
use std::path::PathBuf; // For handling file paths
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
}

// --- Data Structures ---
#[derive(Deserialize, Debug)]
struct SphereProcess {
    // We are not using 'id' yet, so it is commented out to prevent warnings.
    // id: String, 
    entrypoint: String,
    dependencies: Option<HashMap<String, String>>,
}

struct Dependency {
    alias: String,
    process: SphereProcess,
}


// --- Main Application Logic ---
fn main() -> Result<(), Box<dyn Error>> {
    // 1. Parse Command-Line Arguments
    let cli = Cli::parse();

    // --- PARSING LOGIC (Uses the CLI argument) ---
    let content = fs::read_to_string(&cli.file_path)?;
    let sphere_process: SphereProcess = toml::from_str(&content)?;
    println!("-> Parsed entrypoint: '{}'", &sphere_process.entrypoint);

    // --- DEPENDENCY RESOLUTION ---
    let mut resolved_deps: Vec<Dependency> = Vec::new();
    if let Some(deps) = &sphere_process.dependencies {
        println!("-> Resolving dependencies...");
        
        let home = std::env::var("HOME")?;
        let cache_dir = format!("{}/.sphere/cache", home);
        let index_path = format!("{}/index.json", cache_dir);
        let index_content = fs::read_to_string(index_path)?;
        let index: HashMap<String, String> = serde_json::from_str(&index_content)?;
        println!("   - Loaded cache index.");

        for (alias, sphere_id) in deps {
            let dep_filename = index.get(sphere_id).ok_or_else(|| {
                format!("Dependency '{}' not found in cache index!", sphere_id)
            })?;
            
            let dep_path = format!("{}/{}", cache_dir, dep_filename);
            
            println!("   - Loading '{}' as '{}' from {}", sphere_id, alias, dep_path);

            let dep_content = fs::read_to_string(&dep_path)?;
            let dep_process: SphereProcess = toml::from_str(&dep_content)?;

            resolved_deps.push(Dependency {
                alias: alias.clone(),
                process: dep_process,
            });
        }
    }


    // --- SANDBOX SETUP & EXECUTION ---
    let temp_dir = tempdir()?;
    println!("-> Created secure sandbox at: {:?}", temp_dir.path());
    let bin_path = temp_dir.path().join("bin");
    fs::create_dir(&bin_path)?;

    for dep in &resolved_deps {
        let script_path = bin_path.join(&dep.alias);
        let mut script_file = fs::File::create(&script_path)?;
        
        // This is the corrected block for setting file permissions.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Step 1: Get the permissions object into a mutable variable.
            let mut perms = script_file.metadata()?.permissions();
            // Step 2: Modify the object in-place.
            perms.set_mode(0o755); // rwxr-xr-x
            // Step 3: Apply the modified permissions object back to the file.
            fs::set_permissions(&script_path, perms)?;
        }

        writeln!(script_file, "#!/bin/sh")?;
        writeln!(script_file, "{}", dep.process.entrypoint)?;
    }
    
    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_path.to_str().unwrap(), original_path);
    
    println!("-> Executing entrypoint inside sandbox...");
    let output = Command::new("sh")
        .arg("-c")
        .arg(&sphere_process.entrypoint)
        .current_dir(temp_dir.path())
        .env("PATH", new_path)
        .output()?;
    println!("-> Execution finished.\n");
    
    // --- OUTPUT HANDLING ---
    println!("--- Command STDOUT ---");
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    println!("{}", stdout);
    println!("----------------------");

    if !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        println!("\n--- Command STDERR ---");
        println!("{}", stderr);
        println!("----------------------");
    }

    Ok(())
}
