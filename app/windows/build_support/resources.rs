//! Resources shared by the Windows GUI and command-line executables.
pub fn embed(binary: &str) {
    const ICON: &str = "../../assets/windows/icons/app.ico";
    const MANIFEST: &str = "../../assets/windows/manifests/app.manifest";
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed={MANIFEST}");
    let package = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let helper = package
        .parent()
        .unwrap()
        .join("windows/build_support/resources.rs");
    println!("cargo:rerun-if-changed={}", helper.display());

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = root.join(MANIFEST);
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rc = out.join("mail-icon.rc");
    let res = out.join("mail-icon.res");
    std::fs::write(
        &rc,
        format!(
            "1 ICON \"{}\"\n",
            root.join(ICON).to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let sdk = std::path::PathBuf::from(
        std::env::var_os("ProgramFiles(x86)").expect("Windows SDK requires ProgramFiles(x86)"),
    )
    .join("Windows Kits/10/bin");
    let mut compilers: Vec<_> = std::fs::read_dir(sdk)
        .expect("Windows SDK is required")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("x64/rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    compilers.sort();
    let compiler = compilers
        .last()
        .expect("Windows SDK resource compiler rc.exe is required");
    let result = std::process::Command::new(compiler)
        .arg("/nologo")
        .arg(format!("/fo{}", res.display()))
        .arg(rc)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Icon resource compilation failed: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("cargo:rustc-link-arg-bin={binary}={}", res.display());
    println!("cargo:rustc-link-arg-bin={binary}=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin={binary}=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
