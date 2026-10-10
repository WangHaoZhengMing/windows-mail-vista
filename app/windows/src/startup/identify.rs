//! Typed implementation shared by historical identification commands and ABI adapters.

use mail_core::{message, store};

/// Handle the historical `/ident*` commands without entering the UI or a C ABI.
pub fn run(command: &str) {
    if command.to_lowercase().starts_with("/identcatalog") {
        println!("{}", store::default_root().display());
    } else if let Some((_, path)) = command.split_once(':') {
        if command.to_lowercase().starts_with("/identfile:") {
            match store::read_bounded(std::path::Path::new(path.trim_matches('"')))
                .and_then(|raw| message::parse(&raw))
            {
                Ok(mail) => println!(
                    "From: {}\nSubject: {}\n{}",
                    mail.summary.from, mail.summary.subject, mail.body
                ),
                Err(error) => eprintln!("{error}"),
            }
        } else if command.to_lowercase().starts_with("/identfileslist:")
            && let Ok(files) = std::fs::read_dir(path.trim_matches('"'))
        {
            for file in files.flatten() {
                if file
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("eml"))
                {
                    println!("{}", file.path().display());
                }
            }
        }
    }
}
