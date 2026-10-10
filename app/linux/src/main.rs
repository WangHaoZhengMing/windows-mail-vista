fn main() {
    #[cfg(target_os = "linux")]
    {
        if let Err(error) = mail_linux::ui::run("", 1) {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("This entry point requires Linux.");
        std::process::exit(2);
    }
}
