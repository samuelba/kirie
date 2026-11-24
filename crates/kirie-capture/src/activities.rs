//! KDE Plasma's current activity: windows kept to other activities are hidden.

use std::time::Duration;

use zbus::proxy;

/// How long the activity manager may take; without an answer, every activity counts.
const ANSWER: Duration = Duration::from_secs(1);

#[proxy(
    interface = "org.kde.ActivityManager.Activities",
    default_service = "org.kde.ActivityManager",
    default_path = "/ActivityManager/Activities"
)]
trait Activities {
    fn current_activity(&self) -> zbus::Result<String>;
}

/// The id of the current activity, if Plasma's activity manager answers in time.
pub async fn current() -> Option<String> {
    let asking = async {
        let connection = crate::bus::session().await?;
        ActivitiesProxy::new(&connection)
            .await?
            .current_activity()
            .await
    };
    match tokio::time::timeout(ANSWER, asking).await {
        Ok(Ok(activity)) => Some(activity),
        Ok(Err(error)) => {
            tracing::debug!(%error, "cannot tell the current activity");
            None
        }
        Err(_) => {
            tracing::debug!("the activity manager did not answer in time");
            None
        }
    }
}
