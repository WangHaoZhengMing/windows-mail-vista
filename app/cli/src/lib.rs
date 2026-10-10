//! WinMail command-line entry point and operations over the shared mail engine.
use mail_core::{
    Result,
    message::{self, Draft},
    store::{Store, default_root, read_bounded},
};
use std::path::Path;

#[cfg(windows)]
mod windows;

pub const VERSION: &str = concat!("WinMail ", env!("CARGO_PKG_VERSION"), " (Rust)");

#[cfg(windows)]
pub fn main() -> i32 {
    windows::main()
}

#[cfg(not(windows))]
pub fn main() -> i32 {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let first = args
        .first()
        .and_then(|argument| argument.to_str())
        .unwrap_or("");
    if matches!(first, "--help" | "-h") {
        println!(
            "WinMail command-line application\n\
            winmail-rust --version / --check\n\
            winmail-rust --mail-list / --mail-import PATH / --mail-show ID\n\
            winmail-rust --mail-export ID PATH / --mail-receive\n\
            winmail-rust --mail-draft JSON / --mail-send JSON\n\
            WINMAIL_DATA_DIR selects the data store.\n\
            Platform storage and credential services are not implemented yet."
        );
        return 0;
    }
    match execute(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub fn execute(args: &[std::ffi::OsString]) -> Result<i32> {
    let cmd = args.first().and_then(|s| s.to_str()).unwrap_or("");
    match cmd {
        "--version" | "--runtime-version" => {
            println!("{}", crate::VERSION);
            return Ok(0);
        }
        "--check" | "--runtime-check" => {
            println!(
                "{}\nMail engine: Rust\nMIME/store: Rust\nTransport: SMTP / IMAP / POP3, certificate-validated TLS\nStore: {}\nLegacy Microsoft DLL required: no",
                crate::VERSION,
                default_root().display()
            );
            return Ok(0);
        }
        _ => (),
    }
    let mut store = Store::open(&default_root())?;
    let required = |i: usize| {
        args.get(i)
            .ok_or_else(|| "Missing command argument".to_owned())
    };
    match cmd {
        "--mail-list" => {
            for item in store.entries() {
                println!(
                    "{}\t{}\t{}\t{}",
                    item.id, item.folder, item.summary.from, item.summary.subject
                );
            }
        }
        "--mail-import" => {
            let count = store.import_tree(Path::new(required(1)?))?;
            println!("Imported {count} EML/NWS files");
        }
        "--mail-export" => {
            let id = required(1)?.to_str().ok_or("Invalid id")?;
            let path = Path::new(required(2)?);
            std::fs::write(path, store.raw(id)?).map_err(|e| e.to_string())?;
        }
        "--mail-show" => {
            let parsed = message::parse(&store.raw(required(1)?.to_str().ok_or("Invalid id")?)?)?;
            println!(
                "From: {}\nTo: {}\nSubject: {}\nDate: {}\n\n{}",
                parsed.summary.from,
                parsed.summary.to,
                parsed.summary.subject,
                parsed.summary.date,
                parsed.body
            );
            for a in parsed.attachments {
                println!("Attachment: {} ({} bytes)", a.name, a.bytes.len());
            }
        }
        "--mail-receive" => {
            let account_id = store
                .default_account_id()
                .ok_or("Configure an account in the Account window first")?
                .clone();
            let mut account = store
                .account_record(&account_id)
                .ok_or("Default account not found")?
                .account
                .clone();
            if account.incoming == mail_core::account::Incoming::Imap {
                let mut count = 0;
                for round in 0..50 {
                    refresh_credentials(&mut store, &account_id, &mut account)?;
                    let report = mail_core::network::sync_imap(
                        &account,
                        &store.imap_sync_request(&account_id)?,
                    )?;
                    let summary = store.apply_imap_sync(&account_id, report)?;
                    count += summary.downloaded;
                    for warning in &summary.warnings {
                        eprintln!("{warning}");
                    }
                    println!(
                        "Synchronized {count} messages; {} pending server operations",
                        summary.pending
                    );
                    if !summary.has_more || summary.downloaded == 0 || !summary.warnings.is_empty()
                    {
                        break;
                    }
                    if round == 49 {
                        println!("More messages remain; run --mail-receive again to continue.");
                    }
                }
            } else {
                refresh_credentials(&mut store, &account_id, &mut account)?;
                let downloads =
                    mail_core::network::receive(&account, &store.known_remote_for(&account_id))?;
                let count = downloads.len();
                for item in downloads {
                    store.import_for(Some(&account_id), &item.raw, "Inbox", Some(item.key))?;
                }
                println!("Received {count} messages; server copies retained");
            }
        }
        "--mail-draft" | "--mail-send" => {
            let draft: Draft = serde_json::from_slice(&read_bounded(Path::new(required(1)?))?)
                .map_err(|e| e.to_string())?;
            if cmd == "--mail-send" {
                let account_id = store
                    .default_account_id()
                    .ok_or("Configure an account in the Account window first")?
                    .clone();
                let mut account = store
                    .account_record(&account_id)
                    .ok_or("Default account not found")?
                    .account
                    .clone();
                let raw = draft.build(&account.email)?;
                let id = store.import_for(Some(&account_id), &raw, "Outbox", None)?;
                refresh_credentials(&mut store, &account_id, &mut account)?;
                mail_core::network::smtp_send(&account, &draft, &raw)?;
                store.move_to(&id, "Sent")?;
                println!("SMTP server accepted message {id}");
            } else {
                let sender = store
                    .account()
                    .map(|a| a.email.clone())
                    .unwrap_or_else(|| "draft@winmail-rust.local".into());
                let id = store.save_draft(&sender, &draft)?;
                println!("Saved draft {id}");
            }
        }
        _ => return Err("Unknown mail command".into()),
    }
    Ok(0)
}

fn refresh_credentials(
    store: &mut Store,
    id: &mail_core::account::AccountId,
    account: &mut mail_core::account::Account,
) -> Result<()> {
    if matches!(account.auth, mail_core::account::Authentication::OAuth2(_)) {
        mail_core::oauth::access_token(account)?;
        store.update_account(id, account.clone())?;
    }
    Ok(())
}
