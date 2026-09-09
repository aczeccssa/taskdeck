use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=BUN");
    println!("cargo:rerun-if-env-changed=TASKDECK_FRONTEND_SKIP_INSTALL");
    for path in [
        "frontend/package.json",
        "frontend/bun.lock",
        "frontend/tsconfig.json",
        "frontend/vite.config.ts",
        "frontend/index.html",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    emit_rerun_for_tree(Path::new("frontend/src"));
    emit_rerun_for_tree(Path::new("frontend/public"));

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("Cargo provides CARGO_MANIFEST_DIR"));
    let frontend_dir = manifest_dir.join("frontend");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo provides OUT_DIR"));
    let dist_dir = out_dir.join("taskdeck-frontend");
    let bun = env::var("BUN").unwrap_or_else(|_| "bun".to_owned());

    require_bun(&bun, &frontend_dir);
    if env::var("TASKDECK_FRONTEND_SKIP_INSTALL").as_deref() != Ok("1") {
        run(&bun, &frontend_dir, ["install", "--frozen-lockfile"]);
    }
    let status = Command::new(&bun)
        .current_dir(&frontend_dir)
        .env("TASKDECK_FRONTEND_OUT_DIR", &dist_dir)
        .args(["run", "build"])
        .status()
        .unwrap_or_else(|error| panic!("Taskdeck frontend build could not start Bun ({error}). Install Bun 1.3.14+ or set BUN=/absolute/path/to/bun."));
    if !status.success() {
        panic!(
            "Taskdeck frontend build failed. Run `cd frontend && bun install --frozen-lockfile && bun run build` to reproduce."
        );
    }
    generate_asset_module(&out_dir, &dist_dir);
}

fn require_bun(bun: &str, frontend_dir: &Path) {
    let status = Command::new(bun).current_dir(frontend_dir).arg("--version").status().unwrap_or_else(|error| {
        panic!("Taskdeck requires Bun 1.3.14+ to build its embedded React frontend ({error}). Install Bun or set BUN=/absolute/path/to/bun.")
    });
    if !status.success() {
        panic!("Taskdeck requires a working Bun 1.3.14+ to build its embedded React frontend.");
    }
}

fn run<const N: usize>(bun: &str, cwd: &Path, args: [&str; N]) {
    let status = Command::new(bun)
        .current_dir(cwd)
        .args(args)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "Taskdeck frontend command `bun {}` could not start ({error}).",
                args.join(" ")
            )
        });
    if !status.success() {
        panic!("Taskdeck frontend command `bun {}` failed.", args.join(" "));
    }
}

fn emit_rerun_for_tree(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            emit_rerun_for_tree(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn generate_asset_module(out_dir: &Path, dist_dir: &Path) {
    let mut files = Vec::new();
    collect_files(dist_dir, dist_dir, &mut files);
    files.sort();
    if !files.iter().any(|path| path == Path::new("index.html")) {
        panic!("Vite frontend build did not produce index.html");
    }
    let mut source = String::from("pub(crate) static EMBEDDED_ASSETS: &[EmbeddedAsset] = &[\n");
    for relative in files {
        let path = relative.to_string_lossy().replace('\\', "/");
        source.push_str(&format!("    EmbeddedAsset {{ path: \"/{path}\", bytes: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/taskdeck-frontend/{path}\")) }},\n"));
    }
    source.push_str("];\n");
    fs::write(out_dir.join("embedded_assets.rs"), source)
        .expect("write generated frontend asset module");
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(current)
        .expect("read frontend build output")
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(
                path.strip_prefix(root)
                    .expect("frontend path under build output")
                    .to_path_buf(),
            );
        }
    }
}
