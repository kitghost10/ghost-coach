use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let frontend_root = manifest_dir.join("..");
    let dist_index = frontend_root.join("dist/index.html");

    println!("cargo:rerun-if-changed=../index.html");
    println!("cargo:rerun-if-changed=../overlay.html");
    println!("cargo:rerun-if-changed=../src");
    println!("cargo:rerun-if-changed=../package.json");
    println!("cargo:rerun-if-changed=../vite.config.ts");

    if !dist_index.exists() {
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

        // Try to find npm in common locations if not in PATH
        let npm_paths = [
            npm.to_string(),
            "/usr/bin/npm".to_string(),
            "/usr/local/bin/npm".to_string(),
            format!("{}/.npm-global/bin/npm", std::env::var("HOME").unwrap_or_default()),
        ];

        let mut last_error = None;
        let mut success = false;

        for npm_path in &npm_paths {
            match Command::new(npm_path)
                .args(["run", "build"])
                .current_dir(&frontend_root)
                .status() {
                Ok(status) if status.success() => {
                    success = true;
                    break;
                }
                Ok(status) => {
                    last_error = Some(format!("npm build failed with exit code: {:?}", status.code()));
                    break;
                }
                Err(e) => {
                    last_error = Some(format!("Failed to run npm at {}: {}", npm_path, e));
                }
            }
        }

        if !success {
            if let Some(err) = last_error {
                panic!("Failed to build frontend: {}", err);
            } else {
                panic!("Failed to build frontend: npm not found");
            }
        }
    }

    tauri_build::build()
}
