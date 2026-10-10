use mail_windows::startup::{command, native, pe::Pe};
use std::path::{Path, PathBuf};

pub fn main() -> i32 {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let first = args.first().and_then(|s| s.to_str()).unwrap_or("");
    if first.starts_with("--mail-")
        || matches!(
            first,
            "--version" | "--check" | "--runtime-version" | "--runtime-check"
        )
    {
        return match crate::execute(&args) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
    }
    let runtime = native::runtime_dir();
    match first {
        "--help" | "-h" => {
            println!(
                "WinMail command-line application\n\
                winmail-rust [original WinMail arguments]\n\
                winmail-rust --diagnose [legacy DLL directory]  (static inspection only)\n\
                winmail-rust --probe [legacy DLL directory]     (loads DLL, calls no mail entry)\n\
                winmail-rust --plan [original arguments]     (no execution)\n\
                Default application: WinMail in Rust (no legacy DLL needed).\n\
                --version / --check / --mail-list / --mail-import PATH / --mail-show ID\n\
                --mail-export ID PATH / --mail-receive / --mail-draft JSON / --mail-send JSON\n\
                WINMAIL_DATA_DIR selects the data store.\n\
                WINMAIL_RUNTIME_DIR explicitly enables an external/legacy DLL.\n\
                WinMail.exe provides the Windows graphical application.\n\
                OCInstall* is available only with an explicitly selected legacy DLL."
            );
            0
        }
        "--diagnose" | "--probe" => {
            if args.len() > 2 {
                eprintln!("Expected one optional legacy DLL directory.");
                return 2;
            }
            let directory = args.get(1).map(PathBuf::from).unwrap_or(runtime);
            if first == "--diagnose" {
                diagnose(&directory)
            } else {
                match native::probe(&directory) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("{} (exit {})", e.detail, e.exit_code);
                        e.exit_code
                    }
                }
            }
        }
        "--plan" => {
            let raw = native::raw_tail();
            let tail = command::command_tail(&raw);
            let route = command::route(tail);
            println!(
                "Legacy DLL directory: {}\nRoute: {route:?}\nRaw UTF-16 tail: {}\nManual launch allowed: {}\nImplementation: {}",
                runtime.display(),
                String::from_utf16_lossy(tail),
                native::manual_launch_allowed(),
                if route == command::Route::Install {
                    if native::external_runtime() {
                        "original WinMail.exe compatibility adapter"
                    } else {
                        "unsupported legacy installer command"
                    }
                } else {
                    if native::external_runtime() {
                        "External DLL"
                    } else {
                        "WinMail Rust application"
                    }
                }
            );
            0
        }
        _ => match native::run(&native::raw_tail(), &runtime, native::show_command()) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("{} (exit {})", e.detail, e.exit_code);
                e.exit_code
            }
        },
    }
}

fn diagnose(runtime: &Path) -> i32 {
    println!(
        "Legacy DLL directory: {}\nStatic inspection; no DLL loaded, no mail data accessed.",
        runtime.display()
    );
    let mut failed = false;
    for file in ["WinMail.exe", "msoe.dll"] {
        let path = runtime.join(file);
        match Pe::read(&path) {
            Err(e) => {
                println!("{file}: {e}");
                if file == "msoe.dll" {
                    failed = true;
                }
            }
            Ok(pe) => {
                println!(
                    "{file}: machine=0x{:04x}, image_base=0x{:x}, entry_rva=0x{:x}",
                    pe.machine, pe.image_base, pe.entry_rva
                );
                if pe.machine != 0x8664 {
                    println!("  x64 required");
                    failed = true;
                }
                if file == "msoe.dll" {
                    let export = pe.exports.iter().any(|s| s == "CoStartOutlookExpressW");
                    println!(
                        "  CoStartOutlookExpressW: {}",
                        if export { "present" } else { "missing" }
                    );
                    failed |= !export;
                    let system32 = std::env::var_os("SystemRoot")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
                        .join("System32");
                    for dependency in pe.imports {
                        let local = runtime.join(&dependency);
                        let system = system32.join(&dependency);
                        if local.is_file() || system.is_file() {
                            continue;
                        }
                        if dependency.to_ascii_lowercase().starts_with("api-ms-")
                            || dependency.to_ascii_lowercase().starts_with("ext-ms-")
                        {
                            continue;
                        }
                        println!("  Unresolved file candidate: {dependency}");
                        failed = true;
                    }
                }
            }
        }
    }
    println!(
        "Static file checks cannot verify imported ordinals, transitive dependencies or DLL initialization. Use --probe for loader verification."
    );
    if failed { 1 } else { 0 }
}
