// --- Imports ---
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::error::Error;
use std::path::PathBuf;
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
    // id: String, // Commented out to avoid dead_code warning for now
    entrypoint: String,
    dependencies: Option<HashMap<String, String>>,
}

struct Dependency {
    alias: String,
    process: SphereProcess,
}

// --- Main Application Logic (in its own function) ---
fn run_sphere(cli_args: &Cli) -> Result<(), Box<dyn Error>> { // Pass the whole Cli struct for the quiet flag
    // --- PARSING LOGIC ---
    let content = fs::read_to_string(&cli_args.file_path)?;
    let sphere_process: SphereProcess = toml::from_str(&content)?;
    
    if !cli_args.quiet {
        println!("-> Parsed entrypoint: '{}'", &sphere_process.entrypoint);
    }

    // --- DEPENDENCY RESOLUTION ---
    let mut resolved_deps: Vec<Dependency> = Vec::new();
    if let Some(deps) = &sphere_process.dependencies {
        if !cli_args.quiet {
            println!("-> Resolving dependencies...");
        }
        
        let home = std::env::var("HOME")?;
        let cache_dir = format!("{}/.sphere/cache", home);
        let index_path = format!("{}/index.json", cache_dir);
        let index_content = fs::read_to_string(&index_path)
            .map_err(|e| format!("Failed to read cache index at '{}': {}", index_path, e))?; // Improved error for index
        
        let index: HashMap<String, String> = serde_json::from_str(&index_content)
            .map_err(|e| format!("Failed to parse cache index at '{}': {}", index_path, e))?; // Improved error for index parsing
        
        if !cli_args.quiet {
            println!("   - Loaded cache index.");
        }

        for (alias, sphere_id) in deps {
            let dep_filename = index.get(sphere_id).ok_or_else(|| {
                format!("Dependency ID '{}' (aliased as '{}') not found in cache index ('{}').", sphere_id, alias, index_path)
            })?;
            
            let dep_path_str = format!("{}/{}", cache_dir, dep_filename); // Keep as String for format!
            let dep_path = PathBuf::from(&dep_path_str); // Convert to PathBuf for fs operations
            
            if !cli_args.quiet {
                println!("   - Loading '{}' as '{}' from {}", sphere_id, alias, dep_path.display());
            }

            let dep_content = fs::read_to_string(&dep_path)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        let err_msg = format!(
                            "Dependency file '{}' (for Sphere ID '{}', aliased as '{}') not found in cache at path '{}'. Please ensure it is installed correctly.",
                            dep_filename,
                            sphere_id,
                            alias, // Added alias for better context
                            dep_path.display()
                        );
                        Box::new(std::io::Error::new(std::io::ErrorKind::Other, err_msg)) as Box<dyn Error>
                    } else {
                        let err_msg = format!(
                            "Failed to read dependency file '{}' (for Sphere ID '{}', aliased as '{}') from path '{}': {}",
                            dep_filename,
                            sphere_id,
                            alias,
                            dep_path.display(),
                            e
                        );
                        Box::new(std::io::Error::new(e.kind(), err_msg)) as Box<dyn Error> // Preserve original error kind if possible
                    }
                })?;
            
            let dep_process: SphereProcess = toml::from_str(&dep_content)
                .map_err(|e| format!("Failed to parse dependency Sphere file '{}' (for Sphere ID '{}', aliased as '{}'): {}", dep_filename, sphere_id, alias, e))?;


            resolved_deps.push(Dependency {
                alias: alias.clone(),
                process: dep_process,
            });
        }
    }


    // --- SANDBOX & EXECUTION ---
    let temp_dir = tempdir()?;
    if !cli_args.quiet {
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
        // NEW: Ensure the dependency also respects quiet mode if we decide to pass it down
        // For now, dependency scripts run as defined in their entrypoint
        writeln!(script_file, "{}", dep.process.entrypoint)?;
    }
    
    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_path.to_str().unwrap(), original_path);
    
    if !cli_args.quiet {
        println!("-> Executing entrypoint inside sandbox...");
    }
    let output = Command::new("sh")
        .arg("-c")
        .arg(&sphere_process.entrypoint)
        .current_dir(temp_dir.path())
        .env("PATH", new_path)
        .output()?;
    
    if !cli_args.quiet {
        println!("-> Execution finished.\n");
    }
    
    // Always print STDOUT and STDERR from the executed command, regardless of quiet mode
    if !output.stdout.is_empty() {
        // No "--- Command STDOUT ---" header in quiet mode for cleaner scripting output
        if !cli_args.quiet { println!("--- Command STDOUT ---"); }
        // Use print! and manually add newline for precise control, or handle if stdout already has one
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        print!("{}", stdout_str); // Use print! not println! if output has its own newlines
        if !cli_args.quiet && !stdout_str.ends_with('\n') { println!(); } // Add newline if not present in quiet mode off
        if !cli_args.quiet { println!("----------------------"); }
    }


    if !output.stderr.is_empty() {
        // No "--- Command STDERR ---" header in quiet mode
        if !cli_args.quiet { eprintln!("\n--- Command STDERR ---"); } // eprintln for stderr
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        eprint!("{}", stderr_str); // Use eprint!
        if !cli_args.quiet && !stderr_str.ends_with('\n') { eprintln!(); }
        if !cli_args.quiet { eprintln!("----------------------"); }
    }

    Ok(())
}


// --- Main function: Now just a simple parser and error handler ---
fn main() {
    let cli = Cli::parse();
    
    if let Err(e) = run_sphere(&cli) { // Pass the whole cli struct
        // Try to downcast the error to a TOML deserialization error.
        if let Some(toml_error) = e.downcast_ref::<toml::de::Error>() {
            if toml_error.message().contains("missing field `entrypoint`") {
                eprintln!("\nError: The file '{}' is missing the required 'entrypoint' field.", cli.file_path.display());
            } else {
                eprintln!("\nError: Failed to parse Sphere file '{}'.", cli.file_path.display());
                eprintln!("Reason: {}", toml_error);
            }
        } else {
            // For other errors, just print their message directly.
            // Our custom errors created with Box::new(std::io::Error::new(...)) will print cleanly.
            eprintln!("\nApplication error: {}", e);
        }
        
        std::process::exit(1);
    }
}
