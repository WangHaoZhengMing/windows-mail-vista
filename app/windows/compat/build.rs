fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let def =
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("msoe.def");
        println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
    }
    println!("cargo:rerun-if-changed=msoe.def");
}
