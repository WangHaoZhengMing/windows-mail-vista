use super::*;
use crate::html::InlineImage;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use windows::Win32::System::{
    Com::{
        DISPATCH_METHOD,
        Urlmon::{
            IInternetHostSecurityManager, URLACTION_ACTIVEX_RUN, URLACTION_SCRIPT_RUN,
            URLPOLICY_DISALLOW,
        },
    },
    Variant::VARIANT,
};

struct Parent(HWND);
impl Parent {
    fn new() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("MSHTML test").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                800,
                600,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null_mut()),
                null_mut(),
            )
        };
        assert!(!hwnd.is_null());
        Self(hwnd)
    }
}
impl Drop for Parent {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

#[track_caller]
fn pump_until(renderer: &MshtmlRenderer, phase: &str, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        assert!(
            !matches!(renderer.status(), HtmlStatus::Failed(_)),
            "{:?}",
            renderer.status()
        );
        if predicate() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "MSHTML did not finish {phase}: {:?}",
            renderer.status()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn element(renderer: &MshtmlRenderer, id: &str) -> IDispatch {
    let document = renderer.state.document.borrow().clone().unwrap();
    let value = dispatch::invoke(
        &document,
        "getElementById",
        DISPATCH_METHOD,
        &mut [VARIANT::from(id)],
    )
    .unwrap();
    IDispatch::try_from(&value).unwrap()
}

fn pixel_image() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 320, 200);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // A varying image keeps the data URI above IE8's 32 KiB limit.
        let mut seed = 0x1357_2468u32;
        let pixels: Vec<u8> = (0..320 * 200 * 4)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
    }
    bytes
}

struct ImageServer {
    url: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl ImageServer {
    fn new(image: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/image.png", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = requests.clone();
        let thread_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    // Winsock inherits the listener's nonblocking mode. Restore
                    // blocking I/O so a temporarily empty TCP buffer waits for
                    // the remaining headers instead of closing the connection.
                    stream.set_nonblocking(false).unwrap();
                    stream.set_nodelay(true).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    // TCP may split a request at any byte. Closing with unread
                    // request headers can reset the connection and discard the
                    // image response, even though the DOM is already Ready.
                    let mut request = Vec::new();
                    let mut buffer = [0; 4096];
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n")
                        && request.len() < 65536
                    {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(count) => request.extend_from_slice(&buffer[..count]),
                        }
                    }
                    if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        thread_requests.fetch_add(1, Ordering::SeqCst);
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
                            image.len()
                        );
                        let mut response = header.into_bytes();
                        response.extend_from_slice(&image);
                        let _ = stream.write_all(&response);
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for ImageServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn image_fixture_waits_for_complete_fragmented_http_headers() {
    let image = pixel_image();
    let server = ImageServer::new(image.clone());
    let address = server
        .url
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    stream
        .write_all(b"GET /image.png HTTP/1.1\r\nHost: localhost")
        .unwrap();
    let mut byte = [0];
    let error = stream
        .read(&mut byte)
        .expect_err("The fixture must wait for the request headers before responding");
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    stream.write_all(b"\r\nConnection: close\r\n\r\n").unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let body_start = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert_eq!(&response[body_start..], image);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[test]
fn real_mshtml_renders_dom_css_and_cid_and_only_loads_remote_images_on_request() {
    let parent = Parent::new();
    let renderer = unsafe { MshtmlRenderer::new(parent.0, parent.0, WM_APP + 95) }
        .expect("MSHTML is required; do not silently substitute another renderer");
    unsafe {
        MoveWindow(renderer.hwnd(), 0, 0, 700, 500, 0);
    }
    let image = pixel_image();
    assert!(
        image.len() > 100 * 1024,
        "Regression image must exceed legacy data-URI limits"
    );
    let server = ImageServer::new(image.clone());
    let html = format!(
        "<style>@import url('{}?stylesheet'); @font-face {{ font-family:remote;src:url('{}?font') }} #cell {{ color:rgb(12,34,56); font-weight:bold; }} </style><table id=table><tr><td id=cell>你好 MSHTML</td></tr></table><img id=inline src=cid:picture><img id=remote src='{}'><script src='{}?script'>document.title='executed'</script><object classid='clsid:00000000-0000-0000-0000-000000000000'></object>",
        server.url, server.url, server.url, server.url
    );
    let mut document = HtmlDocument {
        html,
        inline_images: vec![InlineImage {
            content_id: "picture".into(),
            content_type: "image/png".into(),
            bytes: image,
        }],
        remote_images: RemoteImagePolicy::Block,
    };
    renderer.render(document.clone()).unwrap();
    pump_until(&renderer, "blocked document Ready", || {
        renderer.status() == HtmlStatus::Ready
    });
    assert!(renderer.has_blocked_images());
    assert_eq!(
        dispatch::text(&element(&renderer, "cell"), "innerText").unwrap(),
        "你好 MSHTML"
    );
    let style = dispatch::object(&element(&renderer, "cell"), "currentStyle").unwrap();
    assert_eq!(dispatch::text(&style, "fontWeight").unwrap(), "700");
    let rows = dispatch::object(&element(&renderer, "table"), "rows").unwrap();
    assert_eq!(
        i32::try_from(&dispatch::get(&rows, "length").unwrap()).unwrap(),
        1
    );
    pump_until(&renderer, "CID naturalWidth = 320", || {
        i32::try_from(&dispatch::get(&element(&renderer, "inline"), "naturalWidth").unwrap())
            .unwrap_or(0)
            == 320
    });
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "Block must not issue a tracking request"
    );
    assert!(
        !dispatch::text(
            &dispatch::object(&renderer.state.document.borrow().clone().unwrap(), "body").unwrap(),
            "innerHTML"
        )
        .unwrap()
        .to_ascii_lowercase()
        .contains("<script")
    );

    document.remote_images = RemoteImagePolicy::Allow;
    renderer.render(document).unwrap();
    pump_until(&renderer, "allowed document Ready", || {
        renderer.status() == HtmlStatus::Ready
    });
    pump_until(&renderer, "remote naturalWidth = 320", || {
        i32::try_from(&dispatch::get(&element(&renderer, "remote"), "naturalWidth").unwrap())
            .unwrap_or(0)
            == 320
    });
    assert!(!renderer.has_blocked_images());
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        1,
        "Allow may only download the requested image"
    );

    // Rendering another message replaces both the DOM and its image permission.
    renderer
        .render(HtmlDocument {
            html: "<p id=replaced>Second message</p>".into(),
            ..Default::default()
        })
        .unwrap();
    pump_until(&renderer, "replacement document Ready", || {
        renderer.status() == HtmlStatus::Ready
    });
    assert_eq!(
        dispatch::text(&element(&renderer, "replaced"), "innerText").unwrap(),
        "Second message"
    );
    renderer
        .render(HtmlDocument {
            html: "<p id=old>Obsolete document</p>".into(),
            ..Default::default()
        })
        .unwrap();
    renderer
        .render(HtmlDocument {
            html: "<p id=current>Current document</p>".into(),
            ..Default::default()
        })
        .unwrap();
    pump_until(&renderer, "latest document Ready", || {
        renderer.status() == HtmlStatus::Ready
    });
    assert_eq!(
        dispatch::text(&element(&renderer, "current"), "innerText").unwrap(),
        "Current document"
    );
    renderer.render(HtmlDocument::default()).unwrap();
    pump_until(&renderer, "empty document Ready", || {
        renderer.status() == HtmlStatus::Ready
    });
    let body =
        dispatch::object(&renderer.state.document.borrow().clone().unwrap(), "body").unwrap();
    assert!(
        dispatch::text(&body, "innerText")
            .unwrap()
            .trim()
            .is_empty()
    );
    let child = renderer.hwnd();
    drop(renderer);
    assert_eq!(unsafe { IsWindow(child) }, 0);

    // Closing during parsing must release the host and prevent queued timers
    // from retaining a document or notifying a destroyed reading window.
    let pending = unsafe { MshtmlRenderer::new(parent.0, parent.0, WM_APP + 97) }.unwrap();
    pending
        .render(HtmlDocument {
            html: "<table><tr><td>Closing</td></tr></table>".repeat(1000),
            ..Default::default()
        })
        .unwrap();
    let child = pending.hwnd();
    drop(pending);
    assert_eq!(unsafe { IsWindow(child) }, 0);
}

#[test]
fn mshtml_host_rejects_script_and_activex_even_without_the_sanitizer() {
    let parent = Parent::new();
    let renderer = unsafe { MshtmlRenderer::new(parent.0, parent.0, WM_APP + 96) }.unwrap();
    let site = renderer.state.site.borrow().clone().unwrap();
    let security: IInternetHostSecurityManager = site.cast().unwrap();
    for action in [URLACTION_SCRIPT_RUN, URLACTION_ACTIVEX_RUN] {
        let mut policy = [0u8; 4];
        unsafe {
            security
                .ProcessUrlAction(action, &mut policy, None, 0, 0)
                .unwrap();
        }
        assert_eq!(u32::from_ne_bytes(policy), URLPOLICY_DISALLOW);
    }
    // Bypass sanitization only in this regression test, proving the host policy
    // is a second defense rather than relying on removal of script tags.
    let raw = b"<html><head><title>safe</title><script>document.title='executed';</script></head><body>Content</body></html>";
    for remote_images in [RemoteImagePolicy::Block, RemoteImagePolicy::Allow] {
        renderer
            .render(HtmlDocument {
                remote_images,
                ..Default::default()
            })
            .unwrap();
        renderer.state.set_status(HtmlStatus::Loading);
        let ole = renderer.state.ole.borrow().clone().unwrap();
        unsafe {
            let stream = SHCreateMemStream(Some(raw)).unwrap();
            ole.cast::<IPersistStreamInit>()
                .unwrap()
                .Load(&stream)
                .unwrap();
            SetTimer(renderer.hwnd(), POLL, 20, None);
        }
        pump_until(&renderer, "unsanitized host policy Ready", || {
            renderer.status() == HtmlStatus::Ready
        });
        let doc = renderer.state.document.borrow().clone().unwrap();
        assert_eq!(dispatch::text(&doc, "title").unwrap(), "safe");
    }
}
