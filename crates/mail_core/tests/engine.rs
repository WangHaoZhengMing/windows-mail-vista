use mail_core::{
    message::{self, Draft},
    store::Store,
};
use std::path::PathBuf;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("winmail-engine-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn mime_unicode_binary_and_envelope_only_bcc_roundtrip() {
    let tmp = Temp::new();
    let path = tmp.0.join("中文 附件.bin");
    let bytes = [0, 255, 128, 13, 10, 42];
    std::fs::write(&path, bytes).unwrap();
    let draft = Draft {
        to: "Recipient <to@example.com>".into(),
        cc: "cc@example.com".into(),
        bcc: "secret@example.com".into(),
        subject: "测试邮件主题很长，确认编码分行仍能还原。".repeat(3),
        body: "中文正文\n.second line".into(),
        attachments: vec![path],
    };
    let raw = draft.build("sender@example.com").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains("secret@example.com"));
    assert!(!text.contains("Bcc:"));
    assert!(
        draft
            .recipients()
            .unwrap()
            .contains(&"secret@example.com".into())
    );
    let parsed = message::parse(&raw).unwrap();
    assert_eq!(parsed.summary.subject, draft.subject);
    assert_eq!(parsed.body, draft.body);
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.attachments[0].bytes, bytes);
    assert!(parsed.attachments[0].name.contains("中文"));
}

#[test]
fn mailto_decoding_and_header_injection_rejection() {
    let draft = Draft::mailto("mailto:name+tag@example.com?cc=cc%40example.com&bcc=secret%40example.com&subject=%E4%BD%A0%E5%A5%BD&body=first%0D%0Asecond").unwrap();
    assert_eq!(draft.to, "name+tag@example.com");
    assert_eq!(draft.subject, "你好");
    assert_eq!(draft.body, "first\r\nsecond");
    assert_eq!(draft.recipients().unwrap().len(), 3);
    let mut poisoned = draft;
    poisoned.subject = "Hi\r\nBcc: victim@example.com".into();
    assert!(poisoned.build("sender@example.com").is_err());
    assert!(message::addresses("a@example.com\r\nRCPT TO:<b@example.com>").is_err());
}

#[test]
fn store_reopen_dedup_draft_and_exclusive_lock() {
    let tmp = Temp::new();
    let draft = Draft {
        to: "to@example.com".into(),
        bcc: "hidden@example.com".into(),
        body: "persist me".into(),
        ..Draft::default()
    };
    let raw = draft.build("from@example.com").unwrap();
    let id;
    let draft_id;
    {
        let mut store = Store::open(&tmp.0).unwrap();
        #[cfg(windows)]
        assert!(Store::open(&tmp.0).is_err());
        id = store
            .import(&raw, "Inbox", Some("server:uid1".into()))
            .unwrap();
        assert_eq!(
            id,
            store
                .import(&raw, "Inbox", Some("server:uid2".into()))
                .unwrap()
        );
        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.known_remote().len(), 2);
        store.mark_read(&id).unwrap();
        store.move_to(&id, "Trash").unwrap();
        draft_id = store.save_draft("from@example.com", &draft).unwrap();
        assert!(store.raw("../index.json").is_err());
    }
    let store = Store::open(&tmp.0).unwrap();
    assert_eq!(store.raw(&id).unwrap(), raw);
    assert!(
        store
            .entries()
            .iter()
            .any(|e| e.id == id && e.read && e.folder == "Trash")
    );
    assert_eq!(store.draft(&draft_id).unwrap().bcc, "hidden@example.com");
    assert_eq!(store.known_remote().len(), 2);
}

#[test]
fn interrupted_import_is_recovered_and_corrupt_index_is_reported() {
    let tmp = Temp::new();
    let raw = b"From: from@example.com\r\nSubject: recovered\r\n\r\nbody";
    let id = message::hash(raw);
    std::fs::create_dir_all(tmp.0.join("messages")).unwrap();
    std::fs::write(tmp.0.join("messages").join(format!("{id}.eml")), raw).unwrap();
    {
        let store = Store::open(&tmp.0).unwrap();
        assert_eq!(store.entries()[0].folder, "Recovered");
    }
    std::fs::write(tmp.0.join("index.json"), b"invalid-json").unwrap();
    assert!(Store::open(&tmp.0).is_err());
    assert_eq!(
        std::fs::read(tmp.0.join("index.json")).unwrap(),
        b"invalid-json"
    );
}

#[test]
fn incomplete_drafts_survive_source_attachment_removal() {
    let tmp = Temp::new();
    let attachment = tmp.0.join("source.bin");
    std::fs::write(&attachment, b"retain attachment").unwrap();
    let draft = Draft {
        bcc: "hidden@example.com".into(),
        attachments: vec![attachment.clone()],
        ..Draft::default()
    };
    assert!(draft.build("from@example.com").is_err());
    let id = {
        let mut store = Store::open(&tmp.0.join("store")).unwrap();
        store.save_draft("from@example.com", &draft).unwrap()
    };
    std::fs::remove_file(attachment).unwrap();
    let store = Store::open(&tmp.0.join("store")).unwrap();
    let mut reopened = store.draft(&id).unwrap().clone();
    assert_eq!(
        std::fs::read(&reopened.attachments[0]).unwrap(),
        b"retain attachment"
    );
    reopened.to = "to@example.com".into();
    let parsed = message::parse(&reopened.build("from@example.com").unwrap()).unwrap();
    assert_eq!(parsed.attachments[0].bytes, b"retain attachment");
}
