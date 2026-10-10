fn main() {
    #[cfg(target_os = "macos")]
    {
        if let Err(error) = mail_macos::ui::run("", 1) {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("This entry point requires macOS.");
        std::process::exit(2);
    }
}
