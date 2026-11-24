//! The portal client against a fake `org.freedesktop.portal.Desktop` on a private bus:
//!
//!     dbus-run-session -- cargo test -p kirie-capture --test fake_portal -- --ignored

use std::collections::HashMap;
use std::time::Duration;

use zbus::message::Header;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Variant, connection, interface};

const DESKTOP: &str = "/org/freedesktop/portal/desktop";

struct Portal {
    picture: std::path::PathBuf,
    answer: u32,
    /// Whether the portal writes the picture while answering, as a real one does.
    writes: bool,
}

#[interface(name = "org.freedesktop.portal.Screenshot")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the portal's D-Bus method, as the interface macro wants it"
)]
impl Portal {
    fn screenshot(
        &self,
        parent_window: String,
        options: HashMap<String, OwnedVariant>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> OwnedObjectPath {
        let _ = parent_window;
        let interactive = options
            .get("interactive")
            .and_then(|value| bool::try_from(value.clone()).ok());
        assert_eq!(interactive, Some(false), "Kirie captures without a dialog");
        let token = options
            .get("handle_token")
            .and_then(|value| String::try_from(value.clone()).ok())
            .expect("a handle token");
        let sender = header
            .sender()
            .map(|name| name.trim_start_matches(':').replace('.', "_"))
            .expect("a sender");
        let path = format!("{DESKTOP}/request/{sender}/{token}");
        let connection = connection.clone();
        let (answer, picture, writes) = (self.answer, self.picture.clone(), self.writes);
        let uri = format!("file://{}", picture.to_string_lossy().replace(' ', "%20"));
        let reply = path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if writes {
                image::RgbaImage::new(64, 32)
                    .save(&picture)
                    .expect("a picture");
            }
            let results = HashMap::from([("uri", Variant::from(uri))]);
            connection
                .emit_signal(
                    None::<&str>,
                    reply.as_str(),
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(answer, results),
                )
                .await
                .expect("the response");
        });
        OwnedObjectPath::try_from(path).expect("a request path")
    }
}

async fn serve(portal: Portal) -> Connection {
    connection::Builder::session()
        .name("org.freedesktop.portal.Desktop")
        .serve_at(DESKTOP, portal)
        .build()
        .await
        .expect("the fake portal; run inside dbus-run-session")
}

#[tokio::test]
#[ignore = "needs a private bus: dbus-run-session -- cargo test --test fake_portal -- --ignored"]
async fn fake_portal_screenshot_and_cancel() {
    let dir = std::env::temp_dir().join(format!("kirie-fake-portal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let picture = dir.join("shot one.png");

    let portal = serve(Portal {
        picture: picture.clone(),
        answer: 0,
        writes: true,
    })
    .await;
    let image = tokio::time::timeout(
        Duration::from_secs(5),
        kirie_capture::portal::screenshot(false),
    )
    .await
    .expect("an answer in time")
    .expect("a screenshot");
    assert_eq!(image.dimensions(), (64, 32));
    assert!(!picture.exists(), "the portal's file is cleaned up");
    drop(portal);

    let mine = dir.join("mine.png");
    image::RgbaImage::new(8, 8).save(&mine).expect("a picture");
    std::fs::File::options()
        .write(true)
        .open(&mine)
        .and_then(|file| file.set_modified(std::time::SystemTime::now() - Duration::from_secs(60)))
        .expect("an older picture");
    let portal = serve(Portal {
        picture: mine.clone(),
        answer: 0,
        writes: false,
    })
    .await;
    let image = kirie_capture::portal::screenshot(false)
        .await
        .expect("a screenshot");
    assert_eq!(image.dimensions(), (8, 8));
    assert!(mine.exists(), "a file the portal did not just make is kept");
    drop(portal);

    let _portal = serve(Portal {
        picture: dir.join("none.png"),
        answer: 1,
        writes: false,
    })
    .await;
    let canceled = kirie_capture::portal::screenshot(false).await;
    assert!(matches!(
        canceled,
        Err(kirie_capture::CaptureError::Canceled)
    ));
    let _ = std::fs::remove_dir_all(dir);
}
